//! Recordings to a JSON file: F9 (until F9 again), Shift+F9 sweep, `--perf-capture` / `--perf-sweep` benchmarks.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Instant;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use fb_shared::game::ArenaKind;
use serde_json::{Value, json};

use super::gpu::{self, Pass};
use super::scene::Scene;
use super::stats::{Frame, Summary};
use super::{CpuAcc, Perf, RenderShared, Spike};
use crate::game::Map;
use crate::logs::{self, Logs};
use crate::opts::Opts;
use crate::render::quality::{Preset, Quality};
use crate::session::Session;
use crate::settings::Graphics;
use crate::ui::text;
use crate::view::MainCamera;

/// A manual recording stops by itself after this long, s.
const MAX_S: f32 = 300.0;
/// A sweep step: at least this long to settle, and then until no pipeline is compiling (at most
/// `SETTLE_MAX_S`), then seconds measured.
const SETTLE_S: f32 = 2.0;
const SETTLE_MAX_S: f32 = 15.0;
const STEP_S: f32 = 5.0;
/// Long frames a recording keeps (the rest are only counted).
const SPIKES_MAX: usize = 256;

#[derive(Resource, Default)]
pub struct Recording(pub Option<Capture>);

/// Recordings being written on a thread of their own, and whether the game quits once one is.
#[derive(Resource, Default)]
struct Saving(Vec<(JoinHandle<Option<PathBuf>>, bool)>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    Manual,
    Benchmark,
    Sweep,
}

#[derive(Default)]
struct Passes(BTreeMap<String, (Pass, u32)>);

impl Passes {
    fn add(&mut self, passes: Vec<Pass>) {
        for p in passes {
            let e = self.0.entry(p.name.clone()).or_default();
            e.0.gpu += p.gpu;
            e.0.cpu += p.cpu;
            e.0.tris += p.tris;
            e.0.frags += p.frags;
            e.1 += 1;
        }
    }

    fn averages(&self) -> Vec<Pass> {
        let mut v: Vec<Pass> = self
            .0
            .iter()
            .map(|(name, (p, n))| {
                let n = *n as f32;
                Pass {
                    name: name.clone(),
                    gpu: p.gpu / n,
                    cpu: p.cpu / n,
                    tris: p.tris / n,
                    frags: p.frags / n,
                }
            })
            .collect();
        v.sort_by(|a, b| b.gpu.total_cmp(&a.gpu));
        v
    }
}

/// A stretch of a recording in one scene (the menu, the lobby, a round of a map, the podium).
struct Segment {
    scene: String,
    started: f32,
    frames: Vec<Frame>,
    cpu: CpuAcc,
    passes: Passes,
    counts: Option<Scene>,
}

struct Step {
    name: &'static str,
    graphics: Graphics,
    /// When the measuring began (the step settled).
    from: Option<f32>,
    frames: Vec<Frame>,
    passes: Passes,
}

pub struct Capture {
    what: What,
    started: f32,
    stop_at: Option<f32>,
    exit: bool,
    frames: Vec<Frame>,
    cpu: CpuAcc,
    passes: Passes,
    /// The first `SPIKES_MAX` long frames; how many there were; the time of the last one taken.
    spikes: Vec<Spike>,
    spike_count: usize,
    spike_t: f32,
    segments: Vec<Segment>,
    steps: Vec<Step>,
    step: usize,
    step_started: f32,
    restore: Option<Graphics>,
    /// The tier and preset as the recording began (a sweep ends on another preset).
    quality: Option<Quality>,
    out: Option<PathBuf>,
}

impl Capture {
    fn new(what: What, now: f32) -> Capture {
        Capture {
            what,
            started: now,
            stop_at: None,
            exit: false,
            frames: Vec::new(),
            cpu: CpuAcc::default(),
            passes: Passes::default(),
            spikes: Vec::new(),
            spike_count: 0,
            spike_t: f32::NEG_INFINITY,
            segments: Vec::new(),
            steps: Vec::new(),
            step: 0,
            step_started: now,
            restore: None,
            quality: None,
            out: None,
        }
    }

    /// `recording 12 s (F9: stop)`, `sweep 3/8: shadows off`.
    pub fn status(&self, now: f32) -> String {
        match self.what {
            What::Sweep => format!(
                "sweep {}/{}: {} (F9: stop)",
                self.step + 1,
                self.steps.len(),
                self.steps.get(self.step).map_or("", |s| s.name)
            ),
            What::Benchmark => format!("benchmark {:.0} s (F9: stop)", now - self.started),
            What::Manual => format!("recording {:.0} s (F9: stop)", now - self.started),
        }
    }
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recording>().init_resource::<Saving>();
        app.add_systems(
            Update,
            (keys, auto_start, saved.run_if(|s: Res<Saving>| !s.0.is_empty())),
        );
        app.add_systems(
            Last,
            (
                record.after(super::collect),
                restore_on_exit.before(crate::settings::save_on_exit),
            ),
        );
    }
}

/// The sweep's steps: everything uncapped, then each feature that is on switched off alone.
fn steps(g: &Graphics, q: &Quality) -> Vec<(&'static str, Graphics)> {
    let base = Graphics {
        vsync: false,
        fps_limit: 0,
        ..g.clone()
    };
    let with = |f: &dyn Fn(&mut Graphics)| {
        let mut g = base.clone();
        f(&mut g);
        g
    };
    let mut v = vec![("as set, uncapped", base.clone())];
    if base.shadows {
        v.push(("shadows off", with(&|g| g.shadows = false)));
    }
    if base.aa {
        v.push(("AA off", with(&|g| g.aa = false)));
    }
    if base.grade {
        v.push(("grade off", with(&|g| g.grade = false)));
    }
    if base.motes {
        v.push(("motes off", with(&|g| g.motes = false)));
    }
    if Quality::scale(&base.upscale) > Quality::scale("performance") {
        v.push(("FSR performance", with(&|g| g.upscale = "performance".into())));
    }
    if q.preset > Preset::Low {
        v.push(("preset low", with(&|g| g.preset = "low".into())));
    }
    v
}

fn start(what: What, now: f32, g: &mut Graphics, q: Option<&Quality>) -> Capture {
    let mut c = Capture::new(what, now);
    c.quality = q.cloned();
    match what {
        What::Sweep => {
            c.restore = Some(g.clone());
            let Some(q) = q else { return c };
            c.steps = steps(g, q)
                .into_iter()
                .map(|(name, graphics)| Step {
                    name,
                    graphics,
                    from: None,
                    frames: Vec::new(),
                    passes: Passes::default(),
                })
                .collect();
            *g = c.steps[0].graphics.clone();
        }
        What::Benchmark => {
            c.restore = Some(g.clone());
            g.vsync = false;
            g.fps_limit = 0;
        }
        What::Manual => c.stop_at = Some(now + MAX_S),
    }
    c
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    real: Res<Time<Real>>,
    time: Res<Time>,
    mut recording: ResMut<Recording>,
    mut perf: ResMut<Perf>,
    mut g: ResMut<Graphics>,
    q: Option<Res<Quality>>,
    mut session: ResMut<Session>,
) {
    if !keys.just_pressed(KeyCode::F9) {
        return;
    }
    let now = real.elapsed_secs();
    // (Any recording stops: a benchmark of `--perf-capture` then saves what it has and quits.)
    if let Some(c) = recording.0.as_mut() {
        c.stop_at = Some(now);
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let what = if shift { What::Sweep } else { What::Manual };
    let c = start(what, now, &mut g, q.as_deref());
    perf.busy = what == What::Sweep;
    info!("perf: {what:?} recording started");
    session.note(
        time.elapsed_secs(),
        if shift { text::PERF_SWEEP } else { text::PERF_RECORDING }.into(),
    );
    recording.0 = Some(c);
}

/// `--perf-capture` / `--perf-sweep`: once a round has run for `--perf-warmup` seconds.
fn auto_start(
    opts: Res<Opts>,
    real: Res<Time<Real>>,
    map: Option<Res<Map>>,
    mut recording: ResMut<Recording>,
    mut perf: ResMut<Perf>,
    mut g: ResMut<Graphics>,
    q: Option<Res<Quality>>,
    mut round_since: Local<Option<f32>>,
    mut done: Local<bool>,
    warm: Option<Res<crate::render::warmup::Warmup>>,
) {
    if *done || (opts.perf_capture.is_none() && !opts.perf_sweep) {
        return;
    }
    let now = real.elapsed_secs();
    // (Not while the loading screen is up, nor over the maps it builds.)
    if recording.0.is_some() || warm.is_some_and(|w| w.busy()) {
        return;
    }
    if !opts.perf_from_start {
        if !map.is_some_and(|m| m.round.kind == ArenaKind::Round && !m.warmup) {
            *round_since = None;
            return;
        }
        let since = *round_since.get_or_insert(now);
        if now - since < opts.perf_warmup {
            return;
        }
    }
    *done = true;
    let what = if opts.perf_sweep { What::Sweep } else { What::Benchmark };
    let mut c = start(what, now, &mut g, q.as_deref());
    if what == What::Benchmark {
        c.stop_at = opts.perf_capture.map(|s| now + s);
    }
    c.exit = true;
    c.out = opts.perf_out.clone();
    perf.busy = true;
    info!("perf: {what:?} started");
    recording.0 = Some(c);
}

/// `menu`, `lobby`, `podium`, or the map of a round.
fn scene_name(map: Option<&Map>) -> String {
    match map {
        None => "menu".into(),
        Some(m) => match m.round.kind {
            ArenaKind::Lobby => "lobby".into(),
            ArenaKind::Podium => "podium".into(),
            ArenaKind::Round => m.round.map.clone(),
        },
    }
}

/// A recording's frame. A plain system (an exclusive one would be a sync point every frame): the end, which
/// needs the whole world, is a command.
fn record(
    mut commands: Commands,
    real: Res<Time<Real>>,
    map: Option<Res<Map>>,
    store: Res<DiagnosticsStore>,
    perf: Res<Perf>,
    shared: Res<RenderShared>,
    mut recording: ResMut<Recording>,
    mut g: ResMut<Graphics>,
) {
    if recording.0.is_none() {
        return;
    }
    let Some(frame) = perf.frames.back().copied() else {
        return;
    };
    let Some(c) = recording.0.as_mut() else { return };
    let now = real.elapsed_secs();
    let passes = gpu::passes(&store, Instant::now());
    // (The overlay's spikes come in time order: the ones after the last taken are new.)
    let (started, last) = (c.started, c.spike_t);
    for s in perf.spikes.iter().filter(|s| s.t >= started && s.t > last) {
        c.spike_t = s.t;
        c.spike_count += 1;
        if c.spikes.len() < SPIKES_MAX {
            c.spikes.push(s.clone());
        }
    }
    let mut finished = c.stop_at.is_some_and(|t| now >= t);
    if c.what == What::Sweep {
        let settled = now - c.step_started >= SETTLE_S
            && (shared.pipelines_waiting() == 0 || now - c.step_started >= SETTLE_MAX_S);
        let advance = match c.steps.get_mut(c.step) {
            None => {
                finished = true;
                false
            }
            Some(step) => {
                if step.from.is_none() && settled {
                    step.from = Some(now);
                }
                if step.from.is_some() {
                    step.frames.push(frame);
                    step.passes.add(passes);
                }
                step.from.is_some_and(|f| now - f >= STEP_S)
            }
        };
        if advance && !finished {
            c.step += 1;
            c.step_started = now;
            match c.steps.get(c.step) {
                Some(step) => *g = step.graphics.clone(),
                None => finished = true,
            }
        }
    } else {
        let scene = scene_name(map.as_deref());
        if c.segments.last().is_none_or(|s| s.scene != scene) {
            c.segments.push(Segment {
                scene,
                started: frame.t,
                frames: Vec::new(),
                cpu: CpuAcc::default(),
                passes: Passes::default(),
                counts: None,
            });
        }
        if let Some(seg) = c.segments.last_mut() {
            seg.frames.push(frame);
            seg.passes.add(passes.clone());
            seg.cpu.add(&perf.taken);
            if perf.scene.is_some() {
                seg.counts.clone_from(&perf.scene);
            }
        }
        c.frames.push(frame);
        c.passes.add(passes);
        c.cpu.add(&perf.taken);
    }
    if finished && let Some(c) = recording.0.take() {
        commands.queue(move |world: &mut World| finish(world, c));
    }
}

/// The end of a recording: the graphics back, and the file written on a thread of its own (a long
/// recording is millions of numbers).
fn finish(world: &mut World, c: Capture) {
    if let Some(g) = &c.restore {
        *world.resource_mut::<Graphics>() = g.clone();
    }
    world.resource_mut::<Perf>().busy = false;
    let meta = meta(world, &c);
    let dir = world.resource::<Logs>().0.clone();
    let quit = c.exit;
    let job = std::thread::Builder::new()
        .name("perf-save".into())
        .spawn(move || save(&c, &meta, dir));
    match job {
        Ok(h) => world.resource_mut::<Saving>().0.push((h, quit)),
        Err(e) => {
            warn!("perf: cannot start writing the recording: {e}");
            let (note, exit) = outcome(None, quit);
            let now = world.resource::<Time>().elapsed_secs();
            if let Some(n) = note {
                world.resource_mut::<Session>().note(now, n);
            }
            if let Some(e) = exit {
                world.write_message(e);
            }
        }
    }
}

/// The saves that are done: the player is told where the file is, a benchmark quits.
fn saved(mut saving: ResMut<Saving>, time: Res<Time>, mut session: ResMut<Session>, mut exit: MessageWriter<AppExit>) {
    let (done, pending): (Vec<_>, Vec<_>) = core::mem::take(&mut saving.0)
        .into_iter()
        .partition(|(h, _)| h.is_finished());
    saving.0 = pending;
    for (h, quit) in done {
        let path = h.join().ok().flatten();
        let (note, quit) = outcome(path.as_deref(), quit);
        if let Some(n) = note {
            session.note(time.elapsed_secs(), n);
        }
        if let Some(e) = quit {
            exit.write(e);
        }
    }
}

/// What the log and the player are told once a recording is written (or is not), and the exit of a
/// benchmark.
fn outcome(saved: Option<&Path>, quit: bool) -> (Option<String>, Option<AppExit>) {
    let note = match saved {
        Some(p) => {
            info!("perf: saved {}", p.display());
            Some(text::perf_saved(&p.display().to_string()))
        }
        None => {
            warn!("perf: the recording could not be saved");
            None
        }
    };
    let exit = quit.then(|| {
        if saved.is_some() {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        }
    });
    (note, exit)
}

/// The recording's document and its file (on the saving thread).
fn save(c: &Capture, meta: &Meta, dir: Option<PathBuf>) -> Option<PathBuf> {
    let doc = document(c, meta);
    for l in summary_lines(&doc).lines() {
        info!("perf: {l}");
    }
    let (file, path) = match c.out.clone() {
        Some(p) => (File::create(&p).ok()?, p),
        None => logs::create(&dir?, "perf", "json")?,
    };
    let mut w = BufWriter::new(file);
    serde_json::to_writer(&mut w, &doc).ok()?;
    w.flush().ok()?;
    Some(path)
}

fn columns(frames: &[Frame]) -> Value {
    let col = |f: fn(&Frame) -> f32| frames.iter().map(f).collect::<Vec<f32>>();
    json!({
        "t": col(|f| f.t),
        "frame": col(|f| f.frame),
        "sleep": col(|f| f.sleep),
        "main": col(|f| f.main),
        "render": col(|f| f.render),
        "wait": col(|f| f.wait),
        "gpu": col(|f| f.gpu),
    })
}

fn graphics_json(g: &Graphics) -> Value {
    json!({
        "preset": g.preset, "shadows": g.shadows, "aa": g.aa, "grade": g.grade, "motes": g.motes,
        "upscale": g.upscale, "vsync": g.vsync, "fps_limit": g.fps_limit, "backend": g.backend,
    })
}

/// What the file says about the machine and the game, read from the world as the recording ends.
struct Meta {
    graphics: Graphics,
    quality: Option<Quality>,
    adapter: Option<Value>,
    window: Option<Value>,
    round: Option<Value>,
    scene: Option<Scene>,
    profiler: bool,
}

fn meta(world: &mut World, c: &Capture) -> Meta {
    let graphics = c
        .restore
        .clone()
        .unwrap_or_else(|| world.resource::<Graphics>().clone());
    let quality = c.quality.clone().or_else(|| world.get_resource::<Quality>().cloned());
    let upscaler = world
        .get_resource::<crate::render::upscale::Upscaling>()
        .map(|u| u.active.name());
    let adapter = world.get_resource::<RenderAdapterInfo>().map(|i| {
        json!({
            "name": i.0.name, "backend": format!("{:?}", i.0.backend), "type": format!("{:?}", i.0.device_type),
            "driver": i.0.driver, "driver_info": i.0.driver_info, "upscaler": upscaler,
        })
    });
    let window = world
        .query_filtered::<&Camera, With<MainCamera>>()
        .iter(world)
        .next()
        .and_then(Camera::physical_target_size)
        .map(|s| json!([s.x, s.y]));
    let round = world
        .get_resource::<Map>()
        .map(|m| json!({"map": m.round.map, "seed": m.round.seed, "kind": format!("{:?}", m.round.kind)}));
    let perf = world.resource::<Perf>();
    Meta {
        graphics,
        quality,
        adapter,
        window,
        round,
        scene: perf.scene.clone(),
        profiler: perf.profiler,
    }
}

fn document(c: &Capture, m: &Meta) -> Value {
    let g = &m.graphics;
    let q = m.quality.as_ref();
    let mut doc = json!({
        "format": 1,
        "kind": format!("{:?}", c.what).to_lowercase(),
        "build": fb_net::build(),
        "time": logs::stamp(logs::now_secs()),
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "adapter": m.adapter,
        "tier": q.map(|q| format!("{:?}", q.tier)),
        "preset": q.map(|q| format!("{:?}", q.preset)),
        "graphics": graphics_json(g),
        "window": m.window,
        "round": m.round,
        "profiler": m.profiler,
        "scene": m.scene,
        "spikes": c.spikes,
        "spike_count": c.spike_count,
    });
    if c.what == What::Sweep {
        doc["steps"] = c
            .steps
            .iter()
            .filter(|s| !s.frames.is_empty())
            .map(|s| {
                json!({
                    "name": s.name,
                    "graphics": graphics_json(&s.graphics),
                    "summary": Summary::of(&s.frames, false),
                    "passes": s.passes.averages(),
                })
            })
            .collect();
    } else {
        doc["summary"] = json!(Summary::of(&c.frames, g.vsync && c.what == What::Manual));
        doc["passes"] = json!(c.passes.averages());
        doc["cpu"] = json!(c.cpu.rows().into_iter().take(60).collect::<Vec<_>>());
        doc["frames"] = columns(&c.frames);
        let vsync = g.vsync && c.what == What::Manual;
        doc["segments"] = c
            .segments
            .iter()
            .map(|s| {
                json!({
                    "scene": s.scene,
                    "t": s.started,
                    "summary": Summary::of(&s.frames, vsync),
                    "passes": s.passes.averages(),
                    "cpu": s.cpu.rows().into_iter().take(40).collect::<Vec<_>>(),
                    "counts": s.counts,
                })
            })
            .collect();
    }
    doc
}

/// What the log says about a recording: the totals, or a line a sweep step.
fn summary_lines(doc: &Value) -> String {
    let f = |v: &Value| v.as_f64().map_or("—".into(), |x| format!("{x:.2}"));
    let line = |s: &Value| {
        format!(
            "{:.0} fps (1% low {:.0}) | frame p50 {} p99 {} | GPU p50 {} | main p50 {} | render p50 {} | {} | stutters {}",
            s["fps"].as_f64().unwrap_or(0.0),
            s["fps_low1"].as_f64().unwrap_or(0.0),
            f(&s["frame"]["p50"]),
            f(&s["frame"]["p99"]),
            f(&s["gpu"]["p50"]),
            f(&s["main"]["p50"]),
            f(&s["render"]["p50"]),
            s["bound"].as_str().unwrap_or("—"),
            s["stutters"],
        )
    };
    match doc["steps"].as_array() {
        Some(steps) => steps
            .iter()
            .map(|s| format!("sweep {}: {}", s["name"].as_str().unwrap_or(""), line(&s["summary"])))
            .collect::<Vec<_>>()
            .join("\n"),
        None => format!(
            "{:.1} s: {}",
            doc["summary"]["secs"].as_f64().unwrap_or(0.0),
            line(&doc["summary"])
        ),
    }
}

/// A sweep or benchmark cut short by quitting leaves the player's graphics as they were; a recording still
/// being written is finished first.
fn restore_on_exit(
    mut exit: MessageReader<AppExit>,
    mut recording: ResMut<Recording>,
    mut g: ResMut<Graphics>,
    mut saving: ResMut<Saving>,
) {
    if exit.read().next().is_none() {
        return;
    }
    if let Some(r) = recording.0.take().and_then(|c| c.restore) {
        *g = r;
    }
    for (h, _) in saving.0.drain(..) {
        let _ = h.join();
    }
}
