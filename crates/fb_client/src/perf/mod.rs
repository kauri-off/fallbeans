//! Render performance: frame costs, the F4 overlay, F9 recordings, `cargo xtask perf` (docs/perf.md).
pub mod capture;
mod gpu;
mod overlay;
pub mod profiler;
mod scene;
pub mod stats;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::render_resource::PipelineCache;
use bevy::render::view::prepare_windows;
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
    /// The spans of systems and schedules are there (`--profiler`): the profiler can sum them.
    pub profiler: bool,
}

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        if self.gpu_timers {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        // (Not Bevy's mesh allocator and system information plugins: they measure every frame, or five times
        // a second, for every player; `scene::count` reads the same only while the counts are shown.)
        app.add_plugins((overlay::OverlayPlugin, capture::CapturePlugin));
        let shared = RenderShared::default();
        app.insert_resource(shared.clone());
        app.insert_resource(Perf {
            profiler: self.profiler && profiler::BUILT,
            ..default()
        });
        app.init_resource::<MainStart>().init_resource::<scene::Probe>();
        app.add_systems(First, main_start);
        app.add_systems(Last, collect.before(crate::render::quality::limit_fps));
        app.add_systems(Update, count_scene);
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.insert_resource(shared).init_resource::<RenderStart>().add_systems(
                Render,
                (
                    render_start.before(RenderSystems::ExtractCommands),
                    acquire_start
                        .in_set(RenderSystems::PrepareViews)
                        .before(prepare_windows),
                    acquire_end.in_set(RenderSystems::PrepareViews).after(prepare_windows),
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
    /// Since the frame before (`collect` to `collect`).
    pub ms: f32,
    /// This frame's main world (`Frame::main`).
    pub main: f32,
    /// The render thread's last finished frame: the one of the frame before (`Frame::render`).
    pub render: f32,
    /// The costliest spans that ended since the frame before: this frame's systems, and the render
    /// thread's that finished meanwhile.
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
    /// Built in and started with `--profiler`: the spans of systems and schedules are there to sum.
    pub profiler: bool,
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

    /// Frames a second over the last `secs`, as the F4 line counts them (the frames over their time).
    pub fn fps(&self, secs: f32) -> Option<f32> {
        let last = self.frames.back()?.t;
        let (n, ms) = self
            .frames
            .iter()
            .rev()
            .take_while(|f| last - f.t <= secs)
            .filter(|f| f.frame.is_finite())
            .fold((0u32, 0.0f32), |(n, ms), f| (n + 1, ms + f.frame));
        (ms > 0.0).then(|| n as f32 * 1000.0 / ms)
    }
}

#[derive(Resource, Default)]
struct MainStart(Option<Instant>);

#[derive(Resource, Default)]
struct RenderStart {
    frame: Option<Instant>,
    acquire: Option<Instant>,
}

/// What the render thread tells the main world.
struct Shared {
    /// The `Render` schedule's time of its last frame, ms (f32 bits).
    render_ms: AtomicU32,
    /// Spent acquiring swapchain images since the main world last looked, ns.
    acquire_ns: AtomicU64,
    /// The scene's counts are wanted (the overlay shows them, or a recording runs): the next two are read.
    counting: AtomicBool,
    /// The mesh allocator's buffers, bytes.
    mesh_bytes: AtomicU64,
    /// Pipelines still compiling.
    pipelines_waiting: AtomicU32,
}

#[derive(Resource, Clone)]
pub(crate) struct RenderShared(Arc<Shared>);

impl Default for RenderShared {
    fn default() -> RenderShared {
        RenderShared(Arc::new(Shared {
            render_ms: AtomicU32::new(f32::NAN.to_bits()),
            acquire_ns: AtomicU64::new(0),
            counting: AtomicBool::new(false),
            mesh_bytes: AtomicU64::new(0),
            pipelines_waiting: AtomicU32::new(0),
        }))
    }
}

impl RenderShared {
    /// Pipelines still compiling, as of the render thread's last frame while counting.
    pub(crate) fn pipelines_waiting(&self) -> u32 {
        self.0.pipelines_waiting.load(Ordering::Relaxed)
    }
}

fn main_start(mut s: ResMut<MainStart>) {
    s.0 = Some(Instant::now());
}

fn render_start(mut s: ResMut<RenderStart>) {
    s.frame = Some(Instant::now());
}

fn acquire_start(mut s: ResMut<RenderStart>) {
    s.acquire = Some(Instant::now());
}

fn acquire_end(mut s: ResMut<RenderStart>, shared: Res<RenderShared>) {
    if let Some(a) = s.acquire.take() {
        shared
            .0
            .acquire_ns
            .fetch_add(a.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

fn render_end(
    start: Res<RenderStart>,
    shared: Res<RenderShared>,
    meshes: Option<Res<MeshAllocator>>,
    pipelines: Option<Res<PipelineCache>>,
) {
    let s = &shared.0;
    if let Some(f) = start.frame {
        s.render_ms
            .store((f.elapsed().as_secs_f32() * 1000.0).to_bits(), Ordering::Relaxed);
    }
    if !s.counting.load(Ordering::Relaxed) {
        return;
    }
    if let Some(m) = meshes {
        s.mesh_bytes.store(m.slabs_size(), Ordering::Relaxed);
    }
    if let Some(p) = pipelines {
        s.pipelines_waiting
            .store(p.waiting_pipelines().count() as u32, Ordering::Relaxed);
    }
}

fn collect(
    time: Res<Time<Real>>,
    start: Res<MainStart>,
    shared: Res<RenderShared>,
    slept: Res<crate::render::quality::Slept>,
    store: Res<DiagnosticsStore>,
    recording: Res<capture::Recording>,
    mut gpu_paths: Local<gpu::FramePaths>,
    mut perf: ResMut<Perf>,
) {
    let now = Instant::now();
    let t = time.elapsed_secs();
    let counting = perf.mode == Mode::Full || recording.0.is_some();
    shared.0.counting.store(counting, Ordering::Relaxed);
    let cpu_on = matches!(perf.mode, Mode::Full | Mode::Cpu) || recording.0.is_some();
    let taken = profiler::take();
    let acquire = shared.0.acquire_ns.swap(0, Ordering::Relaxed) as f32 / 1e6;
    let wait = if profiler::BUILT {
        acquire
            + taken
                .iter()
                .filter(|(s, ..)| s.always)
                .map(|(_, ms, _)| ms)
                .sum::<f32>()
    } else {
        f32::NAN
    };
    let frame = Frame {
        t,
        frame: time.delta_secs() * 1000.0,
        sleep: slept.0 * 1000.0,
        main: start.0.map_or(f32::NAN, |s| (now - s).as_secs_f32() * 1000.0),
        render: f32::from_bits(shared.0.render_ms.load(Ordering::Relaxed)),
        wait,
        gpu: gpu_paths.frame_ms(&store, now),
    };
    perf.frames.push_back(frame);
    while perf.frames.front().is_some_and(|f| t - f.t > HISTORY_S) {
        perf.frames.pop_front();
    }
    // (The profiler's spans ran between this call and the last one: the spike is measured over the same.)
    let interval = perf.last.map_or(0.0, |l| (now - l).as_secs_f32() * 1000.0);
    perf.last = Some(now);
    if perf.usual_ms <= 0.0 {
        perf.usual_ms = interval;
    } else {
        // (Frames near the usual pull it fast; slower ones slowly, so a slowdown that lasts becomes the usual
        // pace within seconds while a single long frame barely moves it.)
        let rate = if interval < 2.0 * perf.usual_ms { 0.05 } else { 0.01 };
        let usual = perf.usual_ms;
        perf.usual_ms += (interval.min(4.0 * usual) - usual) * rate;
    }
    if profiler::is_on() {
        perf.window.add(&taken);
        if t - perf.window_since >= 1.0 {
            perf.cpu = perf.window.rows();
            perf.window = CpuAcc::default();
            perf.window_since = t;
        }
        if interval > SPIKE_MS && interval > SPIKE_TIMES * perf.usual_ms {
            spike(&mut perf, &taken, &frame, interval);
        }
    } else {
        perf.cpu.clear();
    }
    perf.taken = taken;
    let on = cpu_on && perf.profiler;
    profiler::set_on(on);
}

fn spike(perf: &mut Perf, taken: &[(Arc<Slot>, f32, u32)], frame: &Frame, ms: f32) {
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
        t: frame.t,
        ms,
        main: frame.main,
        render: frame.render,
        top,
    };
    if ms >= SPIKE_LOG_MS && frame.t - perf.spike_logged >= SPIKE_LOG_QUIET_S {
        perf.spike_logged = frame.t;
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

/// The scene's counts, twice a second while the overlay shows them or a recording runs. A command: the
/// counting needs the whole world, and an exclusive system would stop `Update` at a sync point every frame
/// even when it has nothing to do; the command runs where `Update` applies its commands anyway.
fn count_scene(
    mut commands: Commands,
    time: Res<Time<Real>>,
    perf: Res<Perf>,
    recording: Res<capture::Recording>,
    mut at: Local<f32>,
) {
    let now = time.elapsed_secs();
    let want = perf.mode == Mode::Full || recording.0.is_some();
    if !want || now - *at < 0.5 {
        return;
    }
    *at = now;
    commands.queue(move |world: &mut World| {
        let scene = scene::count(world, now);
        let passes = gpu::passes(world.resource::<DiagnosticsStore>(), Instant::now());
        let mut perf = world.resource_mut::<Perf>();
        perf.scene = Some(scene);
        perf.passes = passes;
    });
}
