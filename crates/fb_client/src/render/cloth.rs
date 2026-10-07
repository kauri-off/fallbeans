//! Cloth: the flags' pennants wave in the wind. A surface material (`surface.rs`, the same fragment
//! stage) with a vertex stage that bends the pennant by a wave running from the pole to the tip, the
//! normals with it; the prepass does the same, so the shadows wave too. The wind is one for the whole
//! world: gusts roll across it along `WIND` (`gust`), and the trees sway with them (`props.rs`).
use bevy::asset::embedded_asset;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use super::surface::{Kind, Spec, SurfaceMaterial, SurfaceUniform, Surfaces};
use crate::view::MainCamera;

pub type ClothMaterial = ExtendedMaterial<StandardMaterial, Cloth>;

/// Where the wind blows (world x, z; `WIND` in `cloth.wgsl`).
pub const WIND: Vec2 = Vec2::new(0.9439, 0.3303);

/// The wind's strength at a place (0.2…1): gusts rolling across the world along `WIND`, on the
/// renderer's clock (`Time::elapsed_secs_wrapped`; `gust` in `cloth.wgsl`).
pub fn gust(t: f32, p: Vec2) -> f32 {
    let d = p.dot(WIND);
    0.6 + 0.25 * (t * 0.9 - d * 0.11).sin() + 0.15 * (t * 2.3 - d * 0.31 + 1.7).sin()
}

// flag.glb's pennant: from the pole at x = 0.07 to the tip at x = 2.07, between y = 2.8 and 4, 0.04 thick.
const POLE_X: f32 = 0.07;
const LENGTH: f32 = 2.0;
/// Swing across the cloth at the tip, in full wind (model units).
const SWING: f32 = 0.2;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct ClothUniform {
    /// Pole edge along the length (local x), 1 / length, swing at the tip, waves along the length.
    pub shape: Vec4,
    /// Wave speed (rad/s), flutter share, phase per unit of height (rad), unused.
    pub motion: Vec4,
}

const PENNANT: ClothUniform = ClothUniform {
    shape: Vec4::new(POLE_X, 1.0 / LENGTH, SWING, 1.3),
    motion: Vec4::new(7.0, 0.18, 0.9, 0.0),
};

/// The pennant's bounds with its swing (the mesh's own are flat: culled while still in view).
pub fn pennant_bounds() -> (Aabb, NoAutoAabb) {
    let z = SWING * 1.2 + 0.08;
    (
        Aabb::from_min_max(Vec3::new(0.0, 2.7, -z), Vec3::new(POLE_X + LENGTH + 0.1, 4.1, z)),
        NoAutoAabb,
    )
}

/// A surface (bindings 100–102 as `Surface`) that waves (103).
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct Cloth {
    #[uniform(100)]
    pub u: SurfaceUniform,
    #[texture(101)]
    #[sampler(102)]
    pub detail: Handle<Image>,
    #[uniform(103)]
    pub wave: ClothUniform,
}

impl MaterialExtension for Cloth {
    fn vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/cloth.wgsl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/cloth.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/surface.wgsl".into()
    }

    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The fragment shader reads the mesh's transform (object-space mapping), as a surface's.
        let def = "VERTEX_OUTPUT_INSTANCE_INDEX";
        descriptor.vertex.shader_defs.push(def.into());
        if let Some(f) = &mut descriptor.fragment {
            f.shader_defs.push(def.into());
        }
        Ok(())
    }
}

/// The waving twin of each surface material a pennant wears (kept in step with it by `follow`).
#[derive(Resource, Default)]
pub struct Cloths(Vec<(Handle<SurfaceMaterial>, Handle<ClothMaterial>)>);

impl Cloths {
    pub fn pennant(
        &mut self,
        surface: &Handle<SurfaceMaterial>,
        surfaces: &Assets<SurfaceMaterial>,
        cloths: &mut Assets<ClothMaterial>,
    ) -> Option<Handle<ClothMaterial>> {
        if let Some((_, c)) = self.0.iter().find(|(s, _)| s == surface) {
            return Some(c.clone());
        }
        let src = surfaces.get(surface)?;
        let c = cloths.add(ExtendedMaterial {
            base: src.base.clone(),
            extension: Cloth {
                u: src.extension.u,
                detail: src.extension.detail.clone(),
                wave: PENNANT,
            },
        });
        self.0.push((surface.clone(), c.clone()));
        Some(c)
    }

    /// Forgets the twins of surface materials no longer made for models.
    pub fn retain(&mut self, keep: impl Fn(AssetId<SurfaceMaterial>) -> bool) {
        self.0.retain(|(s, _)| keep(s.id()));
    }
}

/// The surfaces' switches (the Low preset's plain look) carried over to their twins.
fn follow(cloths: Res<Cloths>, surfaces: Res<Assets<SurfaceMaterial>>, mut mats: ResMut<Assets<ClothMaterial>>) {
    for (s, c) in &cloths.0 {
        let Some(src) = surfaces.get(s) else { continue };
        if mats
            .get(c)
            .is_some_and(|m| m.extension.u.extra != src.extension.u.extra)
            && let Some(mut m) = mats.get_mut(c)
        {
            m.extension.u = src.extension.u;
        }
    }
}

/// How long the warm-up pennant stays (s of real time), as `warmup.rs`'s objects.
const KEEP_S: f32 = 4.0;

#[derive(Component)]
struct Warm;

/// A tiny pennant in front of the camera at the start: its pipelines (and its shadow's) compile before
/// the first round.
fn warm_up(
    mut commands: Commands,
    camera: Query<Entity, With<MainCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
    mut cloths: ResMut<Cloths>,
    mut cloth_mats: ResMut<Assets<ClothMaterial>>,
) {
    let Ok(cam) = camera.single() else { return };
    let s = surfaces.material(
        &Spec::plain(LinearRgba::WHITE, Some(Kind::Cloth)),
        &mut images,
        &mut surface_mats,
    );
    let Some(m) = cloths.pennant(&s, &surface_mats, &mut cloth_mats) else {
        return;
    };
    commands.spawn((
        Warm,
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(m),
        Transform::from_xyz(0.03, 0.0, -1.0).with_scale(Vec3::splat(0.0005)),
        ChildOf(cam),
    ));
}

fn cool_down(mut commands: Commands, q: Query<Entity, With<Warm>>, time: Res<Time<Real>>) {
    if time.elapsed_secs() < KEEP_S {
        return;
    }
    for e in &q {
        commands.entity(e).despawn();
    }
}

pub struct ClothPlugin;

impl Plugin for ClothPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "cloth.wgsl");
        app.add_plugins(MaterialPlugin::<ClothMaterial>::default());
        app.init_resource::<Cloths>();
        app.add_systems(Startup, warm_up.after(crate::view::setup_camera));
        app.add_systems(Update, (follow, cool_down));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gusts_stay_in_range() {
        for i in 0..2000 {
            let t = i as f32 * 0.37;
            let g = gust(t, Vec2::new(i as f32 * 1.3 - 900.0, i as f32 * -0.7));
            assert!((0.19..=1.01).contains(&g), "{g}");
        }
    }
}
