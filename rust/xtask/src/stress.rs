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
    #[command(flatten)]
    shared: Shared,
}

pub fn stress(a: &StressArgs) -> bool {
    if !a.shared.build() {
        return false;
    }
    let dir = root().join("target").join("stress");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("target/stress");
    let log = |name: &str| File::create(dir.join(name)).expect("log file");

    let mut server_cmd = Command::new(a.shared.bin("fb_server"));
    server_cmd
        .args(a.shared.server_args())
        .args([
            "--intro",
            "2",
            "--metrics-every",
            "5",
            "--exit-after",
            &(a.secs + 4).to_string(),
        ])
        .arg("--trace")
        .arg(dir.join("server.trace"))
        .stdout(log("server.log"))
        .stderr(log("server.err.log"));
    let Ok(mut server) = server_cmd.spawn() else {
        eprintln!("cannot start the server");
        return false;
    };
    std::thread::sleep(Duration::from_millis(800));
    let mut clients: Vec<Child> = Vec::new();
    for i in 0..a.clients {
        let mut c = Command::new(a.shared.bin("fb_client"));
        c.args([
            "--headless",
            "--id",
            &(1000 + i).to_string(),
            "--transport",
            &a.transport,
        ])
        .args(["--exit-after", &a.secs.to_string()])
        .args(a.shared.net_args())
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
        "stress: {} clients for {} s over {} at lag {} ms jitter {} ms loss {}; logs in {}",
        a.clients,
        a.secs,
        a.transport,
        a.shared.lag,
        a.shared.jitter,
        a.shared.loss,
        dir.display()
    );
    let deadline = Instant::now() + Duration::from_secs(a.secs + 30);
    for c in clients.iter_mut().chain(std::iter::once(&mut server)) {
        wait_or_kill(c, deadline);
    }
    report(a, &dir)
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

fn parse_trace(path: &Path, tag: &str) -> Vec<(u32, u32, Row)> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut it = l.split(' ');
            if it.next()? != tag {
                return None;
            }
            let v: Vec<&str> = it.collect();
            if v.len() != 8 {
                return None;
            }
            let i = |k: usize| v[k].parse::<i32>().ok();
            let f = |k: usize| v[k].parse::<f64>().ok();
            Some((
                v[0].parse().ok()?,
                v[1].parse().ok()?,
                Row {
                    input: (i(2)?, i(3)?, i(4)?),
                    pos: [f(5)?, f(6)?, f(7)?],
                },
            ))
        })
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
    server: &HashMap<(u32, u32), Row>,
    by_tick: &BTreeMap<u32, Vec<(u32, [f64; 3])>>,
    teleports: &BTreeSet<(u32, u32)>,
    client: &[(u32, u32, Row)],
) -> Divergence {
    let mut first: BTreeMap<u32, (u32, Row)> = BTreeMap::new();
    for &(tick, id, row) in client {
        first.entry(tick).or_insert((id, row));
    }
    let mut d = Divergence::default();
    let mut prev_bad = false;
    for (&tick, &(id, c)) in &first {
        let Some(s) = server.get(&(tick, id)) else { continue };
        d.ticks += 1;
        let late = c.input != s.input;
        if late {
            d.late += 1;
        }
        let bad = dist(&c.pos, &s.pos) > 1e-3;
        if bad && !prev_bad {
            if (tick.saturating_sub(1)..=tick + 1).any(|t| teleports.contains(&(id, t))) {
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

/// Another bean within reach (two bean widths) during the last 400 ms: the client draws, and pushes
/// against, the others that far in the past (interpolation delay plus half the RTT).
fn near_other(by_tick: &BTreeMap<u32, Vec<(u32, [f64; 3])>>, tick: u32, id: u32, pos: &[f64; 3]) -> bool {
    by_tick
        .range(tick.saturating_sub(48)..=tick)
        .flat_map(|(_, beans)| beans)
        .any(|(o, p)| *o != id && dist(p, pos) < 2.5)
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
    let mut by_id: BTreeMap<u32, Vec<(u32, Row)>> = BTreeMap::new();
    for &(tick, id, row) in &server_rows {
        by_id.entry(id).or_default().push((tick, row));
    }
    let mut teleports = BTreeSet::new();
    for (id, rows) in &by_id {
        for w in rows.windows(2) {
            // Within a few ticks: the tick that starts a new round is not traced.
            if w[1].0 <= w[0].0 + 3 && dist(&w[0].1.pos, &w[1].1.pos) > 2.0 {
                teleports.insert((*id, w[1].0));
            }
        }
    }
    let server: HashMap<(u32, u32), Row> = server_rows.iter().map(|&(t, id, r)| ((t, id), r)).collect();
    let mut by_tick: BTreeMap<u32, Vec<(u32, [f64; 3])>> = BTreeMap::new();
    for &(t, id, r) in &server_rows {
        by_tick.entry(t).or_default().push((id, r.pos));
    }

    println!("client  ticks   late   late%  respawn  late-runs  push  other  other/min  rollbacks  out B/s");
    for i in 0..a.clients {
        let rows = parse_trace(&dir.join(format!("client-{i}.trace")), "C");
        let log = read_log(dir, &format!("client-{i}"));
        // The message only: the target (`fb_client::stats:`) would match the keys too.
        let stats = log
            .lines()
            .rev()
            .find_map(|l| l.split_once(" stats: ").map(|(_, m)| m))
            .unwrap_or("");
        let d = compare(&server, &by_tick, &teleports, &rows);
        let minutes = d.ticks as f64 / 120.0 / 60.0;
        let late = d.late as f64 / d.ticks.max(1) as f64;
        let other = d.runs_other as f64 / minutes.max(1e-9);
        println!(
            "{i:>6} {:>6} {:>6} {:>6.3}% {:>8} {:>10} {:>5} {:>6} {:>10.1} {:>10} {:>8}",
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
    let cpu = worst(&|l| num_after(l, "cpu "));
    let mem = worst(&|l| num_after(l, "mem "));
    println!(
        "        tick p99 {p99:.0} µs, max {max:.0} µs | out {per_player:.0} B/s per player | cpu {cpu:.1}% | mem {mem:.0} MB"
    );
    if metrics.is_empty() {
        failures.push("server: no metric line with every client in".into());
    }
    if p99 > a.max_tick_us as f64 {
        failures.push(format!("server: tick p99 {p99:.0} µs (max {})", a.max_tick_us));
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
