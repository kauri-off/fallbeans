//! `cargo xtask stress`: a server and N headless autopilot clients under a simulated network. Afterwards
//! each client's own-bean prediction is compared tick by tick with the server's trace, and the run fails
//! on late inputs, unexplained divergences, panics, map hash mismatches or tick cost (traffic is reported only).
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use clap::Args;

use crate::{Shared, root};

#[derive(Args)]
pub struct StressArgs {
    #[arg(long, default_value_t = 8)]
    clients: u32,
    /// Rooms the clients are spread over (round robin), each playing its own game.
    #[arg(long, default_value_t = 1)]
    rooms: u32,
    /// Seconds the clients play (a round of jump-club is 3 s intro + 75 s + 8 s results).
    #[arg(long, default_value_t = 60)]
    secs: u64,
    #[arg(long, value_parser = ["udp", "ws", "auto"], default_value = "udp")]
    transport: String,
    /// Fails above this share of a client's ticks predicted with an input the server did not have in time.
    #[arg(long, default_value_t = 0.002)]
    max_late: f64,
    /// Fails above this many divergences a minute per client not explained by a respawn or a late input
    /// (pushes between beans: each client sees the others in the past).
    #[arg(long, default_value_t = 30.0)]
    max_other: f64,
    /// Fails when the room tick's 99th percentile is above this, µs.
    #[arg(long, default_value_t = 1000)]
    max_tick_us: u64,
    /// Fails when the server process used more than this share of one core in a window, %.
    #[arg(long, default_value_t = 60.0)]
    max_cpu: f64,
    /// Fails when the server's resident memory went above this, MB.
    #[arg(long, default_value_t = 300.0)]
    max_mem: f64,
    /// Runs the local server as on the 1-vCPU host (Linux): alone on the last core, the clients on the
    /// others, and in a systemd scope with `MemoryMax=300M` and no swap.
    #[arg(long)]
    limit_server: bool,
    #[command(flatten)]
    shared: Shared,
    /// Against a probe server on a server host (`--host`, `--domain`; UDP 5890, wss …/fallbeans/ws-probe)
    /// instead of a local one: the network between here and there is the real one (the VPN matrix, port/plan.md §5).
    #[arg(long)]
    remote: bool,
    #[command(flatten)]
    host: crate::deploy::Host,
    /// Extra flags for every client, e.g. `--client-arg=--sync-max-error=40`.
    #[arg(long, allow_hyphen_values = true)]
    client_arg: Vec<String>,
    /// WSL distribution the Linux build of `--remote` runs in (Windows only).
    #[arg(long, default_value = "Ubuntu")]
    wsl_distro: String,
}

/// Where the probe server of `--remote` lives on the host (the deploy user's home: no root needed).
const PROBE_DIR: &str = "fb-probe";
/// Its ports: UDP open in ufw, WebSocket and HTTP behind nginx at /fallbeans/ws-probe and /fallbeans/probe/ (`deploy/`).
const PROBE_UDP: u16 = 5890;
const PROBE_WS: u16 = 5891;
const PROBE_HTTP: u16 = 5892;
/// Matches only the probe (a server started from PROBE_DIR), never the game's service.
const PROBE_MATCH: &str = "pgrep -u \"$(id -u)\" -f '^./fb_server --udp-port 5890'";

pub fn stress(a: &StressArgs) -> bool {
    if !a.shared.build() {
        return false;
    }
    // The build leaves hundreds of MB of dirty pages: flushed in the middle of the run they block the
    // processes' log writes for seconds, and every connection times out (port/phases/2.md).
    #[cfg(unix)]
    let _ = Command::new("sync").status();
    let dir = root().join("target").join("stress");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("target/stress");
    let log = |name: &str| File::create(dir.join(name)).expect("log file");
    let rooms: Vec<String> = (1..=a.rooms.max(1)).map(|i| format!("s{i}")).collect();
    let server_flags = |trace: &str| {
        let mut v = a.shared.server_args();
        let exit = (a.secs + 4).to_string();
        let open = rooms.join(",");
        v.extend(
            [
                "--open-rooms",
                &open,
                "--intro",
                "2",
                "--metrics-every",
                "5",
                "--exit-after",
                &exit,
                "--trace",
                trace,
            ]
            .map(String::from),
        );
        v
    };

    let pin = match a.limit_server.then(server_cores) {
        None => None,
        Some(Some(p)) if !a.remote => Some(p),
        Some(_) => {
            eprintln!("--limit-server: a local server on Linux with at least 2 cores only");
            return false;
        }
    };
    let mut server = None;
    let mut client_net: Vec<String> = Vec::new();
    if a.remote {
        if !a.host.complete() {
            return false;
        }
        let ip = a.host.public_ip().to_string();
        if !start_remote(a, &dir, &server_flags("server.trace"), &ip) {
            return false;
        }
        let domain = a.host.domain();
        let ws_url = format!("wss://{domain}/fallbeans/ws-probe");
        let http_url = format!("https://{domain}/fallbeans/probe");
        client_net.extend(["--server", &ip, "--ws-url", &ws_url, "--http-url", &http_url].map(String::from));
    } else {
        let mut server_cmd = match &pin {
            Some(p) => {
                let mut c = Command::new("systemd-run");
                c.args([
                    "--user",
                    "--scope",
                    "--quiet",
                    "-p",
                    "MemoryMax=300M",
                    "-p",
                    "MemorySwapMax=0",
                    "--",
                ])
                .args(["taskset", "-c", &p.server])
                .arg(a.shared.bin("fb_server"));
                c
            }
            None => Command::new(a.shared.bin("fb_server")),
        };
        server_cmd
            .args(server_flags(&dir.join("server.trace").to_string_lossy()))
            .stdout(log("server.log"))
            .stderr(log("server.err.log"));
        let Ok(child) = server_cmd.spawn() else {
            eprintln!("cannot start the server");
            return false;
        };
        server = Some(child);
        std::thread::sleep(Duration::from_millis(800));
    }
    let mut clients: Vec<Child> = Vec::new();
    for i in 0..a.clients {
        let room = &rooms[i as usize % rooms.len()];
        let in_room = (0..a.clients).filter(|j| *j as usize % rooms.len() == i as usize % rooms.len());
        let mut c = match &pin {
            Some(p) => {
                let mut c = Command::new("taskset");
                c.args(["-c", &p.clients]).arg(a.shared.bin("fb_client"));
                c
            }
            None => Command::new(a.shared.bin("fb_client")),
        };
        c.args([
            "--headless",
            "--name",
            &format!("stress {i}"),
            "--transport",
            &a.transport,
        ])
        .args(a.shared.play_args(room, in_room.count() as u32))
        .args(["--exit-after", &a.secs.to_string()])
        .args(&client_net)
        .args(a.shared.net_args())
        .args(&a.client_arg)
        .arg("--trace")
        .arg(dir.join(format!("client-{i}.trace")))
        .stdout(log(&format!("client-{i}.log")))
        .stderr(log(&format!("client-{i}.err.log")));
        match c.spawn() {
            Ok(child) => clients.push(child),
            Err(e) => eprintln!("client {i}: {e}"),
        }
    }
    eprintln!(
        "stress: {} clients in {} rooms for {} s over {} at lag {} ms jitter {} ms loss {}{}; logs in {}",
        a.clients,
        rooms.len(),
        a.secs,
        a.transport,
        a.shared.lag,
        a.shared.jitter,
        a.shared.loss,
        if a.remote { " against the host" } else { "" },
        dir.display()
    );
    let deadline = Instant::now() + Duration::from_secs(a.secs + 30);
    for c in clients.iter_mut().chain(server.as_mut()) {
        wait_or_kill(c, deadline);
    }
    if a.remote && !fetch_remote(a, &dir) {
        return false;
    }
    report(a, &dir)
}

struct Pin {
    server: String,
    clients: String,
}

/// The last core for the server, the rest for the clients.
fn server_cores() -> Option<Pin> {
    let n = std::thread::available_parallelism().ok()?.get();
    (cfg!(target_os = "linux") && n >= 2).then(|| Pin {
        server: (n - 1).to_string(),
        clients: format!("0-{}", n - 2),
    })
}

/// The first word of `sha256sum`'s output.
fn first_word(out: std::io::Result<std::process::Output>) -> String {
    out.ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split(' ')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// The probe server on the host: a fresh Linux build of this tree (local clients and the server must
/// speak the same protocol), uploaded when it differs from the one there, started in the background.
fn start_remote(a: &StressArgs, dir: &Path, flags: &[String], public_ip: &str) -> bool {
    let bin = dir.join("fb_server-linux");
    eprintln!("stress: Linux build of the server for the host");
    if !crate::deploy::build_linux_server(&a.wsl_distro, &bin) {
        return false;
    }
    let local = first_word(Command::new("sha256sum").arg(&bin).output());
    let there = first_word(
        a.host
            .ssh(&format!(
                "mkdir -p {PROBE_DIR} && (sha256sum {PROBE_DIR}/fb_server 2>/dev/null || true)"
            ))
            .output(),
    );
    if local.is_empty() || local != there {
        eprintln!(
            "stress: uploading the server ({} MB)",
            fs::metadata(&bin).map_or(0, |m| m.len() >> 20)
        );
        if !crate::run(&mut a.host.scp(&bin.to_string_lossy(), &format!(":{PROBE_DIR}/fb_server"))) {
            return false;
        }
    }
    let args: Vec<String> = flags.iter().map(|f| format!("'{f}'")).collect();
    // A probe left over from an interrupted run would hold the ports.
    let script = format!(
        "cd {PROBE_DIR} && chmod +x fb_server && ({kill} || true) && rm -f server.trace server.log \
         && (setsid nohup ./fb_server --udp-port {PROBE_UDP} --ws-addr 127.0.0.1 --ws-port {PROBE_WS} \
         --http-addr 127.0.0.1 --http-port {PROBE_HTTP} --public-host {public_ip} {} \
         > server.log 2>&1 < /dev/null &) && sleep 1 && {PROBE_MATCH} > /dev/null",
        args.join(" "),
        kill = PROBE_MATCH.replacen("pgrep", "pkill", 1),
    );
    if !crate::run(&mut a.host.ssh(&script)) {
        eprintln!("stress: the probe server did not start on the host");
        let _ = crate::run(&mut a.host.ssh(&format!("tail -20 {PROBE_DIR}/server.log")));
        return false;
    }
    true
}

/// Waits for the probe server to finish and brings its trace and log here.
fn fetch_remote(a: &StressArgs, dir: &Path) -> bool {
    let wait = format!(
        "for i in $(seq 1 40); do {PROBE_MATCH} > /dev/null || exit 0; sleep 1; done; {}; exit 0",
        PROBE_MATCH.replacen("pgrep", "pkill", 1)
    );
    let to = |f: &str| dir.join(f).to_string_lossy().into_owned();
    crate::run(&mut a.host.ssh(&wait))
        && crate::run(&mut a.host.scp(&format!(":{PROBE_DIR}/server.trace"), &to("server.trace")))
        && crate::run(&mut a.host.scp(&format!(":{PROBE_DIR}/server.log"), &to("server.log")))
}

fn wait_or_kill(c: &mut Child, deadline: Instant) {
    while Instant::now() < deadline {
        if let Ok(Some(_)) = c.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = c.kill();
    let _ = c.wait();
}

/// A traced tick of one bean: input and feet position.
#[derive(Clone, Copy)]
struct Row {
    input: (i32, i32, i32),
    pos: [f64; 3],
}

/// A bean: its room and its player id there.
type Bean = (String, u32);

/// Lines `tag tick room id mx mz buttons x y z`.
fn parse_trace(path: &Path, tag: &str) -> Vec<(u32, Bean, Row)> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut it = l.split(' ');
            if it.next()? != tag {
                return None;
            }
            let v: Vec<&str> = it.collect();
            if v.len() != 9 {
                return None;
            }
            let i = |k: usize| v[k].parse::<i32>().ok();
            let f = |k: usize| v[k].parse::<f64>().ok();
            Some((
                v[0].parse().ok()?,
                (v[1].to_string(), v[2].parse().ok()?),
                Row {
                    input: (i(3)?, i(4)?, i(5)?),
                    pos: [f(6)?, f(7)?, f(8)?],
                },
            ))
        })
        .collect()
}

/// Ticks where a client's connection (the first one, or one after a reconnect) starts its trace: `R tick`.
fn connects(path: &Path) -> BTreeSet<u32> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("R ")?.trim().parse().ok())
        .collect()
}

fn dist(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[derive(Default)]
struct Divergence {
    ticks: u64,
    late: u64,
    runs_respawn: u64,
    runs_late: u64,
    runs_push: u64,
    runs_other: u64,
    first_other: Vec<u32>,
}

/// The first prediction of each tick (what the player saw) against the server's tick. Each run of
/// mismatches is classified by its first tick: a respawn (the server teleported the bean), a late input,
/// a push (another bean was within reach: the client sees it in the past) or something else.
fn compare(
    server: &HashMap<(u32, Bean), Row>,
    by_tick: &BTreeMap<u32, Vec<(Bean, [f64; 3])>>,
    teleports: &BTreeSet<(Bean, u32)>,
    client: &[(u32, Bean, Row)],
    connects: &BTreeSet<u32>,
) -> Divergence {
    let mut first: BTreeMap<u32, (&Bean, Row)> = BTreeMap::new();
    for (tick, id, row) in client {
        first.entry(*tick).or_insert((id, *row));
    }
    let mut d = Divergence::default();
    let mut prev_bad = false;
    let mut streaming = false;
    for (&tick, &(id, c)) in &first {
        let Some(s) = server.get(&(tick, id.clone())) else {
            continue;
        };
        let late = c.input != s.input;
        // Just after joining (and after a reconnect) the server has nothing from the client yet (its pawn is
        // there before the client knows it is its own): compared from the first input the server got.
        if connects.contains(&tick) {
            streaming = false;
        }
        streaming |= !late;
        if !streaming {
            continue;
        }
        d.ticks += 1;
        if late {
            d.late += 1;
        }
        let bad = dist(&c.pos, &s.pos) > 1e-3;
        if bad && !prev_bad {
            if (tick.saturating_sub(1)..=tick + 1).any(|t| teleports.contains(&(id.clone(), t))) {
                d.runs_respawn += 1;
            } else if late {
                d.runs_late += 1;
            } else if near_other(by_tick, tick, id, &s.pos) {
                d.runs_push += 1;
            } else {
                d.runs_other += 1;
                if d.first_other.len() < 8 {
                    d.first_other.push(tick);
                }
            }
        }
        prev_bad = bad;
    }
    d
}

/// Another bean of the room within reach (two bean widths) during the last 400 ms: the client draws, and
/// pushes against, the others that far in the past (interpolation delay plus half the RTT).
fn near_other(by_tick: &BTreeMap<u32, Vec<(Bean, [f64; 3])>>, tick: u32, id: &Bean, pos: &[f64; 3]) -> bool {
    by_tick
        .range(tick.saturating_sub(48)..=tick)
        .flat_map(|(_, beans)| beans)
        .any(|(o, p)| o.0 == id.0 && o.1 != id.1 && dist(p, pos) < 2.5)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut esc = false;
    for ch in s.chars() {
        match (esc, ch) {
            (false, '\u{1b}') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => out.push(c),
        }
    }
    out
}

fn read_log(dir: &Path, name: &str) -> String {
    let mut s = String::new();
    for f in [format!("{name}.log"), format!("{name}.err.log")] {
        s += &strip_ansi(&fs::read_to_string(dir.join(f)).unwrap_or_default());
    }
    s
}

/// The number right after `key` in `line`.
fn num_after(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The number right before `key` in `line`.
fn num_before(line: &str, key: &str) -> Option<f64> {
    let head = &line[..line.find(key)?];
    let start = head
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |i| i + 1);
    head[start..].parse().ok()
}

fn report(a: &StressArgs, dir: &Path) -> bool {
    let mut failures: Vec<String> = Vec::new();
    let server_rows = parse_trace(&dir.join("server.trace"), "S");
    let mut by_id: BTreeMap<Bean, Vec<(u32, Row)>> = BTreeMap::new();
    for (tick, id, row) in &server_rows {
        by_id.entry(id.clone()).or_default().push((*tick, *row));
    }
    let mut teleports = BTreeSet::new();
    for (id, rows) in &by_id {
        for w in rows.windows(2) {
            // Within a few ticks: the tick that starts a new round is not traced, and its gap is a move
            // to the new spawn however near (a respawn is a jump of more than 2 m).
            let gap = w[1].0 > w[0].0 + 1;
            if w[1].0 <= w[0].0 + 3 && (gap || dist(&w[0].1.pos, &w[1].1.pos) > 2.0) {
                teleports.insert((id.clone(), w[1].0));
            }
        }
    }
    let server: HashMap<(u32, Bean), Row> = server_rows.iter().map(|(t, id, r)| ((*t, id.clone()), *r)).collect();
    let mut by_tick: BTreeMap<u32, Vec<(Bean, [f64; 3])>> = BTreeMap::new();
    for (t, id, r) in &server_rows {
        by_tick.entry(*t).or_default().push((id.clone(), r.pos));
    }

    println!(
        "client  ticks   late   late%  respawn  late-runs  push  other  other/min  rollbacks  out B/s  frame-max  shifts"
    );
    for i in 0..a.clients {
        let rows = parse_trace(&dir.join(format!("client-{i}.trace")), "C");
        let log = read_log(dir, &format!("client-{i}"));
        // The message only: the target (`fb_client::stats:`) would match the keys too.
        let stats = log
            .lines()
            .rev()
            .find_map(|l| l.split_once(" stats: ").map(|(_, m)| m))
            .unwrap_or("");
        let connects = connects(&dir.join(format!("client-{i}.trace")));
        let d = compare(&server, &by_tick, &teleports, &rows, &connects);
        // Stalls of this machine (the longest frame) and clock resyncs, the first (joining) one aside.
        let stat_lines: Vec<&str> = log
            .lines()
            .filter_map(|l| l.split_once(" stats: ").map(|(_, m)| m))
            .collect();
        let frame_max = stat_lines
            .iter()
            .filter_map(|l| num_after(l, "frame max "))
            .fold(0.0, f64::max);
        let shifts: Vec<&str> = stat_lines
            .iter()
            .filter_map(|l| {
                let rest = &l[l.find("shifts [")? + 8..];
                Some(&rest[..rest.find(']')?])
            })
            .filter(|s| !s.is_empty())
            .skip(1)
            .collect();
        let minutes = d.ticks as f64 / 120.0 / 60.0;
        let late = d.late as f64 / d.ticks.max(1) as f64;
        let other = d.runs_other as f64 / minutes.max(1e-9);
        println!(
            "{i:>6} {:>6} {:>6} {:>6.3}% {:>8} {:>10} {:>5} {:>6} {:>10.1} {:>10} {:>8} {:>8.0}ms  {}",
            d.ticks,
            d.late,
            late * 100.0,
            d.runs_respawn,
            d.runs_late,
            d.runs_push,
            d.runs_other,
            other,
            num_after(stats, "rollbacks ").unwrap_or(f64::NAN),
            num_after(stats, "out ").unwrap_or(f64::NAN),
            frame_max,
            shifts.join(" "),
        );
        if !d.first_other.is_empty() {
            println!("        unexplained divergences start at ticks {:?}", d.first_other);
        }
        if d.ticks < (a.secs * 120) / 2 {
            failures.push(format!(
                "client {i}: only {} ticks compared (did it connect and play?)",
                d.ticks
            ));
        }
        if late > a.max_late {
            failures.push(format!(
                "client {i}: {:.3}% of inputs late (max {:.3}%)",
                late * 100.0,
                a.max_late * 100.0
            ));
        }
        if other > a.max_other {
            failures.push(format!(
                "client {i}: {other:.1} unexplained divergences a minute (max {})",
                a.max_other
            ));
        }
        if log.contains("MAP HASH MISMATCH") {
            failures.push(format!("client {i}: map hash mismatch"));
        }
        if log.contains("panicked") {
            failures.push(format!("client {i}: panicked (see client-{i}.err.log)"));
        }
    }

    let slog = read_log(dir, "server");
    if slog.contains("panicked") {
        failures.push("server panicked (see server.err.log)".into());
    }
    // Steady state: metric lines once everybody is in.
    let metrics: Vec<&str> = slog
        .lines()
        .filter(|l| l.contains("metrics: ") && num_after(l, "players ") == Some(a.clients as f64))
        .collect();
    println!("server  (steady-state metric lines: {})", metrics.len());
    let worst = |f: &dyn Fn(&str) -> Option<f64>| metrics.iter().filter_map(|l| f(l)).fold(f64::NAN, f64::max);
    let p99 = worst(&|l| num_after(l, "p99 "));
    let max = worst(&|l| num_after(l, "max "));
    let per_player = worst(&|l| num_before(l, " per player"));
    let mem = worst(&|l| num_after(l, "mem "));
    // The first line also holds the start and the joins (the clients' clocks not synced yet).
    let settled = metrics.iter().skip(1);
    let cpu = settled
        .clone()
        .filter_map(|l| num_after(l, "cpu "))
        .fold(f64::NAN, f64::max);
    let frame_max = settled
        .clone()
        .filter_map(|l| num_after(l, "frame max "))
        .fold(f64::NAN, f64::max);
    let missed: f64 = settled.filter_map(|l| num_after(l, "input missed ")).sum();
    println!(
        "        tick p99 {p99:.0} µs, max {max:.0} µs | frame max {frame_max:.0} ms | input missed {missed:.0} ticks | out {per_player:.0} B/s per player | cpu {cpu:.1}% | mem {mem:.0} MB"
    );
    if metrics.is_empty() {
        failures.push("server: no metric line with every client in".into());
    }
    if p99 > a.max_tick_us as f64 {
        failures.push(format!("server: tick p99 {p99:.0} µs (max {})", a.max_tick_us));
    }
    if cpu > a.max_cpu {
        failures.push(format!("server: cpu {cpu:.1}% of a core (max {})", a.max_cpu));
    }
    if mem > a.max_mem {
        failures.push(format!("server: mem {mem:.0} MB (max {})", a.max_mem));
    }

    if failures.is_empty() {
        println!("stress: ok");
        true
    } else {
        for f in &failures {
            println!("FAIL {f}");
        }
        false
    }
}
