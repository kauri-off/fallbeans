//! Native client: connects, enters a room, predicts its bean, interpolates the others, draws the map.
//! `--headless` runs the same game without a window or GPU (stress runs).
mod assets;
mod audio;
mod backend;
mod bean;
mod beans;
#[cfg(feature = "brp")]
mod brp;
mod camera;
mod clicks;
mod clock;
mod crash;
mod diag;
mod face;
mod game;
#[cfg(test)]
mod harness;
mod hud;
mod keys;
mod logs;
#[cfg(test)]
mod monkey;
mod net;
mod opts;
mod outfit;
mod perf;
mod probe;
mod render;
mod report;
#[cfg(test)]
mod scenarios;
mod servers;
mod session;
mod settings;
mod shapes;
mod specials;
mod stats;
mod transforms;
mod ui;
mod update;
mod view;
mod watch;

use core::time::Duration;
use std::path::PathBuf;

use bevy::app::{ScheduleRunnerPlugin, TaskPoolOptions, TaskPoolPlugin};
use bevy::asset::AssetMetaCheck;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::RenderCreation;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{ExitCondition, MonitorSelection, PresentMode, WindowMode};
use bevy::winit::WinitSettings;
use clap::Parser;
use fb_net::{NetStatsPlugin, ProtocolPlugin, TICK};
use lightyear::prelude::client::ClientPlugins;

use crate::opts::Opts;

const LOG_FILTER: &str = "wgpu=error,bevy_ecs=warn,lightyear=warn,aeronet=warn";
/// `--profiler`: the spans of systems and schedules (`bevy_ecs` above), for the built-in profiler.
const PROFILER_SPANS: &str = ",bevy_ecs::system::function_system=info,bevy_ecs::schedule::schedule=info";
/// Without it, Bevy's per-frame spans of the app and the render world go too (a span filtered out where it is
/// made costs nothing). `bevy_render::renderer` stays: `present_frames` is the swapchain wait of every frame.
const NO_PROFILER_SPANS: &str = ",bevy_app::sub_app=warn,bevy_render::batching=warn,bevy_render::extract_plugin=warn,bevy_render::pipelined_rendering=warn,bevy_render::renderer::render_context=warn,bevy_render::view::visibility=warn";

/// The log filter: which spans exist is decided here, at the start (a system's span is made once).
fn log_filter(profiler: bool) -> String {
    let spans = if profiler { PROFILER_SPANS } else { NO_PROFILER_SPANS };
    format!("{LOG_FILTER}{spans}")
}

/// The console log: events only (spans would prefix every line with the system it came from).
fn fmt_layer(_: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    use bevy::log::tracing_subscriber::{Layer, filter::FilterFn, fmt};
    Some(Box::new(
        fmt::Layer::default()
            .with_writer(std::io::stderr)
            .with_filter(FilterFn::new(|m| m.is_event())),
    ))
}

/// Where the assets are: next to the executable in a distribution, else the workspace's.
fn asset_dir() -> String {
    if let Ok(dir) = std::env::var("BEVY_ASSET_ROOT") {
        return PathBuf::from(dir).join("assets").to_string_lossy().into();
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("assets")))
        && dir.join("models").is_dir()
    {
        return dir.to_string_lossy().into();
    }
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").into()
}

fn main() -> AppExit {
    if let Some(code) = render::quality::intel_gen9_relaunch() {
        return AppExit::from_code(code);
    }
    let mut app = App::new();
    build(&mut app, Opts::parse(), None);
    let exit = app.run();
    logs::flush();
    exit
}

/// The whole client. `noop`: tests, with wgpu's noop device and a window nothing opens; they step the app.
fn build(app: &mut App, opts: Opts, noop: Option<RenderCreation>) {
    let test = noop.is_some();
    let profiler = opts.profiler();
    // (First: the graphics API may come from the settings.)
    app.add_plugins(settings::ClientSettingsPlugin {
        profile: opts.profile.clone(),
        stored: !opts.headless && !test,
    });
    app.insert_resource(logs::Logs(
        settings::dir(opts.profile.as_deref())
            .filter(|_| !opts.headless && !test)
            .map(|d| d.join("logs")),
    ));
    app.add_plugins(crash::CrashPlugin);
    if opts.headless {
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / opts.fps))),
            LogPlugin {
                filter: log_filter(profiler),
                fmt_layer,
                ..default()
            },
            bevy::state::app::StatesPlugin,
        ));
    } else {
        // (Before the renderer: the graphics API, from the GPUs this machine has.)
        let render_creation = match noop {
            Some(noop) => noop,
            None => RenderCreation::Automatic(Box::new(backend::choose(app, &opts))),
        };
        let window = if opts.offscreen {
            None
        } else {
            Some(Window {
                title: opts.title.clone(),
                // The Wayland app id / X11 class: the desktop file and its icon.
                name: Some("io.github.kauri_off.fallbeans".into()),
                resolution: (1280, 720).into(),
                present_mode: PresentMode::AutoVsync,
                mode: if opts.fullscreen {
                    WindowMode::BorderlessFullscreen(MonitorSelection::Current)
                } else {
                    WindowMode::Windowed
                },
                ..default()
            })
        };
        let plugins = DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_dir(),
                meta_check: AssetMetaCheck::Never,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: window,
                exit_condition: if opts.offscreen || test {
                    ExitCondition::DontExit
                } else {
                    ExitCondition::OnPrimaryClosed
                },
                ..default()
            })
            .set(RenderPlugin {
                render_creation,
                ..default()
            })
            .set(LogPlugin {
                filter: log_filter(profiler),
                custom_layer: logs::layer,
                fmt_layer,
                ..default()
            });
        // At most 4 compute threads (Bevy takes all the cores the IO and async pools leave): every parallel query
        // and scope of the frame wakes them all and waits, spinning, for the last; the scene's work is small. With
        // 12 on a 20-thread CPU the client burned ~15 ms of CPU a frame, 10 with 4, at the same frame rate.
        let mut pools = TaskPoolOptions::default();
        pools.compute.max_threads = std::env::var("FB_COMPUTE_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(4);
        let plugins = plugins.set(TaskPoolPlugin {
            task_pool_options: pools,
        });
        if test {
            // (Silent: tests must not play through the speakers. The render world on the main thread: tests
            // read its pipelines.)
            app.add_plugins(
                plugins
                    .disable::<bevy::winit::WinitPlugin>()
                    .disable::<bevy::audio::AudioPlugin>()
                    .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
            );
        } else if opts.offscreen {
            // No window at all: the frames go to an image (`--screenshot`, `fb/shot`).
            app.add_plugins((
                plugins.disable::<bevy::winit::WinitPlugin>(),
                ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / opts.fps)),
            ));
            app.add_systems(PreStartup, offscreen_target);
        } else {
            app.add_plugins(plugins);
            // (In the background at most 60 frames a second, not as many as it can: laptops' batteries. The
            // loop still runs at that rate without events, so the network and the fixed ticks carry on.)
            app.insert_resource(WinitSettings {
                focused_mode: bevy::winit::UpdateMode::Continuous,
                unfocused_mode: bevy::winit::UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 60.0)),
            });
        }
        app.insert_resource(bevy::render::error_handler::RenderErrorHandler(crash::render_failed));
        if opts.check_assets {
            app.add_plugins(assets::CheckAssetsPlugin);
            return;
        }
        app.add_plugins(transforms::TransformsPlugin);
        app.add_plugins((
            view::ViewPlugin,
            beans::BeansPlugin,
            hud::HudPlugin,
            logs::LogsPlugin,
            report::ReportPlugin,
            ui::UiPlugin,
            camera::CameraPlugin,
            render::GfxPlugin {
                // (Off on the bench, where every test would build every map first; `--warmup` there tests it.)
                warmup: !opts.no_warmup && (!test || opts.warmup),
            },
            face::FacePlugin,
            perf::PerfPlugin {
                gpu_timers: !opts.no_gpu_timers && !test,
                profiler,
            },
        ));
        if opts.trace_clicks {
            app.add_plugins(clicks::ClickTracePlugin);
        }
        if !opts.offscreen && !test {
            app.add_plugins((audio::AudioPlugin, update::UpdatePlugin));
        }
        app.add_systems(Update, screenshot);
    }
    app.add_plugins(ClientPlugins { tick_duration: TICK });
    app.add_plugins((ProtocolPlugin, NetStatsPlugin, fb_net::errors::ErrorPolicyPlugin));
    app.add_plugins((
        net::NetPlugin,
        clock::ClockPlugin,
        diag::DiagPlugin,
        session::SessionPlugin,
        game::GamePlugin,
        stats::StatsPlugin,
        watch::WatchPlugin,
    ));
    #[cfg(feature = "brp")]
    if let Some(port) = opts.brp {
        app.add_plugins(brp::BrpPlugin { port });
    }
    app.add_systems(Update, (exit_after, servers::poll));
    app.insert_resource(servers::Target(opts.direct()));
    app.init_resource::<servers::States>();
    app.insert_resource(opts);
}

fn exit_after(opts: Res<Opts>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if opts.exit_after.is_some_and(|s| time.elapsed_secs() >= s) {
        exit.write(AppExit::Success);
    }
}

/// `--screenshot`: a picture of the game a second before `--exit-after` (smoke tests).
fn screenshot(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    offscreen: Option<Res<Offscreen>>,
    mut done: Local<bool>,
) {
    let (Some(path), Some(after)) = (&opts.screenshot, opts.exit_after) else {
        return;
    };
    if !*done && time.elapsed_secs() >= after - 1.0 {
        *done = true;
        commands
            .spawn(shot_of(offscreen.as_deref()))
            .observe(save_to_disk(path.clone()));
    }
}

/// `--offscreen`: the image the game is drawn to instead of a window.
#[derive(Resource)]
pub struct Offscreen(pub Handle<Image>);

const OFFSCREEN_SIZE: (u32, u32) = (1600, 900);

fn offscreen_target(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    use bevy::render::render_resource::{TextureFormat, TextureUsages};
    let mut image = Image::new_target_texture(OFFSCREEN_SIZE.0, OFFSCREEN_SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    commands.insert_resource(Offscreen(images.add(image)));
}

/// A screenshot of what the game shows (the window, or the offscreen image).
pub fn shot_of(offscreen: Option<&Offscreen>) -> Screenshot {
    match offscreen {
        Some(o) => Screenshot::image(o.0.clone()),
        None => Screenshot::primary_window(),
    }
}
