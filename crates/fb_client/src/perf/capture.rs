//! Recordings to a JSON file: F9 (until F9 again), Shift+F9 sweep, `--perf-capture` / `--perf-sweep` benchmarks.
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use fb_shared::game::ArenaKind;
use serde_json::{Value, json};

use super::gpu::{self, Pass};
use super::stats::{Frame, Summary};
use super::{CpuAcc, Perf, Spike};
use crate::game::Map;
use crate::logs::{self, Logs};
use crate::opts::Opts;
use crate::render::quality::{Preset, Quality, Tier};
use crate::session::Session;
use crate::settings::Graphics;
use crate::ui::text;
use crate::view::MainCamera;

/// A manual recording stops by itself after this long, s.
const MAX_S: f32 = 300.0;
/// A sweep step: seconds to settle (pipelines compile), then seconds measured.
const SETTLE_S: f32 = 2.0;
const STEP_S: f32 = 5.0;

#[derive(Resource, Default)]
pub struct Recording(pub Option<Capture>);

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

struct Step {
    name: &'static str,
    graphics: Graphics,
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
    spikes: Vec<Spike>,
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
            What::Benchmark => format!("benchmark {:.0} s", now - self.started),
            What::Manual => format!("recording {:.0} s (F9: stop)", now - self.started),
        }
    }
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recording>();
        app.add_systems(Update, (keys, auto_start));
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
    if base.ao && q.preset == Preset::High && q.tier == Tier::T2 && Quality::scale(&base.upscale) >= 1.0 {
        v.push(("AO off", with(&|g| g.ao = false)));
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
    if Quality::scale(&base.upscale) >= 1.0 {
        v.push(("FSR quality", with(&|g| g.upscale = "quality".into())));
        v.push(("FSR performance", with(&|g| g.upscale = "performance".into())));
    }
    if q.preset > Preset::Medium {
        v.push(("preset medium", with(&|g| g.preset = "medium".into())));
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
    if let Some(c) = recording.0.as_mut() {
        // (A benchmark of `--perf-capture` runs to its end.)
        if c.what != What::Benchmark {
            c.stop_at = Some(now);
        }
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
) {
    if *done || (opts.perf_capture.is_none() && !opts.perf_sweep) {
        return;
    }
    let now = real.elapsed_secs();
    if !map.is_some_and(|m| m.round.kind == ArenaKind::Round) {
        *round_since = None;
        return;
    }
    let since = *round_since.get_or_insert(now);
    if now - since < opts.perf_warmup || recording.0.is_some() {
        return;
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

fn record(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let Some(mut c) = world.resource_mut::<Recording>().0.take() else {
        return;
    };
    let passes = gpu::passes(world.resource::<DiagnosticsStore>(), Instant::now());
    {
        let perf = world.resource::<Perf>();
        let Some(frame) = perf.frames.back().copied() else {
            world.resource_mut::<Recording>().0 = Some(c);
            return;
        };
        let new_spikes = perf
            .spikes
            .iter()
            .filter(|s| s.t >= c.started && c.spikes.iter().all(|o| o.t != s.t));
        c.spikes.extend(new_spikes.cloned().collect::<Vec<_>>());
        if c.what == What::Sweep {
            if let Some(step) = c.steps.get_mut(c.step)
                && now - c.step_started >= SETTLE_S
            {
                step.frames.push(frame);
                step.passes.add(passes);
            }
        } else {
            c.frames.push(frame);
            c.passes.add(passes);
            c.cpu.add(&perf.taken);
        }
    }
    let mut finished = c.stop_at.is_some_and(|t| now >= t);
    if c.what == What::Sweep && !finished && now - c.step_started >= SETTLE_S + STEP_S {
        c.step += 1;
        c.step_started = now;
        match c.steps.get(c.step) {
            Some(step) => *world.resource_mut::<Graphics>() = step.graphics.clone(),
            None => finished = true,
        }
    }
    if c.what == What::Sweep && c.steps.is_empty() {
        finished = true;
    }
    if finished {
        finish(world, c);
    } else {
        world.resource_mut::<Recording>().0 = Some(c);
    }
}

fn finish(world: &mut World, c: Capture) {
    if let Some(g) = &c.restore {
        *world.resource_mut::<Graphics>() = g.clone();
    }
    world.resource_mut::<Perf>().busy = false;
    let doc = document(world, &c);
    let summary = summary_lines(&doc);
    for l in summary.lines() {
        info!("perf: {l}");
    }
    let file = match c.out.clone() {
        Some(p) => std::fs::File::create(&p).ok().map(|f| (f, p)),
        None => world
            .resource::<Logs>()
            .0
            .clone()
            .and_then(|d| logs::create(&d, "perf", "json")),
    };
    let saved = file.and_then(|(mut f, p)| {
        f.write_all(serde_json::to_string(&doc).ok()?.as_bytes()).ok()?;
        Some(p)
    });
    let now = world.resource::<Time>().elapsed_secs();
    match &saved {
        Some(p) => {
            info!("perf: saved {}", p.display());
            world
                .resource_mut::<Session>()
                .note(now, text::perf_saved(&p.display().to_string()));
        }
        None => warn!("perf: the recording could not be saved"),
    }
    if c.exit {
        world.write_message(if saved.is_some() {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
    }
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
        "preset": g.preset, "shadows": g.shadows, "ao": g.ao, "aa": g.aa, "grade": g.grade, "motes": g.motes,
        "upscale": g.upscale, "vsync": g.vsync, "fps_limit": g.fps_limit, "backend": g.backend,
    })
}

fn document(world: &mut World, c: &Capture) -> Value {
    let g = c
        .restore
        .clone()
        .unwrap_or_else(|| world.resource::<Graphics>().clone());
    let q = c.quality.clone().or_else(|| world.get_resource::<Quality>().cloned());
    let info = world.get_resource::<RenderAdapterInfo>().map(|i| {
        json!({
            "name": i.0.name, "backend": format!("{:?}", i.0.backend), "type": format!("{:?}", i.0.device_type),
            "driver": i.0.driver, "driver_info": i.0.driver_info,
        })
    });
    let window = world
        .query_filtered::<&Camera, With<MainCamera>>()
        .iter(world)
        .next()
        .and_then(Camera::physical_target_size)
        .map(|s| json!([s.x, s.y]));
    let map = world
        .get_resource::<Map>()
        .map(|m| json!({"map": m.round.map, "seed": m.round.seed, "kind": format!("{:?}", m.round.kind)}));
    let perf = world.resource::<Perf>();
    let mut doc = json!({
        "format": 1,
        "kind": format!("{:?}", c.what).to_lowercase(),
        "build": fb_net::build(),
        "time": logs::stamp(logs::now_secs()),
        "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "adapter": info,
        "tier": q.as_ref().map(|q| format!("{:?}", q.tier)),
        "preset": q.as_ref().map(|q| format!("{:?}", q.preset)),
        "graphics": graphics_json(&g),
        "window": window,
        "round": map,
        "profiler": super::profiler::BUILT,
        "scene": perf.scene,
        "spikes": c.spikes,
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

/// A sweep or benchmark cut short by quitting leaves the player's graphics as they were.
fn restore_on_exit(mut exit: MessageReader<AppExit>, mut recording: ResMut<Recording>, mut g: ResMut<Graphics>) {
    if exit.read().next().is_some()
        && let Some(r) = recording.0.take().and_then(|c| c.restore)
    {
        *g = r;
    }
}
