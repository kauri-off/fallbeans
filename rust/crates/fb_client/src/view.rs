//! Drawing: the map from its `SceneDesc` (primitives and glTF models), beans, bonuses, camera.
use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use fb_arena::ArenaKind;
use fb_net::*;
use fb_shared::COLORS;
use fb_sim::math::M4;
use fb_sim::physics::power;
use fb_sim::scene::{PrimKind, SceneItem};
use lightyear::prelude::*;

use crate::game::{CameraAngles, Map, PrevPos};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.53, 0.78, 0.98)));
        app.add_systems(Startup, setup);
        app.add_systems(
            PostUpdate,
            (
                spawn_map,
                pose_map,
                spawn_beans,
                place_beans,
                place_bonuses,
                follow_camera,
            )
                .chain()
                .before(TransformSystems::Propagate),
        );
        app.init_resource::<Spectate>();
        app.add_systems(Update, (spectate, mouse_look).chain());
        app.add_observer(tint_bean);
    }
}

/// Entities of the drawn map (despawned with it), each following one node.
#[derive(Component)]
struct MapPiece {
    node: u32,
}

#[derive(Component)]
struct MapRoot(u32);

#[derive(Component)]
struct BonusView(u32);

#[derive(Component)]
pub struct BeanView;

/// Colour of the bean model to paint once its scene is in.
#[derive(Component)]
struct BeanTint(Color);

#[derive(Component)]
pub struct MainCamera;

/// Who the camera follows while the player has no bean in a round (finished, out, or not taking part).
#[derive(Resource, Default)]
pub struct Spectate {
    /// None: the overview of the map.
    pub target: Option<u32>,
    /// The player picked the target; until then (or until it leaves) the first bean in play is followed.
    manual: bool,
    generation: u32,
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        Transform::from_xyz(0.0, 14.0, 22.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
        DistanceFog {
            color: Color::srgb(0.62, 0.8, 0.98),
            falloff: FogFalloff::Linear {
                start: 60.0,
                end: 220.0,
            },
            ..default()
        },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(20.0, 40.0, 14.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.85, 0.9, 1.0),
        brightness: 600.0,
        ..default()
    });
}

pub fn hex(c: &str) -> Color {
    Srgba::hex(c.trim_start_matches('#'))
        .map(Color::from)
        .unwrap_or(Color::WHITE)
}

fn mat4(m: &M4) -> Mat4 {
    Mat4::from_cols_array(&m.0.map(|v| v as f32))
}

fn spawn_map(
    mut commands: Commands,
    map: Option<Res<Map>>,
    roots: Query<(Entity, &MapRoot)>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(map) = map else { return };
    if roots.iter().any(|(_, r)| r.0 == map.generation) {
        return;
    }
    for (e, _) in &roots {
        commands.entity(e).despawn();
    }
    let root = commands
        .spawn((MapRoot(map.generation), Transform::default(), Visibility::default()))
        .id();
    // Shared handles: identical primitives and colours batch into one draw.
    let mut mats: HashMap<&str, Handle<StandardMaterial>> = HashMap::new();
    let mut prims: HashMap<(PrimKind, [u64; 3]), Handle<Mesh>> = HashMap::new();
    for item in &map.scene.items {
        let (node, child) = match item {
            SceneItem::Prim {
                node, kind, dims, pal, ..
            } => {
                let mesh = prims
                    .entry((*kind, dims.map(f64::to_bits)))
                    .or_insert_with(|| match kind {
                        PrimKind::Box => meshes.add(Cuboid::new(dims[0] as f32, dims[1] as f32, dims[2] as f32)),
                        PrimKind::Cyl => meshes.add(
                            Cylinder::new(dims[0] as f32, dims[1] as f32)
                                .mesh()
                                .resolution(dims[2] as u32),
                        ),
                        PrimKind::Sphere => meshes.add(Sphere::new(dims[0] as f32).mesh().uv(32, 18)),
                    })
                    .clone();
                let mat = mats
                    .entry(pal[0])
                    .or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: hex(pal[0]),
                            perceptual_roughness: 0.6,
                            ..default()
                        })
                    })
                    .clone();
                (*node, commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat))).id())
            }
            SceneItem::Model { node, name, .. } => {
                let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
                (*node, commands.spawn(WorldAssetRoot(scene)).id())
            }
            SceneItem::Special { .. } => continue,
        };
        commands.entity(child).insert((
            MapPiece { node },
            Transform::from_matrix(mat4(&map.render.get(node).world)),
            Visibility::default(),
            ChildOf(root),
        ));
    }
    for b in &map.bonuses.list {
        let color = match b.kind {
            power::GIANT => "#ff6f91",
            power::JUMP => "#58d68d",
            _ => "#ffd23f",
        };
        commands.spawn((
            BonusView(b.i),
            Mesh3d(meshes.add(Sphere::new(0.62).mesh().uv(32, 18))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: hex(color).with_alpha(0.55),
                emissive: hex(color).to_linear() * 0.6,
                alpha_mode: AlphaMode::Blend,
                ..default()
            })),
            Transform::from_translation(b.pos.as_vec3() + Vec3::Y * 1.05),
            Visibility::Hidden,
            ChildOf(root),
        ));
    }
}

/// The frame's sim time: the predicted tick plus how far into the next one the frame is.
pub fn frame_tick(timeline: &LocalTimeline, fixed: &Time<Fixed>) -> f64 {
    timeline.tick().0 as f64 + fixed.overstep_fraction() as f64
}

fn pose_map(
    map: Option<ResMut<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut pieces: Query<(&MapPiece, &mut Transform, &mut Visibility)>,
) {
    let Some(mut map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed) - 1.0);
    let map = &mut *map;
    map.world.pose(t, &mut map.render);
    for (piece, mut tf, mut vis) in &mut pieces {
        let next = Transform::from_matrix(mat4(&map.render.get(piece.node).world));
        if *tf != next {
            *tf = next;
        }
        let shown = if map.render.shown(piece.node) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        vis.set_if_neq(shown);
    }
}

fn spawn_beans(
    mut commands: Commands,
    beans: Query<
        (Entity, &BeanColor),
        (
            With<PlayerId>,
            Without<BeanView>,
            Or<(With<Predicted>, With<Interpolated>)>,
        ),
    >,
    assets: Res<AssetServer>,
) {
    for (e, color) in &beans {
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/bean.glb"));
        let tint = hex(COLORS[color.0 as usize % COLORS.len()]);
        commands
            .entity(e)
            .insert((BeanView, Transform::default(), Visibility::default()))
            .with_child((WorldAssetRoot(scene), BeanTint(tint)));
    }
}

/// Paints the bean model's body in the player's colour.
fn tint_bean(
    trigger: On<WorldInstanceReady>,
    tints: Query<&BeanTint>,
    children: Query<&Children>,
    mut parts: Query<(&GltfMaterialName, &mut MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(tint) = tints.get(trigger.entity) else { return };
    let mut body = None;
    for e in children.iter_descendants(trigger.entity) {
        if let Ok((name, mut mat)) = parts.get_mut(e)
            && name.0 == "Body"
        {
            let h = body.get_or_insert_with(|| {
                let mut m = materials.get(&mat.0).cloned().unwrap_or_default();
                m.base_color = tint.0;
                materials.add(m)
            });
            mat.0 = h.clone();
        }
    }
}

fn bean_transform(pos: Vec3, yaw: f32, tilt: f32, tilt_dir: f32, size: f32) -> Transform {
    let pivot = Vec3::Y * 0.5 * size;
    let axis = Vec3::new(tilt_dir.cos(), 0.0, -tilt_dir.sin());
    let tip = if tilt.abs() > 1e-4 {
        Quat::from_axis_angle(axis, tilt)
    } else {
        Quat::IDENTITY
    };
    let rot = tip * Quat::from_rotation_y(yaw);
    Transform {
        translation: pos + pivot - tip * pivot,
        rotation: rot,
        scale: Vec3::splat(size),
    }
}

type OtherBeans<'w, 's> = Query<
    'w,
    's,
    (
        &'static PlayerId,
        &'static RemotePose,
        &'static mut Transform,
        &'static mut Visibility,
    ),
    (With<Interpolated>, With<BeanView>, Without<Predicted>),
>;

fn place_beans(
    fixed: Res<Time<Fixed>>,
    map: Option<Res<Map>>,
    own: Query<(&BodyFull, &PrevPos, &mut Transform), (With<Predicted>, With<BeanView>)>,
    mut others: OtherBeans,
) {
    let a = fixed.overstep_fraction();
    for (full, prev, mut tf) in own {
        let b = &full.body;
        let pos = prev.0.as_vec3().lerp(b.pos.as_vec3(), a);
        *tf = bean_transform(pos, b.yaw as f32, b.tilt as f32, b.tilt_dir as f32, b.size as f32);
    }
    for (id, p, mut tf, mut vis) in &mut others {
        *tf = bean_transform(p.pos, p.yaw, p.tilt, p.tilt_dir, p.size);
        // Finished or out: gone at once (the last snapshots may still carry the bean).
        let gone = map.as_ref().is_some_and(|m| m.gone(id.0));
        vis.set_if_neq(if gone {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
}

fn place_bonuses(
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    mut views: Query<(&BonusView, &mut Transform, &mut Visibility)>,
) {
    let Some(map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed));
    for (v, mut tf, mut vis) in &mut views {
        let Some(b) = map.bonuses.list.get(v.0 as usize) else {
            continue;
        };
        let shown = t >= b.appear_at && b.taken_by.is_none();
        vis.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        tf.translation.y = b.pos.y as f32 + 1.05 + (time.elapsed_secs() * 2.4 + v.0 as f32).sin() * 0.15;
    }
}

fn follow_camera(
    cam: Res<CameraAngles>,
    map: Option<Res<Map>>,
    spectate: Res<Spectate>,
    own: Query<&Transform, (With<Predicted>, With<BeanView>, Without<MainCamera>)>,
    others: Query<(&PlayerId, &Transform), (With<Interpolated>, With<BeanView>, Without<MainCamera>)>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
) {
    let Ok(mut tf) = camera.single_mut() else { return };
    let followed = own.single().ok().or_else(|| {
        let id = spectate.target?;
        others.iter().find(|(p, _)| p.0 == id).map(|(_, t)| t)
    });
    let target = match followed {
        Some(t) => t.translation + Vec3::Y * 1.3,
        None => map
            .and_then(|m| m.spec.view)
            .map_or(Vec3::new(0.0, 2.0, 0.0), |v| v.as_vec3()),
    };
    let fwd = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
    let dist = 10.0;
    let eye = target - fwd * dist * cam.pitch.cos() + Vec3::Y * dist * cam.pitch.sin();
    *tf = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

const PREV_KEYS: [KeyCode; 3] = [KeyCode::ArrowLeft, KeyCode::KeyA, KeyCode::KeyQ];
const NEXT_KEYS: [KeyCode; 3] = [KeyCode::ArrowRight, KeyCode::KeyD, KeyCode::KeyE];

/// Without a bean of one's own in a round: follow someone still in play, or look over the map. The keys
/// (and, with the mouse captured, its buttons) go through the beans and the overview.
fn spectate(
    map: Option<Res<Map>>,
    own: Query<(), (With<Predicted>, With<PlayerId>)>,
    beans: Query<&PlayerId, (With<Interpolated>, Without<Predicted>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut spec: ResMut<Spectate>,
) {
    let Some(map) = map else { return };
    if spec.generation != map.generation {
        *spec = Spectate {
            generation: map.generation,
            ..default()
        };
    }
    if !own.is_empty() || map.round.kind != ArenaKind::Round {
        spec.target = None;
        return;
    }
    let mut ids: Vec<u32> = beans.iter().map(|p| p.0).filter(|id| !map.gone(*id)).collect();
    ids.sort_unstable();
    ids.dedup();
    if spec.target.is_some_and(|t| !ids.contains(&t)) {
        spec.manual = false;
    }
    // (Checked before `mouse_look`: the click that captures the mouse does not switch.)
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    let dir = if keys.any_just_pressed(PREV_KEYS) || (captured && mouse.just_pressed(MouseButton::Right)) {
        -1
    } else if keys.any_just_pressed(NEXT_KEYS) || (captured && mouse.just_pressed(MouseButton::Left)) {
        1
    } else {
        0
    };
    if dir != 0 {
        let all: Vec<Option<u32>> = core::iter::once(None).chain(ids.iter().copied().map(Some)).collect();
        let i = all.iter().position(|t| *t == spec.target).unwrap_or(0) as i32;
        spec.target = all[(i + dir).rem_euclid(all.len() as i32) as usize];
        spec.manual = true;
    } else if !spec.manual {
        spec.target = ids.first().copied();
    }
}

fn mouse_look(
    mut motion: MessageReader<MouseMotion>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: ResMut<CameraAngles>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        // Locked (pointer constraints) where the platform has it; Windows only confines.
        cursor.grab_mode = if cfg!(target_os = "windows") {
            CursorGrabMode::Confined
        } else {
            CursorGrabMode::Locked
        };
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    if cursor.grab_mode == CursorGrabMode::None {
        motion.clear();
        return;
    }
    for m in motion.read() {
        cam.yaw -= m.delta.x * 0.004;
        cam.pitch = (cam.pitch + m.delta.y * 0.003).clamp(-0.2, 1.2);
    }
}
