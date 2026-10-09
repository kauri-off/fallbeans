//! FSR 1 upscaling: the main pass draws at a lower resolution
//! (`MainPassResolutionOverride`), EASU brings it to the full one before the post-processing, and Bevy's
//! robust contrast-adaptive sharpening (RCAS, the second half of FSR 1) restores the detail. The fallback when
//! neither temporal upscaler runs (`upscale.rs`); the main pass's size is set here for all of them.
use bevy::anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::upscale::{Upscaler, Upscaling};
use crate::settings::Graphics;
use crate::view::MainCamera;

/// The camera upscales its main pass (to `MainPassResolutionOverride`).
#[derive(Component, Clone, Copy)]
pub struct Fsr;

#[derive(Component, ShaderType, Clone, Copy)]
pub struct EasuUniform {
    in_size: Vec2,
    out_size: Vec2,
}

impl SyncComponent for Fsr {
    type Target = (EasuUniform, MainPassResolutionOverride);
}

impl ExtractComponent for Fsr {
    type QueryData = (
        &'static Fsr,
        &'static Camera,
        Option<&'static MainPassResolutionOverride>,
    );
    type QueryFilter = ();
    type Out = (EasuUniform, MainPassResolutionOverride);

    // (Bevy extracts the override only for DLSS: the main passes see it only if it is carried over here.)
    fn extract_component((_, camera, low): QueryItem<Self::QueryData>) -> Option<Self::Out> {
        let out = camera.physical_viewport_size()?;
        let low = low?;
        Some((
            EasuUniform {
                in_size: low.0.as_vec2(),
                out_size: out.as_vec2(),
            },
            MainPassResolutionOverride(low.0),
        ))
    }
}

pub struct FsrPlugin;

impl Plugin for FsrPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "easu.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<Fsr>::default(),
            UniformComponentPlugin::<EasuUniform>::default(),
        ));
        app.add_systems(PostUpdate, resolution);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            // (Before the post-processing, on linear HDR, not after the tone mapping as FSR 1 would have it:
            // the vignette (effect stack) runs before the tone mapping over the whole target, so it would
            // centre on the full frame instead of the small one. `easu.wgsl` tone-maps its taps reversibly.)
            .add_systems(Core3d, easu.in_set(Core3dSystems::EarlyPostProcess));
    }
}

/// A camera, the main pass's resolution if overridden, and whether FSR 1 is on.
type Resolution = (
    Entity,
    &'static Camera,
    Option<&'static MainPassResolutionOverride>,
    Has<Fsr>,
);

/// The main pass's resolution as the setting asks (for every upscaler), and FSR 1 with its sharpening when it is
/// the one in use (a temporal upscaler is the anti-aliasing and sharpens itself: no EASU, no CAS on top).
fn resolution(mut commands: Commands, g: Res<Graphics>, up: Res<Upscaling>, cams: Query<Resolution, With<MainCamera>>) {
    let scale = g.upscale.scale();
    let fsr1 = up.active == Upscaler::Fsr1;
    for (e, cam, now, has) in &cams {
        if scale >= 1.0 {
            if now.is_some() || has {
                commands
                    .entity(e)
                    .remove::<(MainPassResolutionOverride, Fsr, ContrastAdaptiveSharpening)>();
            }
            continue;
        }
        let Some(size) = cam.physical_viewport_size() else {
            continue;
        };
        let low = super::upscale::render_size(size, scale);
        if now.map(|n| n.0) != Some(low) {
            commands.entity(e).insert(MainPassResolutionOverride(low));
        }
        if fsr1 && !has {
            commands.entity(e).insert((
                Fsr,
                ContrastAdaptiveSharpening {
                    enabled: true,
                    sharpening_strength: 0.6,
                    denoise: false,
                },
            ));
        } else if !fsr1 && has {
            commands.entity(e).remove::<(Fsr, ContrastAdaptiveSharpening)>();
        }
    }
}

#[derive(Resource)]
struct EasuPipeline {
    layout: BindGroupLayoutDescriptor,
    variants: Variants<RenderPipeline, EasuSpecializer>,
}

fn init_pipeline(mut commands: Commands, fullscreen: Res<FullscreenShader>, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "easu_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<EasuUniform>(true),
            ),
        ),
    );
    let shader = load_embedded_asset!(assets.as_ref(), "easu.wgsl");
    let variants = Variants::new(
        EasuSpecializer,
        RenderPipelineDescriptor {
            label: Some("easu".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    commands.insert_resource(EasuPipeline { layout, variants });
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, SpecializerKey)]
struct EasuKey {
    format: TextureFormat,
}

struct EasuSpecializer;

impl Specializer<RenderPipeline> for EasuSpecializer {
    type Key = EasuKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut <RenderPipeline as Specializable>::Descriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.fragment_mut()?.set_target(
            0,
            ColorTargetState {
                format: key.format,
                blend: None,
                write_mask: ColorWrites::ALL,
            },
        );
        Ok(key)
    }
}

#[derive(Component)]
struct ViewEasuPipeline(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipeline: ResMut<EasuPipeline>,
    views: Query<(Entity, &ExtractedView), Added<EasuUniform>>,
    mut removed: RemovedComponents<EasuUniform>,
) -> Result<(), BevyError> {
    for e in removed.read() {
        if let Ok(mut e) = commands.get_entity(e) {
            e.remove::<ViewEasuPipeline>();
        }
    }
    for (e, view) in &views {
        let id = pipeline.variants.specialize(
            &cache,
            EasuKey {
                format: view.target_format,
            },
        )?;
        commands.entity(e).insert(ViewEasuPipeline(id));
    }
    Ok(())
}

fn easu(
    view: ViewQuery<(&ViewTarget, &ViewEasuPipeline, &DynamicUniformIndex<EasuUniform>), With<ExtractedView>>,
    pipeline: Res<EasuPipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<EasuUniform>>,
    mut ctx: RenderContext,
) {
    let (target, id, index) = view.into_inner();
    let Some(binding) = uniforms.binding() else { return };
    let Some(p) = cache.get_render_pipeline(id.0) else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "easu_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, binding)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("easu"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(p);
    pass.set_bind_group(0, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
}
