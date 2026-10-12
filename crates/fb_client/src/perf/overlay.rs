//! The F4 overlay: a line, then percentiles, graph, GPU passes and scene, then the profiler.
use std::time::Duration;

use bevy::asset::embedded_asset;
use bevy::camera::MainPassResolutionOverride;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::renderer::RenderAdapterInfo;
use bevy::shader::ShaderRef;
use bevy::text::{FontFeatureTag, FontFeatures};
use bevy::time::common_conditions::on_real_timer;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

use super::profiler::{self, Kind};
use super::scene::big;
use super::stats::{Frame, Stat, Summary};
use super::{Mode, Perf};
use crate::render::quality::Quality;
use crate::render::upscale::Upscaling;
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

/// A text or table of the overlay, top to bottom (the graph sits between `Info` and `Passes`).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Head,
    Stats,
    Info,
    Passes,
    Foot,
}

/// A grid's rows as shown, the first one its header.
#[derive(Component, Default)]
struct Table(Vec<Vec<String>>);

#[derive(Component)]
struct Cell;

/// What the overlay shows this time.
#[derive(Default)]
struct Page {
    head: String,
    stats: Vec<Vec<String>>,
    info: String,
    passes: Vec<Vec<String>>,
    foot: String,
}

impl Page {
    fn text(&self, slot: Slot) -> &str {
        match slot {
            Slot::Head => &self.head,
            Slot::Info => &self.info,
            Slot::Foot => &self.foot,
            Slot::Stats | Slot::Passes => "",
        }
    }

    fn table(&mut self, slot: Slot) -> Vec<Vec<String>> {
        match slot {
            Slot::Stats => std::mem::take(&mut self.stats),
            Slot::Passes => std::mem::take(&mut self.passes),
            _ => Vec::new(),
        }
    }
}

const TEXT: Color = Color::srgb(0.88, 0.9, 0.94);
const HEADER: Color = Color::srgb(0.55, 0.6, 0.68);

fn font() -> TextFont {
    TextFont {
        font_size: FontSize::Px(13.0),
        font_features: FontFeatures::builder().enable(FontFeatureTag::TABULAR_FIGURES).build(),
        ..default()
    }
}

#[derive(Component)]
struct Graph;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "graph.wgsl");
        app.add_plugins(UiMaterialPlugin::<GraphMaterial>::default());
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (
                toggle,
                text.run_if(on_real_timer(Duration::from_millis(250))),
                graph.run_if(on_real_timer(Duration::from_secs_f32(1.0 / 30.0))),
            )
                .chain(),
        );
    }
}

fn setup(mut commands: Commands, mut graphs: ResMut<Assets<GraphMaterial>>) {
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
                row_gap: px(8),
                padding: UiRect::axes(px(12), px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.82)),
            BorderRadius::all(px(6)),
            GlobalZIndex(101),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            let text = |slot: Slot, color: Color| {
                (
                    slot,
                    Text::new(""),
                    font(),
                    TextColor(color),
                    Node {
                        display: Display::None,
                        ..default()
                    },
                    Pickable::IGNORE,
                )
            };
            let table = |slot: Slot| {
                (
                    slot,
                    Table::default(),
                    Node {
                        display: Display::None,
                        grid_template_columns: Vec::new(),
                        column_gap: px(18),
                        row_gap: px(1),
                        ..default()
                    },
                    Pickable::IGNORE,
                )
            };
            p.spawn(text(Slot::Head, Color::WHITE));
            p.spawn(table(Slot::Stats));
            p.spawn(text(Slot::Info, TEXT));
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
            p.spawn(table(Slot::Passes));
            p.spawn(text(Slot::Foot, TEXT));
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

fn row(name: &str, s: Option<Stat>) -> Vec<String> {
    let mut r = vec![name.to_string()];
    match s {
        Some(s) => r.extend([s.avg, s.p50, s.p95, s.p99, s.max].map(|v| format!("{v:.2}"))),
        None => r.extend(std::iter::repeat_n("—".to_string(), 5)),
    }
    r
}

fn header(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
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

/// The resolution, render scale and upscaler, preset and switches, and the adapter.
fn setup_line(
    g: &Graphics,
    q: Option<&Quality>,
    up: Option<&Upscaling>,
    target: Option<UVec2>,
    low: Option<UVec2>,
    info: Option<&RenderAdapterInfo>,
) -> String {
    // (The upscaler, and the one it fell back from.)
    let upscaler = up.map_or("—".into(), |u| {
        if u.active == u.chosen {
            u.active.name().to_string()
        } else {
            format!("{}, {} failed", u.active.name(), u.chosen.name())
        }
    });
    let size = target.map_or("—".into(), |t| {
        let (pw, ph) = (t.x, t.y);
        match low {
            Some(l) => format!("{pw}×{ph} → main pass {}×{} ({upscaler} {})", l.x, l.y, g.upscale.id()),
            None => format!("{pw}×{ph} ({upscaler} off)"),
        }
    });
    let mut on = Vec::new();
    for (flag, name) in [
        (g.shadows, "shadows"),
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

/// The graphics setup the overlay describes: the settings, the quality and upscaler in use, the adapter, and the
/// main camera's size.
#[derive(SystemParam)]
struct Setup<'w, 's> {
    g: Res<'w, Graphics>,
    q: Option<Res<'w, Quality>>,
    up: Option<Res<'w, Upscaling>>,
    info: Option<Res<'w, RenderAdapterInfo>>,
    camera: Query<'w, 's, (&'static Camera, Option<&'static MainPassResolutionOverride>), With<MainCamera>>,
}

fn text(
    mut commands: Commands,
    time: Res<Time<Real>>,
    perf: Res<Perf>,
    setup: Setup,
    recording: Res<super::capture::Recording>,
    mut texts: Query<(&Slot, &mut Text, &mut Node), (Without<Table>, Without<Cell>)>,
    mut tables: Query<(Entity, &Slot, &mut Table, &mut Node, Option<&Children>)>,
    mut cells: Query<&mut Text, With<Cell>>,
) {
    if perf.mode == Mode::Off {
        return;
    }
    let mut page = page(&time, &perf, &setup, &recording);
    for (slot, mut text, mut node) in &mut texts {
        let t = page.text(*slot);
        if text.0 != t {
            t.clone_into(&mut text.0);
        }
        show(&mut node, !t.is_empty(), Display::Flex);
    }
    for (entity, slot, mut table, mut node, children) in &mut tables {
        let rows = page.table(*slot);
        show(&mut node, !rows.is_empty(), Display::Grid);
        if table.0 == rows {
            continue;
        }
        let cols = rows.first().map_or(0, Vec::len);
        let same = table.0.len() == rows.len() && table.0.first().map_or(0, Vec::len) == cols;
        match children {
            Some(children) if same => {
                for (&cell, value) in children.iter().zip(rows.iter().flatten()) {
                    if let Ok(mut text) = cells.get_mut(cell)
                        && text.0 != *value
                    {
                        value.clone_into(&mut text.0);
                    }
                }
            }
            _ => {
                node.grid_template_columns = vec![RepeatedGridTrack::auto(cols as u16)];
                commands.entity(entity).despawn_children().with_children(|p| {
                    for (r, row) in rows.iter().enumerate() {
                        for (c, value) in row.iter().enumerate() {
                            p.spawn((
                                Cell,
                                Text::new(value.clone()),
                                font(),
                                TextColor(if r == 0 { HEADER } else { TEXT }),
                                TextLayout::new_with_no_wrap(),
                                Node {
                                    justify_self: if c == 0 { JustifySelf::Start } else { JustifySelf::End },
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                        }
                    }
                });
            }
        }
        table.0 = rows;
    }
}

fn show(node: &mut Node, on: bool, display: Display) {
    let d = if on { display } else { Display::None };
    if node.display != d {
        node.display = d;
    }
}

fn page(time: &Time<Real>, perf: &Perf, setup: &Setup, recording: &super::capture::Recording) -> Page {
    let summary = Summary::of(&perf.recent(2.0), setup.g.vsync);
    let mut head = line(&summary);
    if let Some(r) = recording.0.as_ref() {
        head += &format!("\n● {}", r.status(time.elapsed_secs()));
    }
    let (target, low) = setup
        .camera
        .single()
        .map_or((None, None), |(c, l)| (c.physical_target_size(), l.map(|l| l.0)));
    let page = Page { head, ..default() };
    match perf.mode {
        Mode::Off | Mode::Line => page,
        Mode::Full => {
            let s = Summary::of(&perf.recent(5.0), setup.g.vsync);
            full_page(
                Page {
                    stats: vec![
                        header(&["ms, 5 s", "avg", "p50", "p95", "p99", "max"]),
                        row("frame", Some(s.frame)),
                        row("gpu", s.gpu),
                        row("main", Some(s.main)),
                        row("render", Some(s.render)),
                        row("wait", s.wait),
                        row("sleep", Some(s.sleep)),
                    ],
                    info: format!(
                        "stutters {} in 5 s\n{}",
                        s.stutters,
                        setup_line(
                            &setup.g,
                            setup.q.as_deref(),
                            setup.up.as_deref(),
                            target,
                            low,
                            setup.info.as_deref()
                        )
                    ),
                    ..page
                },
                perf,
                target,
            )
        }
        Mode::Cpu => cpu_page(page, perf),
    }
}

fn full_page(mut page: Page, perf: &Perf, target: Option<UVec2>) -> Page {
    let pixels = target.map_or(1.0, |t| t.element_product().max(1) as f32);
    // (Bevy 0.19 times no shadow pass: their cost shows in the sweep.)
    page.passes.push(header(&[
        "GPU passes (no shadows)",
        "GPU ms",
        "CPU ms",
        "tris",
        "frag/px",
    ]));
    for p in perf.passes.iter().take(14) {
        page.passes.push(vec![
            p.name.clone(),
            ms(p.gpu),
            ms(p.cpu),
            big(p.tris as u64),
            if p.frags > 0.0 {
                format!("{:.2}", p.frags / pixels)
            } else {
                "—".into()
            },
        ]);
    }
    if perf.passes.is_empty() {
        page.foot += "no GPU timers: --no-gpu-timers, or the GPU has no timestamp queries\n";
    }
    if let Some(scene) = &perf.scene {
        page.foot += &scene.line();
        page.foot += "\n";
    }
    page.foot += "F4: profiler | F9: record | Shift+F9: graphics sweep";
    page
}

/// The Cpu page of a game started without `--profiler`: the spans it sums are not there.
const NO_SPANS: &str = "Время систем и расписаний меряется, только если игра запущена с флагом --profiler\n\
                        (или с FB_PROFILER=1): без него профайлер ничего не стоит.\nF4: скрыть | F9: запись";

fn cpu_page(mut page: Page, perf: &Perf) -> Page {
    if !profiler::BUILT {
        page.foot = "the profiler is not in this build (feature `profiler`)".into();
        return page;
    }
    if !perf.profiler {
        page.foot = NO_SPANS.into();
        return page;
    }
    let rows = &perf.cpu;
    let find = |kind: Kind, name: &str| {
        rows.iter()
            .find(|r| r.kind == kind && r.name.ends_with(name))
            .map_or(f32::NAN, |r| r.ms)
    };
    page.info = format!(
        "CPU, ms a frame (1 s): main app {} | render app {} | acquire {} | present {}\nschedules:",
        ms(find(Kind::Frame, "main app")),
        ms(find(Kind::Frame, "RenderApp")),
        ms(find(Kind::System, "::prepare_windows")),
        ms(find(Kind::Frame, "present_frames")),
    );
    for r in rows.iter().filter(|r| r.kind == Kind::Schedule).take(10) {
        page.info += &format!(" {} {:.2} ·", r.name, r.ms);
    }
    page.info.pop();
    page.passes.push(header(&["system", "ms", "runs"]));
    for r in rows
        .iter()
        .filter(|r| matches!(r.kind, Kind::System | Kind::Commands))
        .take(24)
    {
        let tag = if r.kind == Kind::Commands { " (commands)" } else { "" };
        page.passes.push(vec![
            format!("{}{tag}", r.name),
            format!("{:.2}", r.ms),
            format!("{:.1}", r.calls),
        ]);
    }
    if !perf.spikes.is_empty() {
        page.foot += "long frames:\n";
        let now = perf.frames.back().map_or(0.0, |f: &Frame| f.t);
        for sp in perf.spikes.iter().rev().take(4) {
            page.foot += &format!("  {:.0} s ago: {}\n", now - sp.t, sp.line());
        }
    }
    page.foot += "F4: hide | F9: record";
    page
}

/// The graph's frames: the last 256, 30 times a second.
fn graph(perf: Res<Perf>, node: Query<&MaterialNode<GraphMaterial>>, mut graphs: ResMut<Assets<GraphMaterial>>) {
    if perf.mode != Mode::Full {
        return;
    }
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
