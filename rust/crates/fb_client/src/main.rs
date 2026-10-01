//! Native client (phase 0): connects, predicts its bean, interpolates the others, draws jump-club.
mod game;
mod hud;
mod net;
mod opts;
mod view;

use std::path::PathBuf;

use bevy::asset::{AssetMetaCheck, LoadState};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PresentMode;
use bevy::winit::WinitSettings;
use fb_net::{ProtocolPlugin, TICK};
use lightyear::prelude::client::ClientPlugins;

use crate::opts::Opts;

/// Where the assets are: next to the executable in a distribution, else the workspace's.
fn asset_dir() -> String {
    if let Ok(dir) = std::env::var("BEVY_ASSET_ROOT") {
        return PathBuf::from(dir).join("assets").to_string_lossy().into();
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("assets")))
        && dir.join("models").is_dir()
    {
        return dir.to_string_lossy().into();
    }
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").into()
}

fn backends(name: Option<&str>) -> Option<Backends> {
    match name? {
        "vulkan" => Some(Backends::VULKAN),
        "dx12" => Some(Backends::DX12),
        "gl" => Some(Backends::GL),
        "metal" => Some(Backends::METAL),
        _ => None,
    }
}

fn main() {
    let opts = opts::parse();
    let mut wgpu = WgpuSettings {
        backends: backends(opts.backend.as_deref()).or(WgpuSettings::default().backends),
        ..default()
    };
    if wgpu.backends == Some(Backends::GL) {
        // GL 3.3 has no compute shaders: wgpu's indirect-call validation would fail device creation.
        wgpu.instance_flags.remove(InstanceFlags::VALIDATION_INDIRECT_CALL);
    }
    let mut app = App::new();
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
                render_creation: RenderCreation::Automatic(Box::new(wgpu)),
                ..default()
            }),
    );
    app.insert_resource(WinitSettings::continuous());
    if opts.check_assets {
        app.insert_resource(opts);
        app.add_systems(Startup, load_models);
        app.add_systems(Update, report_models);
        app.run();
        return;
    }
    app.add_plugins(ClientPlugins { tick_duration: TICK });
    app.add_plugins(ProtocolPlugin);
    app.add_plugins((net::NetPlugin, game::GamePlugin, view::ViewPlugin, hud::HudPlugin));
    app.add_systems(Update, smoke_test);
    app.insert_resource(opts);
    app.run();
}

/// `--screenshot` / `--exit-after`: a picture of the game after a while, then quit (smoke tests).
fn smoke_test(mut commands: Commands, opts: Res<Opts>, time: Res<Time>, mut shot: Local<bool>, mut exit: MessageWriter<AppExit>) {
    let Some(after) = opts.exit_after else { return };
    let t = time.elapsed_secs();
    if let Some(path) = &opts.screenshot
        && !*shot
        && t >= after - 1.0
    {
        *shot = true;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
    }
    if t >= after {
        exit.write(AppExit::Success);
    }
}

#[derive(Resource)]
struct Models(Vec<(&'static str, Handle<Gltf>)>);

/// `--check-assets`: loads every model through Bevy's glTF loader and reports.
fn load_models(mut commands: Commands, assets: Res<AssetServer>) {
    let list = fb_sim::builder::MODEL_NAMES
        .iter()
        .map(|n| (*n, assets.load(format!("models/{n}.glb"))))
        .collect();
    commands.insert_resource(Models(list));
}

fn report_models(
    models: Res<Models>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    time: Res<Time>,
    mut done: Local<Option<(f32, u8)>>,
    mut exit: MessageWriter<AppExit>,
) {
    // Quitting while the driver still compiles pipelines crashes it (NVIDIA 615): let it finish first.
    if let Some((at, code)) = *done {
        if time.elapsed_secs() - at > 3.0 {
            exit.write(if code == 0 { AppExit::Success } else { AppExit::from_code(code) });
        }
        return;
    }
    let states: Vec<_> = models.0.iter().map(|(n, h)| (*n, assets.load_state(h))).collect();
    if states.iter().any(|(_, s)| matches!(s, LoadState::Loading | LoadState::NotLoaded)) {
        return;
    }
    let mut failed = 0;
    for ((name, state), (_, h)) in states.iter().zip(&models.0) {
        match state {
            LoadState::Loaded => {
                let g = gltfs.get(h).unwrap();
                let prims: usize = g.meshes.iter().filter_map(|m| meshes.get(m)).map(|m| m.primitives.len()).sum();
                println!(
                    "ok      {name:<9} scenes {} nodes {:>3} meshes {:>3} primitives {:>3} materials {:>2}",
                    g.scenes.len(),
                    g.nodes.len(),
                    g.meshes.len(),
                    prims,
                    g.materials.len()
                );
            }
            LoadState::Failed(e) => {
                failed += 1;
                println!("FAILED  {name:<9} {e}");
            }
            _ => {}
        }
    }
    println!("{} of {} models load in Bevy", states.len() - failed, states.len());
    *done = Some((time.elapsed_secs(), u8::from(failed > 0)));
}
