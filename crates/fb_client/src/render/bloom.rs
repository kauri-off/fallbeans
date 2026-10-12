//! The slightest glow around what is bright, in place of Bevy's bloom: its filters and energy-conserving blend
//! (`Bloom::NATURAL` at intensity 0.06), but four levels of the pyramid instead of eight (the four smallest, a few
//! texels tall, carried under a tenth of the glow) and its textures, views and bind groups kept between frames: 8
//! render passes instead of 16, and none of the objects Bevy makes and drops each frame.
use std::collections::HashMap;

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};

/// On the camera: its HDR target glows.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct Bloom;

/// Levels of the pyramid; the first is `TOP` texels tall.
const LEVELS: usize = 4;
const TOP: f32 = 512.0;
const FORMAT: TextureFormat = TextureFormat::Rg11b10Ufloat;

/// Bevy's `compute_blend_factor` for `Bloom::NATURAL` at intensity 0.06 and a low-frequency boost of 0.5 over its
/// eight levels: how much of the blurrier level goes into each one (0: into the picture).
fn blend(level: usize) -> f32 {
    const INTENSITY: f32 = 0.06;
    const BOOST: f32 = 0.5;
    const CURVATURE: f32 = 0.95;
    const MAX: f32 = 7.0;
    let boost = (1.0 - (1.0 - level as f32 / MAX).powf(1.0 / (1.0 - CURVATURE))) * BOOST * (1.0 - INTENSITY);
    INTENSITY + boost
}

pub struct BloomPlugin;

impl Plugin for BloomPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "bloom.wgsl");
        app.add_plugins(ExtractComponentPlugin::<Bloom>::default());
        let Some(r) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        r.add_systems(RenderStartup, init_pipelines)
            .add_systems(Core3d, bloom.before(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Resource)]
struct Pipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    first: CachedRenderPipelineId,
    down: CachedRenderPipelineId,
    up: CachedRenderPipelineId,
    last: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "bloom_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let shader = load_embedded_asset!(assets.as_ref(), "bloom.wgsl");
    // (Mixed by the pass's blend constant: the level below, or the picture, keeps the rest.)
    let mix = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::Constant,
            dst_factor: BlendFactor::OneMinusConstant,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
    };
    let queue = |label: &'static str, entry: &'static str, format, blend| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
                ..Default::default()
            }),
            ..Default::default()
        })
    };
    let first = queue("bloom_first", "downsample_first", FORMAT, None);
    let down = queue("bloom_down", "downsample", FORMAT, None);
    let up = queue("bloom_up", "upsample", FORMAT, Some(mix));
    // (Into the camera's HDR target.)
    let last = queue("bloom_last", "upsample", TextureFormat::Rgba16Float, Some(mix));
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("bloom_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..Default::default()
    });
    commands.insert_resource(Pipelines {
        layout,
        sampler,
        first,
        down,
        up,
        last,
    });
}

/// A view's pyramid, made again only when its size changes.
struct Chain {
    size: UVec2,
    levels: Vec<TextureView>,
    /// Reading level i, for the pass that downsamples it into i + 1 and the one that upsamples it into i − 1.
    reads: Vec<BindGroup>,
    /// Reading the view's main texture (the post-processing swaps between two), by its view's id.
    sources: Vec<(TextureViewId, BindGroup)>,
}

impl Chain {
    fn new(device: &RenderDevice, layout: &BindGroupLayout, sampler: &Sampler, size: UVec2) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("bloom_texture"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: LEVELS as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let levels: Vec<TextureView> = (0..LEVELS as u32)
            .map(|mip| {
                texture.create_view(&TextureViewDescriptor {
                    label: Some("bloom_level"),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let reads = levels
            .iter()
            .map(|v| device.create_bind_group("bloom_read", layout, &BindGroupEntries::sequential((v, sampler))))
            .collect();
        Chain {
            size,
            levels,
            reads,
            sources: Vec::new(),
        }
    }
}

fn pass<'a>(
    encoder: &'a mut CommandEncoder,
    target: &'a TextureView,
    load: LoadOp<wgpu::Color>,
    pipeline: &'a RenderPipeline,
    bind_group: &'a BindGroup,
    constant: Option<f32>,
) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("bloom"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load,
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    if let Some(c) = constant.map(f64::from) {
        pass.set_blend_constant(wgpu::Color { r: c, g: c, b: c, a: c });
    }
    pass.draw(0..3, 0..1);
}

fn bloom(
    view: ViewQuery<(Entity, &ViewTarget), With<Bloom>>,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut chains: Local<HashMap<Entity, Chain>>,
    mut ctx: RenderContext,
) {
    let (entity, target) = view.into_inner();
    let (Some(first), Some(down), Some(up), Some(last)) = (
        cache.get_render_pipeline(pipelines.first),
        cache.get_render_pipeline(pipelines.down),
        cache.get_render_pipeline(pipelines.up),
        cache.get_render_pipeline(pipelines.last),
    ) else {
        return;
    };
    let source = target.main_texture_view();
    let input = target.main_texture().size();
    if input.height == 0 {
        return;
    }
    let size = (Vec2::new(input.width as f32, input.height as f32) * (TOP / input.height as f32))
        .round()
        .as_uvec2()
        .max(UVec2::splat(1 << LEVELS));
    let layout = cache.get_bind_group_layout(&pipelines.layout);
    let device = ctx.render_device().clone();
    let chain = chains
        .entry(entity)
        .or_insert_with(|| Chain::new(&device, &layout, &pipelines.sampler, size));
    if chain.size != size {
        *chain = Chain::new(&device, &layout, &pipelines.sampler, size);
    }
    let id = source.id();
    if !chain.sources.iter().any(|(s, _)| *s == id) {
        // (Two main textures at most; any older ones went with a resize.)
        if chain.sources.len() >= 2 {
            chain.sources.clear();
        }
        let bg = device.create_bind_group(
            "bloom_source",
            &layout,
            &BindGroupEntries::sequential((source, &pipelines.sampler)),
        );
        chain.sources.push((id, bg));
    }
    let Some((_, source_bg)) = chain.sources.iter().find(|(s, _)| *s == id) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "bloom");
    let encoder = ctx.command_encoder();
    let clear = LoadOp::Clear(wgpu::Color::BLACK);
    pass(encoder, &chain.levels[0], clear, first, source_bg, None);
    for i in 1..LEVELS {
        pass(encoder, &chain.levels[i], clear, down, &chain.reads[i - 1], None);
    }
    for i in (1..LEVELS).rev() {
        pass(
            encoder,
            &chain.levels[i - 1],
            LoadOp::Load,
            up,
            &chain.reads[i],
            Some(blend(i)),
        );
    }
    pass(encoder, source, LoadOp::Load, last, &chain.reads[0], Some(blend(0)));
    time_span.end(ctx.command_encoder());
}
