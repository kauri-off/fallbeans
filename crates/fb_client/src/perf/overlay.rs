//! The F4 overlay: a line, then percentiles, graph, GPU passes and scene, then the profiler.
use bevy::asset::embedded_asset;
use bevy::camera::MainPassResolutionOverride;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::renderer::RenderAdapterInfo;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

use super::profiler::{self, Kind};
use super::scene::big;
use super::stats::{Frame, Stat, Summary};
use super::{Mode, Perf};
use crate::render::quality::Quality;
use crate::settings::Graphics;
use crate::view::MainCamera;

const SAMPLES: usize = 256;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct GraphMaterial {
    #[uniform(0)]
    u: GraphUniform,
}

#[derive(Clone, Copy, ShaderType)]
struct GraphUniform {
    frame: [Vec4; SAMPLES / 4],
    gpu: [Vec4; SAMPLES / 4],
    params: Vec4,
}

impl UiMaterial for GraphMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/perf/graph.wgsl".into()
    }
}

#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct Head;

#[derive(Component)]
struct Body;

#[derive(Component)]
struct Graph;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "graph.wgsl");
        app.add_plugins(UiMaterialPlugin::<GraphMaterial>::default());
        app.add_systems(Startup, setup);
        app.add_systems(Update, (toggle, text, graph).chain());
    }
}

fn setup(mut commands: Commands, mut graphs: ResMut<Assets<GraphMaterial>>) {
    let font = || TextFont {
        font_size: FontSize::Px(13.0),
        ..default()
    };
    let material = graphs.add(GraphMaterial {
        u: GraphUniform {
            frame: [Vec4::ZERO; SAMPLES / 4],
            gpu: [Vec4::ZERO; SAMPLES / 4],
            params: Vec4::new(33.4, 0.0, 0.0, 0.0),
        },
    });
    commands
        .spawn((
            Overlay,
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: px(8),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(101),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            p.spawn((Head, Text::new(""), font(), TextColor(Color::WHITE), Pickable::IGNORE));
            p.spawn((
                Graph,
                MaterialNode(material),
                Node {
                    width: px(SAMPLES as f32 * 2.0),
                    height: px(72),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            p.spawn((
                Body,
                Text::new(""),
                font(),
                TextColor(Color::srgb(0.85, 0.88, 0.92)),
                Pickable::IGNORE,
            ));
        });
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut perf: ResMut<Perf>,
    mut overlay: Query<&mut Visibility, (With<Overlay>, Without<Graph>)>,
    mut graph: Query<&mut Node, With<Graph>>,
) {
    if keys.just_pressed(KeyCode::F4) {
        perf.mode = perf.mode.next();
    }
    for mut v in &mut overlay {
        v.set_if_neq(if perf.mode == Mode::Off {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    let shown = if perf.mode == Mode::Full {
        Display::Flex
    } else {
        Display::None
    };
    for mut n in &mut graph {
        if n.display != shown {
            n.display = shown;
        }
    }
}

fn ms(v: f32) -> String {
    if v.is_finite() { format!("{v:.2}") } else { "—".into() }
}

fn row(name: &str, s: Option<Stat>) -> String {
    match s {
        Some(s) => format!(
            "{name:<7}{:>8.2}{:>8.2}{:>8.2}{:>8.2}{:>8.2}",
            s.avg, s.p50, s.p95, s.p99, s.max
        ),
        None => format!("{name:<7}       —"),
    }
}

/// `144 fps | frame 6.94 ms p99 9.1 | 1% low 118 fps | GPU 4.20 | CPU main 2.10 render 1.80 (wait 0.40) | GPU-bound`
/// (over the last 2 s: fps is the frames over their time, as F3 shows it).
fn line(s: &Summary) -> String {
    let avg = |st: Option<Stat>| ms(st.map_or(f32::NAN, |s| s.avg));
    format!(
        "{:.0} fps | frame {} ms p99 {:.1} | 1% low {:.0} fps | GPU {} | CPU main {} render {} (wait {}) | {}",
        s.fps,
        ms(s.frame.avg),
        s.frame.p99,
        s.fps_low1,
        avg(s.gpu),
        ms(s.main.avg),
        ms(s.render.avg),
        avg(s.wait),
        s.bound.map_or("—", |b| b.label()),
    )
}

/// The resolution, render scale, preset and switches, and the adapter.
fn setup_line(
    g: &Graphics,
    q: Option<&Quality>,
    target: Option<UVec2>,
    low: Option<UVec2>,
    info: Option<&RenderAdapterInfo>,
) -> String {
    let size = target.map_or("—".into(), |t| {
        let (pw, ph) = (t.x, t.y);
        match low {
            Some(l) => format!("{pw}×{ph} → main pass {}×{} (FSR {})", l.x, l.y, g.upscale),
            None => format!("{pw}×{ph}"),
        }
    });
    let mut on = Vec::new();
    for (flag, name) in [
        (g.shadows, "shadows"),
        (g.ao, "AO"),
        (g.aa, "AA"),
        (g.grade, "grade"),
        (g.motes, "motes"),
    ] {
        if flag {
            on.push(name);
        }
    }
    let limit = if g.fps_limit > 0 {
        g.fps_limit.to_string()
    } else {
        "off".into()
    };
    let adapter = q.map_or("—".into(), |q| q.adapter.clone());
    let driver = info.map_or(String::new(), |i| format!(" | {} {}", i.0.driver, i.0.driver_info));
    format!(
        "{size} | preset {} ({}) | {} | vsync {} limit {limit}\n{adapter}{driver}",
        q.map_or("—".into(), |q| format!("{:?}", q.preset)),
        q.map_or("—".into(), |q| format!("{:?}", q.tier)),
        on.join(" "),
        if g.vsync { "on" } else { "off" },
    )
}

fn text(
    time: Res<Time<Real>>,
    perf: Res<Perf>,
    g: Res<Graphics>,
    q: Option<Res<Quality>>,
    info: Option<Res<RenderAdapterInfo>>,
    camera: Query<(&Camera, Option<&MainPassResolutionOverride>), With<MainCamera>>,
    recording: Res<super::capture::Recording>,
    mut head: Query<&mut Text, (With<Head>, Without<Body>)>,
    mut body: Query<&mut Text, (With<Body>, Without<Head>)>,
    mut at: Local<f32>,
) {
    let now = time.elapsed_secs();
    if perf.mode == Mode::Off || now - *at < 0.25 {
        return;
    }
    *at = now;
    let (Ok(mut head), Ok(mut body)) = (head.single_mut(), body.single_mut()) else {
        return;
    };
    let summary = Summary::of(&perf.recent(2.0), g.vsync);
    let mut h = line(&summary);
    if let Some(r) = recording.0.as_ref() {
        h += &format!("\n● {}", r.status(now));
    }
    let (target, low) = camera
        .single()
        .map_or((None, None), |(c, l)| (c.physical_target_size(), l.map(|l| l.0)));
    let b = match perf.mode {
        Mode::Off | Mode::Line => String::new(),
        Mode::Full => {
            let s = Summary::of(&perf.recent(5.0), g.vsync);
            h += &format!(
                "\n{}\n{}\n{}\n{}\n{}\n{}\n{}\nstutters {} in 5 s\n{}",
                "ms, 5 s   avg     p50     p95     p99     max",
                row("frame", Some(s.frame)),
                row("gpu", s.gpu),
                row("main", Some(s.main)),
                row("render", Some(s.render)),
                row("wait", s.wait),
                row("sleep", Some(s.sleep)),
                s.stutters,
                setup_line(&g, q.as_deref(), target, low, info.as_deref()),
            );
            full_body(&perf, target)
        }
        Mode::Cpu => cpu_body(&perf),
    };
    head.0 = h;
    body.0 = b;
}

fn full_body(perf: &Perf, target: Option<UVec2>) -> String {
    let pixels = target.map_or(1.0, |t| t.element_product().max(1) as f32);
    // (Bevy 0.19 times no shadow pass: their cost shows in the sweep.)
    let mut s = String::from("GPU passes (no shadows)     GPU ms  CPU ms     tris  frag/px\n");
    if perf.passes.is_empty() {
        s += "  (no GPU timers: --no-gpu-timers, or the GPU has no timestamp queries)\n";
    }
    for p in perf.passes.iter().take(14) {
        s += &format!(
            "  {:<26}{:>6}{:>8}{:>9}{:>9}\n",
            p.name,
            ms(p.gpu),
            ms(p.cpu),
            big(p.tris as u64),
            if p.frags > 0.0 {
                format!("{:.2}", p.frags / pixels)
            } else {
                "—".into()
            }
        );
    }
    if let Some(scene) = &perf.scene {
        s += &scene.line();
    }
    s += "\nF4: profiler | F9: record | Shift+F9: graphics sweep";
    s
}

/// The Cpu page of a game started without `--profiler`: the spans it sums are not there.
const NO_SPANS: &str = "Время систем и расписаний меряется, только если игра запущена с флагом --profiler\n\
                        (или с FB_PROFILER=1): без него профайлер ничего не стоит.\nF4: скрыть | F9: запись";

fn cpu_body(perf: &Perf) -> String {
    if !profiler::BUILT {
        return "the profiler is not in this build (feature `profiler`)".into();
    }
    if !perf.profiler {
        return NO_SPANS.into();
    }
    let rows = &perf.cpu;
    let find = |kind: Kind, name: &str| {
        rows.iter()
            .find(|r| r.kind == kind && r.name.ends_with(name))
            .map_or(f32::NAN, |r| r.ms)
    };
    let mut s = format!(
        "CPU, ms a frame (1 s): main app {} | render app {} | acquire {} | present {}\nschedules:",
        ms(find(Kind::Frame, "main app")),
        ms(find(Kind::Frame, "RenderApp")),
        ms(find(Kind::System, "::prepare_windows")),
        ms(find(Kind::Frame, "present_frames")),
    );
    for r in rows.iter().filter(|r| r.kind == Kind::Schedule).take(10) {
        s += &format!(" {} {:.2} ·", r.name, r.ms);
    }
    s.pop();
    s += "\n    ms   runs  system\n";
    for r in rows
        .iter()
        .filter(|r| matches!(r.kind, Kind::System | Kind::Commands))
        .take(24)
    {
        let tag = if r.kind == Kind::Commands { " (commands)" } else { "" };
        s += &format!("{:>6.2} {:>6.1}  {}{tag}\n", r.ms, r.calls, r.name);
    }
    if !perf.spikes.is_empty() {
        s += "long frames:\n";
        let now = perf.frames.back().map_or(0.0, |f: &Frame| f.t);
        for sp in perf.spikes.iter().rev().take(4) {
            s += &format!("  {:.0} s ago: {}\n", now - sp.t, sp.line());
        }
    }
    s += "F4: hide | F9: record";
    s
}

/// The graph's frames: the last 256, 30 times a second.
fn graph(
    time: Res<Time<Real>>,
    perf: Res<Perf>,
    node: Query<&MaterialNode<GraphMaterial>>,
    mut graphs: ResMut<Assets<GraphMaterial>>,
    mut at: Local<f32>,
) {
    let now = time.elapsed_secs();
    if perf.mode != Mode::Full || now - *at < 1.0 / 30.0 {
        return;
    }
    *at = now;
    let Ok(node) = node.single() else { return };
    let Some(mut m) = graphs.get_mut(&node.0) else {
        return;
    };
    let n = perf.frames.len().min(SAMPLES);
    let mut frame = [0.0f32; SAMPLES];
    let mut gpu = [0.0f32; SAMPLES];
    let mut top: f32 = 33.4;
    for (i, f) in perf.frames.iter().skip(perf.frames.len() - n).enumerate() {
        let at = SAMPLES - n + i;
        frame[at] = f.frame;
        gpu[at] = if f.gpu.is_finite() { f.gpu } else { 0.0 };
        top = top.max(f.frame);
    }
    let pack = |v: &[f32; SAMPLES]| core::array::from_fn(|i| Vec4::from_slice(&v[i * 4..i * 4 + 4]));
    m.u = GraphUniform {
        frame: pack(&frame),
        gpu: pack(&gpu),
        params: Vec4::new((top * 1.1).min(100.0), n as f32, 0.0, 0.0),
    };
}
