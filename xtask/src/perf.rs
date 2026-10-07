//! `cargo xtask perf`: a benchmark run of the windowed client, and the reports of its recordings (and F9 ones).
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use clap::{Args, Subcommand};
use serde_json::Value;

use crate::{Shared, target_dir};

#[derive(Args)]
pub struct PerfArgs {
    #[command(subcommand)]
    cmd: PerfCmd,
}

#[derive(Subcommand)]
enum PerfCmd {
    /// Server plus one windowed client in a round with bots: records the frames, prints the report.
    Run(RunArgs),
    /// The report of a recording (`perf-*.json` from the logs folder, or `target/perf-runs` of `run`).
    Show { file: PathBuf },
    /// What changed from one recording to another.
    Compare { base: PathBuf, new: PathBuf },
    /// A recording's frames as CSV (stdout, or a file).
    Csv { file: PathBuf, out: Option<PathBuf> },
}

/// `--secs`: what the client takes for `--perf-capture` (more than 0, at most an hour).
fn record_secs(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    (f32::MIN_POSITIVE..=3600.0)
        .contains(&v)
        .then_some(v)
        .ok_or_else(|| "more than 0, at most 3600".into())
}

/// `--warmup`: what the client takes for `--perf-warmup`.
fn warmup_secs(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    (0.0..=600.0)
        .contains(&v)
        .then_some(v)
        .ok_or_else(|| "between 0 and 600".into())
}

#[derive(Args)]
struct RunArgs {
    /// Seconds recorded.
    #[arg(long, default_value_t = 30.0, value_parser = record_secs)]
    secs: f32,
    /// Seconds of the round before the recording.
    #[arg(long, default_value_t = 8.0, value_parser = warmup_secs)]
    warmup: f32,
    /// The graphics features off one at a time instead (about a minute).
    #[arg(long)]
    sweep: bool,
    #[arg(long)]
    fullscreen: bool,
    /// The client's settings profile: its graphics settings are the ones measured.
    #[arg(long, default_value = "perf")]
    profile: String,
    /// Compares with this recording afterwards.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Builds with this cargo profile: `perf` (release code, rebuilt in seconds), `release` (thin LTO, what
    /// players run) or `dev`.
    #[arg(long, default_value = "perf")]
    cargo_profile: String,
    /// Extra flags for the client, e.g. `--client-arg=--backend=vulkan`.
    #[arg(long, allow_hyphen_values = true)]
    client_arg: Vec<String>,
    #[arg(long, default_value = "jump-club")]
    map: String,
    #[arg(long)]
    seed: Option<u32>,
}

pub fn perf(a: &PerfArgs) -> bool {
    match &a.cmd {
        PerfCmd::Run(r) => run(r),
        PerfCmd::Show { file } => load(file).map(|d| print!("{}", report(&d))).is_some(),
        PerfCmd::Compare { base, new } => match (load(base), load(new)) {
            (Some(b), Some(n)) => {
                print!("{}", compare(&b, &n));
                true
            }
            _ => false,
        },
        PerfCmd::Csv { file, out } => load(file).is_some_and(|d| {
            let csv = to_csv(&d);
            match out {
                Some(p) => std::fs::write(p, csv).is_ok(),
                None => {
                    print!("{csv}");
                    true
                }
            }
        }),
    }
}

fn load(path: &Path) -> Option<Value> {
    let r = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            None
        }
    }
}

fn run(r: &RunArgs) -> bool {
    let shared = Shared {
        lag: 0,
        jitter: 0,
        loss: 0.0,
        map: r.map.clone(),
        seed: r.seed,
        release: false,
    };
    let built = crate::run(crate::cargo().args([
        "build",
        "--profile",
        &r.cargo_profile,
        "-p",
        "fb_server",
        "-p",
        "fb_client",
    ]));
    if !built {
        return false;
    }
    let target = target_dir().join(match r.cargo_profile.as_str() {
        "dev" => "debug",
        p => p,
    });
    let bin = |name: &str| target.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let dir = target_dir().join("perf-runs");
    let _ = std::fs::create_dir_all(&dir);
    let secs = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();
    let kind = if r.sweep { "sweep" } else { "run" };
    let out = dir.join(format!("{}-{kind}-{secs}.json", r.map));
    let Ok(mut server) = Command::new(bin("fb_server"))
        .args(shared.server_args())
        .args(["--dev", "--solo", "--open-rooms", "perf"])
        .spawn()
    else {
        eprintln!("cannot start the server");
        return false;
    };
    std::thread::sleep(Duration::from_millis(500));
    if let Ok(Some(status)) = server.try_wait() {
        eprintln!("the server exited at once ({status})");
        return false;
    }
    let mut c = Command::new(bin("fb_client"));
    c.args([
        "--profile",
        &r.profile,
        "--name",
        "Бенч",
        "--title",
        "Fall Beans — perf",
    ])
    .args(shared.play_args("perf", 1))
    .args([
        "--fill",
        "--autopilot",
        "--no-update",
        // (The spans of systems: the recording's CPU rows.)
        "--profiler",
        "--perf-warmup",
        &r.warmup.to_string(),
    ])
    .arg("--perf-out")
    .arg(&out);
    if r.sweep {
        c.arg("--perf-sweep");
    } else {
        c.args(["--perf-capture", &r.secs.to_string()]);
    }
    if r.fullscreen {
        c.arg("--fullscreen");
    }
    c.args(&r.client_arg);
    eprintln!("$ {c:?}");
    let ok = match c.spawn() {
        Ok(mut client) => {
            // (Connecting, the lobby, the intro, the warmup, the recording: well under this.)
            let limit = Duration::from_secs_f32(r.warmup + r.secs.max(90.0) + 120.0);
            let started = Instant::now();
            loop {
                match client.try_wait() {
                    Ok(Some(s)) => break s.success(),
                    Ok(None) if started.elapsed() > limit => {
                        eprintln!("perf: the client did not finish in {limit:?}");
                        let _ = client.kill();
                        break false;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                    Err(_) => break false,
                }
            }
        }
        Err(e) => {
            eprintln!("cannot start the client: {e}");
            false
        }
    };
    let _ = server.kill();
    let _ = server.wait();
    let Some(doc) = ok.then(|| load(&out)).flatten() else {
        eprintln!("perf: no recording");
        return false;
    };
    print!("{}", report(&doc));
    if let Some(b) = r.baseline.as_deref().and_then(load) {
        print!("\n{}", compare(&b, &doc));
    }
    eprintln!("\nrecording: {}", out.display());
    true
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

fn f2(v: &Value) -> String {
    num(v).map_or("—".into(), |x| format!("{x:.2}"))
}

fn head(d: &Value) -> String {
    let a = &d["adapter"];
    let g = &d["graphics"];
    let w = d["window"].as_array().map_or("—".into(), |w| {
        format!("{}×{}", w[0], w.get(1).unwrap_or(&Value::Null))
    });
    let on: Vec<&str> = ["shadows", "ao", "aa", "grade", "motes"]
        .into_iter()
        .filter(|k| g[*k].as_bool() == Some(true))
        .collect();
    format!(
        "{} {} | {} | {}\n{} ({}, {}) {} {}\n{w} | tier {} preset {} (setting {}) | {} | upscale {} | vsync {} limit {}\nround: {}\n",
        d["kind"].as_str().unwrap_or("?"),
        d["time"].as_str().unwrap_or(""),
        d["build"].as_str().unwrap_or(""),
        d["os"].as_str().unwrap_or(""),
        a["name"].as_str().unwrap_or("?"),
        a["backend"].as_str().unwrap_or("?"),
        a["type"].as_str().unwrap_or("?"),
        a["driver"].as_str().unwrap_or(""),
        a["driver_info"].as_str().unwrap_or(""),
        d["tier"].as_str().unwrap_or("?"),
        d["preset"].as_str().unwrap_or("?"),
        g["preset"].as_str().unwrap_or("?"),
        on.join(" "),
        g["upscale"].as_str().filter(|s| !s.is_empty()).unwrap_or("off"),
        g["vsync"],
        g["fps_limit"],
        d["round"]["map"].as_str().unwrap_or("—"),
    )
}

fn stats_table(s: &Value) -> String {
    let mut t = format!(
        "{:.1} s, {} frames: {:.1} fps, 1% low {:.1} fps, {} stutters | {}\n",
        num(&s["secs"]).unwrap_or(0.0),
        s["frames"],
        num(&s["fps"]).unwrap_or(0.0),
        num(&s["fps_low1"]).unwrap_or(0.0),
        s["stutters"],
        s["bound"].as_str().unwrap_or("—"),
    );
    t += "ms         avg     p50     p95     p99     max\n";
    for k in ["frame", "gpu", "main", "render", "wait", "sleep"] {
        let v = &s[k];
        if v.is_null() {
            continue;
        }
        let _ = writeln!(
            t,
            "{k:<8}{:>7}{:>8}{:>8}{:>8}{:>8}",
            f2(&v["avg"]),
            f2(&v["p50"]),
            f2(&v["p95"]),
            f2(&v["p99"]),
            f2(&v["max"])
        );
    }
    t
}

fn passes_table(passes: &Value, pixels: f64) -> String {
    let mut t = String::from("GPU passes (no shadows)     GPU ms  CPU ms     tris  frag/px\n");
    for p in passes.as_array().into_iter().flatten().take(16) {
        let frags = num(&p["frags"]).unwrap_or(0.0);
        let _ = writeln!(
            t,
            "  {:<26}{:>6}{:>8}{:>9}{:>9}",
            p["name"].as_str().unwrap_or("?"),
            f2(&p["gpu"]),
            f2(&p["cpu"]),
            big(num(&p["tris"]).unwrap_or(0.0)),
            if frags > 0.0 {
                format!("{:.2}", frags / pixels)
            } else {
                "—".into()
            }
        );
    }
    t
}

/// `1.2M`, `34.5k`.
fn big(n: f64) -> String {
    match n {
        n if n >= 1e6 => format!("{:.2}M", n / 1e6),
        n if n >= 1e3 => format!("{:.1}k", n / 1e3),
        n => format!("{n:.0}"),
    }
}

fn pixels(d: &Value) -> f64 {
    d["window"]
        .as_array()
        .and_then(|w| Some(w.first()?.as_f64()? * w.get(1)?.as_f64()?))
        .unwrap_or(1.0)
        .max(1.0)
}

fn report(d: &Value) -> String {
    let mut t = head(d);
    if let Some(steps) = d["steps"].as_array() {
        t += "\nsweep (p50, ms; Δ from the first step)\nstep                 fps    frame      Δ      gpu      Δ     main   render\n";
        let base = steps.first().map(|s| &s["summary"]);
        for s in steps {
            let m = &s["summary"];
            let delta = |k: &str| match (num(&m[k]["p50"]), base.and_then(|b| num(&b[k]["p50"]))) {
                (Some(x), Some(b)) => format!("{:+.2}", x - b),
                _ => "—".into(),
            };
            let _ = writeln!(
                t,
                "{:<18}{:>7.1}{:>9}{:>7}{:>9}{:>7}{:>9}{:>9}",
                s["name"].as_str().unwrap_or("?"),
                num(&m["fps"]).unwrap_or(0.0),
                f2(&m["frame"]["p50"]),
                delta("frame"),
                f2(&m["gpu"]["p50"]),
                delta("gpu"),
                f2(&m["main"]["p50"]),
                f2(&m["render"]["p50"]),
            );
        }
        if let Some(first) = steps.first() {
            t += "\n";
            t += &passes_table(&first["passes"], pixels(d));
        }
    } else {
        t += &segments_table(d);
        t += "\n";
        t += &stats_table(&d["summary"]);
        t += "\n";
        t += &passes_table(&d["passes"], pixels(d));
        let cpu = d["cpu"].as_array().map(Vec::as_slice).unwrap_or_default();
        if !cpu.is_empty() {
            t += "\nCPU (profiler), ms a frame\n";
            for r in cpu.iter().filter(|r| r["kind"] != "system").take(12) {
                let _ = writeln!(
                    t,
                    "  {:>6}  {} {}",
                    f2(&r["ms"]),
                    r["kind"].as_str().unwrap_or(""),
                    r["name"].as_str().unwrap_or("")
                );
            }
            t += "systems:\n";
            for r in cpu.iter().filter(|r| r["kind"] == "system").take(20) {
                let _ = writeln!(
                    t,
                    "  {:>6} ×{:<5.1} {}",
                    f2(&r["ms"]),
                    num(&r["calls"]).unwrap_or(0.0),
                    r["name"].as_str().unwrap_or("")
                );
            }
        }
    }
    let s = &d["scene"];
    if !s.is_null() {
        let _ = writeln!(
            t,
            "\nscene: entities {} | meshes {} visible {} (shadow {}) | tris {} visible {} | lights {} | materials {} | textures {} ({} MB) | mesh buffers {} MB | GPU alloc {} MB | RAM {} MB",
            s["entities"],
            s["meshes"],
            s["visible"],
            s["casters"],
            s["tris"],
            s["tris_visible"],
            s["lights"],
            s["materials"],
            s["textures"],
            f2(&s["texture_mb"]),
            f2(&s["mesh_mb"]),
            f2(&s["gpu_mb"]),
            f2(&s["ram_mb"]),
        );
    }
    let spikes = d["spikes"].as_array().map(Vec::as_slice).unwrap_or_default();
    if !spikes.is_empty() {
        // (A recording keeps the first few hundred; `spike_count` is all of them.)
        let all = d["spike_count"].as_u64().unwrap_or(spikes.len() as u64);
        let _ = writeln!(t, "\nlong frames ({all}):");
        for sp in spikes.iter().take(8) {
            let top: Vec<String> = sp["top"]
                .as_array()
                .into_iter()
                .flatten()
                .take(5)
                .map(|p| format!("{} {}", p[0].as_str().unwrap_or("?"), f2(&p[1])))
                .collect();
            let _ = writeln!(
                t,
                "  {} ms at {} s ({}): {}",
                f2(&sp["ms"]),
                f2(&sp["t"]),
                scene_at(d, num(&sp["t"]).unwrap_or(0.0)),
                top.join(" · ")
            );
        }
    }
    t
}

fn count(v: &Value) -> String {
    num(v).map_or("—".into(), |x| format!("{x:.0}"))
}

fn segments(d: &Value) -> &[Value] {
    d["segments"].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The scene a recording was in at `t` (its segments).
fn scene_at(d: &Value, t: f64) -> &str {
    segments(d)
        .iter()
        .rev()
        .find(|s| num(&s["t"]).is_some_and(|s0| s0 <= t))
        .and_then(|s| s["scene"].as_str())
        .unwrap_or("?")
}

/// Each scene of a recording: frame times, the scene's load and its costliest systems.
fn segments_table(d: &Value) -> String {
    let segs = segments(d);
    if segs.is_empty() {
        return String::new();
    }
    let mut t = String::from(
        "\nscenes (p50, ms)       from     secs     fps   frame   p99     gpu    main  render    wait  meshes visible shadow\n",
    );
    for s in segs {
        let m = &s["summary"];
        let c = &s["counts"];
        let _ = writeln!(
            t,
            "{:<20}{:>7.1}{:>9.1}{:>8.0}{:>8}{:>6}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>7}",
            s["scene"].as_str().unwrap_or("?"),
            num(&s["t"]).unwrap_or(0.0),
            num(&m["secs"]).unwrap_or(0.0),
            num(&m["fps"]).unwrap_or(0.0),
            f2(&m["frame"]["p50"]),
            num(&m["frame"]["p99"]).map_or("—".into(), |x| format!("{x:.1}")),
            f2(&m["gpu"]["p50"]),
            f2(&m["main"]["p50"]),
            f2(&m["render"]["p50"]),
            f2(&m["wait"]["p50"]),
            count(&c["meshes"]),
            count(&c["visible"]),
            count(&c["casters"]),
        );
    }
    if segs.len() > 1 {
        for s in segs {
            let systems: Vec<String> = s["cpu"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["kind"] == "system")
                .take(8)
                .map(|r| format!("{} {}", r["name"].as_str().unwrap_or("?"), f2(&r["ms"])))
                .collect();
            let _ = writeln!(t, "  {}: {}", s["scene"].as_str().unwrap_or("?"), systems.join(" · "));
        }
    }
    t
}

fn pct(base: f64, new: f64) -> String {
    if base.abs() < 1e-9 {
        return String::new();
    }
    format!("{:+.1}%", (new - base) / base * 100.0)
}

fn compare_row(t: &mut String, name: &str, b: Option<f64>, n: Option<f64>) {
    match (b, n) {
        (Some(b), Some(n)) => {
            let _ = writeln!(t, "{name:<18}{b:>9.2}{n:>9.2}{:>+9.2}{:>9}", n - b, pct(b, n));
        }
        _ => {
            let f = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.2}"));
            let _ = writeln!(t, "{name:<18}{:>9}{:>9}", f(b), f(n));
        }
    }
}

fn compare(b: &Value, n: &Value) -> String {
    let first = |d: &Value| head(d).lines().next().unwrap_or("").to_string();
    let mut t = format!(
        "base: {}\nnew:  {}\n{:<18}{:>9}{:>9}{:>9}\n",
        first(b),
        first(n),
        "",
        "base",
        "new",
        "Δ"
    );
    if let (Some(bs), Some(ns)) = (b["steps"].as_array(), n["steps"].as_array()) {
        for s in ns {
            let name = s["name"].as_str().unwrap_or("?");
            let o = bs.iter().find(|x| x["name"] == s["name"]);
            let get = |v: Option<&Value>, k: &str| v.and_then(|v| num(&v["summary"][k]["p50"]));
            compare_row(&mut t, &format!("{name} frame"), get(o, "frame"), get(Some(s), "frame"));
            compare_row(&mut t, &format!("{name} gpu"), get(o, "gpu"), get(Some(s), "gpu"));
        }
        return t;
    }
    let (bs, ns) = (&b["summary"], &n["summary"]);
    compare_row(&mut t, "fps", num(&bs["fps"]), num(&ns["fps"]));
    compare_row(&mut t, "1% low fps", num(&bs["fps_low1"]), num(&ns["fps_low1"]));
    compare_row(&mut t, "stutters", num(&bs["stutters"]), num(&ns["stutters"]));
    for k in ["frame", "gpu", "main", "render", "wait"] {
        for q in ["avg", "p50", "p99"] {
            compare_row(&mut t, &format!("{k} {q}"), num(&bs[k][q]), num(&ns[k][q]));
        }
    }
    t += "\nGPU passes, ms\n";
    let names = |d: &Value, key: &str| -> Vec<String> {
        d[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["name"].as_str().map(String::from))
            .collect()
    };
    let mut all = names(n, "passes");
    for x in names(b, "passes") {
        if !all.contains(&x) {
            all.push(x);
        }
    }
    let find = |d: &Value, key: &str, name: &str, field: &str| {
        d[key]
            .as_array()
            .and_then(|v| v.iter().find(|p| p["name"] == name))
            .and_then(|p| num(&p[field]))
    };
    for name in &all {
        compare_row(
            &mut t,
            name,
            find(b, "passes", name, "gpu"),
            find(n, "passes", name, "gpu"),
        );
    }
    // (By kind and name: a system and its commands, or a schedule, can have one name.)
    let rows = |d: &Value| -> Vec<(String, String)> {
        d["cpu"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| Some((r["kind"].as_str()?.to_string(), r["name"].as_str()?.to_string())))
            .collect()
    };
    let mut keys = rows(n);
    for k in rows(b) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let ms_of = |d: &Value, (kind, name): &(String, String)| {
        d["cpu"]
            .as_array()
            .and_then(|v| {
                v.iter()
                    .find(|r| r["kind"] == kind.as_str() && r["name"] == name.as_str())
            })
            .and_then(|r| num(&r["ms"]))
    };
    let mut deltas: Vec<(String, Option<f64>, Option<f64>)> = keys
        .iter()
        .map(|k| {
            let label = if k.0 == "system" {
                k.1.clone()
            } else {
                format!("{} ({})", k.1, k.0)
            };
            (label, ms_of(b, k), ms_of(n, k))
        })
        .collect();
    deltas.sort_by(|a, b| {
        let d = |x: &(String, Option<f64>, Option<f64>)| (x.2.unwrap_or(0.0) - x.1.unwrap_or(0.0)).abs();
        d(b).total_cmp(&d(a))
    });
    if !deltas.is_empty() {
        t += "\nCPU (profiler), the largest changes, ms a frame\n";
        for (s, x, y) in deltas.iter().take(15) {
            compare_row(&mut t, s, *x, *y);
        }
    }
    t
}

fn to_csv(d: &Value) -> String {
    let cols = ["t", "frame", "sleep", "main", "render", "wait", "gpu"];
    let f = &d["frames"];
    let n = f["t"].as_array().map_or(0, Vec::len);
    let mut s = cols.join(",") + "\n";
    for i in 0..n {
        let row: Vec<String> = cols
            .iter()
            .map(|c| num(&f[*c][i]).map_or(String::new(), |v| format!("{v:.3}")))
            .collect();
        s += &row.join(",");
        s.push('\n');
    }
    s
}
