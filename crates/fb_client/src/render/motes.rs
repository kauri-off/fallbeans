//! Specks of pollen and sparkle drifting around the camera in the look's colour (snow falls, embers rise).
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use crate::game::Map;
use crate::settings::Graphics;

const COUNT: usize = 420;
const BOX: f32 = 44.0;

#[derive(Clone, Copy, Default, ShaderType)]
pub struct MotesUniform {
    pub tint: Vec4,
    pub box_rise: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct MotesMaterial {
    #[uniform(0)]
    pub u: MotesUniform,
}

impl Material for MotesMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/motes.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/motes.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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
            Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![layout];
        descriptor.primitive.cull_mode = None;
        if let Some(d) = &mut descriptor.depth_stencil {
            d.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

#[derive(Component)]
struct Motes;

/// The specks: a quad each, scattered over the box with a seed (any randomness will do: visual only).
fn mesh() -> Mesh {
    let mut x = 0x2545_f491u32;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as f32 / u32::MAX as f32
    };
    let (mut pos, mut corner, mut seed, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..COUNT {
        let p = [rnd() * BOX, rnd() * BOX * 0.5, rnd() * BOX];
        let s = rnd();
        for c in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            pos.push(p);
            corner.push(c);
            seed.push([s, 0.0]);
        }
        let b = (i * 4) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, corner)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, seed)
        .with_inserted_indices(Indices::U32(idx))
}

pub struct MotesPlugin;

impl Plugin for MotesPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "motes.wgsl");
        app.add_plugins(MaterialPlugin::<MotesMaterial>::default());
        app.add_systems(Startup, setup);
        app.add_systems(Update, apply);
    }
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<MotesMaterial>>) {
    commands.spawn((
        Motes,
        Mesh3d(meshes.add(mesh())),
        MeshMaterial3d(mats.add(MotesMaterial {
            u: MotesUniform::default(),
        })),
        Transform::default(),
        Visibility::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// The look's colour and drift; off without a map, and not drawn at all when switched off.
fn apply(
    map: Option<Res<Map>>,
    g: Res<Graphics>,
    mut q: Query<(&MeshMaterial3d<MotesMaterial>, &mut Visibility), With<Motes>>,
    mut mats: ResMut<Assets<MotesMaterial>>,
    mut last: Local<Option<(u32, bool)>>,
) {
    let now = map.as_ref().map(|m| (m.generation, g.motes));
    if *last == now {
        return;
    }
    *last = now;
    let Ok((h, mut vis)) = q.single_mut() else { return };
    // (Without a map they stay drawn, invisibly: that compiles their pipeline before the first round.)
    vis.set_if_neq(if map.is_some() && !g.motes {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
    let Some(mut m) = mats.get_mut(&h.0) else { return };
    m.u = match map {
        Some(map) if g.motes => {
            let c = crate::view::color(map.look.look.motes.color).to_linear();
            MotesUniform {
                tint: Vec4::new(c.red, c.green, c.blue, 1.0),
                box_rise: Vec4::new(BOX, BOX * 0.5, BOX, map.look.look.motes.rise as f32),
            }
        }
        _ => MotesUniform {
            tint: Vec4::ZERO,
            box_rise: Vec4::new(BOX, BOX * 0.5, BOX, 0.0),
        },
    };
}
