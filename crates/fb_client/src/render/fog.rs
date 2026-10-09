//! Haze in the air around the camera, on High: the sky's light and the sun's scattered in it, shafts of
//! sunlight where the course shades it, thicker low down (a sea of mist under the course) and drifting in wisps
//! with the wind. Beyond it the look's distance fog takes over; on Low that fog alone, beginning nearer.
//!
//! Bevy's own volumetric fog marches its rays over the whole target and ignores `MainPassResolutionOverride`
//! (FSR draws the main pass into a corner of it, `fsr.rs`): its rays and the depth it reads would not match. The
//! haze is drawn in the main pass instead, as slices across the view at growing distances, far to near in one
//! draw, each depth-tested against the scene (no prepass needed) and sampling the sun's shadow map once.
//!
//! It leaves the upscalers' reactive mask (`reactive.rs`) as it is: smooth, and over the whole scene, it lies
//! almost as far as what it covers, which keeps it in the history; marked, all far away would shimmer.
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use fb_sim::looks::Look;

use super::quality::{Preset, Quality};
use crate::game::Map;
use crate::view::MainCamera;

/// Slices across the view, from `NEAR` to `REACH` metres, closer together near the eye (where the haze
/// in front of the beans and the course is seen through the fewest).
const SLICES: usize = 24;
const NEAR: f32 = 1.0;
const REACH: f32 = 90.0;

#[derive(Clone, Copy, Default, ShaderType)]
pub struct HazeUniform {
    /// rgb: the sky's light scattered in the haze (linear); a: its density at height 0 (1/m; 0: none).
    pub haze: Vec4,
    /// rgb: the tint of the sunlight scattered in it; a: how much of it.
    pub sun: Vec4,
    /// x: how fast it thins going up (1/m), y: how many times thicker it gets below at most, z: how much of the
    /// sunlight goes on forward (Henyey-Greenstein g: the glow towards the sun), w: drift (m/s).
    pub shape: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct HazeMaterial {
    #[uniform(0)]
    pub u: HazeUniform,
}

impl Material for HazeMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/fog.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/fog.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![layout];
        descriptor.primitive.cull_mode = None;
        if let Some(d) = &mut descriptor.depth_stencil {
            d.depth_write_enabled = Some(false);
        }
        let over = BlendState::PREMULTIPLIED_ALPHA_BLENDING.color;
        super::reactive::blend(descriptor, over, super::reactive::KEEP);
        Ok(())
    }
}

#[derive(Component)]
struct Haze;

/// The slices' bounds: `SLICES + 1` distances from `NEAR` to `REACH`, growing with the square.
fn bounds() -> Vec<f32> {
    (0..=SLICES)
        .map(|j| {
            let f = j as f32 / SLICES as f32;
            NEAR + (REACH - NEAR) * f * f
        })
        .collect()
}

/// A quad across the view per slice, far to near (drawn in that order, each over what is behind it): the corner
/// (−1…1, a little past the edge) and the distance in the position, the thickness in the first UV.
fn mesh() -> Mesh {
    let (mut pos, mut thick, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for w in bounds().windows(2).rev() {
        let d = (w[0] + w[1]) * 0.5;
        let t = w[1] - w[0];
        let base = pos.len() as u32;
        for [x, y] in [[-1.02f32, -1.02], [1.02, -1.02], [1.02, 1.02], [-1.02, 1.02]] {
            pos.push([x, y, d]);
            thick.push([t, 0.0]);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, thick)
        .with_inserted_indices(Indices::U32(idx))
}

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "fog.wgsl");
        app.add_plugins(MaterialPlugin::<HazeMaterial>::default());
        app.add_systems(Startup, setup);
        app.add_systems(Update, apply);
        app.add_systems(
            PostUpdate,
            follow
                .after(crate::camera::place_camera)
                .before(TransformSystems::Propagate),
        );
    }
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<HazeMaterial>>) {
    commands.spawn((
        Haze,
        Mesh3d(meshes.add(mesh())),
        MeshMaterial3d(mats.add(HazeMaterial {
            u: HazeUniform::default(),
        })),
        Transform::default(),
        Visibility::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// The haze of a look: the colour its distance fog fades to, as thick as that fog is near (it begins
/// nearer in thicker looks), the sunlight tinted half way to it.
pub fn haze_of(l: &Look) -> HazeUniform {
    let fog = super::linear(l.fog.color);
    let tint = LinearRgba::WHITE.mix(&fog, 0.5);
    let density = 0.25 / (l.fog.near as f32).max(30.0);
    HazeUniform {
        haze: (fog.to_vec3() * 0.8).extend(density),
        sun: tint.to_vec3().extend(0.16),
        shape: Vec4::new(0.025, 3.0, 0.45, 1.2),
    }
}

/// The look's distance fog (its colour is set with the rest of the look, `apply_look`): from where the look says
/// with the haze in front of it; without the haze (Low), from a little over half as far, a little thicker,
/// for the depth the haze would give.
fn falloff(l: &Look, haze: bool) -> FogFalloff {
    let (near, far) = (l.fog.near as f32, l.fog.far as f32);
    let (start, end) = if haze { (near, far) } else { (near * 0.55, far * 0.9) };
    FogFalloff::Linear { start, end }
}

/// On High only, in the round's look; while there is no map it stays drawn, as thin as nothing (its pipeline
/// compiles with the warm-up's maps all the same). The distance fog as the haze is there or not.
fn apply(
    map: Option<Res<Map>>,
    quality: Option<Res<Quality>>,
    mut q: Query<(&MeshMaterial3d<HazeMaterial>, &mut Visibility), With<Haze>>,
    mut mats: ResMut<Assets<HazeMaterial>>,
    mut fog: Query<&mut DistanceFog, With<MainCamera>>,
    mut last: Local<Option<(Option<u32>, bool)>>,
) {
    let on = quality.is_some_and(|q| q.preset != Preset::Low);
    let now = (map.as_ref().map(|m| m.generation), on);
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    if let (Some(map), Ok(mut fog)) = (&map, fog.single_mut()) {
        fog.falloff = falloff(map.look.look, on);
    }
    let Ok((h, mut vis)) = q.single_mut() else { return };
    vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    let Some(mut m) = mats.get_mut(&h.0) else { return };
    m.u = match &map {
        Some(map) => haze_of(map.look.look),
        None => HazeUniform::default(),
    };
}

/// With the camera: the slices are placed in the view by the shader, this only sorts the haze after every other
/// see-through thing (nearest last), so it lies over them too.
fn follow(camera: Query<&Transform, (With<MainCamera>, Without<Haze>)>, mut haze: Query<&mut Transform, With<Haze>>) {
    let (Ok(cam), Ok(mut tf)) = (camera.single(), haze.single_mut()) else {
        return;
    };
    tf.translation = cam.translation;
}

#[cfg(test)]
mod tests {
    use bevy::mesh::VertexAttributeValues;

    use super::*;

    #[test]
    fn slices_fill_the_reach_far_to_near() {
        let b = bounds();
        assert_eq!(b.len(), SLICES + 1);
        assert!((b[0] - NEAR).abs() < 1e-5 && (b[SLICES] - REACH).abs() < 1e-3);
        let m = mesh();
        let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("no positions");
        };
        let Some(VertexAttributeValues::Float32x2(thick)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("no thickness");
        };
        assert_eq!(pos.len(), SLICES * 4);
        // Every slice's thickness once: they add up to the whole reach.
        let total: f32 = thick.iter().step_by(4).map(|t| t[0]).sum();
        assert!((total - (REACH - NEAR)).abs() < 1e-3, "{total}");
        // Far to near.
        assert!(pos.windows(5).step_by(4).all(|w| w[0][2] > w[4][2]));
    }

    #[test]
    fn every_look_has_some_haze() {
        for l in fb_sim::looks::LOOKS {
            let h = haze_of(l);
            assert!(h.haze.w > 0.0 && h.haze.w < 0.01, "{:?}: {}", l.id, h.haze.w);
            // A hundred metres of it at height 0 hide less than half of what is behind.
            assert!((-h.haze.w * 100.0).exp() > 0.5, "{:?}", l.id);
        }
    }
}
