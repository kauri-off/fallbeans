//! Native client: connects, enters a room, predicts its bean, interpolates the others, draws the map.
//! `--headless` runs the same game without a window or GPU (stress runs, CI).
mod assets;
mod audio;
mod bean;
mod beans;
#[cfg(feature = "brp")]
mod brp;
mod camera;
mod clock;
mod diag;
mod face;
mod game;
mod hud;
mod keys;
mod net;
mod opts;
mod outfit;
mod probe;
mod render;
mod session;
mod settings;
mod shapes;
mod specials;
mod stats;
mod ui;
mod view;

use core::time::Duration;
use std::path::PathBuf;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetMetaCheck;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{Backends, InstanceFlags, RenderCreation, WgpuLimits, WgpuSettings};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{ExitCondition, PresentMode};
use bevy::winit::WinitSettings;
use clap::Parser;
use fb_net::{NetStatsPlugin, ProtocolPlugin, TICK};
use lightyear::prelude::client::ClientPlugins;

use crate::opts::{Backend, Opts};

const LOG_FILTER: &str = "wgpu=error,bevy_ecs=warn,lightyear=warn,aeronet=warn";

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

fn wgpu_settings(backend: Option<Backend>) -> WgpuSettings {
    let backends = backend.map(|b| match b {
        Backend::Vulkan => Backends::VULKAN,
        Backend::Dx12 => Backends::DX12,
        Backend::Gl => Backends::GL,
    });
    let mut wgpu = WgpuSettings {
        backends: backends.or(WgpuSettings::default().backends),
        ..default()
    };
    if wgpu.backends == Some(Backends::GL) {
        // GL 3.3 has no compute shaders: wgpu's indirect-call validation would fail device creation.
        wgpu.instance_flags.remove(InstanceFlags::VALIDATION_INDIRECT_CALL);
        // Bevy compiles the SSAO compute pipelines up front unless this limit is under 5, and naga's GLSL
        // for them does not compile (textureGatherOffset on a depth texture). T0 has no AO anyway.
        wgpu.constrained_limits = Some(WgpuLimits {
            max_storage_textures_per_shader_stage: 4,
            ..default()
        });
    }
    wgpu
}

fn main() -> AppExit {
    let opts = Opts::parse();
    let mut app = App::new();
    // (First: the graphics API may come from the settings.)
    app.add_plugins(settings::ClientSettingsPlugin {
        profile: opts.profile.clone(),
        stored: !opts.headless,
    });
    if opts.headless {
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / opts.fps))),
            LogPlugin {
                filter: LOG_FILTER.into(),
                ..default()
            },
            bevy::state::app::StatesPlugin,
        ));
    } else {
        let backend = opts
            .backend
            .or_else(|| match app.world().resource::<settings::Graphics>().backend.as_str() {
                "vulkan" => Some(Backend::Vulkan),
                "dx12" => Some(Backend::Dx12),
                "gl" => Some(Backend::Gl),
                _ => None,
            });
        let window = if opts.offscreen {
            None
        } else {
            Some(Window {
                title: opts.title.clone(),
                resolution: (1280, 720).into(),
                present_mode: PresentMode::AutoVsync,
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
                exit_condition: if opts.offscreen {
                    ExitCondition::DontExit
                } else {
                    ExitCondition::OnPrimaryClosed
                },
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(wgpu_settings(backend))),
                ..default()
            })
            .set(LogPlugin {
                filter: LOG_FILTER.into(),
                ..default()
            });
        if opts.offscreen {
            // No window at all: the frames go to an image (`--screenshot`, `fb/shot`).
            app.add_plugins((
                plugins.disable::<bevy::winit::WinitPlugin>(),
                ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)),
            ));
            app.add_systems(PreStartup, offscreen_target);
        } else {
            app.add_plugins(plugins);
            app.insert_resource(WinitSettings::continuous());
        }
        if opts.check_assets {
            app.add_plugins(assets::CheckAssetsPlugin);
            return app.run();
        }
        app.add_plugins((
            view::ViewPlugin,
            beans::BeansPlugin,
            hud::HudPlugin,
            ui::UiPlugin,
            camera::CameraPlugin,
            render::GfxPlugin,
            face::FacePlugin,
        ));
        if !opts.offscreen {
            app.add_plugins(audio::AudioPlugin);
        }
        app.add_systems(Update, screenshot);
    }
    app.add_plugins(ClientPlugins { tick_duration: TICK });
    app.add_plugins((ProtocolPlugin, NetStatsPlugin));
    app.add_plugins((
        net::NetPlugin,
        clock::ClockPlugin,
        diag::DiagPlugin,
        session::SessionPlugin,
        game::GamePlugin,
        stats::StatsPlugin,
    ));
    #[cfg(feature = "brp")]
    if let Some(port) = opts.brp {
        app.add_plugins(brp::BrpPlugin { port });
    }
    app.add_systems(Update, exit_after);
    app.insert_resource(opts);
    app.run()
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
