//! Which upscaler draws the frame: the graphics setting's (`Graphics::upscaler`, switched in play) when this machine
//! offers it, else the best offered, told in the log: NVIDIA DLSS 4.5 Super Resolution on an RTX GPU (Vulkan, the
//! `dlss` feature: `dlss.rs`), else AMD FSR 3.1 (Vulkan, AMD's library beside the game: `fsr3.rs`), else FSR 1
//! (DX12, no library, anything that failed: `fsr.rs`). All draw the main pass at the scale of the player's mode
//! (`Graphics::upscale`, switched in play: the vendors' ultra quality 0.77, quality 0.67 or balanced 0.59) and bring
//! it to the full resolution. A temporal one (DLSS, FSR 3.1) failing
//! at run time is not offered again this session: the next best takes over.
//!
//! A temporal upscaler is the anti-aliasing too: no SMAA, FXAA or CAS with it (`quality.rs`, `fsr.rs`). It needs
//! the depth and motion vector prepasses (every material writes its motion, the cloth's waves too:
//! `cloth.wgsl`), the projection jittered by a sub-pixel offset each frame (`TemporalJitter`, set in the render
//! world from the upscaler's own sequence), a negative texture mip bias for the scale (`MipBias`), its history
//! dropped on camera cuts (`Temporal::reset`), and the main texture writable by a compute shader.
use std::sync::{Arc, Mutex};

use bevy::camera::{CameraMainTextureUsages, MainPassResolutionOverride};
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::TextureUsages;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::sync_component::SyncComponent;
use serde::{Deserialize, Serialize};

use crate::settings::Graphics;
use crate::view::MainCamera;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Upscaler {
    /// NVIDIA DLSS 4.5 Super Resolution (preset M, the second-generation transformer model).
    Dlss,
    /// AMD FSR 3.1 (temporal).
    Fsr3,
    /// AMD FSR 1 (spatial: EASU and RCAS, with SMAA or FXAA before it).
    Fsr1,
}

impl Upscaler {
    /// It keeps a history of frames: the anti-aliasing is its own.
    pub fn temporal(self) -> bool {
        self != Upscaler::Fsr1
    }

    pub const ALL: [Upscaler; 3] = [Upscaler::Dlss, Upscaler::Fsr3, Upscaler::Fsr1];

    /// Its name in the settings file and `--upscaler`.
    pub fn id(self) -> &'static str {
        match self {
            Upscaler::Dlss => "dlss",
            Upscaler::Fsr3 => "fsr3",
            Upscaler::Fsr1 => "fsr1",
        }
    }

    pub fn from_id(id: &str) -> Option<Upscaler> {
        Upscaler::ALL.into_iter().find(|u| u.id() == id)
    }

    /// Its name, as the vendors write it (the log, F4, the graphics settings).
    pub fn name(self) -> &'static str {
        match self {
            Upscaler::Dlss => "NVIDIA DLSS 4.5",
            Upscaler::Fsr3 => "AMD FSR 3.1",
            Upscaler::Fsr1 => "AMD FSR 1",
        }
    }
}

/// The upscaler setting, saved by its id: "" (and any other name) is `None`, automatic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct UpscalerSetting(pub Option<Upscaler>);

impl Serialize for UpscalerSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.map_or("", Upscaler::id))
    }
}

impl<'de> Deserialize<'de> for UpscalerSetting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(Upscaler::from_id(&String::deserialize(d)?)))
    }
}

/// What this machine offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    /// The renderer runs on Vulkan (both temporal upscalers are bound to it here).
    pub vulkan: bool,
    /// DLSS Super Resolution is supported (an RTX GPU, its driver's NGX) and its SDK started.
    pub dlss: bool,
    /// AMD's DLL loaded and made an upscaling context.
    pub fsr3: bool,
}

impl Offer {
    pub fn has(self, u: Upscaler) -> bool {
        match u {
            Upscaler::Dlss => self.vulkan && self.dlss,
            Upscaler::Fsr3 => self.vulkan && self.fsr3,
            Upscaler::Fsr1 => true,
        }
    }

    /// Without what failed this session.
    fn without_failed(self, faults: &Faults) -> Offer {
        Offer {
            dlss: self.dlss && !faults.failed(Upscaler::Dlss),
            fsr3: self.fsr3 && !faults.failed(Upscaler::Fsr3),
            ..self
        }
    }
}

/// The one asked for when it is offered, else the best offered.
pub fn pick(o: Offer, want: Option<Upscaler>) -> Upscaler {
    want.filter(|u| o.has(*u)).unwrap_or_else(|| choose(o))
}

/// The upscaler for what is offered: DLSS, then FSR 3.1, then FSR 1.
pub fn choose(o: Offer) -> Upscaler {
    if !o.vulkan {
        Upscaler::Fsr1
    } else if o.dlss {
        Upscaler::Dlss
    } else if o.fsr3 {
        Upscaler::Fsr3
    } else {
        Upscaler::Fsr1
    }
}

/// `--upscaler` asks for another one than `id` (tests): `id` is not offered.
pub fn asked_for_other(app: &App, id: &str) -> bool {
    let asked = app
        .world()
        .get_resource::<crate::opts::Opts>()
        .and_then(|o| o.upscaler.clone());
    let other = asked.as_deref().is_some_and(|a| a != id);
    if other {
        info!("upscaling: {id} not offered (--upscaler {})", asked.unwrap_or_default());
    }
    other
}

/// The main pass's size for the full one at `scale` (at least a pixel each way).
pub fn render_size(full: UVec2, scale: f32) -> UVec2 {
    (full.as_vec2() * scale).round().as_uvec2().max(UVec2::ONE)
}

/// The texture mip bias for a render scale (NVIDIA's and AMD's formula): textures as sharp as at the full
/// resolution, the upscaler's history smooths what they alias.
pub fn mip_bias(scale: f32) -> f32 {
    scale.log2() - 1.0
}

/// What the upscalers' plugins found when the renderer started (their `finish`).
#[derive(Resource, Default)]
pub struct Available {
    pub dlss: bool,
    pub fsr3: bool,
}

/// The upscaler in use, the one the setting chose (they differ once it failed), and what this machine offers.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Upscaling {
    pub active: Upscaler,
    pub chosen: Upscaler,
    pub offer: Offer,
}

impl Default for Upscaling {
    fn default() -> Self {
        Self {
            active: Upscaler::Fsr1,
            chosen: Upscaler::Fsr1,
            offer: Offer::default(),
        }
    }
}

impl Upscaling {
    /// A temporal upscaler draws now (the main pass is smaller than the target).
    pub fn temporal(&self, g: &Graphics) -> bool {
        self.active.temporal() && g.upscale.scale() < 1.0
    }
}

#[derive(Default)]
struct FaultLog {
    /// Not yet seen by the main world.
    pending: Vec<(Upscaler, String)>,
    /// Every upscaler that failed this session.
    failed: Vec<Upscaler>,
}

/// The render world's word that an upscaler failed (both worlds hold it): the main world falls back to FSR 1.
#[derive(Resource, Clone, Default)]
pub struct Faults(Arc<Mutex<FaultLog>>);

impl Faults {
    /// `who` failed: said once, and its work stops.
    pub fn report(&self, who: Upscaler, why: impl Into<String>) {
        let Ok(mut log) = self.0.lock() else { return };
        if log.failed.contains(&who) {
            return;
        }
        let why = why.into();
        error!("upscaling: {} failed ({why}): not used again this session", who.name());
        log.failed.push(who);
        log.pending.push((who, why));
    }

    /// `who` failed before: its passes skip their work until the main world switches.
    pub fn failed(&self, who: Upscaler) -> bool {
        self.0.lock().is_ok_and(|log| log.failed.contains(&who))
    }

    fn take(&self) -> Vec<(Upscaler, String)> {
        let Ok(mut log) = self.0.lock() else { return Vec::new() };
        std::mem::take(&mut log.pending)
    }
}

/// The camera upscales with DLSS or FSR 3.1 (main world; `apply` keeps it).
#[derive(Component, Clone, Copy, Debug)]
pub struct Temporal {
    pub kind: Upscaler,
    /// The history goes this frame: a cut, a new map, another upscaler.
    pub reset: bool,
    /// The frame's time (ms).
    pub delta_ms: f32,
}

/// What the render world's upscaling passes need of the camera (extracted from `Temporal`).
#[derive(Component, Clone, Copy, Debug)]
pub struct TemporalView {
    pub kind: Upscaler,
    pub reset: bool,
    pub delta_ms: f32,
    /// The near plane (m) and the vertical field of view (rad); the depth is reversed and infinite.
    pub near: f32,
    pub fov_y: f32,
    /// The main pass's size (DLSS may move it inside its range) and the output's.
    pub render: UVec2,
    pub out: UVec2,
}

impl SyncComponent for Temporal {
    type Target = (TemporalView, MainPassResolutionOverride);
}

impl ExtractComponent for Temporal {
    type QueryData = (
        &'static Temporal,
        &'static Camera,
        &'static Projection,
        Option<&'static MainPassResolutionOverride>,
    );
    type QueryFilter = ();
    type Out = (TemporalView, MainPassResolutionOverride);

    // (The override reaches the main passes only through an extraction, as FSR 1's: `fsr.rs`.)
    fn extract_component((t, camera, projection, low): QueryItem<Self::QueryData>) -> Option<Self::Out> {
        let Projection::Perspective(p) = projection else {
            return None;
        };
        let out = camera.physical_viewport_size()?;
        let render = low?.0;
        Some((
            TemporalView {
                kind: t.kind,
                reset: t.reset,
                delta_ms: t.delta_ms,
                near: p.near,
                fov_y: p.fov,
                render,
                out,
            },
            MainPassResolutionOverride(render),
        ))
    }
}

/// A camera moving this far in a frame (m), or turning this much (rad), has cut: the history goes.
const CUT_M: f32 = 3.0;
const CUT_RAD: f32 = 0.8;

pub struct UpscalePlugin;

impl Plugin for UpscalePlugin {
    fn build(&self, app: &mut App) {
        let faults = Faults::default();
        app.insert_resource(faults.clone())
            .init_resource::<Available>()
            .init_resource::<Upscaling>()
            .add_plugins(ExtractComponentPlugin::<Temporal>::default())
            .add_systems(Startup, decide)
            .add_systems(Update, follow)
            .add_systems(PostUpdate, apply.after(crate::camera::place_camera));
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.insert_resource(faults);
        }
        // (Both are offered where they run: the setting switches between them in play.)
        #[cfg(feature = "dlss")]
        app.add_plugins(super::dlss::DlssPlugin);
        #[cfg(any(windows, target_os = "linux"))]
        app.add_plugins(super::fsr3::Fsr3Plugin);
    }
}

fn decide(
    available: Res<Available>,
    info: Option<Res<RenderAdapterInfo>>,
    g: Res<Graphics>,
    faults: Res<Faults>,
    mut up: ResMut<Upscaling>,
) {
    let offer = Offer {
        vulkan: info.is_some_and(|i| i.0.backend == wgpu_types::Backend::Vulkan),
        dlss: available.dlss,
        fsr3: available.fsr3,
    };
    *up = resolve(offer, &g, &faults);
    info!(
        "upscaling: {} at {:.0}% of the resolution (setting {:?}; Vulkan {}, DLSS {}, FSR 3.1 {})",
        up.active.name(),
        g.upscale.scale() * 100.0,
        g.upscaler,
        offer.vulkan,
        offer.dlss,
        offer.fsr3
    );
}

fn resolve(offer: Offer, g: &Graphics, faults: &Faults) -> Upscaling {
    let want = g.upscaler.0;
    Upscaling {
        active: pick(offer.without_failed(faults), want),
        chosen: pick(offer, want),
        offer,
    }
}

/// The setting changed, or an upscaler the render world gave up on: the one to use now.
fn follow(faults: Res<Faults>, g: Res<Graphics>, mut up: ResMut<Upscaling>) {
    let failed = faults.take();
    if failed.is_empty() && !g.is_changed() {
        return;
    }
    let next = resolve(up.offer, &g, &faults);
    for (who, why) in &failed {
        if *who == up.active {
            warn!(
                "upscaling: {} gave up ({why}): {} instead",
                who.name(),
                next.active.name()
            );
        }
    }
    if next.active != up.active || next.chosen != up.chosen {
        if failed.is_empty() {
            info!("upscaling: {} (setting {:?})", next.active.name(), g.upscaler);
        }
        *up = next;
    }
}

type TemporalCamera = (
    Entity,
    &'static Transform,
    Option<&'static mut Temporal>,
    (Has<DepthPrepass>, Has<MotionVectorPrepass>, Has<TemporalJitter>),
    Option<&'static MipBias>,
    Option<&'static CameraMainTextureUsages>,
);

/// The camera's components for the upscaler in use (the main pass's size itself is `fsr.rs`'s).
fn apply(
    mut commands: Commands,
    up: Res<Upscaling>,
    g: Res<Graphics>,
    time: Res<Time<Real>>,
    map: Option<Res<crate::game::Map>>,
    mut cams: Query<TemporalCamera, With<MainCamera>>,
    mut last: Local<Option<(Vec3, Quat, Option<u32>)>>,
) {
    let Ok((e, tf, temporal, (depth, motion, jitter), bias, usages)) = cams.single_mut() else {
        return;
    };
    if !up.temporal(&g) {
        if temporal.is_some() {
            // (The depth prepass stays for `quality.rs` to keep or take: Low has it.)
            commands
                .entity(e)
                .remove::<(Temporal, TemporalJitter, MipBias, MotionVectorPrepass)>()
                .insert(CameraMainTextureUsages::default());
        }
        *last = None;
        return;
    }
    let generation = map.map(|m| m.generation);
    let cut = last.is_none_or(|(at, turn, seen)| {
        seen != generation || at.distance(tf.translation) > CUT_M || turn.angle_between(tf.rotation) > CUT_RAD
    });
    *last = Some((tf.translation, tf.rotation, generation));
    let delta_ms = time.delta_secs() * 1000.0;
    let mut ec = commands.entity(e);
    match temporal {
        Some(mut t) => {
            t.reset = cut || t.kind != up.active;
            t.kind = up.active;
            t.delta_ms = delta_ms;
        }
        None => {
            ec.insert(Temporal {
                kind: up.active,
                reset: true,
                delta_ms,
            });
        }
    }
    if !depth {
        ec.insert(DepthPrepass);
    }
    if !motion {
        ec.insert(MotionVectorPrepass);
    }
    if !jitter {
        ec.insert(TemporalJitter::default());
    }
    let want = mip_bias(g.upscale.scale());
    if bias.is_none_or(|b| b.0 != want) {
        ec.insert(MipBias(want));
    }
    if usages.is_none_or(|u| !u.0.contains(TextureUsages::STORAGE_BINDING)) {
        let usage = CameraMainTextureUsages::default();
        ec.insert(usage.with(TextureUsages::STORAGE_BINDING));
    }
}

#[cfg(test)]
mod tests {
    use super::super::quality::Upscale;
    use super::*;

    #[test]
    fn dlss_then_fsr3_then_fsr1() {
        let o = |vulkan, dlss, fsr3| Offer { vulkan, dlss, fsr3 };
        assert_eq!(choose(o(true, true, true)), Upscaler::Dlss);
        assert_eq!(choose(o(true, true, false)), Upscaler::Dlss);
        assert_eq!(choose(o(true, false, true)), Upscaler::Fsr3);
        assert_eq!(choose(o(true, false, false)), Upscaler::Fsr1);
        // DX12 (or the tests' noop device): FSR 1 whatever else is there.
        assert_eq!(choose(o(false, true, true)), Upscaler::Fsr1);
        assert_eq!(choose(Offer::default()), Upscaler::Fsr1);
        assert!(Upscaler::Dlss.temporal());
        assert!(Upscaler::Fsr3.temporal());
        assert!(!Upscaler::Fsr1.temporal());
    }

    #[test]
    fn the_setting_picks_among_what_is_offered() {
        let rtx = Offer {
            vulkan: true,
            dlss: true,
            fsr3: true,
        };
        assert_eq!(pick(rtx, None), Upscaler::Dlss);
        assert_eq!(pick(rtx, Some(Upscaler::Fsr3)), Upscaler::Fsr3);
        assert_eq!(pick(rtx, Some(Upscaler::Fsr1)), Upscaler::Fsr1);
        let dx12 = Offer { vulkan: false, ..rtx };
        assert_eq!(pick(dx12, Some(Upscaler::Dlss)), Upscaler::Fsr1);
        for u in Upscaler::ALL {
            assert_eq!(Upscaler::from_id(u.id()), Some(u));
        }
        assert_eq!(Upscaler::from_id(""), None);
        // DLSS failed: the setting's DLSS goes to the next best, FSR 3.1.
        let faults = Faults::default();
        faults.report(Upscaler::Dlss, "evaluate");
        let g = Graphics {
            upscaler: UpscalerSetting(Some(Upscaler::Dlss)),
            ..Graphics::default()
        };
        let up = resolve(rtx, &g, &faults);
        assert_eq!((up.chosen, up.active), (Upscaler::Dlss, Upscaler::Fsr3));
    }

    #[test]
    fn ultra_quality_is_77_percent() {
        let s = Upscale::default().scale();
        assert_eq!(s, 0.77);
        assert_eq!(render_size(UVec2::new(1600, 900), s), UVec2::new(1232, 693));
        assert_eq!(render_size(UVec2::new(1920, 1080), s), UVec2::new(1478, 832));
        assert_eq!(render_size(UVec2::new(2560, 1440), s), UVec2::new(1971, 1109));
        assert_eq!(render_size(UVec2::new(3840, 2160), s), UVec2::new(2957, 1663));
        assert_eq!(render_size(UVec2::ONE, s), UVec2::ONE);
        // log2(0.77) − 1 ≈ −1.377.
        assert!((mip_bias(s) + 1.377).abs() < 1e-3);
        let full = UVec2::new(1920, 1080);
        assert_eq!(render_size(full, Upscale::Quality.scale()), UVec2::new(1286, 724));
        assert_eq!(render_size(full, Upscale::Balanced.scale()), UVec2::new(1133, 637));
    }

    #[test]
    fn a_fault_is_told_once_and_falls_back() {
        let f = Faults::default();
        assert!(!f.failed(Upscaler::Fsr3));
        f.report(Upscaler::Fsr3, "dispatch");
        f.report(Upscaler::Fsr3, "dispatch again");
        assert!(f.failed(Upscaler::Fsr3) && !f.failed(Upscaler::Dlss));
        assert_eq!(f.take().len(), 1);
        assert!(f.take().is_empty());
    }
}
