//! `cargo xtask perf`: a benchmark run of the windowed client, and the reports of its recordings (and F9 ones).
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;

use crate::{Shared, dlss_features, target_dir, with_features};

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
    #[arg(long, value_enum, default_value_t = CargoProfile::Perf)]
    cargo_profile: CargoProfile,
    /// Extra flags for the client, e.g. `--client-arg=--backend=vulkan`.
    #[arg(long, allow_hyphen_values = true)]
    client_arg: Vec<String>,
    #[arg(long, default_value = "jump-club")]
    map: String,
    #[arg(long)]
    seed: Option<u32>,
    /// The bean stays at the start instead of playing (with `--seed`, the frames repeat between runs).
    #[arg(long)]
    still: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CargoProfile {
    Perf,
    Release,
    Dev,
}

impl CargoProfile {
    fn name(self) -> &'static str {
        match self {
            CargoProfile::Perf => "perf",
            CargoProfile::Release => "release",
            CargoProfile::Dev => "dev",
        }
    }

    /// Its directory under target/.
    fn dir(self) -> &'static str {
        match self {
            CargoProfile::Dev => "debug",
            p => p.name(),
        }
    }
}

pub fn perf(a: &PerfArgs) -> Result<()> {
    match &a.cmd {
        PerfCmd::Run(r) => run(r),
        PerfCmd::Show { file } => {
            print!("{}", report(&load(file)?));
            Ok(())
        }
        PerfCmd::Compare { base, new } => {
            let (b, n) = (load(base)?, load(new)?);
            print!("{}", compare(&b, &n));
            Ok(())
        }
        PerfCmd::Csv { file, out } => {
            let csv = to_csv(&load(file)?);
            match out {
                Some(p) => std::fs::write(p, csv).with_context(|| p.display().to_string()),
                None => {
                    print!("{csv}");
                    Ok(())
                }
            }
        }
    }
}

fn load(path: &Path) -> Result<Doc> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    serde_json::from_str(&text).with_context(|| path.display().to_string())
}

/// DXC beside the built client, as the installer puts it: without it DX12 compiles with FXC, which keeps four
/// threads busy (63 fps against 103). From `FB_DXC_DIR` or the Windows SDK's bin.
pub fn dxc_beside(target: &Path) {
    if !cfg!(windows) || target.join("dxcompiler.dll").is_file() {
        return;
    }
    let from_env = crate::sdk::var("FB_DXC_DIR").map(|d| d.join("bin").join("x64"));
    let sdk = std::fs::read_dir(r"C:\Program Files (x86)\Windows Kits\10\bin")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("x64"))
        .filter(|d| d.join("dxcompiler.dll").is_file() && d.join("dxil.dll").is_file())
        .max();
    let Some(src) = from_env
        .into_iter()
        .chain(sdk)
        .find(|d| d.join("dxcompiler.dll").is_file())
    else {
        eprintln!("perf: no DXC (FB_DXC_DIR or the Windows SDK): DX12 compiles shaders with FXC during the recording");
        return;
    };
    for f in ["dxcompiler.dll", "dxil.dll"] {
        if let Err(e) = std::fs::copy(src.join(f), target.join(f)) {
            eprintln!("perf: {f}: {e}");
        }
    }
}

fn run(r: &RunArgs) -> Result<()> {
    let shared = Shared {
        map: r.map.clone(),
        seed: r.seed,
        ..Shared::default()
    };
    let mut build = crate::cargo();
    build.args([
        "build",
        "--profile",
        r.cargo_profile.name(),
        "-p",
        "fb_server",
        "-p",
        "fb_client",
    ]);
    // (DLSS when its SDK is there, as in the packages: the recording says which upscaler ran.)
    with_features(&mut build, &dlss_features());
    crate::run(&mut build)?;
    let target = target_dir().join(r.cargo_profile.dir());
    let bin = |name: &str| target.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    dxc_beside(&target);
    let _ = crate::dist::upscalers_into(&target, None);
    let dir = target_dir().join("perf-runs");
    let _ = std::fs::create_dir_all(&dir);
    let secs = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();
    let kind = if r.sweep { "sweep" } else { "run" };
    let out = dir.join(format!("{}-{kind}-{secs}.json", r.map));
    let mut server = crate::start_server(Command::new(bin("fb_server")).args(shared.server_args()).args([
        "--dev",
        "--solo",
        "--open-rooms",
        "perf",
    ]))?;
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
        "--no-update",
        // (The spans of systems: the recording's CPU rows.)
        "--profiler",
        "--perf-warmup",
        &r.warmup.to_string(),
    ])
    .arg("--perf-out")
    .arg(&out);
    if !r.still {
        c.arg("--autopilot");
    }
    if r.sweep {
        c.arg("--perf-sweep");
    } else {
        c.args(["--perf-capture", &r.secs.to_string()]);
    }
    // (A window by default: the measurements compare at its size, whatever the profile's setting.)
    c.arg(if r.fullscreen { "--fullscreen" } else { "--windowed" });
    c.args(&r.client_arg);
    eprintln!("$ {c:?}");
    let recorded = c
        .spawn()
        .map_err(|e| anyhow!("cannot start the client: {e}"))
        .and_then(|mut client| {
            // (Connecting, the lobby, the intro, the warmup, the recording: well under this.)
            let limit = Duration::from_secs_f32(r.warmup + r.secs.max(90.0) + 120.0);
            let started = Instant::now();
            loop {
                match client.try_wait() {
                    Ok(Some(s)) => break crate::reported(s.success()),
                    Ok(None) if started.elapsed() > limit => {
                        let _ = client.kill();
                        break Err(anyhow!("perf: the client did not finish in {limit:?}"));
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                    Err(_) => break crate::reported(false),
                }
            }
        })
        .and_then(|()| load(&out));
    let _ = server.kill();
    let _ = server.wait();
    let doc = match recorded {
        Ok(doc) => doc,
        Err(e) => {
            crate::report(Err(e));
            bail!("perf: no recording");
        }
    };
    print!("{}", report(&doc));
    if let Some(base) = &r.baseline {
        match load(base) {
            Ok(b) => print!("\n{}", compare(&b, &doc)),
            Err(e) => eprintln!("{e:#}"),
        }
    }
    eprintln!("\nrecording: {}", out.display());
    Ok(())
}

/// A recording, as the client writes it (`fb_client::perf::capture`); what an older one lacks is empty.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Doc {
    kind: Option<String>,
    build: Option<String>,
    time: Option<String>,
    os: Option<String>,
    adapter: Option<Adapter>,
    tier: Option<String>,
    preset: Option<String>,
    graphics: Graphics,
    window: Option<[u32; 2]>,
    round: Option<Round>,
    scene: Option<Scene>,
    spikes: Vec<Spike>,
    spike_count: Option<u64>,
    /// A sweep's; the rest is a run's.
    steps: Option<Vec<Step>>,
    summary: Summary,
    passes: Vec<Pass>,
    cpu: Vec<CpuRow>,
    frames: Frames,
    segments: Vec<Segment>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Adapter {
    name: Option<String>,
    backend: Option<String>,
    #[serde(rename = "type")]
    device_type: Option<String>,
    driver: Option<String>,
    driver_info: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Graphics {
    preset: Option<String>,
    shadows: bool,
    ao: bool,
    aa: bool,
    grade: bool,
    motes: bool,
    upscale: Option<String>,
    vsync: bool,
    fps_limit: u32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Round {
    map: Option<String>,
}

/// `fb_client::perf::scene::Scene`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Scene {
    entities: u64,
    meshes: u64,
    visible: u64,
    casters: u64,
    tris: u64,
    tris_visible: u64,
    lights: u64,
    materials: u64,
    textures: u64,
    texture_mb: Option<f64>,
    mesh_mb: Option<f64>,
    gpu_mb: Option<f64>,
    ram_mb: Option<f64>,
}

/// `fb_client::perf::stats::Stat`, ms.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct Stat {
    avg: Option<f64>,
    p50: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
    max: Option<f64>,
}

/// `fb_client::perf::stats::Summary` (a NaN or infinite value is written as null).
#[derive(Deserialize, Default)]
#[serde(default)]
struct Summary {
    frames: u64,
    secs: Option<f64>,
    fps: Option<f64>,
    fps_low1: Option<f64>,
    stutters: u64,
    frame: Option<Stat>,
    sleep: Option<Stat>,
    main: Option<Stat>,
    render: Option<Stat>,
    wait: Option<Stat>,
    gpu: Option<Stat>,
    bound: Option<String>,
}

impl Summary {
    fn stats(&self) -> [(&'static str, Option<Stat>); 6] {
        [
            ("frame", self.frame),
            ("gpu", self.gpu),
            ("main", self.main),
            ("render", self.render),
            ("wait", self.wait),
            ("sleep", self.sleep),
        ]
    }
}

fn p50(s: Option<Stat>) -> Option<f64> {
    s.and_then(|s| s.p50)
}

/// `fb_client::perf::gpu::Pass`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Pass {
    name: Option<String>,
    gpu: Option<f64>,
    cpu: Option<f64>,
    tris: Option<f64>,
    frags: Option<f64>,
}

/// `fb_client::perf::Row`: a span's ms and runs a frame.
#[derive(Deserialize, Default)]
#[serde(default)]
struct CpuRow {
    kind: Option<String>,
    name: Option<String>,
    ms: Option<f64>,
    calls: Option<f64>,
}

impl CpuRow {
    fn is_system(&self) -> bool {
        self.kind.as_deref() == Some("system")
    }
}

/// `fb_client::perf::Spike`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Spike {
    t: Option<f64>,
    ms: Option<f64>,
    top: Vec<(String, Option<f64>)>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Step {
    name: Option<String>,
    summary: Summary,
    passes: Vec<Pass>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Segment {
    scene: Option<String>,
    t: Option<f64>,
    summary: Summary,
    cpu: Vec<CpuRow>,
    counts: Option<Scene>,
}

/// Every frame, by column.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Frames {
    t: Vec<Option<f64>>,
    frame: Vec<Option<f64>>,
    sleep: Vec<Option<f64>>,
    main: Vec<Option<f64>>,
    render: Vec<Option<f64>>,
    wait: Vec<Option<f64>>,
    gpu: Vec<Option<f64>>,
}

fn f2(v: Option<f64>) -> String {
    v.map_or("—".into(), |x| format!("{x:.2}"))
}

fn head(d: &Doc) -> String {
    let none = Adapter::default();
    let a = d.adapter.as_ref().unwrap_or(&none);
    let g = &d.graphics;
    let or = |v: &Option<String>, default: &'static str| v.as_deref().unwrap_or(default).to_string();
    let w = d.window.map_or("—".into(), |[w, h]| format!("{w}×{h}"));
    let on: Vec<&str> = [
        ("shadows", g.shadows),
        ("ao", g.ao),
        ("aa", g.aa),
        ("grade", g.grade),
        ("motes", g.motes),
    ]
    .into_iter()
    .filter_map(|(k, on)| on.then_some(k))
    .collect();
    format!(
        "{} {} | {} | {}\n{} ({}, {}) {} {}\n{w} | tier {} preset {} (setting {}) | {} | upscale {} | vsync {} limit {}\nround: {}\n",
        or(&d.kind, "?"),
        or(&d.time, ""),
        or(&d.build, ""),
        or(&d.os, ""),
        or(&a.name, "?"),
        or(&a.backend, "?"),
        or(&a.device_type, "?"),
        or(&a.driver, ""),
        or(&a.driver_info, ""),
        or(&d.tier, "?"),
        or(&d.preset, "?"),
        or(&g.preset, "?"),
        on.join(" "),
        g.upscale.as_deref().filter(|s| !s.is_empty()).unwrap_or("off"),
        g.vsync,
        g.fps_limit,
        d.round.as_ref().and_then(|r| r.map.as_deref()).unwrap_or("—"),
    )
}

fn stats_table(s: &Summary) -> String {
    let mut t = format!(
        "{:.1} s, {} frames: {:.1} fps, 1% low {:.1} fps, {} stutters | {}\n",
        s.secs.unwrap_or(0.0),
        s.frames,
        s.fps.unwrap_or(0.0),
        s.fps_low1.unwrap_or(0.0),
        s.stutters,
        s.bound.as_deref().unwrap_or("—"),
    );
    t += "ms         avg     p50     p95     p99     max\n";
    for (k, v) in s.stats() {
        let Some(v) = v else {
            continue;
        };
        let _ = writeln!(
            t,
            "{k:<8}{:>7}{:>8}{:>8}{:>8}{:>8}",
            f2(v.avg),
            f2(v.p50),
            f2(v.p95),
            f2(v.p99),
            f2(v.max)
        );
    }
    t
}

fn passes_table(passes: &[Pass], pixels: f64) -> String {
    let mut t = String::from("GPU passes (no shadows)     GPU ms  CPU ms     tris  frag/px\n");
    for p in passes.iter().take(16) {
        let frags = p.frags.unwrap_or(0.0);
        let _ = writeln!(
            t,
            "  {:<26}{:>6}{:>8}{:>9}{:>9}",
            p.name.as_deref().unwrap_or("?"),
            f2(p.gpu),
            f2(p.cpu),
            big(p.tris.unwrap_or(0.0)),
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

fn pixels(d: &Doc) -> f64 {
    d.window.map_or(1.0, |[w, h]| f64::from(w) * f64::from(h)).max(1.0)
}

fn report(d: &Doc) -> String {
    let mut t = head(d);
    if let Some(steps) = &d.steps {
        t += "\nsweep (p50, ms; Δ from the first step)\nstep                 fps    frame      Δ      gpu      Δ     main   render\n";
        let base = steps.first().map(|s| &s.summary);
        for s in steps {
            let m = &s.summary;
            let delta = |stat: fn(&Summary) -> Option<Stat>| match (p50(stat(m)), base.and_then(|b| p50(stat(b)))) {
                (Some(x), Some(b)) => format!("{:+.2}", x - b),
                _ => "—".into(),
            };
            let _ = writeln!(
                t,
                "{:<18}{:>7.1}{:>9}{:>7}{:>9}{:>7}{:>9}{:>9}",
                s.name.as_deref().unwrap_or("?"),
                m.fps.unwrap_or(0.0),
                f2(p50(m.frame)),
                delta(|s| s.frame),
                f2(p50(m.gpu)),
                delta(|s| s.gpu),
                f2(p50(m.main)),
                f2(p50(m.render)),
            );
        }
        if let Some(first) = steps.first() {
            t += "\n";
            t += &passes_table(&first.passes, pixels(d));
        }
    } else {
        t += &segments_table(d);
        t += "\n";
        t += &stats_table(&d.summary);
        t += "\n";
        t += &passes_table(&d.passes, pixels(d));
        if !d.cpu.is_empty() {
            t += "\nCPU (profiler), ms a frame\n";
            for r in d.cpu.iter().filter(|r| !r.is_system()).take(12) {
                let _ = writeln!(
                    t,
                    "  {:>6}  {} {}",
                    f2(r.ms),
                    r.kind.as_deref().unwrap_or(""),
                    r.name.as_deref().unwrap_or("")
                );
            }
            t += "systems:\n";
            for r in d.cpu.iter().filter(|r| r.is_system()).take(20) {
                let _ = writeln!(
                    t,
                    "  {:>6} ×{:<5.1} {}",
                    f2(r.ms),
                    r.calls.unwrap_or(0.0),
                    r.name.as_deref().unwrap_or("")
                );
            }
        }
    }
    if let Some(s) = &d.scene {
        let _ = writeln!(
            t,
            "\nscene: entities {} | meshes {} visible {} (shadow {}) | tris {} visible {} | lights {} | materials {} | textures {} ({} MB) | mesh buffers {} MB | GPU alloc {} MB | RAM {} MB",
            s.entities,
            s.meshes,
            s.visible,
            s.casters,
            s.tris,
            s.tris_visible,
            s.lights,
            s.materials,
            s.textures,
            f2(s.texture_mb),
            f2(s.mesh_mb),
            f2(s.gpu_mb),
            f2(s.ram_mb),
        );
    }
    if !d.spikes.is_empty() {
        // (A recording keeps the first few hundred; `spike_count` is all of them.)
        let all = d.spike_count.unwrap_or(d.spikes.len() as u64);
        let _ = writeln!(t, "\nlong frames ({all}):");
        for sp in d.spikes.iter().take(8) {
            let top: Vec<String> = sp
                .top
                .iter()
                .take(5)
                .map(|(name, ms)| format!("{name} {}", f2(*ms)))
                .collect();
            let _ = writeln!(
                t,
                "  {} ms at {} s ({}): {}",
                f2(sp.ms),
                f2(sp.t),
                scene_at(d, sp.t.unwrap_or(0.0)),
                top.join(" · ")
            );
        }
    }
    t
}

/// The scene a recording was in at `t` (its segments).
fn scene_at(d: &Doc, t: f64) -> &str {
    d.segments
        .iter()
        .rev()
        .find(|s| s.t.is_some_and(|s0| s0 <= t))
        .and_then(|s| s.scene.as_deref())
        .unwrap_or("?")
}

/// Each scene of a recording: frame times, the scene's load and its costliest systems.
fn segments_table(d: &Doc) -> String {
    let segs = &d.segments;
    if segs.is_empty() {
        return String::new();
    }
    let mut t = String::from(
        "\nscenes (p50, ms)       from     secs     fps   frame   p99     gpu    main  render    wait  meshes visible shadow\n",
    );
    for s in segs {
        let m = &s.summary;
        let count = |n: fn(&Scene) -> u64| s.counts.as_ref().map_or("—".into(), |c| n(c).to_string());
        let _ = writeln!(
            t,
            "{:<20}{:>7.1}{:>9.1}{:>8.0}{:>8}{:>6}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>7}",
            s.scene.as_deref().unwrap_or("?"),
            s.t.unwrap_or(0.0),
            m.secs.unwrap_or(0.0),
            m.fps.unwrap_or(0.0),
            f2(p50(m.frame)),
            m.frame.and_then(|f| f.p99).map_or("—".into(), |x| format!("{x:.1}")),
            f2(p50(m.gpu)),
            f2(p50(m.main)),
            f2(p50(m.render)),
            f2(p50(m.wait)),
            count(|c| c.meshes),
            count(|c| c.visible),
            count(|c| c.casters),
        );
    }
    if segs.len() > 1 {
        for s in segs {
            let systems: Vec<String> = s
                .cpu
                .iter()
                .filter(|r| r.is_system())
                .take(8)
                .map(|r| format!("{} {}", r.name.as_deref().unwrap_or("?"), f2(r.ms)))
                .collect();
            let _ = writeln!(t, "  {}: {}", s.scene.as_deref().unwrap_or("?"), systems.join(" · "));
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
            let _ = writeln!(t, "{name:<18}{:>9}{:>9}", f2(b), f2(n));
        }
    }
}

/// `first`, then what of `more` is not in it.
fn union<T: PartialEq>(mut first: Vec<T>, more: Vec<T>) -> Vec<T> {
    for x in more {
        if !first.contains(&x) {
            first.push(x);
        }
    }
    first
}

fn pass_names(d: &Doc) -> Vec<&str> {
    d.passes.iter().filter_map(|p| p.name.as_deref()).collect()
}

/// By kind and name: a system and its commands, or a schedule, can have one name.
fn cpu_keys(d: &Doc) -> Vec<(&str, &str)> {
    d.cpu
        .iter()
        .filter_map(|r| Some((r.kind.as_deref()?, r.name.as_deref()?)))
        .collect()
}

/// One number of a stat (a quantile, the mean).
type StatOf = fn(&Stat) -> Option<f64>;

fn compare(b: &Doc, n: &Doc) -> String {
    let first = |d: &Doc| head(d).lines().next().unwrap_or("").to_string();
    let mut t = format!(
        "base: {}\nnew:  {}\n{:<18}{:>9}{:>9}{:>9}\n",
        first(b),
        first(n),
        "",
        "base",
        "new",
        "Δ"
    );
    if let (Some(bs), Some(ns)) = (&b.steps, &n.steps) {
        for s in ns {
            let name = s.name.as_deref().unwrap_or("?");
            let o = bs.iter().find(|x| x.name == s.name);
            let frame = |s: &Step| p50(s.summary.frame);
            let gpu = |s: &Step| p50(s.summary.gpu);
            compare_row(&mut t, &format!("{name} frame"), o.and_then(frame), frame(s));
            compare_row(&mut t, &format!("{name} gpu"), o.and_then(gpu), gpu(s));
        }
        return t;
    }
    let (bs, ns) = (&b.summary, &n.summary);
    compare_row(&mut t, "fps", bs.fps, ns.fps);
    compare_row(&mut t, "1% low fps", bs.fps_low1, ns.fps_low1);
    compare_row(&mut t, "stutters", Some(bs.stutters as f64), Some(ns.stutters as f64));
    let quantiles: [(&str, StatOf); 3] = [("avg", |s| s.avg), ("p50", |s| s.p50), ("p99", |s| s.p99)];
    for ((k, bstat), (_, nstat)) in bs
        .stats()
        .into_iter()
        .zip(ns.stats())
        .filter(|((k, _), _)| *k != "sleep")
    {
        for (q, of) in quantiles {
            compare_row(
                &mut t,
                &format!("{k} {q}"),
                bstat.as_ref().and_then(of),
                nstat.as_ref().and_then(of),
            );
        }
    }
    t += "\nGPU passes, ms\n";
    let gpu_of = |d: &Doc, name: &str| {
        d.passes
            .iter()
            .find(|p| p.name.as_deref() == Some(name))
            .and_then(|p| p.gpu)
    };
    for name in union(pass_names(n), pass_names(b)) {
        compare_row(&mut t, name, gpu_of(b, name), gpu_of(n, name));
    }
    let ms_of = |d: &Doc, (kind, name): (&str, &str)| {
        d.cpu
            .iter()
            .find(|r| r.kind.as_deref() == Some(kind) && r.name.as_deref() == Some(name))
            .and_then(|r| r.ms)
    };
    let mut deltas: Vec<(String, Option<f64>, Option<f64>)> = union(cpu_keys(n), cpu_keys(b))
        .into_iter()
        .map(|k @ (kind, name)| {
            let label = if kind == "system" {
                name.to_string()
            } else {
                format!("{name} ({kind})")
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

fn to_csv(d: &Doc) -> String {
    let f = &d.frames;
    let cols = [
        ("t", &f.t),
        ("frame", &f.frame),
        ("sleep", &f.sleep),
        ("main", &f.main),
        ("render", &f.render),
        ("wait", &f.wait),
        ("gpu", &f.gpu),
    ];
    let mut s = cols.map(|(name, _)| name).join(",") + "\n";
    for i in 0..f.t.len() {
        let row: Vec<String> = cols
            .iter()
            .map(|(_, c)| c.get(i).copied().flatten().map_or(String::new(), |v| format!("{v:.3}")))
            .collect();
        s += &row.join(",");
        s.push('\n');
    }
    s
}
