//! Render performance: frame costs, the F4 overlay, F9 recordings, `cargo xtask perf` (docs/perf.md).
pub mod capture;
mod gpu;
mod overlay;
pub mod profiler;
mod scene;
pub mod stats;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use bevy::diagnostic::{DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::diagnostic::{MeshAllocatorDiagnosticPlugin, RenderDiagnosticsPlugin};
use bevy::render::{Render, RenderApp, RenderSystems};

use self::profiler::{Kind, Slot};
use self::stats::Frame;

/// Seconds of frames kept for the overlay.
const HISTORY_S: f32 = 10.0;
/// A frame this long (ms) and this many times the usual one is a spike: the profiler keeps what it went on.
const SPIKE_MS: f32 = 25.0;
const SPIKE_TIMES: f32 = 2.5;
/// A spike this long goes to the log too, at most one every `SPIKE_LOG_QUIET_S`.
const SPIKE_LOG_MS: f32 = 100.0;
const SPIKE_LOG_QUIET_S: f32 = 5.0;
const SPIKES_KEPT: usize = 8;

pub struct PerfPlugin {
    /// GPU timestamp and pipeline statistics queries (`--no-gpu-timers` turns them off).
    pub gpu_timers: bool,
}

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        if self.gpu_timers {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        app.add_plugins((
            MeshAllocatorDiagnosticPlugin,
            SystemInformationDiagnosticsPlugin,
            overlay::OverlayPlugin,
            capture::CapturePlugin,
        ));
        let render = RenderMs(Arc::new(AtomicU32::new(f32::NAN.to_bits())));
        app.insert_resource(render.clone());
        app.init_resource::<Perf>().init_resource::<MainStart>();
        app.add_systems(First, |mut s: ResMut<MainStart>| s.0 = Some(Instant::now()));
        app.add_systems(Last, collect.before(crate::render::quality::limit_fps));
        app.add_systems(Update, count_scene);
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.insert_resource(render).init_resource::<RenderStart>().add_systems(
                Render,
                (
                    (|mut s: ResMut<RenderStart>| s.0 = Some(Instant::now())).before(RenderSystems::ExtractCommands),
                    render_end.in_set(RenderSystems::PostCleanup),
                ),
            );
        }
    }
}

/// What the overlay shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    /// One line: fps, frame, GPU, CPU, what holds the frames back.
    Line,
    /// Percentiles, the graph, the GPU's passes, the scene.
    Full,
    /// The profiler: the CPU's time by schedule and system, the last spikes.
    Cpu,
}

impl Mode {
    fn next(self) -> Mode {
        match self {
            Mode::Off => Mode::Line,
            Mode::Line => Mode::Full,
            Mode::Full => Mode::Cpu,
            Mode::Cpu => Mode::Off,
        }
    }
}

/// A frame that took far longer than the ones around it, and what the CPU spent it on (ms).
#[derive(Clone, Debug, serde::Serialize)]
pub struct Spike {
    pub t: f32,
    pub ms: f32,
    pub main: f32,
    pub render: f32,
    pub top: Vec<(String, f32)>,
}

/// A span's share of the frame (ms and runs a frame) over a while.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Row {
    pub kind: Kind,
    pub name: String,
    pub ms: f32,
    pub calls: f32,
}

/// The profiler's numbers added up over frames.
#[derive(Default)]
pub struct CpuAcc {
    slots: BTreeMap<usize, (Arc<Slot>, f64, u64)>,
    frames: u64,
}

impl CpuAcc {
    fn add(&mut self, taken: &[(Arc<Slot>, f32, u32)]) {
        self.frames += 1;
        for (s, ms, calls) in taken {
            let e = self
                .slots
                .entry(Arc::as_ptr(s) as usize)
                .or_insert_with(|| (s.clone(), 0.0, 0));
            e.1 += f64::from(*ms);
            e.2 += u64::from(*calls);
        }
    }

    /// A frame's average, the costliest first.
    pub fn rows(&self) -> Vec<Row> {
        let n = self.frames.max(1) as f64;
        let mut v: Vec<Row> = self
            .slots
            .values()
            .map(|(s, ms, calls)| Row {
                kind: s.kind,
                name: s.name.clone(),
                ms: (ms / n) as f32,
                calls: (*calls as f64 / n) as f32,
            })
            .collect();
        v.sort_by(|a, b| b.ms.total_cmp(&a.ms));
        v
    }
}

#[derive(Resource, Default)]
pub struct Perf {
    pub mode: Mode,
    pub frames: VecDeque<Frame>,
    /// The GPU's passes, as of the last look.
    pub passes: Vec<gpu::Pass>,
    pub scene: Option<scene::Scene>,
    /// The profiler over the last second.
    pub cpu: Vec<Row>,
    pub spikes: VecDeque<Spike>,
    /// The profiler's numbers of this frame (for a recording).
    pub taken: Vec<(Arc<Slot>, f32, u32)>,
    /// A sweep or benchmark runs: "auto" graphics do not lower the preset meanwhile.
    pub busy: bool,
    window: CpuAcc,
    window_since: f32,
    last: Option<Instant>,
    usual_ms: f32,
    spike_logged: f32,
}

impl Perf {
    /// The frames of the last `secs`.
    pub fn recent(&self, secs: f32) -> Vec<Frame> {
        let Some(last) = self.frames.back() else {
            return Vec::new();
        };
        self.frames.iter().filter(|f| last.t - f.t <= secs).copied().collect()
    }
}

#[derive(Resource, Default)]
struct MainStart(Option<Instant>);

#[derive(Resource, Default)]
struct RenderStart(Option<Instant>);

/// The render world's time of its last frame, ms (f32 bits): written on the render thread.
#[derive(Resource, Clone)]
struct RenderMs(Arc<AtomicU32>);

fn render_end(start: Res<RenderStart>, ms: Res<RenderMs>) {
    if let Some(s) = start.0 {
        ms.0.store((s.elapsed().as_secs_f32() * 1000.0).to_bits(), Ordering::Relaxed);
    }
}

fn collect(
    time: Res<Time<Real>>,
    start: Res<MainStart>,
    render: Res<RenderMs>,
    slept: Res<crate::render::quality::Slept>,
    store: Res<DiagnosticsStore>,
    recording: Res<capture::Recording>,
    mut perf: ResMut<Perf>,
) {
    let now = Instant::now();
    let t = time.elapsed_secs();
    let cpu_on = matches!(perf.mode, Mode::Full | Mode::Cpu) || recording.0.is_some();
    let taken = profiler::take();
    let wait = if profiler::BUILT {
        taken.iter().filter(|(s, ..)| s.always).map(|(_, ms, _)| ms).sum()
    } else {
        f32::NAN
    };
    let frame = Frame {
        t,
        frame: time.delta_secs() * 1000.0,
        sleep: slept.0 * 1000.0,
        main: start.0.map_or(f32::NAN, |s| (now - s).as_secs_f32() * 1000.0),
        render: f32::from_bits(render.0.load(Ordering::Relaxed)),
        wait,
        gpu: gpu::frame_ms(&store, now),
    };
    perf.frames.push_back(frame);
    while perf.frames.front().is_some_and(|f| t - f.t > HISTORY_S) {
        perf.frames.pop_front();
    }
    // (The profiler's spans ran between this call and the last one: the spike is measured over the same.)
    let interval = perf.last.map_or(0.0, |l| (now - l).as_secs_f32() * 1000.0);
    perf.last = Some(now);
    if perf.usual_ms == 0.0 || interval < 2.0 * perf.usual_ms {
        perf.usual_ms += (interval - perf.usual_ms) * 0.05;
    }
    if profiler::is_on() {
        perf.window.add(&taken);
        if t - perf.window_since >= 1.0 {
            perf.cpu = perf.window.rows();
            perf.window = CpuAcc::default();
            perf.window_since = t;
        }
        if interval > SPIKE_MS && interval > SPIKE_TIMES * perf.usual_ms {
            spike(&mut perf, &taken, t, interval);
        }
    } else {
        perf.cpu.clear();
    }
    perf.taken = taken;
    profiler::set_on(cpu_on && profiler::BUILT);
}

fn spike(perf: &mut Perf, taken: &[(Arc<Slot>, f32, u32)], t: f32, ms: f32) {
    let frame_of = |name: &str| {
        taken
            .iter()
            .filter(|(s, ..)| s.kind == Kind::Frame && s.name == name)
            .map(|(_, ms, _)| ms)
            .sum::<f32>()
    };
    let mut top: Vec<(String, f32)> = taken
        .iter()
        .filter(|(s, ..)| matches!(s.kind, Kind::System | Kind::Commands))
        .map(|(s, ms, _)| {
            let name = if s.kind == Kind::Commands {
                format!("{} (commands)", s.name)
            } else {
                s.name.clone()
            };
            (name, *ms)
        })
        .collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    top.truncate(8);
    let s = Spike {
        t,
        ms,
        main: frame_of("main app"),
        render: frame_of("RenderApp"),
        top,
    };
    if ms >= SPIKE_LOG_MS && t - perf.spike_logged >= SPIKE_LOG_QUIET_S {
        perf.spike_logged = t;
        info!("long frame: {}", s.line());
    }
    perf.spikes.push_back(s);
    if perf.spikes.len() > SPIKES_KEPT {
        perf.spikes.pop_front();
    }
}

impl Spike {
    /// `743 ms (main 700, render 21): view::build 612 · …`
    pub fn line(&self) -> String {
        let top: Vec<String> = self.top.iter().map(|(n, ms)| format!("{n} {ms:.1}")).collect();
        format!(
            "{:.0} ms (main {:.0}, render {:.0}): {}",
            self.ms,
            self.main,
            self.render,
            top.join(" · ")
        )
    }
}

/// The scene's counts, twice a second while the overlay shows them or a recording runs.
fn count_scene(world: &mut World, mut at: Local<f32>) {
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let want = world.resource::<Perf>().mode == Mode::Full || world.resource::<capture::Recording>().0.is_some();
    if !want || now - *at < 0.5 {
        return;
    }
    *at = now;
    let scene = scene::count(world);
    let passes = gpu::passes(world.resource::<DiagnosticsStore>(), Instant::now());
    let mut perf = world.resource_mut::<Perf>();
    perf.scene = Some(scene);
    perf.passes = passes;
}
