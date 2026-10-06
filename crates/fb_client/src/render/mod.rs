//! How the game is drawn: the camera's pipeline
//! (HDR, PBR Neutral tone mapping, the grade), the sun, fog, sky and ambient light of the round's look,
//! and the graphics settings: the hardware tier, presets and switches (`quality.rs`).
mod decor;
pub mod emoji;
mod env;
mod fsr;
mod lod;
pub mod meshes;
mod motes;
pub mod portal;
pub mod props;
pub mod quality;
pub mod surface;
mod warmup;

use bevy::asset::embedded_asset;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::view::{ColorGrading, ColorGradingGlobal};
use bevy::shader::ShaderRef;
use fb_sim::looks::ResolvedLook;

use crate::game::Map;
use crate::view::MainCamera;

/// Lux of the sun per unit of a look's light, and the exposure that makes one unit
/// of light on a white surface come out as one: the look's colours keep their meaning.
pub const LUX: f32 = 1000.0;

/// Linear colour of a hex string.
pub fn linear(hex: &str) -> LinearRgba {
    crate::view::hex(hex).to_linear()
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

pub struct GfxPlugin;

impl Plugin for GfxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sky.wgsl");
        embedded_asset!(app, "surface.wgsl");
        app.add_plugins((
            MaterialPlugin::<SkyMaterial>::default(),
            surface::SurfacePlugin,
            quality::QualityPlugin,
            props::PropsPlugin,
            decor::DecorPlugin,
            motes::MotesPlugin,
            fsr::FsrPlugin,
            warmup::WarmupPlugin,
        ));
        app.init_resource::<LookShown>();
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
    camera: Query<Entity, With<MainCamera>>,
) {
    if let Ok(cam) = camera.single() {
        commands.entity(cam).insert((
            // (From the start, as in every round: the shaders warmed up before it are the round's.)
            env_light(&fb_sim::looks::classic(), &mut images),
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
    let v = |c: &str| linear(c).to_vec4();
    SkyUniform {
        top: v(l.sky.top),
        horizon: v(l.sky.horizon),
        cloud: v(l.sky.cloud),
        sun_color: v(l.sun.color),
        sun_dir: sun_dir(look).extend(l.sky.stars as f32),
        params: Vec4::new(LUX, 1.0, 0.0, 0.0),
    }
}

/// Lights and sky for the round's look: sky colours and stars, the sun's colour, strength and direction,
/// ambient light, fog, exposure and the grade.
fn apply_look(
    map: Option<Res<Map>>,
    mut shown: ResMut<LookShown>,
    mut commands: Commands,
    sky: Query<&MeshMaterial3d<SkyMaterial>, With<Sky>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut camera: Query<(Entity, &mut DistanceFog, &mut ColorGrading), With<MainCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut clear: ResMut<ClearColor>,
) {
    let Some(map) = map else { return };
    if shown.0 == Some(map.generation) {
        return;
    }
    shown.0 = Some(map.generation);
    let look = &map.look;
    let l = look.look;
    if let Ok(h) = sky.single()
        && let Some(mut m) = skies.get_mut(&h.0)
    {
        m.u = sky_uniform(look);
    }
    if let Ok((mut light, mut tf)) = sun.single_mut() {
        light.color = crate::view::hex(l.sun.color);
        light.illuminance = l.sun.intensity as f32 * LUX;
        *tf = Transform::from_translation(sun_dir(look) * SUN_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y);
    }
    let Ok((cam, mut fog, mut grade)) = camera.single_mut() else {
        return;
    };
    fog.color = crate::view::hex(l.fog.color);
    fog.falloff = FogFalloff::Linear {
        start: l.fog.near as f32,
        end: l.fog.far as f32,
    };
    grade.global = ColorGradingGlobal {
        exposure: (l.exposure as f32).log2(),
        post_saturation: l.saturation as f32,
        ..default()
    };
    clear.0 = crate::view::hex(l.sky.horizon);
    commands.entity(cam).insert(env_light(look, &mut images));
}

/// The look's ambient light (`env.rs`).
fn env_light(look: &ResolvedLook, images: &mut Assets<Image>) -> EnvironmentMapLight {
    let l = look.look;
    let c = |hex: &str| {
        let c = linear(hex);
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
