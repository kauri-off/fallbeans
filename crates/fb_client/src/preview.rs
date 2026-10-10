//! The player's bean on a stage of its own, drawn to an image the interface shows (the look's picker).
use bevy::app::{HierarchyPropagatePlugin, Propagate};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Exposure, Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use fb_net::Anim;
use fb_shared::outfit::Outfit;

use crate::bean::{BeanAnim, Expr, Frame};
use crate::beans::{Dress, Paints, Puppets, Rig, dress_rig, pose_rig, spawn_rig};
use crate::outfit::Tailor;
use crate::render::vfx::GlowMaterial;
use crate::render::{EnvLights, LUX};

/// Seen by the stage's camera and lit by its lights only.
const LAYER: usize = 7;
/// Far below every map.
const STAGE: Vec3 = Vec3::new(0.0, -600.0, 0.0);
/// Seconds after a turn by hand before the bean faces the front again.
const SETTLE_AFTER: f32 = 2.5;

/// What the bean on the stage wears: the interface sets it (the player's look, or one tried on) and turns the
/// stage on while it is shown.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct Showcase {
    pub on: bool,
    pub color: u8,
    pub outfit: Outfit,
}

/// The image the stage is drawn to (sized by the interface to where it shows it).
#[derive(Resource)]
pub struct Stage {
    pub image: Handle<Image>,
}

/// The bean's turn: dragged by hand, then back to its idle sway.
#[derive(Resource, Default)]
pub struct Turntable {
    yaw: f32,
    spin: f32,
    since_drag: f32,
}

impl Turntable {
    /// A drag of `dx` logical pixels.
    pub fn drag(&mut self, dx: f32, dt: f32) {
        let turn = dx * 0.01;
        self.yaw += turn;
        let rate = if dt > 0.0 { (turn / dt).clamp(-6.0, 6.0) } else { 0.0 };
        self.spin = (self.spin + rate) / 2.0;
        self.since_drag = 0.0;
    }
}

#[derive(Component)]
struct StageBean;

#[derive(Component)]
struct StageCamera;

pub struct PreviewPlugin {
    /// The stage's camera ever draws (not on the tests' noop device, whose limits no pass survives).
    pub draws: bool,
}

impl Plugin for PreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate));
        app.init_resource::<Showcase>();
        app.init_resource::<Turntable>();
        app.add_systems(Startup, setup);
        app.add_systems(
            PostUpdate,
            (switch.run_if(draws(self.draws)), dress, animate)
                .chain()
                .after(crate::beans::animate_beans)
                .before(TransformSystems::Propagate),
        );
    }
}

/// What the stage is made of.
#[derive(SystemParam)]
struct Props<'w> {
    assets: Res<'w, AssetServer>,
    paints: ResMut<'w, Paints>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    glows: ResMut<'w, Assets<GlowMaterial>>,
    images: ResMut<'w, Assets<Image>>,
    env: ResMut<'w, EnvLights>,
}

fn setup(mut commands: Commands, mut props: Props) {
    let Props {
        assets,
        paints,
        meshes,
        materials,
        glows,
        images,
        env,
    } = &mut props;
    let image = images.add(Image::new_target_texture(256, 256, TextureFormat::Rgba8UnormSrgb, None));
    let look = fb_sim::looks::classic().look;
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        Exposure {
            ev100: (LUX / 1.2).log2(),
        },
        env.of(look, images),
        Projection::Perspective(PerspectiveProjection {
            fov: 21f32.to_radians(),
            near: 0.5,
            far: 30.0,
            ..default()
        }),
        Transform::from_translation(STAGE + Vec3::new(0.0, 1.35, 6.6))
            .looking_at(STAGE + Vec3::new(0.0, 1.03, 0.0), Vec3::Y),
        RenderLayers::layer(LAYER),
        StageCamera,
    ));
    commands.insert_resource(Stage { image });
    commands.spawn((
        DirectionalLight {
            illuminance: 2.4 * LUX,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(STAGE + Vec3::new(3.0, 5.0, 6.0)).looking_at(STAGE, Vec3::Y),
        RenderLayers::layer(LAYER),
    ));
    // A soft shadow under the feet.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(1.7, 1.7))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.24, 0.16, 0.08, 0.3),
            base_color_texture: Some(images.add(blob())),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })),
        Transform::from_translation(STAGE + Vec3::Y * 0.005),
        RenderLayers::layer(LAYER),
    ));
    let bean = commands
        .spawn((
            StageBean,
            Transform::from_translation(STAGE),
            Visibility::default(),
            Propagate(RenderLayers::layer(LAYER)),
        ))
        .id();
    let rig = spawn_rig(&mut commands, bean, assets, paints, meshes, materials, glows);
    commands
        .entity(bean)
        .insert((rig, Dress::default(), BeanAnim::new(fb_shared::PlayerId(u32::MAX))));
}

/// A round blot, dark in the middle and fading out to its edge.
fn blob() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension};
    const N: u32 = 64;
    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / N as f32 * 2.0 - Vec2::ONE;
            let a = (1.0 - d.length_squared()).clamp(0.0, 1.0).powi(2);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn draws(on: bool) -> impl FnMut() -> bool {
    move || on
}

/// The stage draws only while the interface shows it.
fn switch(showcase: Res<Showcase>, mut camera: Single<&mut Camera, With<StageCamera>>) {
    if camera.is_active != showcase.on {
        camera.is_active = showcase.on;
    }
}

fn dress(
    mut commands: Commands,
    showcase: Res<Showcase>,
    mut bean: Single<(&Rig, &mut Dress, &mut BeanAnim), With<StageBean>>,
    mut paints: ResMut<Paints>,
    mut tailor: Tailor,
    mut worn: Local<Option<(u8, Outfit)>>,
) {
    let (rig, dress, anim) = &mut *bean;
    if !rig.ready() {
        return;
    }
    let now = (showcase.color, showcase.outfit);
    // A change shown on a ready bean: a little joy at it.
    if worn.replace(now).is_some_and(|was| was != now) {
        anim.react(Expr::Grin, 0.9);
    }
    dress_rig(
        &mut commands,
        rig,
        dress,
        (showcase.color, showcase.outfit, false, false),
        &mut paints,
        &mut tailor,
    );
}

fn animate(
    time: Res<Time<Real>>,
    showcase: Res<Showcase>,
    mut table: ResMut<Turntable>,
    mut puppets: Puppets,
    mut bean: Single<(&Rig, &Dress, &mut BeanAnim, &mut Transform), With<StageBean>>,
) {
    if !showcase.on {
        return;
    }
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let (rig, dress, anim, root) = &mut *bean;
    table.since_drag += dt;
    if table.since_drag > 0.05 {
        // Let go: it spins on a little, then turns back to the front, swaying.
        table.yaw += table.spin * dt;
        table.spin *= (-dt * 6.0).exp();
        if table.since_drag > SETTLE_AFTER {
            let sway = (t * 0.6).sin() * 0.35;
            let front = sway + (table.yaw / core::f32::consts::TAU).round() * core::f32::consts::TAU;
            table.yaw += (front - table.yaw) * (1.0 - (-dt * 2.5).exp());
        }
    }
    root.rotation = Quat::from_rotation_y(table.yaw);
    anim.animate(
        dt,
        &Frame {
            vel: Vec3::ZERO,
            anim: Anim::Idle,
            t,
            land_impact: 0.0,
            tilt: 0.0,
            tilt_dir: 0.0,
            yaw: table.yaw,
            grab_at: None,
            size: 1.0,
            power: None,
            pose: None,
        },
    );
    pose_rig(&mut puppets, rig, dress, anim, root, None, t, dt);
}
