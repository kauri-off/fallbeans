//! NVIDIA DLSS 4.5 Super Resolution through `dlss_wgpu` (`vendor/`), preset K in the quality or balanced mode; an error
//! falls back to FSR 1.
//! Bevy's `DlssPlugin` is left out: it panics on any error and renders only at a mode's own size.
use std::sync::{Arc, Mutex};

use bevy::anti_alias::contrast_adaptive_sharpening::CasPlugin;
use bevy::anti_alias::dlss::{DlssProjectId, DlssSuperResolutionSupported};
use bevy::anti_alias::fxaa::FxaaPlugin;
use bevy::anti_alias::smaa::SmaaPlugin;
use bevy::anti_alias::taa::TemporalAntiAliasPlugin;
use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::camera::TemporalJitter;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::renderer::raw_vulkan_init::{AdditionalVulkanFeatures, RawVulkanInitSettings};
use bevy::render::renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ViewTarget, prepare_view_targets};
use bevy::render::{Render, RenderApp, RenderSystems};
use dlss_wgpu::super_resolution::{
    DlssSuperResolution, DlssSuperResolutionExposure, DlssSuperResolutionRenderParameters,
};
use dlss_wgpu::{DlssFeatureFlags, DlssPerfQualityMode, DlssSdk};

use super::reactive::ReactiveMask;
use super::upscale::{Available, Faults, TemporalView, Upscaler};

/// The id NGX knows the game by: a GUID of its own (NVIDIA's guide: any, for a title it has not registered).
const PROJECT_ID: u128 = 0x3c7a_a498_eec2_493c_ba9f_a106_4e2a_6ebf;

/// The mode the context is made in for a main pass of `render` out of `out`: quality for ultra quality (0.77, in
/// its range) and quality (0.67, its optimum), balanced for balanced (0.59).
fn mode(render: UVec2, out: UVec2) -> DlssPerfQualityMode {
    if render.x * 100 >= out.x * 63 {
        DlssPerfQualityMode::Quality
    } else {
        DlssPerfQualityMode::Balanced
    }
}

/// `NVSDK_NGX_DLSS_Hint_Render_Preset_K` (`nvsdk_ngx_defs.h`): the first-generation transformer, NVIDIA's pick for
/// the quality and balanced modes; DLSS 4.5's M and L (made for performance and ultra performance) cost far more.
const PRESET_K: u32 = 11;

/// For `DlssInitPlugin`, which reads it before `DefaultPlugins` build.
pub fn project_id() -> DlssProjectId {
    DlssProjectId(bevy::asset::uuid::Uuid::from_u128(PROJECT_ID))
}

/// Bevy's `AntiAliasPlugin` without its `DlssPlugin` (`main.rs` disables the group's one).
pub fn add_anti_alias(app: &mut App) {
    app.add_plugins((FxaaPlugin, SmaaPlugin, TemporalAntiAliasPlugin, CasPlugin));
}

/// Bevy's own Vulkan instance (with the feature) fails on DX12 on purpose, so it falls back to an ordinary one.
#[expect(unsafe_code, reason = "Bevy's Vulkan instance callbacks are unsafe to add")]
pub fn no_vulkan_instance(app: &mut App) {
    let mut settings = app.world_mut().get_resource_or_init::<RawVulkanInitSettings>();
    // SAFETY: the callback only adds an instance extension that no driver has: `vkCreateInstance` fails
    // (VK_ERROR_EXTENSION_NOT_PRESENT) before anything is made, and nothing is removed.
    unsafe {
        settings.add_create_instance_callback(|args, _| args.extensions.push(c"VK_FALLBEANS_dx12_instead"));
    }
}

fn flags() -> DlssFeatureFlags {
    // (As Bevy's: motion at the render size, reversed depth, HDR before the tone mapping, DLSS's own exposure.)
    DlssFeatureFlags::LowResolutionMotionVectors
        | DlssFeatureFlags::InvertedDepth
        | DlssFeatureFlags::HighDynamicRange
        | DlssFeatureFlags::AutoExposure
}

#[derive(Resource)]
struct Sdk(Arc<Mutex<DlssSdk>>);

/// A view's context, for its output size.
#[derive(Component)]
struct DlssContext {
    sr: Mutex<DlssSuperResolution>,
    out: UVec2,
    mode: DlssPerfQualityMode,
}

pub struct DlssPlugin;

impl Plugin for DlssPlugin {
    fn build(&self, _: &mut App) {}

    /// Offered when the instance and device took NGX's extensions for it and its SDK starts on this device.
    fn finish(&self, app: &mut App) {
        let Some(render) = app.get_sub_app(RenderApp) else {
            return;
        };
        let world = render.world();
        let supported = world
            .get_resource::<AdditionalVulkanFeatures>()
            .is_some_and(|f| f.has::<DlssSuperResolutionSupported>());
        if !supported {
            info!("upscaling: NVIDIA DLSS not offered (it needs an NVIDIA RTX GPU on Vulkan)");
            return;
        }
        if super::upscale::asked_for_other(app, "dlss") {
            return;
        }
        let Some(device) = world.get_resource::<RenderDevice>().map(|d| d.wgpu_device().clone()) else {
            return;
        };
        // (The Vulkan instance asked for NGX's extensions, then the renderer went to DX12 after all.)
        // SAFETY: only asks whether the device is a Vulkan one; the guard is dropped at once.
        #[allow(unsafe_code)]
        let vulkan = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.is_some();
        if !vulkan {
            info!("upscaling: NVIDIA DLSS not offered: not on Vulkan");
            return;
        }
        let id = app.world().resource::<DlssProjectId>().0;
        let sdk = match DlssSdk::new(id, device) {
            Ok(sdk) => sdk,
            Err(e) => {
                info!("upscaling: NVIDIA DLSS not offered: {e}");
                return;
            }
        };
        if let Ok(mut s) = sdk.lock() {
            for m in [DlssPerfQualityMode::Quality, DlssPerfQualityMode::Balanced] {
                s.set_render_preset(m, PRESET_K);
            }
        }
        info!("upscaling: NVIDIA DLSS offered (Super Resolution, preset K)");
        app.world_mut().resource_mut::<Available>().dlss = true;
        app.sub_app_mut(RenderApp)
            .insert_resource(Sdk(sdk))
            .add_systems(
                Render,
                (drop_stale, prepare)
                    .chain()
                    .in_set(RenderSystems::PrepareViews)
                    .before(prepare_view_targets),
            )
            .add_systems(
                Core3d,
                upscale
                    .in_set(Core3dSystems::EarlyPostProcess)
                    .after(super::reactive::mask),
            );
    }
}

/// A context its view no longer upscales with goes (DLSS waits for the device as it is released).
fn drop_stale(mut commands: Commands, views: Query<(Entity, Option<&TemporalView>), With<DlssContext>>) {
    for (e, v) in &views {
        if v.is_none_or(|v| v.kind != Upscaler::Dlss) {
            commands.entity(e).remove::<DlssContext>();
        }
    }
}

/// What a DLSS context is made with.
#[derive(SystemParam)]
struct Gpu<'w> {
    sdk: Res<'w, Sdk>,
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
}

type DlssView = (
    Entity,
    &'static TemporalView,
    &'static mut TemporalJitter,
    &'static mut MainPassResolutionOverride,
    Option<&'static DlssContext>,
);

/// Each view's context for its output size, the main pass's size inside its range, this frame's jitter.
fn prepare(
    mut commands: Commands,
    gpu: Gpu,
    frames: Res<FrameCount>,
    faults: Res<Faults>,
    mut views: Query<DlssView>,
    mut told: Local<Option<(UVec2, UVec2)>>,
) {
    for (e, view, mut jitter, mut size, ctx) in &mut views {
        if view.kind != Upscaler::Dlss || faults.failed(Upscaler::Dlss) {
            continue;
        }
        let want = mode(view.render, view.out);
        let fresh = match ctx {
            Some(c) if c.out == view.out && c.mode == want => None,
            _ => {
                let made = DlssSuperResolution::new(
                    view.out.to_array(),
                    want,
                    flags(),
                    Arc::clone(&gpu.sdk.0),
                    gpu.device.wgpu_device(),
                    &gpu.queue,
                );
                match made {
                    Ok(sr) => Some(sr),
                    Err(err) => {
                        let why = format!("a context for {}×{}: {err}", view.out.x, view.out.y);
                        faults.report(Upscaler::Dlss, why);
                        continue;
                    }
                }
            }
        };
        // The size asked for, inside what DLSS takes; the jitter for it.
        let place = |sr: &DlssSuperResolution| {
            let range = sr.render_resolution_range();
            let (lo, hi) = (UVec2::from(*range.start()), UVec2::from(*range.end()));
            let r = view.render.max(lo).min(hi);
            (r, lo, hi, Vec2::from(sr.suggested_jitter(frames.0, r.to_array())))
        };
        let (r, lo, hi, j) = match (&fresh, ctx) {
            (Some(sr), _) => place(sr),
            (None, Some(c)) => match c.sr.lock() {
                Ok(sr) => place(&sr),
                Err(_) => continue,
            },
            (None, None) => continue,
        };
        if *told != Some((view.out, r)) {
            *told = Some((view.out, r));
            info!(
                "upscaling: NVIDIA DLSS {}×{} → {}×{} ({want:?} mode: it takes {}×{} to {}×{})",
                r.x, r.y, view.out.x, view.out.y, lo.x, lo.y, hi.x, hi.y
            );
            if r != view.render {
                warn!(
                    "upscaling: DLSS does not take {}×{} for {}×{}: {}×{} instead",
                    view.render.x, view.render.y, view.out.x, view.out.y, r.x, r.y
                );
            }
        }
        size.0 = r;
        // (Bevy's DLSS pass hands DLSS −offset: `upscale`.)
        jitter.offset = j;
        if let Some(sr) = fresh {
            commands.entity(e).insert(DlssContext {
                sr: Mutex::new(sr),
                out: view.out,
                mode: want,
            });
        }
    }
}

/// What the upscale reads of the view.
type DlssTarget = (
    &'static TemporalView,
    &'static DlssContext,
    &'static MainPassResolutionOverride,
    &'static TemporalJitter,
    &'static ViewTarget,
    &'static ViewPrepassTextures,
    Option<&'static ReactiveMask>,
);

/// The upscale, as Bevy's own DLSS pass records it: after the main pass's commands, a command buffer of its own.
fn upscale(view: ViewQuery<DlssTarget>, adapter: Res<RenderAdapter>, faults: Res<Faults>, mut ctx: RenderContext) {
    let (v, dlss, size, jitter, target, prepass, mask) = view.into_inner();
    if v.kind != Upscaler::Dlss || faults.failed(Upscaler::Dlss) || dlss.out != v.out {
        return;
    }
    let (Some(depth), Some(motion)) = (&prepass.depth, &prepass.motion_vectors) else {
        return;
    };
    let Ok(mut sr) = dlss.sr.lock() else { return };
    let post = target.post_process_write();
    let params = DlssSuperResolutionRenderParameters {
        color: post.source,
        depth: &depth.texture.default_view,
        motion_vectors: &motion.texture.default_view,
        exposure: DlssSuperResolutionExposure::Automatic,
        bias: mask.map(|m| &*m.0.default_view),
        dlss_output: post.destination,
        reset: v.reset,
        jitter_offset: (-jitter.offset).to_array(),
        partial_texture_size: Some(size.0.to_array()),
        // (Bevy's motion vectors: the current position minus the last, in UV units; DLSS's: towards the last,
        // in pixels.)
        motion_vector_scale: Some((-size.0.as_vec2()).to_array()),
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "dlss_super_resolution");
    match sr.render(params, ctx.command_encoder(), &adapter) {
        Ok(commands) => ctx.add_command_buffer(commands),
        Err(e) => faults.report(Upscaler::Dlss, format!("render: {e}")),
    }
    span.end(ctx.command_encoder());
}
