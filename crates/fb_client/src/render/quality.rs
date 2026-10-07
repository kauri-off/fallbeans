//! Graphics presets and the hardware tier: two presets, High (every machine's until the player picks Low) and Low,
//! both upscaled in the ultra quality mode (DLSS 4.5, FSR 3.1 or FSR 1: `upscale.rs`). The tier is read from the
//! adapter at start (T0: a software device; T1: an integrated GPU, or one that says neither; T2: a discrete GPU)
//! and only told: it changes nothing.
use core::time::Duration;

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::core_pipeline::prepass::background_motion_vectors::{
    BackgroundMotionVectorsBindGroup, BackgroundMotionVectorsPipelineId,
};
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass};
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, ShadowFilteringMethod};
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::effect_stack::Vignette;
use bevy::prelude::*;
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::ColorGrading;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::window::{PresentMode, PrimaryWindow};
use wgpu_types::DeviceType;

use super::Sun;
use super::upscale::{Upscaler, Upscaling};
use crate::settings::Graphics;
use crate::view::MainCamera;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// A software device, the CPU draws (WARP, llvmpipe/lavapipe).
    T0,
    /// An integrated GPU (or a virtual one, or one of no known kind).
    T1,
    /// A discrete GPU.
    T2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Preset {
    Low,
    High,
}

impl Preset {
    pub fn of(name: &str) -> Option<Preset> {
        Some(match name {
            "low" => Preset::Low,
            "high" => Preset::High,
            _ => return None,
        })
    }
}

/// The upscaling mode on every preset and upscaler, ultra quality: the main pass at 77% of the resolution.
pub const UPSCALE: &str = "ultra";

/// What the graphics run with now: the tier and the preset in effect.
#[derive(Resource, Clone, Debug)]
pub struct Quality {
    pub tier: Tier,
    pub preset: Preset,
    pub adapter: String,
}

impl Quality {
    /// The render scale of an upscaling mode (1: full resolution).
    pub fn scale(upscale: &str) -> f32 {
        match upscale {
            "ultra" => 0.77,
            "quality" => 0.67,
            "balanced" => 0.59,
            "performance" => 0.5,
            _ => 1.0,
        }
    }
}

pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Slept>();
        app.add_systems(Startup, detect);
        app.add_systems(Update, apply);
        app.add_systems(Last, limit_fps);
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.add_systems(Render, drop_stale_background_motion.in_set(RenderSystems::Prepare));
        }
    }
}

/// Bevy 0.19 keeps this pipeline once the motion vector prepass goes (TAA off, SSAO on): a validation crash.
fn drop_stale_background_motion(
    mut commands: Commands,
    views: Query<Entity, (With<BackgroundMotionVectorsPipelineId>, Without<MotionVectorPrepass>)>,
) {
    for e in &views {
        commands
            .entity(e)
            .remove::<(BackgroundMotionVectorsPipelineId, BackgroundMotionVectorsBindGroup)>();
    }
}

/// Skylake…Comet Lake and the Atom ones of that generation (PCI device ids).
fn gen9(id: u16) -> bool {
    matches!(id >> 8, 0x19 | 0x59 | 0x3e | 0x9b | 0x87) || matches!(id, 0x0a84 | 0x5a84 | 0x5a85 | 0x3184 | 0x3185)
}

fn intel_gen9_present() -> bool {
    let Ok(cards) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    let hex = |p: std::path::PathBuf| {
        let s = std::fs::read_to_string(p).ok()?;
        u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
    };
    cards.flatten().any(|card| {
        let dev = card.path().join("device");
        hex(dev.join("vendor")) == Some(0x8086) && hex(dev.join("device")).is_some_and(gen9)
    })
}

/// Against Mesa 26.2's Gen9 GPU hangs: runs the client again with `INTEL_DEBUG=reemit` (free), its exit code.
pub fn intel_gen9_relaunch() -> Option<u8> {
    if !cfg!(target_os = "linux") || std::env::var_os("INTEL_DEBUG").is_some() || !intel_gen9_present() {
        return None;
    }
    let status = std::process::Command::new(std::env::current_exe().ok()?)
        .args(std::env::args_os().skip(1))
        .env("INTEL_DEBUG", "reemit")
        .status()
        .ok()?;
    Some(status.code().map_or(1, |c| c.clamp(0, 255) as u8))
}

fn detect(mut commands: Commands, info: Option<Res<RenderAdapterInfo>>) {
    if let Ok(v) = std::env::var("INTEL_DEBUG") {
        info!("graphics: INTEL_DEBUG={v}");
    }
    let (tier, adapter) = match info {
        Some(i) => {
            let i = &i.0;
            let tier = match i.device_type {
                DeviceType::Cpu => Tier::T0,
                DeviceType::DiscreteGpu => Tier::T2,
                DeviceType::IntegratedGpu | DeviceType::VirtualGpu | DeviceType::Other => Tier::T1,
            };
            (tier, format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type))
        }
        None => (Tier::T1, "unknown".into()),
    };
    info!("graphics: {adapter}: tier {tier:?}");
    if tier == Tier::T0 {
        info!("graphics: a software device, the CPU draws: expect few frames a second (the Low preset helps)");
    }
    commands.insert_resource(Quality {
        tier,
        preset: Preset::High,
        adapter,
    });
}

/// The preset the settings ask for: High for a name it does not know (an old "auto" or "medium").
pub fn preset_for(g: &Graphics) -> Preset {
    Preset::of(&g.preset).unwrap_or(Preset::High)
}

/// The camera, the sun and the window as the preset and switches say.
fn apply(
    g: Res<Graphics>,
    up: Res<Upscaling>,
    q: Option<ResMut<Quality>>,
    mut commands: Commands,
    camera: Query<Entity, With<MainCamera>>,
    mut grade: Query<&mut ColorGrading, With<MainCamera>>,
    mut sun: Query<(Entity, &mut DirectionalLight), With<Sun>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    map: Option<Res<crate::game::Map>>,
    mut surfaces: ResMut<super::surface::Surfaces>,
    mut surface_mats: ResMut<Assets<super::surface::SurfaceMaterial>>,
    mut done: Local<Option<(Preset, Upscaler, Graphics)>>,
) {
    let Some(mut q) = q else { return };
    let preset = preset_for(&g);
    if q.preset != preset {
        q.preset = preset;
    }
    // (The grade's saturation comes with the look: kept off when switched off.)
    if let Ok(mut c) = grade.single_mut() {
        let sat = if g.grade {
            map.as_ref().map_or(1.0, |m| m.look.look.saturation as f32)
        } else {
            1.0
        };
        if c.global.post_saturation != sat {
            c.global.post_saturation = sat;
        }
    }
    // (The settings are compared, and cloned, only when something may have changed them.)
    let upscaler = up.active;
    if !g.is_changed() && done.as_ref().is_some_and(|(p, u, _)| *p == preset && *u == upscaler) {
        return;
    }
    if done
        .as_ref()
        .is_some_and(|(p, u, d)| *p == preset && *u == upscaler && *d == *g)
    {
        return;
    }
    *done = Some((preset, upscaler, g.clone()));
    // (A temporal upscaler is the anti-aliasing, and its prepasses, jitter and mip bias are `upscale.rs`'s.)
    let temporal = up.temporal(&g);
    surfaces.set_plain(preset == Preset::Low, &mut surface_mats);
    // (Low draws no detail texture.)
    surfaces.anisotropy = match preset {
        Preset::Low => 1,
        Preset::High => 16,
    };
    let Ok(cam) = camera.single() else { return };
    let mut e = commands.entity(cam);
    // (What an older preset may have left on the camera: TAA and SSAO with what they bring along. Left behind,
    // the prepasses would keep running and TAA's texture LOD bias of −1 would make every texture shimmer.)
    e.remove::<(
        TemporalAntiAliasing,
        Smaa,
        Fxaa,
        ScreenSpaceAmbientOcclusion,
        Vignette,
        Bloom,
    )>();
    e.remove::<NormalPrepass>();
    if !temporal {
        e.remove::<(MipBias, TemporalJitter, DepthPrepass, MotionVectorPrepass)>();
    }
    e.insert(Msaa::Off);
    // (The main pass is always upscaled (`fsr.rs`), and Bevy 0.19's TAA and SSAO work on the whole target, not
    // on the smaller main pass (`MainPassResolutionOverride` is only for DLSS): with FSR 1, SMAA before the
    // upscale instead, and no SSAO; DLSS and FSR 3.1 do their own anti-aliasing.)
    if g.aa && !temporal {
        match preset {
            Preset::High => {
                e.insert(Smaa {
                    preset: SmaaPreset::High,
                });
            }
            Preset::Low => {
                e.insert(Fxaa::default());
            }
        }
    }
    if g.grade && preset == Preset::High {
        e.insert(Vignette {
            intensity: 0.22,
            radius: 0.9,
            smoothness: 0.8,
            ..default()
        });
        // The slightest glow around what is bright (the sun on white, the bell, portals): energy-conserving, so
        // the picture keeps its brightness.
        e.insert(Bloom {
            intensity: 0.06,
            low_frequency_boost: 0.5,
            ..Bloom::NATURAL
        });
    }
    // (Low: a depth prepass first, so the main pass shades each pixel once; −0.45 ms on a GeForce 610M. A
    // temporal upscaler has it anyway.)
    if preset == Preset::Low && !temporal {
        e.insert(DepthPrepass);
    }
    // (High: the Gaussian, soft edges; Low one hardware-filtered tap per shadow lookup instead of nine.)
    e.insert(match preset {
        Preset::High => ShadowFilteringMethod::Gaussian,
        Preset::Low => ShadowFilteringMethod::Hardware2x2,
    });
    // (A coarse map shows its texels as stairs along every shadow's edge: High 4096 over two cascades, Low 1024
    // over one; the cascades overlap by a third, so their seams blend instead of showing. A third cascade on High
    // looked the same and cost a view: 2.5–4 ms of the render thread on DX12, without bindless.)
    let (size, cascades, reach) = match preset {
        Preset::High => (4096, 2, 80.0),
        Preset::Low => (1024, 1, 30.0),
    };
    commands.insert_resource(DirectionalLightShadowMap { size });
    if let Ok((sun_e, mut light)) = sun.single_mut() {
        light.shadow_maps_enabled = g.shadows;
        commands.entity(sun_e).insert(
            CascadeShadowConfigBuilder {
                num_cascades: cascades,
                maximum_distance: reach,
                first_cascade_far_bound: if cascades > 1 { 14.0 } else { reach },
                overlap_proportion: 0.33,
                ..default()
            }
            .build(),
        );
    }
    if let Ok(mut w) = windows.single_mut() {
        let mode = if g.vsync {
            PresentMode::AutoVsync
        } else {
            PresentMode::AutoNoVsync
        };
        if w.present_mode != mode {
            w.present_mode = mode;
        }
    }
    info!("graphics: preset {preset:?}, {}, {:?}", upscaler.name(), *g);
}

/// How long the frame limit slept at the end of the last frame (s).
#[derive(Resource, Default)]
pub struct Slept(pub f32);

/// The frame rate limit: the rest of the frame's time is slept away.
pub fn limit_fps(g: Res<Graphics>, mut last: Local<Option<std::time::Instant>>, mut slept: ResMut<Slept>) {
    let now = std::time::Instant::now();
    let mut nap = Duration::ZERO;
    if g.fps_limit > 0
        && let Some(prev) = *last
    {
        let want = Duration::from_secs_f64(1.0 / g.fps_limit.max(10) as f64);
        let spent = now - prev;
        if spent < want {
            std::thread::sleep(want - spent);
            nap = now.elapsed();
        }
    }
    slept.0 = nap.as_secs_f32();
    *last = Some(std::time::Instant::now());
}

#[cfg(test)]
mod tests {
    use super::gen9;

    #[test]
    fn gen9_ids() {
        // UHD 620 (Kaby Lake R), UHD 630 (Coffee Lake), UHD (Comet Lake U), HD 520 (Skylake), Apollo Lake.
        for id in [0x5917, 0x3e92, 0x9b41, 0x1916, 0x5a85] {
            assert!(gen9(id), "{id:04x}");
        }
        // Ice Lake, Tiger Lake, Alder Lake, Broadwell.
        for id in [0x8a52, 0x9a49, 0x46a6, 0x1616] {
            assert!(!gen9(id), "{id:04x}");
        }
    }
}
