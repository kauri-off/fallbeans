//! Native client: connects, enters a room, predicts its bean, interpolates the others, draws the map.
//! `--headless` runs the same game without a window or GPU (stress runs, CI).
mod assets;
mod game;
mod hud;
mod net;
mod opts;
mod session;
mod stats;
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
use bevy::window::PresentMode;
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
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_dir(),
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: opts.title.clone(),
                        resolution: (1280, 720).into(),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(wgpu_settings(opts.backend))),
                    ..default()
                })
                .set(LogPlugin {
                    filter: LOG_FILTER.into(),
                    ..default()
                }),
        );
        app.insert_resource(WinitSettings::continuous());
        if opts.check_assets {
            app.add_plugins(assets::CheckAssetsPlugin);
            return app.run();
        }
        app.add_plugins((view::ViewPlugin, hud::HudPlugin));
        app.add_systems(Update, screenshot);
    }
    app.add_plugins(ClientPlugins { tick_duration: TICK });
    app.add_plugins((ProtocolPlugin, NetStatsPlugin));
    app.add_plugins((
        net::NetPlugin,
        session::SessionPlugin,
        game::GamePlugin,
        stats::StatsPlugin,
    ));
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
fn screenshot(mut commands: Commands, opts: Res<Opts>, time: Res<Time<Real>>, mut done: Local<bool>) {
    let (Some(path), Some(after)) = (&opts.screenshot, opts.exit_after) else {
        return;
    };
    if !*done && time.elapsed_secs() >= after - 1.0 {
        *done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
}
