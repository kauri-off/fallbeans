//! Portal discs, drawn by `portal.wgsl`: a way in is a vortex in the portal's colour (spiral arms winding in,
//! a tunnel of bands sinking towards a white-hot core, sparks drawn in, a glowing rim, a slow pulse); a one-way
//! exit is rings flowing out of a deep middle, sparks thrown out. A trip's flash (the special's tone)
//! brightens the disc and spins it up. Animated on the GPU's clock: one draw a disc, nothing done on the CPU.
//! Its opacity marks the upscalers' reactive mask (`reactive.rs`): it moves with no motion vectors.
//! Also the exit's arrow.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

#[derive(Clone, Copy, Default, ShaderType)]
pub struct PortalUniform {
    /// rgb: the portal's colour (linear); a: the disc's opacity.
    pub color: Vec4,
    /// x: 0 a way in (vortex), 1 an exit (rings); y: flash (0…1); z: phase (portals of other colours pulse out
    /// of step); w: unused.
    pub params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct PortalMaterial {
    #[uniform(0)]
    pub u: PortalUniform,
}

impl PortalMaterial {
    /// A disc in `color`: a way in or an exit, its flash (0…1) and opacity.
    pub fn new(color: Color, exit: bool, flash: f32, alpha: f32, phase: f32) -> Self {
        let c = color.to_linear();
        Self {
            u: PortalUniform {
                color: Vec4::new(c.red, c.green, c.blue, alpha),
                params: Vec4::new(if exit { 1.0 } else { 0.0 }, flash, phase, 0.0),
            },
        }
    }
}

impl Material for PortalMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/portal.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from both sides.
        descriptor.primitive.cull_mode = None;
        let over = BlendState::ALPHA_BLENDING.color;
        super::reactive::blend(descriptor, over, super::reactive::MARK);
        Ok(())
    }
}

pub struct PortalPlugin;

impl Plugin for PortalPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "portal.wgsl");
        app.add_plugins(MaterialPlugin::<PortalMaterial>::default());
    }
}

/// The exit's arrow in the x/y plane, its tip at +y.
pub fn arrow() -> Mesh {
    let v: [[f32; 2]; 7] = [
        [0.0, 0.75],
        [-0.6, 0.0],
        [0.6, 0.0],
        [-0.22, 0.0],
        [0.22, 0.0],
        [-0.22, -0.6],
        [0.22, -0.6],
    ];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.map(|[x, y]| [x, y, 0.0]).to_vec())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 7])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.map(|[x, y]| [x + 0.5, 0.5 - y]).to_vec())
        .with_inserted_indices(Indices::U16(vec![0, 1, 2, 3, 5, 6, 3, 6, 4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discs_carry_their_kind_and_colour() {
        let m = PortalMaterial::new(Color::srgb(1.0, 0.0, 0.0), true, 0.5, 0.85, 1.0);
        assert_eq!(m.u.params.x, 1.0);
        assert_eq!(m.u.params.y, 0.5);
        assert!((m.u.color.x - 1.0).abs() < 1e-6 && m.u.color.y == 0.0);
        assert_eq!(m.u.color.w, 0.85);
        assert_eq!(PortalMaterial::new(Color::WHITE, false, 0.0, 1.0, 0.0).u.params.x, 0.0);
    }
}
