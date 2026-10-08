//! `cargo xtask stress`: a server and N headless autopilot clients under a simulated network. Afterwards
//! each client's own-bean prediction is compared tick by tick with the server's trace, and the run fails
//! on late inputs, unexplained divergences, panics, map hash mismatches or tick cost (traffic is reported only).
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};

use crate::{Shared, reported, strip_ansi, target_dir};

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Transport {
    Udp,
    Ws,
    Auto,
}

impl Transport {
    /// As the client takes it (`--transport`).
    fn name(self) -> &'static str {
        match self {
            Transport::Udp => "udp",
            Transport::Ws => "ws",
            Transport::Auto => "auto",
        }
    }
}

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
    #[arg(long, value_enum, default_value_t = Transport::Udp)]
    transport: Transport,
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
    /// Runs the server as on a 1-vCPU host (Linux): alone on the last core, the clients on the
    /// others, and in a systemd scope with `MemoryMax=300M` and no swap.
    #[arg(long)]
    limit_server: bool,
    #[command(flatten)]
    shared: Shared,
    /// Extra flags for every client, e.g. `--client-arg=--sync-max-error=40`.
    #[arg(long, allow_hyphen_values = true)]
    client_arg: Vec<String>,
}

/// `fb_net::errors::MARK`: a failed system or command in a release build.
const ECS_ERROR: &str = "ECS ERROR";

pub fn stress(a: &StressArgs) -> Result<()> {
    a.shared.build()?;
    // The build leaves hundreds of MB of dirty pages: flushed in the middle of the run they block the
    // processes' log writes for seconds, and every connection times out.
    #[cfg(unix)]
    let _ = Command::new("sync").status();
    let dir = target_dir().join("stress");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let log = |name: &str| {
        let path = dir.join(name);
        File::create(&path).with_context(|| path.display().to_string())
    };
    let room_count = a.rooms.max(1);
    let rooms: Vec<String> = (1..=room_count).map(|i| format!("s{i}")).collect();
    let pin = match a.limit_server.then(server_cores) {
        None => None,
        Some(Some(p)) => Some(p),
        Some(None) => bail!("--limit-server: a local server on Linux with at least 2 cores only"),
    };
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
        None => a.shared.command("fb_server"),
    };
    server_cmd
        .args(a.shared.server_args())
        .args([
            "--open-rooms",
            &rooms.join(","),
            "--intro",
            "2",
            "--respawn",
            "--metrics-every",
            "5",
        ])
        .args(["--exit-after", &(a.secs + 4).to_string(), "--trace"])
        .arg(dir.join("server.trace"))
        .stdout(log("server.log")?)
        .stderr(log("server.err.log")?);
    let mut server = server_cmd.spawn().map_err(|_| anyhow!("cannot start the server"))?;
    std::thread::sleep(Duration::from_millis(800));
    // Gone already (a port taken by a server left running, bad flags): the clients would wait for
    // nothing for the whole run.
    if let Ok(Some(status)) = server.try_wait() {
        bail!(
            "stress: the server exited at once ({status}):\n{}",
            read_log(&dir, "server")
        );
    }
    let mut clients: Vec<Child> = Vec::new();
    for i in 0..a.clients {
        let room = &rooms[(i % room_count) as usize];
        let in_room = (0..a.clients).filter(|j| j % room_count == i % room_count);
        let mut c = match &pin {
            Some(p) => {
                let mut c = Command::new("taskset");
                c.args(["-c", &p.clients]).arg(a.shared.bin("fb_client"));
                c
            }
            None => a.shared.command("fb_client"),
        };
        c.args([
            "--headless",
            "--name",
            &format!("stress {i}"),
            "--transport",
            a.transport.name(),
        ])
        .args(a.shared.play_args(room, in_room.count() as u32))
        .args(["--exit-after", &a.secs.to_string()])
        .args(a.shared.net_args())
        .args(&a.client_arg)
        .arg("--trace")
        .arg(dir.join(format!("client-{i}.trace")))
        .stdout(log(&format!("client-{i}.log"))?)
        .stderr(log(&format!("client-{i}.err.log"))?);
        match c.spawn() {
            Ok(child) => clients.push(child),
            Err(e) => eprintln!("client {i}: {e}"),
        }
    }
    eprintln!(
        "stress: {} clients in {} rooms for {} s over {} at lag {} ms jitter {} ms loss {}; logs in {}",
        a.clients,
        rooms.len(),
        a.secs,
        a.transport.name(),
        a.shared.lag,
        a.shared.jitter,
        a.shared.loss,
        dir.display()
    );
    let deadline = Instant::now() + Duration::from_secs(a.secs + 30);
    for c in clients.iter_mut().chain([&mut server]) {
        wait_or_kill(c, deadline);
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

/// The child's exit status, or `None` once it had to be killed at the deadline.
pub fn wait_or_kill(c: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    while Instant::now() < deadline {
        if let Ok(Some(status)) = c.try_wait() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = c.kill();
    let _ = c.wait();
    None
}

/// A traced tick of one bean: input and feet position.
#[derive(Clone, Copy)]
struct Row {
    input: (i32, i32, i32),
    pos: [f64; 3],
}

/// A bean: its room (`Rooms`) and its player id there.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Bean {
    room: u32,
    id: u32,
}

/// The rooms of the traces, numbered as they are first seen.
#[derive(Default)]
struct Rooms(Vec<String>);

impl Rooms {
    fn index(&mut self, name: &str) -> u32 {
        let i = self.0.iter().position(|r| r == name).unwrap_or_else(|| {
            self.0.push(name.to_string());
            self.0.len() - 1
        });
        i as u32
    }
}

#[derive(Clone, Copy)]
struct Traced {
    tick: u32,
    bean: Bean,
    row: Row,
}

/// Lines `tag tick room id mx mz buttons x y z`.
fn parse_trace(path: &Path, tag: &str, rooms: &mut Rooms) -> Vec<Traced> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let fields: Vec<&str> = l.split(' ').collect();
            let [t, tick, room, id, mx, mz, buttons, x, y, z] = fields[..] else {
                return None;
            };
            if t != tag {
                return None;
            }
            let row = Row {
                input: (mx.parse().ok()?, mz.parse().ok()?, buttons.parse().ok()?),
                pos: [x.parse().ok()?, y.parse().ok()?, z.parse().ok()?],
            };
            let (tick, id) = (tick.parse().ok()?, id.parse().ok()?);
            Some(Traced {
                tick,
                bean: Bean {
                    room: rooms.index(room),
                    id,
                },
                row,
            })
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

/// The server's trace, indexed for the comparisons.
struct Server {
    rows: HashMap<(u32, Bean), Row>,
    by_tick: BTreeMap<u32, Vec<(Bean, [f64; 3])>>,
    teleports: BTreeSet<(Bean, u32)>,
}

impl Server {
    fn new(traced: &[Traced]) -> Server {
        let mut by_bean: BTreeMap<Bean, Vec<(u32, Row)>> = BTreeMap::new();
        for t in traced {
            by_bean.entry(t.bean).or_default().push((t.tick, t.row));
        }
        let mut teleports = BTreeSet::new();
        for (&bean, rows) in &by_bean {
            for w in rows.windows(2) {
                let [(t0, r0), (t1, r1)] = w else { continue };
                // Within a few ticks: the tick that starts a new round is not traced, and its gap is a move
                // to the new spawn however near (a respawn is a jump of more than 2 m).
                let gap = *t1 > t0 + 1;
                if *t1 <= t0 + 3 && (gap || dist(&r0.pos, &r1.pos) > 2.0) {
                    teleports.insert((bean, *t1));
                }
            }
        }
        let mut by_tick: BTreeMap<u32, Vec<(Bean, [f64; 3])>> = BTreeMap::new();
        for t in traced {
            by_tick.entry(t.tick).or_default().push((t.bean, t.row.pos));
        }
        Server {
            rows: traced.iter().map(|t| ((t.tick, t.bean), t.row)).collect(),
            by_tick,
            teleports,
        }
    }

    /// Another bean of the room within reach (two bean widths) during the last 400 ms: the client draws, and
    /// pushes against, the others that far in the past (interpolation delay plus half the RTT).
    fn near_other(&self, tick: u32, bean: Bean, pos: &[f64; 3]) -> bool {
        self.by_tick
            .range(tick.saturating_sub(48)..=tick)
            .flat_map(|(_, beans)| beans)
            .any(|(o, p)| o.room == bean.room && o.id != bean.id && dist(p, pos) < 2.5)
    }

    /// The first prediction of each tick (what the player saw) against the server's tick. Each run of
    /// mismatches is classified by its first tick: a respawn (the server teleported the bean), a late input,
    /// a push (another bean was within reach: the client sees it in the past) or something else.
    fn compare(&self, client: &[Traced], connects: &BTreeSet<u32>) -> Divergence {
        let mut first: BTreeMap<u32, (Bean, Row)> = BTreeMap::new();
        for t in client {
            first.entry(t.tick).or_insert((t.bean, t.row));
        }
        let mut d = Divergence::default();
        let mut prev_bad = false;
        let mut streaming = false;
        for (&tick, &(bean, c)) in &first {
            let Some(s) = self.rows.get(&(tick, bean)) else {
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
                if (tick.saturating_sub(1)..=tick + 1).any(|t| self.teleports.contains(&(bean, t))) {
                    d.runs_respawn += 1;
                } else if late {
                    d.runs_late += 1;
                } else if self.near_other(tick, bean, &s.pos) {
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
    let rest = line.get(line.find(key)? + key.len()..)?;
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest.get(..end)?.parse().ok()
}

/// The number right before `key` in `line`.
fn num_before(line: &str, key: &str) -> Option<f64> {
    let head = line.get(..line.find(key)?)?;
    let start = head
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |i| i + 1);
    head.get(start..)?.parse().ok()
}

/// A client's `stats:` line, the message only (the target, `fb_client::stats:`, would match the keys too).
struct ClientStats {
    rollbacks: Option<f64>,
    out: Option<f64>,
    frame_max: Option<f64>,
    /// The clock resyncs, as the line lists them.
    shifts: String,
}

impl ClientStats {
    fn parse(line: &str) -> Option<ClientStats> {
        let (_, m) = line.split_once(" stats: ")?;
        let shifts = m
            .split_once("shifts [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map_or("", |(s, _)| s);
        Some(ClientStats {
            rollbacks: num_after(m, "rollbacks "),
            out: num_after(m, "out "),
            frame_max: num_after(m, "frame max "),
            shifts: shifts.to_string(),
        })
    }
}

/// A server `metrics:` line (`fb_server::metrics`).
struct Metrics {
    players: Option<f64>,
    tick_p99: Option<f64>,
    tick_max: Option<f64>,
    frame_max: Option<f64>,
    input_missed: Option<f64>,
    out_per_player: Option<f64>,
    cpu: Option<f64>,
    mem: Option<f64>,
}

impl Metrics {
    fn parse(line: &str) -> Option<Metrics> {
        line.contains("metrics: ").then(|| Metrics {
            players: num_after(line, "players "),
            tick_p99: num_after(line, "p99 "),
            tick_max: num_after(line, "max "),
            frame_max: num_after(line, "frame max "),
            input_missed: num_after(line, "input missed "),
            out_per_player: num_before(line, " per player"),
            cpu: num_after(line, "cpu "),
            mem: num_after(line, "mem "),
        })
    }
}

/// The largest of the values there are, NaN without one.
fn worst(values: impl Iterator<Item = Option<f64>>) -> f64 {
    values.flatten().fold(f64::NAN, f64::max)
}

fn report(a: &StressArgs, dir: &Path) -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    let mut rooms = Rooms::default();
    let server = Server::new(&parse_trace(&dir.join("server.trace"), "S", &mut rooms));

    println!(
        "client  ticks   late   late%  respawn  late-runs  push  other  other/min  rollbacks  out B/s  frame-max  shifts"
    );
    for i in 0..a.clients {
        let rows = parse_trace(&dir.join(format!("client-{i}.trace")), "C", &mut rooms);
        let log = read_log(dir, &format!("client-{i}"));
        let stats: Vec<ClientStats> = log.lines().filter_map(ClientStats::parse).collect();
        let connects = connects(&dir.join(format!("client-{i}.trace")));
        let d = server.compare(&rows, &connects);
        // Stalls of this machine (the longest frame) and clock resyncs, the first (joining) one aside.
        let frame_max = stats.iter().filter_map(|s| s.frame_max).fold(0.0, f64::max);
        let shifts: Vec<&str> = stats
            .iter()
            .map(|s| s.shifts.as_str())
            .filter(|s| !s.is_empty())
            .skip(1)
            .collect();
        let last = stats.last();
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
            last.and_then(|s| s.rollbacks).unwrap_or(f64::NAN),
            last.and_then(|s| s.out).unwrap_or(f64::NAN),
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
        if log.contains(ECS_ERROR) {
            failures.push(format!(
                "client {i}: {ECS_ERROR} (a failed system or command; release builds log it)"
            ));
        }
    }

    let slog = read_log(dir, "server");
    if slog.contains("panicked") {
        failures.push("server panicked (see server.err.log)".into());
    }
    if slog.contains(ECS_ERROR) {
        failures.push(format!(
            "server: {ECS_ERROR} (a failed system or command; release builds log it)"
        ));
    }
    // Steady state: metric lines once everybody is in.
    let metrics: Vec<Metrics> = slog
        .lines()
        .filter_map(Metrics::parse)
        .filter(|m| m.players == Some(a.clients as f64))
        .collect();
    println!("server  (steady-state metric lines: {})", metrics.len());
    let p99 = worst(metrics.iter().map(|m| m.tick_p99));
    let max = worst(metrics.iter().map(|m| m.tick_max));
    let per_player = worst(metrics.iter().map(|m| m.out_per_player));
    let mem = worst(metrics.iter().map(|m| m.mem));
    // The first line also holds the start and the joins (the clients' clocks not synced yet); the last one the
    // clients that already quit at `--exit-after` (still players to the server, whose inputs stopped).
    let settled = metrics.get(1..metrics.len().saturating_sub(1)).unwrap_or_default();
    let cpu = worst(settled.iter().map(|m| m.cpu));
    let frame_max = worst(settled.iter().map(|m| m.frame_max));
    let missed: f64 = settled.iter().filter_map(|m| m.input_missed).sum();
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

    for f in &failures {
        println!("FAIL {f}");
    }
    if failures.is_empty() {
        println!("stress: ok");
    }
    reported(failures.is_empty())
}
