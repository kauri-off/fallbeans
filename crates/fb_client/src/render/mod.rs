//! The camera's pipeline (HDR, tone mapping, grade), the round's sun, fog, sky and ambient light, and the graphics
//! settings (`quality.rs`, `upscale.rs`).
pub mod ao;
mod cloth;
mod decor;
#[cfg(feature = "dlss")]
pub mod dlss;
pub mod emoji;
mod env;
#[cfg(any(windows, target_os = "linux"))]
mod ffx;
mod fog;
mod fsr;
#[cfg(any(windows, target_os = "linux"))]
mod fsr3;
mod lod;
pub mod meshes;
mod motes;
pub mod portal;
pub mod props;
pub mod quality;
mod reactive;
pub mod surface;
pub mod upscale;
pub mod vfx;
pub mod warmup;

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::view::{ColorGrading, ColorGradingGlobal};
use bevy::shader::ShaderRef;
use fb_shared::Rgb;
use fb_sim::looks::{Look, ResolvedLook};

use crate::game::Map;
use crate::view::MainCamera;

/// Lux of the sun per unit of a look's light, and the exposure that makes one unit
/// of light on a white surface come out as one: the look's colours keep their meaning.
pub const LUX: f32 = 1000.0;

pub fn linear(c: Rgb) -> LinearRgba {
    crate::view::color(c).to_linear()
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub u: SkyUniform,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub struct SkyUniform {
    pub top: Vec4,
    pub horizon: Vec4,
    pub cloud: Vec4,
    pub sun_color: Vec4,
    pub sun_dir: Vec4,
    pub params: Vec4,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/sky.wgsl".into()
    }

    // (Alpha-masked, though nothing is cut away: the alpha-mask phase is drawn after the opaque one in the
    // same pass, so the sky comes last and the depth test skips its maths wherever the map covers it.)
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.5)
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
        // Seen from inside.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Component)]
pub struct Sky;

#[derive(Component)]
pub struct Sun;

/// Radius of the sky dome around the camera.
const SKY_R: f32 = 900.0;
/// How far the sun's position is from what it lights (only its direction counts).
const SUN_DISTANCE: f32 = 42.0;

pub struct GfxPlugin {
    /// The loading screen's warm-up at the start and after a change of the graphics (`warmup.rs`).
    pub warmup: bool,
}

impl Plugin for GfxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sky.wgsl");
        embedded_asset!(app, "surface.wgsl");
        app.add_plugins((
            MaterialPlugin::<SkyMaterial>::default(),
            surface::SurfacePlugin,
            ao::AoPlugin,
            quality::QualityPlugin,
            props::PropsPlugin,
            decor::DecorPlugin,
            motes::MotesPlugin,
            fog::FogPlugin,
            vfx::VfxPlugin,
            portal::PortalPlugin,
            fsr::FsrPlugin,
            upscale::UpscalePlugin,
            warmup::WarmupPlugin { on: self.warmup },
        ));
        app.init_resource::<LookShown>();
        app.init_resource::<EnvLights>();
        app.add_systems(Startup, setup.after(crate::view::setup_camera));
        app.add_systems(Update, (apply_look, sky_clouds).chain());
        app.add_systems(
            PostUpdate,
            follow_camera
                .after(crate::camera::place_camera)
                .before(TransformSystems::Propagate),
        );
    }
}

/// The look the scene is lit for (the arena's generation).
#[derive(Resource, Default)]
struct LookShown(Option<u32>);

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut env: ResMut<EnvLights>,
    camera: Query<Entity, With<MainCamera>>,
) {
    if let Ok(cam) = camera.single() {
        commands.entity(cam).insert((
            // (From the start, as in every round: the shaders warmed up before it are the round's.)
            env.of(fb_sim::looks::classic().look, &mut images),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Exposure {
                ev100: (LUX / 1.2).log2(),
            },
            ColorGrading::default(),
            DistanceFog {
                color: Color::srgb(0.95, 0.83, 0.97),
                falloff: FogFalloff::Linear {
                    start: 120.0,
                    end: 520.0,
                },
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: 70f32.to_radians(),
                near: 0.25,
                far: 1100.0,
                ..default()
            }),
        ));
    }
    commands.spawn((
        Sky,
        Mesh3d(meshes.add(Sphere::new(SKY_R).mesh().uv(48, 24))),
        MeshMaterial3d(skies.add(SkyMaterial {
            u: sky_uniform(&fb_sim::looks::classic()),
        })),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    ));
    commands.spawn((
        Sun,
        DirectionalLight {
            illuminance: 2.2 * LUX,
            shadow_maps_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder::default().build(),
        Transform::from_xyz(9.0, 40.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // (The ambient light comes from the look's environment map.)
    commands.insert_resource(GlobalAmbientLight::NONE);
}

/// Towards the sun, from its compass angle and height.
fn sun_dir(look: &ResolvedLook) -> Vec3 {
    let az = (look.look.sun.azimuth as f32).to_radians();
    let el = (look.look.sun.elevation as f32).to_radians();
    Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin())
}

fn sky_uniform(look: &ResolvedLook) -> SkyUniform {
    let l = look.look;
    let v = |c: Rgb| linear(c).to_vec4();
    SkyUniform {
        top: v(l.sky.top),
        horizon: v(l.sky.horizon),
        cloud: v(l.sky.cloud),
        sun_color: v(l.sun.color),
        sun_dir: sun_dir(look).extend(l.sky.stars as f32),
        params: Vec4::new(LUX, 1.0, 0.0, 0.0),
    }
}

/// What a look lights: the sky and its material, the sun, the clear colour, and the environment light with
/// the images it is made in.
#[derive(SystemParam)]
struct Lighting<'w, 's> {
    sky: Query<'w, 's, &'static MeshMaterial3d<SkyMaterial>, With<Sky>>,
    skies: ResMut<'w, Assets<SkyMaterial>>,
    sun: Query<'w, 's, (&'static mut DirectionalLight, &'static mut Transform), With<Sun>>,
    clear: ResMut<'w, ClearColor>,
    env: ResMut<'w, EnvLights>,
    images: ResMut<'w, Assets<Image>>,
}

/// Lights and sky for the round's look: sky colours and stars, the sun's colour, strength and direction,
/// ambient light, fog, exposure and the grade.
fn apply_look(
    map: Option<Res<Map>>,
    mut shown: ResMut<LookShown>,
    mut commands: Commands,
    mut lighting: Lighting,
    mut camera: Query<(Entity, &mut DistanceFog, &mut ColorGrading), With<MainCamera>>,
) {
    let Some(map) = map else { return };
    if shown.0 == Some(map.generation) {
        return;
    }
    shown.0 = Some(map.generation);
    let look = &map.look;
    let l = look.look;
    if let Ok(h) = lighting.sky.single()
        && let Some(mut m) = lighting.skies.get_mut(&h.0)
    {
        m.u = sky_uniform(look);
    }
    if let Ok((mut light, mut tf)) = lighting.sun.single_mut() {
        light.color = crate::view::color(l.sun.color);
        light.illuminance = l.sun.intensity as f32 * LUX;
        *tf = Transform::from_translation(sun_dir(look) * SUN_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y);
    }
    let Ok((cam, mut fog, mut grade)) = camera.single_mut() else {
        return;
    };
    // (How near it begins depends on the haze: `fog.rs`.)
    fog.color = crate::view::color(l.fog.color);
    grade.global = ColorGradingGlobal {
        exposure: (l.exposure as f32).log2(),
        post_saturation: l.saturation as f32,
        ..default()
    };
    lighting.clear.0 = crate::view::color(l.sky.horizon);
    commands.entity(cam).insert(lighting.env.of(l, &mut lighting.images));
}

/// The looks' ambient light (`env.rs`), made once per look (≈70 KB each) and kept: a round of a look seen
/// before makes none.
#[derive(Resource, Default)]
pub struct EnvLights(HashMap<fb_sim::looks::LookId, EnvironmentMapLight>);

impl EnvLights {
    pub fn of(&mut self, l: &'static Look, images: &mut Assets<Image>) -> EnvironmentMapLight {
        self.0
            .entry(l.id)
            .or_insert_with(|| {
                let c = |rgb: Rgb| {
                    let c = linear(rgb);
                    [c.red, c.green, c.blue]
                };
                let mut cube = |map| {
                    images.add(env::cube(
                        c(l.hemi.sky),
                        c(l.hemi.ground),
                        l.hemi.intensity as f32,
                        l.env as f32,
                        map,
                    ))
                };
                EnvironmentMapLight {
                    diffuse_map: cube(env::Map::Diffuse),
                    specular_map: cube(env::Map::Specular),
                    intensity: LUX,
                    ..default()
                }
            })
            .clone()
    }

    /// Every look's, ahead (the warm-up).
    pub fn make_all(&mut self, images: &mut Assets<Image>) -> usize {
        for l in fb_sim::looks::LOOKS {
            self.of(l, images);
        }
        self.0.len()
    }
}

/// No clouds on Low (ten octaves of noise over most of the screen); the look sets the rest of the sky.
fn sky_clouds(
    quality: Option<Res<quality::Quality>>,
    sky: Query<&MeshMaterial3d<SkyMaterial>, With<Sky>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
) {
    let on = if quality.is_some_and(|q| q.preset == quality::Preset::Low) {
        0.0
    } else {
        1.0
    };
    let Ok(h) = sky.single() else { return };
    // (Looked at first: touching the material would prepare it again.)
    if skies.get(&h.0).is_some_and(|m| m.u.params.y != on)
        && let Some(mut m) = skies.get_mut(&h.0)
    {
        m.u.params.y = on;
    }
}

fn follow_camera(
    camera: Query<&Transform, (With<MainCamera>, Without<Sky>)>,
    mut sky: Query<&mut Transform, With<Sky>>,
) {
    let (Ok(cam), Ok(mut tf)) = (camera.single(), sky.single_mut()) else {
        return;
    };
    tf.translation = cam.translation;
}
