//! Drawing: the map from its `SceneDesc` (primitives, glTF models and specials), beans, bonuses, camera.
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::world_serialization::WorldAssetRoot;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_sim::math::Affine;
use fb_sim::physics::power;
use fb_sim::scene::{PrimKind, SceneItem};
use lightyear::prelude::*;

use crate::beans;
use crate::camera::{PITCH_MAX, PITCH_MIN};
use crate::game::{CameraAngles, Gate, Map};
use crate::render::meshes;
use crate::render::props::Prop;
use crate::render::quality::Quality;
use crate::render::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::settings::Controls;
use crate::specials::{MapPrim, SpecialCache, SpecialRoot, pose_specials};
use fb_sim::looks::{Pattern, ResolvedLook};
use fb_sim::scene::{Palette, pal};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(1.0, 0.85, 0.95)));
        app.add_systems(Startup, setup_camera);
        app.add_systems(
            PostUpdate,
            (
                spawn_map,
                pose_map,
                pose_specials,
                beans::spawn_beans,
                beans::place_beans,
                beans::dress_beans,
                beans::animate_beans,
                place_bonuses,
            )
                .chain()
                .before(TransformSystems::Propagate),
        );
        app.init_resource::<Spectate>();
        app.init_resource::<SpecialCache>();
        app.add_systems(Update, (spectate, mouse_look).chain());
    }
}

/// A primitive's mesh at one level of detail (a child of its `MapPrim`).
#[derive(Component)]
pub struct PrimLevel;

/// Entities of the drawn map (despawned with it), each following one node.
#[derive(Component)]
pub struct MapPiece {
    pub node: u32,
}

#[derive(Component)]
pub struct MapRoot(pub u32);

#[derive(Component)]
struct BonusView(u32);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BonusPart {
    Bubble,
    Card,
    Ring,
}

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

pub fn setup_camera(mut commands: Commands, offscreen: Option<Res<crate::Offscreen>>) {
    let mut cam = commands.spawn((
        Camera3d::default(),
        MainCamera,
        // (Other beans' sounds are placed against it: `audio.rs`.)
        SpatialListener::new(4.0),
        Transform::from_xyz(0.0, 14.0, 22.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));
    if let Some(o) = offscreen {
        cam.insert((bevy::camera::RenderTarget::Image(o.0.clone().into()), IsDefaultUiCamera));
    }
}

pub fn hex(c: &str) -> Color {
    Srgba::hex(c.trim_start_matches('#'))
        .map(Color::from)
        .unwrap_or(Color::WHITE)
}

fn mat4(m: &Affine) -> Mat4 {
    Mat4::from(bevy::math::Affine3A::from_cols_array(
        &m.to_cols_array().map(|v| v as f32),
    ))
}

fn spawn_map(
    mut commands: Commands,
    map: Option<Res<Map>>,
    roots: Query<(Entity, &MapRoot)>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
    quality: Option<Res<Quality>>,
    display: Res<crate::settings::Display>,
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
    // Shared handles: identical primitives and materials batch into one draw.
    let mut prims: HashMap<(PrimKind, [u64; 3], u32, u32), Handle<Mesh>> = HashMap::new();
    let lod_k = meshes::lod_k(display.fov, quality.as_ref().map(|q| q.preset));
    let mut lift = 0u32;
    for (i, item) in map.scene.items.iter().enumerate() {
        let (node, child) = match item {
            SceneItem::Prim {
                node,
                kind,
                dims,
                pal,
                freq,
                surface,
                pattern,
            } => {
                lift += 1;
                let l = 1 + lift % meshes::LIFTS;
                let spec = prim_spec(&map.look, *kind, *dims, *pal, *freq, *surface, *pattern);
                let mat = surfaces.material(&spec, &mut images, &mut surface_mats);
                let piece = commands.spawn(MapPrim(spec, mat.clone())).id();
                // Each level of detail a child, shown by the camera's distance (cross-faded).
                let ranges = meshes::level_ranges(*kind, *dims, lod_k);
                let single = ranges.len() == 1;
                for (level, range) in ranges {
                    let mesh = prims
                        .entry((*kind, dims.map(f64::to_bits), l, meshes::level_id(*kind, *dims, level)))
                        .or_insert_with(|| meshes.add(meshes::prim(*kind, *dims, level, l)))
                        .clone();
                    let mut child = commands.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(mat.clone()),
                        PrimLevel,
                        Transform::default(),
                        ChildOf(piece),
                    ));
                    if !single {
                        child.insert(range);
                    }
                }
                (*node, piece)
            }
            SceneItem::Model { node, name, tint } => {
                let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
                let n = map.render.get(*node);
                let piece = commands.spawn_empty().id();
                commands.spawn((
                    WorldAssetRoot(scene),
                    Prop::new(name, *tint, n.pos.x, n.pos.z),
                    Transform::default(),
                    Visibility::default(),
                    ChildOf(piece),
                ));
                (*node, piece)
            }
            SceneItem::Special { node, .. } => {
                let root = SpecialRoot {
                    item: i,
                    slots: Vec::new(),
                };
                (*node, commands.spawn(root).id())
            }
        };
        commands.entity(child).insert((
            MapPiece { node },
            Transform::from_matrix(mat4(&map.render.get(node).world)),
            Visibility::default(),
            ChildOf(root),
        ));
    }
    for b in &map.bonuses.list {
        let color = hex(match b.kind {
            power::GIANT => "#ff6f91",
            power::JUMP => "#58d68d",
            _ => "#ffd23f",
        });
        let (icon, _) = crate::ui::text::bonus(b.kind);
        let bubble = MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color.with_alpha(0.45),
            emissive: color.to_linear() * 0.45,
            perceptual_roughness: 0.15,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }));
        let card = MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(crate::render::emoji::board(icon, color))),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        }));
        let ring = MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color.with_alpha(0.6),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            ..default()
        }));
        commands
            .spawn((
                BonusView(b.i),
                Transform::from_translation(b.pos.as_vec3()),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .with_children(|g| {
                g.spawn((
                    BonusPart::Bubble,
                    Mesh3d(meshes.add(Sphere::new(0.62).mesh().uv(32, 20))),
                    bubble,
                    Transform::from_xyz(0.0, 1.05, 0.0),
                ));
                g.spawn((
                    BonusPart::Card,
                    Mesh3d(meshes.add(Circle::new(0.42).mesh().resolution(32))),
                    card,
                    Transform::from_xyz(0.0, 1.05, 0.0),
                ));
                g.spawn((
                    BonusPart::Ring,
                    Mesh3d(meshes.add(Annulus::new(0.75, 1.0).mesh().resolution(40))),
                    ring,
                    Transform::from_xyz(0.0, 0.04, 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                    bevy::light::NotShadowCaster,
                ));
            });
    }
}

/// The classic palettes (`scene::pal`) in the order of the looks' palettes.
pub const PALETTES: [Palette; 9] = [
    pal::BLUE,
    pal::PURPLE,
    pal::PINK,
    pal::YELLOW,
    pal::GREEN,
    pal::WHITE,
    pal::ORANGE,
    pal::RED,
    pal::TEAL,
];

/// What a primitive is painted with: the palette as the
/// round's look repaints it with the look's pattern, or a plain colour; and its surface (padded for big
/// floors, rubber for balls, plastic otherwise).
pub fn prim_spec(
    look: &ResolvedLook,
    kind: PrimKind,
    dims: [f64; 3],
    pal: Palette,
    freq: Option<f64>,
    surface: Option<&'static str>,
    pattern: Option<&'static str>,
) -> Spec {
    let [a, b, c] = dims;
    let big = match kind {
        PrimKind::Box => a * c >= 30.0 && a.min(c) >= 3.0 && b <= 3.0,
        PrimKind::Cyl => a >= 4.0 && b <= 3.0,
        PrimKind::Sphere => false,
    };
    let fallback = match kind {
        PrimKind::Sphere => Kind::Rubber,
        _ if big => Kind::Padded,
        _ => Kind::Plastic,
    };
    let kind = surface.and_then(Kind::of).unwrap_or(fallback);
    if pal[0] == pal[1] {
        return Spec::plain(hex(pal[0]).to_linear(), Some(kind));
    }
    let tones: [String; 2] = match PALETTES.iter().position(|p| *p == pal) {
        Some(i) if look.look.id != "classic" => look.palette[i].clone(),
        _ => [pal[0].to_string(), pal[1].to_string()],
    };
    Spec {
        paint: Some(Paint {
            c1: hex(&tones[0]).to_linear(),
            c2: hex(&tones[1]).to_linear(),
            freq: freq.unwrap_or(0.25) as f32,
            dir: Vec2::ONE,
            speed: 0.0,
            kind: pattern.and_then(Pattern::of).unwrap_or(look.pattern),
        }),
        ..Spec::plain(LinearRgba::WHITE, Some(kind))
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

/// Pops in, bobs and turns; taken: swells and vanishes.
fn place_bonuses(
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut views: Query<(&BonusView, &Children, &mut Visibility)>,
    mut parts: Query<(&BonusPart, &mut Transform)>,
) {
    let Some(map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed));
    for (v, children, mut vis) in &mut views {
        let Some(b) = map.bonuses.list.get(v.0 as usize) else {
            continue;
        };
        let shown = t >= b.appear_at && b.taken_by.is_none_or(|_| t < b.taken_at + 0.35);
        vis.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }
        let grow = ((t - b.appear_at) / 0.5 + if b.appear_at <= 0.0 { 1.0 } else { 0.0 }).min(1.0);
        let gone = if b.taken_by.is_none() {
            0.0
        } else {
            (t - b.taken_at) / 0.35
        };
        let s = (grow * (1.0 + gone * 0.8) * (1.0 - gone)).max(0.01) as f32;
        let i = v.0 as f64;
        let bob = ((t * 2.4 + i).sin() * 0.15) as f32;
        for &c in children {
            let Ok((part, mut tf)) = parts.get_mut(c) else {
                continue;
            };
            match part {
                BonusPart::Ring => tf.scale = Vec3::splat(1.0 + ((t * 3.0 + i).sin() * 0.08) as f32),
                BonusPart::Bubble | BonusPart::Card => {
                    tf.scale = Vec3::splat(s);
                    tf.translation.y = 1.05 + bob;
                    if *part == BonusPart::Card {
                        tf.rotation = Quat::from_rotation_y((t * 1.8) as f32);
                    }
                }
            }
        }
    }
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
    pads: Query<&Gamepad>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut spec: ResMut<Spectate>,
    gate: Res<Gate>,
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
    let pad = |b: [GamepadButton; 2]| pads.iter().any(|p| p.any_just_pressed(b));
    let dir = if !gate.play {
        0
    } else if keys.any_just_pressed(PREV_KEYS)
        || (captured && mouse.just_pressed(MouseButton::Right))
        || pad([GamepadButton::DPadLeft, GamepadButton::LeftTrigger])
    {
        -1
    } else if keys.any_just_pressed(NEXT_KEYS)
        || (captured && mouse.just_pressed(MouseButton::Left))
        || pad([GamepadButton::DPadRight, GamepadButton::RightTrigger])
    {
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

/// The pad's right stick turns the camera (radians a second at full tilt).
const PAD_YAW: f32 = 3.2;
const PAD_PITCH: f32 = 2.2;

fn mouse_look(
    mut motion: MessageReader<MouseMotion>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    mut cam: ResMut<CameraAngles>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    controls: Res<Controls>,
    gate: Res<Gate>,
) {
    let dt = time.delta_secs().min(0.1);
    if gate.play {
        let k = controls.stick_sensitivity;
        let inv = if controls.invert_stick_y { -1.0 } else { 1.0 };
        for pad in &pads {
            let dz = |v: f32| if v.abs() < 0.15 { 0.0 } else { v };
            let s = pad.right_stick();
            cam.yaw -= dz(s.x) * PAD_YAW * k * dt;
            cam.pitch = (cam.pitch - dz(s.y) * PAD_PITCH * k * inv * dt).clamp(PITCH_MIN, PITCH_MAX);
        }
    }
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if !captured {
        motion.clear();
        return;
    }
    let k = controls.mouse_sensitivity;
    let inv = if controls.invert_mouse_y { -1.0 } else { 1.0 };
    for m in motion.read() {
        cam.yaw -= m.delta.x * 0.004 * k;
        cam.pitch = (cam.pitch + m.delta.y * 0.003 * k * inv).clamp(PITCH_MIN, PITCH_MAX);
    }
}
