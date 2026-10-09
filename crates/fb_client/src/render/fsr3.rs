//! AMD FSR 3.1 upscaling (`ffx.rs`, AMD's signed DLL) on wgpu's Vulkan device, recorded into its own command buffer
//! after the main pass.
//! Images are put in the layouts FidelityFX expects (`transition_resources`); the reactive mask (`reactive.rs`) feeds
//! both of its masks.
#![allow(
    unsafe_code,
    reason = "wgpu-hal's raw Vulkan device and command buffer for AMD's DLL"
)]

use std::sync::{Arc, Mutex};

use bevy::camera::MainPassResolutionOverride;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::camera::TemporalJitter;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ViewTarget, prepare_view_targets};
use bevy::render::{Render, RenderApp, RenderSystems};
use wgpu::hal::api::Vulkan;

use super::ffx;
use super::reactive::ReactiveMask;
use super::upscale::{Available, Faults, TemporalView, Upscaler};

/// FSR's own sharpening (RCAS) on its output, 0…1: a touch; the mip bias keeps the textures crisp already.
const SHARPNESS: f32 = 0.25;

/// FSR's constants set on each context: shading changes count for more, and what is new (disoccluded, or in the
/// reactive mask) gathers its history more slowly. Less ghosting, a little more shimmer where a lot changes.
const HISTORY: [(u64, f32); 2] = [
    (ffx::KEY_SHADING_CHANGE_SCALE, 1.25),
    (ffx::KEY_ACCUMULATION_ADDED_PER_FRAME, 0.2),
];

/// A new context with `HISTORY` (an old DLL without the keys keeps FSR's own).
fn tune(u: &mut ffx::Upscaler) {
    for (key, value) in HISTORY {
        if let Err(e) = u.configure(key, value) {
            warn!("upscaling: AMD FSR 3.1 keeps its own constant {key}: {e}");
        }
    }
}

/// Context flags: HDR colour before tone mapping, already exposed, reversed infinite depth; `FB_FSR3_DEBUG=1` logs
/// FidelityFX's input checks.
fn flags() -> u32 {
    let debug = std::env::var_os("FB_FSR3_DEBUG").is_some_and(|v| v != "0");
    let base = ffx::HIGH_DYNAMIC_RANGE | ffx::DEPTH_INVERTED | ffx::DEPTH_INFINITE | ffx::AUTO_EXPOSURE;
    if debug { base | ffx::DEBUG_CHECKING } else { base }
}

#[derive(Resource, Clone)]
struct Fsr3Api(Arc<ffx::Api>);

/// A view's context (made for its sizes).
#[derive(Component)]
struct Fsr3Context(Mutex<ffx::Upscaler>);

pub struct Fsr3Plugin;

impl Plugin for Fsr3Plugin {
    fn build(&self, _: &mut App) {}

    /// Offered when the DLL loads and makes a context on this device (made and dropped here, at 1600×900).
    fn finish(&self, app: &mut App) {
        if super::upscale::asked_for_other(app, "fsr3") {
            return;
        }
        let Some(rd) = app.world().get_resource::<RenderDevice>() else {
            return;
        };
        let device = rd.wgpu_device().clone();
        // SAFETY: only asks whether the device is a Vulkan one; the guard is dropped at once.
        if unsafe { device.as_hal::<Vulkan>() }.is_none() {
            info!("upscaling: AMD FSR 3.1 not offered: not on Vulkan");
            return;
        }
        let api = match ffx::Api::load() {
            Ok(api) => Arc::new(api),
            Err(e) => {
                info!("upscaling: AMD FSR 3.1 not offered: {e}");
                return;
            }
        };
        let out = UVec2::new(1600, 900);
        let render = super::upscale::render_size(out, super::quality::Upscale::default().scale());
        match ffx::Upscaler::new(api.clone(), &device, render, out, flags()) {
            Ok(mut probe) => {
                let version = probe.version().unwrap_or_else(|| "?".into());
                info!(
                    "upscaling: AMD FSR 3.1 offered ({version}, {} jitter phases at 0.77)",
                    probe.phases
                );
            }
            Err(e) => {
                warn!("upscaling: AMD FSR 3.1 not offered: {e}");
                return;
            }
        }
        app.world_mut().resource_mut::<Available>().fsr3 = true;
        let Some(r) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        r.insert_resource(Fsr3Api(api))
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

/// A context its view no longer upscales with goes (with the device idle first: `ffx::Upscaler`'s drop).
fn drop_stale(mut commands: Commands, views: Query<(Entity, Option<&TemporalView>), With<Fsr3Context>>) {
    for (e, v) in &views {
        if v.is_none_or(|v| v.kind != Upscaler::Fsr3) {
            commands.entity(e).remove::<Fsr3Context>();
        }
    }
}

/// Each view's context for its sizes (made again when they change), and this frame's jitter.
fn prepare(
    mut commands: Commands,
    api: Res<Fsr3Api>,
    device: Res<RenderDevice>,
    frames: Res<FrameCount>,
    faults: Res<Faults>,
    mut views: Query<(Entity, &TemporalView, &mut TemporalJitter, Option<&Fsr3Context>)>,
) {
    for (e, view, mut jitter, ctx) in &mut views {
        if view.kind != Upscaler::Fsr3 || faults.failed(Upscaler::Fsr3) {
            continue;
        }
        let fits = |u: &ffx::Upscaler| u.render == view.render && u.out == view.out;
        let mut fresh = match ctx {
            Some(c) if c.0.lock().is_ok_and(|u| fits(&u)) => None,
            _ => match ffx::Upscaler::new(api.0.clone(), device.wgpu_device(), view.render, view.out, flags()) {
                Ok(mut u) => {
                    info!(
                        "upscaling: AMD FSR 3.1 {}×{} → {}×{} ({} jitter phases)",
                        view.render.x, view.render.y, view.out.x, view.out.y, u.phases
                    );
                    tune(&mut u);
                    Some(u)
                }
                Err(err) => {
                    faults.report(Upscaler::Fsr3, err.to_string());
                    continue;
                }
            },
        };
        let j = match (fresh.as_mut(), ctx) {
            (Some(u), _) => u.jitter(frames.0),
            (None, Some(c)) => match c.0.lock() {
                Ok(mut u) => u.jitter(frames.0),
                Err(_) => continue,
            },
            (None, None) => continue,
        };
        // (FSR's offset is how far the projection moves the picture, +y down; Bevy's moves it by minus its
        // offset: `TemporalJitter::jitter_projection`. Bevy's DLSS pass hands DLSS −offset the same way.)
        jitter.offset = -j;
        if let Some(u) = fresh {
            commands.entity(e).insert(Fsr3Context(Mutex::new(u)));
        }
    }
}

/// What the upscale reads of the view.
type Fsr3View = (
    &'static TemporalView,
    &'static Fsr3Context,
    &'static MainPassResolutionOverride,
    &'static TemporalJitter,
    &'static ViewTarget,
    &'static ViewPrepassTextures,
    Option<&'static ReactiveMask>,
);

/// The upscale: the main pass's corner of the colour target into the other, full one.
#[allow(
    clippy::field_reassign_with_default,
    reason = "the descriptor has private fields: no struct update"
)]
fn upscale(
    view: ViewQuery<Fsr3View>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    faults: Res<Faults>,
    mut ctx: RenderContext,
) {
    let (v, fsr, size, jitter, target, prepass, mask) = view.into_inner();
    if v.kind != Upscaler::Fsr3 || faults.failed(Upscaler::Fsr3) {
        return;
    }
    let (Some(depth), Some(motion)) = (&prepass.depth, &prepass.motion_vectors) else {
        return;
    };
    let Ok(mut fsr) = fsr.0.lock() else { return };
    // (A context made for other sizes than this frame's: the next frame has the right one.)
    if fsr.out != v.out || size.0.cmpgt(fsr.render).any() {
        return;
    }
    let post = target.post_process_write();
    let inputs = (
        ffx::Resource::texture(post.source_texture, ffx::USAGE_READ_ONLY, ffx::STATE_SAMPLED),
        ffx::Resource::texture(&depth.texture.texture, ffx::USAGE_DEPTH_TARGET, ffx::STATE_COPY_DEST),
        ffx::Resource::texture(&motion.texture.texture, ffx::USAGE_READ_ONLY, ffx::STATE_SAMPLED),
        ffx::Resource::texture(post.destination_texture, ffx::USAGE_UAV, ffx::STATE_STORAGE),
    );
    let (Some(color), Some(depth_in), Some(motion_in), Some(output)) = inputs else {
        faults.report(Upscaler::Fsr3, "a target FidelityFX cannot take (its format)");
        return;
    };
    let mask = mask.map(|m| &m.0.texture);
    let reactive = match mask {
        Some(m) => ffx::Resource::texture(m, ffx::USAGE_READ_ONLY, ffx::STATE_SAMPLED),
        None => Some(ffx::Resource::NONE),
    };
    let Some(reactive) = reactive else {
        faults.report(Upscaler::Fsr3, "a reactive mask FidelityFX cannot take (its format)");
        return;
    };
    // The images into the layouts named above.
    fn transition(texture: &wgpu::Texture, state: wgpu::TextureUses) -> wgpu::TextureTransition<&wgpu::Texture> {
        wgpu::TextureTransition {
            texture,
            selector: None,
            state,
        }
    }
    ctx.command_encoder().transition_resources(
        core::iter::empty(),
        [
            transition(post.source_texture, wgpu::TextureUses::RESOURCE),
            transition(&depth.texture.texture, wgpu::TextureUses::COPY_DST),
            transition(&motion.texture.texture, wgpu::TextureUses::RESOURCE),
            transition(post.destination_texture, wgpu::TextureUses::STORAGE_READ_WRITE),
        ]
        .into_iter()
        .chain(mask.map(|m| transition(m, wgpu::TextureUses::RESOURCE))),
    );
    let mut d = ffx::DispatchUpscale::default();
    d.color = color;
    d.depth = depth_in;
    d.motion_vectors = motion_in;
    d.reactive = reactive;
    d.transparency_and_composition = reactive;
    d.output = output;
    d.jitter_offset = (-jitter.offset).into();
    // (Bevy's motion vectors: the current position minus the last, in UV units of the main pass; FSR's: towards
    // the last, in its pixels.)
    d.motion_vector_scale = (-size.0.as_vec2()).into();
    d.render_size = size.0.into();
    d.upscale_size = v.out.into();
    d.enable_sharpening = true;
    d.sharpness = SHARPNESS;
    // (The first frame has none: FidelityFX warns below a millisecond.)
    d.frame_time_delta = v.delta_ms.max(1.0);
    d.pre_exposure = 1.0;
    d.reset = v.reset;
    // (Reversed depth: the far plane is the near one and the near at infinity, as AMD's sample has it.)
    d.camera_near = f32::MAX;
    d.camera_far = v.near;
    d.camera_fov_angle_vertical = v.fov_y;
    d.view_space_to_meters_factor = 1.0;

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "fsr3_upscale");
    let mut encoder = device
        .wgpu_device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fsr3_upscale"),
        });
    // SAFETY: the raw buffer is the new encoder's, submitted after the main one put every image in its named state.
    let done = unsafe {
        encoder.as_hal_mut::<Vulkan, _, _>(|e| match e {
            Some(e) => fsr.dispatch(&queue, e.raw_handle(), &mut d),
            None => Err(ffx::FfxError::NotVulkan),
        })
    };
    match done {
        Ok(()) => ctx.add_command_buffer(encoder.finish()),
        Err(e) => faults.report(Upscaler::Fsr3, e.to_string()),
    }
    span.end(ctx.command_encoder());
}
