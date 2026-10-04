//! Shader warm-up: at the room list a few tiny objects in front of the camera use every
//! kind of material the game draws, so their pipelines are compiled before the first round instead of
//! things popping in as it starts. They go once the game has been running a few seconds.
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::view::MainCamera;

/// How long the warm-up objects stay (s of real time).
const KEEP_S: f32 = 4.0;

#[derive(Component)]
struct Warm;

pub struct WarmupPlugin;

impl Plugin for WarmupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn.after(crate::view::setup_camera));
        app.add_systems(Update, despawn);
    }
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
        commands.spawn((
            Warm,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(m),
            at(i),
            NotShadowCaster,
            ChildOf(cam),
        ));
    }
    for (i, m) in plain.into_iter().enumerate() {
        commands.spawn((Warm, Mesh3d(mesh.clone()), MeshMaterial3d(m), at(10 + i), ChildOf(cam)));
    }
}

fn despawn(mut commands: Commands, q: Query<Entity, With<Warm>>, time: Res<Time<Real>>) {
    if time.elapsed_secs() < KEEP_S {
        return;
    }
    for e in &q {
        commands.entity(e).despawn();
    }
}
