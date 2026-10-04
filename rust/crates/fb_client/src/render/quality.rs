//! Hardware tiers and graphics presets (`plan.md` §7): the tier is read from the adapter at start
//! (T0: OpenGL or a software device, no compute; T1: a Vulkan/DX12 integrated GPU; T2: the rest), the
//! preset follows it unless the player picks one, and switches only take work away. With the preset
//! on "auto", frames that stay slow for a while lower it a step (never up: that is the player's call).
use core::time::Duration;

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::post_process::effect_stack::Vignette;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::ColorGrading;
use bevy::window::{PresentMode, PrimaryWindow};
use wgpu_types::{Backend, DeviceType};

use super::Sun;
use crate::settings::Graphics;
use crate::view::MainCamera;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// OpenGL or a software device: no compute shaders.
    T0,
    /// Vulkan or DX12 on an integrated GPU.
    T1,
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
        app.add_systems(Startup, detect);
        app.add_systems(Update, (watch_frames, apply).chain());
        app.add_systems(Last, limit_fps);
    }
}

fn detect(mut commands: Commands, info: Option<Res<RenderAdapterInfo>>) {
    let (tier, adapter) = match info {
        Some(i) => {
            let i = &i.0;
            let tier = match (i.backend, i.device_type) {
                (Backend::Gl, _) | (_, DeviceType::Cpu) => Tier::T0,
                (_, DeviceType::IntegratedGpu) => Tier::T1,
                _ => Tier::T2,
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

/// "auto": a frame time over 24 ms (smoothed) for 6 s lowers the preset a step.
fn watch_frames(
    time: Res<Time<Real>>,
    g: Res<Graphics>,
    mut q: Option<ResMut<Quality>>,
    mut smooth: Local<f32>,
    mut slow: Local<f32>,
    mut session: ResMut<crate::session::Session>,
) {
    let Some(q) = q.as_mut() else { return };
    let dt = time.delta_secs().min(1.0);
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
    let Ok(cam) = camera.single() else { return };
    let mut e = commands.entity(cam);
    e.remove::<(TemporalAntiAliasing, Smaa, Fxaa, ScreenSpaceAmbientOcclusion, Vignette)>();
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
    if g.grade {
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

/// The frame rate limit: the rest of the frame's time is slept away.
fn limit_fps(g: Res<Graphics>, mut last: Local<Option<std::time::Instant>>) {
    let now = std::time::Instant::now();
    if g.fps_limit > 0
        && let Some(prev) = *last
    {
        let want = Duration::from_secs_f64(1.0 / g.fps_limit.max(10) as f64);
        let spent = now - prev;
        if spent < want {
            std::thread::sleep(want - spent);
        }
    }
    *last = Some(std::time::Instant::now());
}
