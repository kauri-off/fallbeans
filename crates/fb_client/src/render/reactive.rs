//! Reactive mask of the temporal upscalers (`dlss.rs`, `fsr3.rs`): particles, motes, portals and auras mark where the
//! current colour replaces the history, else they trail.
//! Written into the main pass colour's alpha (`MARK`), then into an R8 mask after it.
use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::texture_2d;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::upscale::TemporalView;

/// The alpha of what marks the mask: what is behind keeps `1 − a` of its alpha.
pub const MARK: BlendComponent = BlendComponent {
    src_factor: BlendFactor::Zero,
    dst_factor: BlendFactor::OneMinusSrcAlpha,
    operation: BlendOperation::Add,
};

/// The alpha of what leaves the mask as it is.
pub const KEEP: BlendComponent = BlendComponent {
    src_factor: BlendFactor::Zero,
    dst_factor: BlendFactor::One,
    operation: BlendOperation::Add,
};

/// Light added: the colour's blend of what glows (its alpha free for `MARK`).
pub const ADD: BlendComponent = BlendComponent {
    src_factor: BlendFactor::One,
    dst_factor: BlendFactor::One,
    operation: BlendOperation::Add,
};

/// A material pipeline's blend in the main pass: `color` for the colour, `alpha` for the mask.
pub fn blend(descriptor: &mut RenderPipelineDescriptor, color: BlendComponent, alpha: BlendComponent) {
    let target = descriptor
        .fragment
        .as_mut()
        .and_then(|f| f.targets.first_mut())
        .and_then(Option::as_mut);
    if let Some(t) = target {
        t.blend = Some(BlendState { color, alpha });
    }
}

/// A view's mask (the size of its colour target; the main pass's corner of it is written).
#[derive(Component)]
pub struct ReactiveMask(pub CachedTexture);

pub struct ReactivePlugin;

impl Plugin for ReactivePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "reactive.wgsl");
        let Some(r) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        r.add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, mask.in_set(Core3dSystems::EarlyPostProcess));
    }
}

#[derive(Resource)]
pub struct MaskPipeline {
    layout: BindGroupLayoutDescriptor,
    id: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "reactive_mask_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            texture_2d(TextureSampleType::Float { filterable: false }),
        ),
    );
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("reactive_mask".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(assets.as_ref(), "reactive.wgsl"),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::R8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..Default::default()
        }),
        ..Default::default()
    });
    commands.insert_resource(MaskPipeline { layout, id });
}

/// Each temporal view's mask; a view no longer upscaled temporally lets its own go.
fn prepare(
    mut commands: Commands,
    mut textures: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ViewTarget), With<TemporalView>>,
    stale: Query<Entity, (With<ReactiveMask>, Without<TemporalView>)>,
) {
    for e in &stale {
        commands.entity(e).remove::<ReactiveMask>();
    }
    for (e, target) in &views {
        let texture = textures.get(
            &device,
            TextureDescriptor {
                label: Some("reactive_mask"),
                size: target.main_texture().size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R8Unorm,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(e).insert(ReactiveMask(texture));
    }
}

/// The main pass's alpha into the mask, before the upscalers read it.
pub fn mask(
    view: ViewQuery<(&ViewTarget, &ReactiveMask, &MainPassResolutionOverride), With<TemporalView>>,
    pipeline: Res<MaskPipeline>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, mask, size) = view.into_inner();
    let Some(p) = cache.get_render_pipeline(pipeline.id) else {
        return;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "reactive_mask_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::single(target.main_texture_view()),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("reactive_mask"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &mask.0.default_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_viewport(0.0, 0.0, size.0.x as f32, size.0.y as f32, 0.0, 1.0);
    pass.set_pipeline(p);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}
