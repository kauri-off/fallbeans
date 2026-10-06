//! Shader warm-up: at the room list a few tiny objects in front of the camera use every
//! kind of material the game draws, so their pipelines are compiled before the first round instead of
//! things popping in as it starts. They go once the pipeline cache has had nothing left to compile for a
//! while (or after `MAX_S` whatever it says).
//!
//! The camera already has what a round gives it (an environment map, the preset's prepass and shadow
//! filter), the objects cast shadows (the shadow pipelines), and their mesh has the models' vertex layout
//! (position, normal, UV: every glTF model of the game, every primitive): the pipelines match the round's.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};

use super::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::view::MainCamera;

/// The objects stay at least this long (s of real time)…
const MIN_S: f32 = 1.5;
/// …and at most this long, compiled or not.
const MAX_S: f32 = 15.0;
/// Frames in a row with nothing to compile before they go.
const QUIET_FRAMES: u32 = 20;

#[derive(Component)]
struct Warm;

/// Pipelines the render world's cache still waits for (written there each frame, read here).
#[derive(Resource, Clone, Default)]
pub struct Compiling(Arc<AtomicUsize>);

impl Compiling {
    /// Pipelines compiling as of the render world's last frame.
    pub fn now(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct WarmupPlugin;

impl Plugin for WarmupPlugin {
    fn build(&self, app: &mut App) {
        let compiling = Compiling::default();
        app.insert_resource(compiling.clone());
        app.add_systems(Startup, spawn.after(crate::view::setup_camera));
        app.add_systems(Update, despawn);
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.insert_resource(compiling);
            r.add_systems(Render, count_compiling.in_set(RenderSystems::Cleanup));
        }
    }
}

fn count_compiling(cache: Res<PipelineCache>, compiling: Res<Compiling>) {
    compiling.0.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

fn spawn(
    mut commands: Commands,
    camera: Query<Entity, With<MainCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(cam) = camera.single() else { return };
    let mesh = meshes.add(super::meshes::rounded_box(Vec3::ONE, 2, 0.1));
    let mut mats: Vec<Handle<SurfaceMaterial>> = Vec::new();
    for spec in [
        Spec::plain(LinearRgba::WHITE, Some(Kind::Plastic)),
        Spec {
            paint: Some(Paint {
                c1: LinearRgba::WHITE,
                c2: LinearRgba::BLACK,
                freq: 1.0,
                dir: Vec2::ONE,
                speed: 0.0,
                kind: fb_sim::looks::Pattern::Stripes,
            }),
            ..Spec::plain(LinearRgba::WHITE, Some(Kind::Padded))
        },
        Spec {
            alpha: AlphaMode::Blend,
            ..Spec::plain(LinearRgba::new(1.0, 1.0, 1.0, 0.5), Some(Kind::Glass))
        },
    ] {
        mats.push(surfaces.material(&spec, &mut images, &mut surface_mats));
    }
    let plain: Vec<Handle<StandardMaterial>> = [
        // (Also the beans' bodies on Low, without their clearcoat.)
        StandardMaterial::default(),
        StandardMaterial {
            unlit: true,
            ..default()
        },
        StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            ..default()
        },
        StandardMaterial {
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        },
        // The beans' bodies above Low.
        StandardMaterial {
            clearcoat: 0.3,
            ..default()
        },
    ]
    .into_iter()
    .map(|m| standard.add(m))
    .collect();
    // In front of the camera, far too small to see.
    let at = |i: usize| Transform::from_xyz(i as f32 * 0.002, 0.0, -1.0).with_scale(Vec3::splat(0.0005));
    for (i, m) in mats.into_iter().enumerate() {
        commands.spawn((Warm, Mesh3d(mesh.clone()), MeshMaterial3d(m), at(i), ChildOf(cam)));
    }
    for (i, m) in plain.into_iter().enumerate() {
        commands.spawn((Warm, Mesh3d(mesh.clone()), MeshMaterial3d(m), at(10 + i), ChildOf(cam)));
    }
}

fn despawn(
    mut commands: Commands,
    q: Query<Entity, With<Warm>>,
    time: Res<Time<Real>>,
    compiling: Res<Compiling>,
    mut quiet: Local<u32>,
) {
    if q.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    *quiet = if compiling.0.load(Ordering::Relaxed) == 0 {
        *quiet + 1
    } else {
        0
    };
    if t < MIN_S || (*quiet < QUIET_FRAMES && t < MAX_S) {
        return;
    }
    if t >= MAX_S && *quiet < QUIET_FRAMES {
        warn!("warm-up: pipelines still compiling after {MAX_S} s");
    }
    for e in &q {
        commands.entity(e).despawn();
    }
}
