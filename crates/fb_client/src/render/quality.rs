//! Hardware tiers and graphics presets: the tier is read from the adapter at start
//! (T0: a software device; T1: an integrated GPU, or one that says neither; T2: a discrete GPU), the
//! preset follows it unless the player picks one, and switches only take work away. With the preset
//! on "auto", frames that stay slow for a while lower it a step (never up: that is the player's call).
use core::time::Duration;

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::core_pipeline::prepass::background_motion_vectors::{
    BackgroundMotionVectorsBindGroup, BackgroundMotionVectorsPipelineId,
};
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass};
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::post_process::effect_stack::Vignette;
use bevy::prelude::*;
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::ColorGrading;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::window::{PresentMode, PrimaryWindow};
use wgpu_types::DeviceType;

use super::Sun;
use crate::settings::Graphics;
use crate::view::MainCamera;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// A software device, the CPU draws (WARP, llvmpipe/lavapipe): Low.
    T0,
    /// An integrated GPU (or a virtual one, or one of no known kind): Medium.
    T1,
    /// A discrete GPU: High.
    T2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Preset {
    Low,
    Medium,
    High,
}

impl Preset {
    pub fn of(name: &str) -> Option<Preset> {
        Some(match name {
            "low" => Preset::Low,
            "medium" => Preset::Medium,
            "high" => Preset::High,
            _ => return None,
        })
    }

    fn lower(self) -> Preset {
        match self {
            Preset::High => Preset::Medium,
            _ => Preset::Low,
        }
    }
}

/// What the graphics run with now: the tier, the preset in effect, and how many steps "auto" took off.
#[derive(Resource, Clone, Debug)]
pub struct Quality {
    pub tier: Tier,
    pub preset: Preset,
    pub dropped: u32,
    pub adapter: String,
}

impl Quality {
    /// The render scale of an FSR mode (1: full resolution).
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
        app.add_systems(Update, (watch_frames, apply).chain());
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
    let preset = match tier {
        Tier::T0 => Preset::Low,
        Tier::T1 => Preset::Medium,
        Tier::T2 => Preset::High,
    };
    info!("graphics: {adapter}: tier {tier:?}, preset {preset:?}");
    commands.insert_resource(Quality {
        tier,
        preset,
        dropped: 0,
        adapter,
    });
}

/// The preset the settings ask for, as far as the tier allows it.
fn preset_for(g: &Graphics, q: &Quality) -> Preset {
    let most = match q.tier {
        Tier::T0 => Preset::Low,
        Tier::T1 => Preset::Medium,
        Tier::T2 => Preset::High,
    };
    match Preset::of(&g.preset) {
        Some(p) => p.min(most),
        None => {
            let mut p = most;
            for _ in 0..q.dropped {
                p = p.lower();
            }
            p
        }
    }
}

/// "auto": a frame time over 24 ms (smoothed) for 6 s lowers the preset a step. The frame limit's sleep
/// is not the frame's work, and a window in the background may be held back by the compositor: neither
/// counts.
fn watch_frames(
    time: Res<Time<Real>>,
    g: Res<Graphics>,
    mut q: Option<ResMut<Quality>>,
    mut smooth: Local<f32>,
    mut slow: Local<f32>,
    mut session: ResMut<crate::session::Session>,
    slept: Res<Slept>,
    windows: Query<&Window, With<PrimaryWindow>>,
    perf: Option<Res<crate::perf::Perf>>,
) {
    let Some(q) = q.as_mut() else { return };
    // (A sweep switches features off and on: its frames say nothing about the preset.)
    if windows.single().is_ok_and(|w| !w.focused) || perf.is_some_and(|p| p.busy) {
        *slow = 0.0;
        return;
    }
    let dt = (time.delta_secs() - slept.0).clamp(0.0, 1.0);
    *smooth += (dt * 1000.0 - *smooth) * 0.05;
    if g.preset != "auto" || preset_for(&g, q) == Preset::Low {
        *slow = 0.0;
        return;
    }
    if *smooth > 24.0 {
        *slow += dt;
    } else {
        *slow = 0.0;
    }
    if *slow > 6.0 {
        *slow = 0.0;
        q.dropped += 1;
        warn!("frames run slow ({:.0} ms): graphics preset lowered", *smooth);
        session.note(time.elapsed_secs(), crate::ui::text::QUALITY_LOWERED.into());
    }
}

/// The camera, the sun and the window as the preset and switches say.
fn apply(
    g: Res<Graphics>,
    q: Option<ResMut<Quality>>,
    mut commands: Commands,
    camera: Query<Entity, With<MainCamera>>,
    mut grade: Query<&mut ColorGrading, With<MainCamera>>,
    mut sun: Query<(Entity, &mut DirectionalLight), With<Sun>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    map: Option<Res<crate::game::Map>>,
    mut surfaces: ResMut<super::surface::Surfaces>,
    mut surface_mats: ResMut<Assets<super::surface::SurfaceMaterial>>,
    mut done: Local<Option<(Preset, Graphics)>>,
) {
    let Some(mut q) = q else { return };
    let preset = preset_for(&g, &q);
    if q.preset != preset {
        q.preset = preset;
    }
    let now = (preset, g.clone());
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
    if done.as_ref() == Some(&now) {
        return;
    }
    *done = Some(now);
    surfaces.set_plain(preset == Preset::Low, &mut surface_mats);
    let Ok(cam) = camera.single() else { return };
    let mut e = commands.entity(cam);
    e.remove::<(TemporalAntiAliasing, Smaa, Fxaa, ScreenSpaceAmbientOcclusion, Vignette)>();
    // (And what TAA and SSAO brought along: left behind, the prepasses would keep running and TAA's
    // texture LOD bias of −1 would make every texture shimmer. Inserting them again brings them back.)
    e.remove::<(
        MipBias,
        TemporalJitter,
        DepthPrepass,
        NormalPrepass,
        MotionVectorPrepass,
    )>();
    e.insert(Msaa::Off);
    // (Upscaling replaces the temporal anti-aliasing and the AO, which want the full resolution.)
    let upscaling = Quality::scale(&g.upscale) < 1.0;
    if g.aa {
        match preset {
            Preset::High if !upscaling => {
                e.insert(TemporalAntiAliasing::default());
            }
            Preset::High | Preset::Medium => {
                e.insert(Smaa {
                    preset: SmaaPreset::High,
                });
            }
            Preset::Low => {
                e.insert(Fxaa::default());
            }
        }
    }
    if g.ao && preset == Preset::High && q.tier == Tier::T2 && !upscaling {
        e.insert(ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Medium,
            ..default()
        });
    }
    // (Half a millisecond of an integrated GPU for darker corners.)
    if g.grade && preset != Preset::Low {
        e.insert(Vignette {
            intensity: 0.22,
            radius: 0.9,
            smoothness: 0.8,
            ..default()
        });
    }
    let (size, cascades, reach) = match preset {
        Preset::High => (2048, 2, 70.0),
        Preset::Medium => (2048, 1, 40.0),
        Preset::Low => (1024, 1, 30.0),
    };
    commands.insert_resource(DirectionalLightShadowMap { size });
    if let Ok((sun_e, mut light)) = sun.single_mut() {
        light.shadow_maps_enabled = g.shadows;
        commands.entity(sun_e).insert(
            CascadeShadowConfigBuilder {
                num_cascades: cascades,
                maximum_distance: reach,
                first_cascade_far_bound: if cascades > 1 { 18.0 } else { reach },
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
    info!("graphics: preset {preset:?}, {:?}", *g);
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
