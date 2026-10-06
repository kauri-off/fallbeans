//! Beans on screen: the model under a pivot (tumbles) and a body node (lean, squash), painted in the
//! player's colour and outfit, with hat, glasses, crown and tail; placed every frame and animated
//! (`bean.rs`) from what the bean is doing.
use core::f32::consts::FRAC_PI_2;
use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use fb_arena::ArenaKind;
use fb_net::*;
use fb_shared::outfit::Outfit;
use fb_shared::{COLORS, RAINBOW};
use fb_sim::physics::BodyState;
use lightyear::prelude::*;

use crate::bean::{BeanAnim, Expr, Frame, PIVOT_Y, podium_pose};
use crate::game::{Cue, Map, PrevPos};
use crate::outfit::{Base, Wardrobe, Wiggle, make_glasses, make_hat, wiggle};
use crate::session::Session;
use crate::view::hex;

#[derive(Component)]
pub struct BeanView;

/// The model of a bean (the glTF scene's root), and whose it is.
#[derive(Component)]
struct BeanModel(Entity);

const CROWN_SCALE: f32 = 0.62;
const AURA_COLORS: [&str; 4] = ["#ffffff", "#ff6f91", "#58d68d", "#ffd23f"];

/// Nodes of the model the animation moves, and the meshes painted per player.
struct Parts {
    /// ArmL, ArmR, LegL, LegR and their rest rotation (Euler XYZ).
    limbs: [(Entity, Vec3); 4],
    hands: [Entity; 2],
    eyes: [Entity; 2],
    pupils: [(Entity, Vec3); 2],
    body: Vec<Entity>,
    belly: Vec<Entity>,
    shoes: Vec<Entity>,
    body_mat: Handle<StandardMaterial>,
    belly_mat: Handle<StandardMaterial>,
    shoe_mat: Handle<StandardMaterial>,
}

#[derive(Component)]
pub struct Rig {
    pivot: Entity,
    pub model: Entity,
    aura: Entity,
    tears: [Entity; 4],
    parts: Option<Parts>,
}

impl Rig {
    /// The model's scene is in and its parts are found.
    pub fn ready(&self) -> bool {
        self.parts.is_some()
    }
}

#[cfg(test)]
impl Rig {
    /// A rig whose scene is in, with no parts behind it.
    pub fn ready_for_tests(model: Entity) -> Self {
        let e = Entity::PLACEHOLDER;
        Self {
            pivot: e,
            model,
            aura: e,
            tears: [e; 4],
            parts: Some(Parts {
                limbs: [(e, Vec3::ZERO); 4],
                hands: [e; 2],
                eyes: [e; 2],
                pupils: [(e, Vec3::ZERO); 2],
                body: Vec::new(),
                belly: Vec::new(),
                shoes: Vec::new(),
                body_mat: Handle::default(),
                belly_mat: Handle::default(),
                shoe_mat: Handle::default(),
            }),
        }
    }
}

/// What the bean wears now, and the entities of it.
#[derive(Component, Default)]
pub struct Dress {
    worn: Option<(u8, Outfit, bool, bool)>,
    parts: Vec<Entity>,
    wiggles: Vec<Entity>,
    tail: Vec<Entity>,
}

/// Player materials, shared by colour.
#[derive(Resource, Default)]
pub struct Paints {
    mats: HashMap<String, Handle<StandardMaterial>>,
    /// The rainbow suit's materials (body, belly), recoloured every frame.
    rainbow: Vec<(Handle<StandardMaterial>, bool)>,
    aura: Vec<Handle<StandardMaterial>>,
    aura_mesh: Option<Handle<Mesh>>,
    tear: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

pub struct BeansPlugin;

impl Plugin for BeansPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Paints>();
        app.init_resource::<Wardrobe>();
        app.add_observer(rig_bean);
        app.add_systems(Update, tick_rainbow);
    }
}

fn suit_base(color: &str) -> Color {
    if color == RAINBOW { hex("#ff5f5f") } else { hex(color) }
}

fn tint_of(color: u8) -> &'static str {
    COLORS[color as usize % COLORS.len()]
}

pub fn spawn_beans(
    mut commands: Commands,
    beans: Query<
        (Entity, &PlayerId),
        (
            With<BeanColor>,
            Without<BeanView>,
            Or<(With<Predicted>, With<Interpolated>)>,
        ),
    >,
    assets: Res<AssetServer>,
    mut paints: ResMut<Paints>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, id) in &beans {
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/bean.glb"));
        let model = commands
            .spawn((
                WorldAssetRoot(scene),
                BeanModel(e),
                Transform::from_xyz(0.0, -PIVOT_Y, 0.0),
                Visibility::default(),
            ))
            .id();
        let pivot = commands
            .spawn((
                Transform::from_xyz(0.0, PIVOT_Y, 0.0),
                Visibility::default(),
                ChildOf(e),
            ))
            .id();
        commands.entity(model).insert(ChildOf(pivot));
        // A soft ring at the feet while a bonus is in effect.
        let ring = paints
            .aura_mesh
            .get_or_insert_with(|| meshes.add(Annulus::new(0.45, 0.8).mesh().resolution(40).build()))
            .clone();
        if paints.aura.is_empty() {
            paints.aura = AURA_COLORS
                .map(|c| {
                    materials.add(StandardMaterial {
                        base_color: hex(c).with_alpha(0.7),
                        unlit: true,
                        // Light added (additive, not tone mapped against the scene).
                        alpha_mode: AlphaMode::Add,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })
                })
                .into();
        }
        let aura = commands
            .spawn((
                Mesh3d(ring),
                MeshMaterial3d(paints.aura[0].clone()),
                Transform::from_xyz(0.0, 0.05, 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                Visibility::Hidden,
                NotShadowCaster,
                ChildOf(e),
            ))
            .id();
        let (tear_mesh, tear_mat) = paints
            .tear
            .get_or_insert_with(|| {
                (
                    meshes.add(Sphere::new(0.022).mesh().uv(10, 8).scaled_by(Vec3::new(1.0, 1.4, 0.7))),
                    materials.add(StandardMaterial {
                        base_color: hex("#9fdcff").with_alpha(0.85),
                        perceptual_roughness: 0.05,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    }),
                )
            })
            .clone();
        let tears = [0; 4].map(|_| {
            commands
                .spawn((
                    Mesh3d(tear_mesh.clone()),
                    MeshMaterial3d(tear_mat.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    NotShadowCaster,
                    ChildOf(model),
                ))
                .id()
        });
        commands.entity(e).insert((
            BeanView,
            BeanAnim::new(id.0),
            Dress::default(),
            Rig {
                pivot,
                model,
                aura,
                tears,
                parts: None,
            },
            Transform::default(),
            Visibility::default(),
        ));
    }
}

/// Finds the model's moving nodes and painted meshes once its scene is in.
fn rig_bean(
    trigger: On<WorldInstanceReady>,
    mut commands: Commands,
    models: Query<&BeanModel>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform)>,
    mats: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    mut rigs: Query<(&mut Rig, &mut Dress, &mut BeanAnim)>,
) {
    let Ok(BeanModel(bean)) = models.get(trigger.entity) else {
        return;
    };
    let Ok((mut rig, mut dress, mut anim)) = rigs.get_mut(*bean) else {
        return;
    };
    let mut named: HashMap<&str, (Entity, &Transform)> = HashMap::new();
    let (mut body, mut belly, mut shoes) = (Vec::new(), Vec::new(), Vec::new());
    let (mut body_mat, mut belly_mat, mut shoe_mat) = (Handle::default(), Handle::default(), Handle::default());
    for e in children.iter_descendants(trigger.entity) {
        if let Ok((name, tf)) = nodes.get(e) {
            named.insert(name.as_str(), (e, tf));
        }
        if let Ok((name, mat)) = mats.get(e) {
            match name.0.as_str() {
                "Body" => {
                    body.push(e);
                    body_mat = mat.0.clone();
                }
                "Belly" => {
                    belly.push(e);
                    belly_mat = mat.0.clone();
                }
                "Shoe" => {
                    shoes.push(e);
                    shoe_mat = mat.0.clone();
                }
                _ => {}
            }
            // Face parts lie on the body, whose shadow already covers theirs.
            if matches!(
                name.0.as_str(),
                "Visor" | "Belly" | "Blush" | "Eye" | "Glint" | "Sclera"
            ) {
                commands.entity(e).insert(NotShadowCaster);
            }
        }
    }
    let node = |n: &str| named.get(n).map(|(e, tf)| (*e, **tf));
    let euler = |tf: Transform| Vec3::from(tf.rotation.to_euler(EulerRot::XYZ));
    let limb = |n: &str| node(n).map(|(e, tf)| (e, euler(tf)));
    let (Some(arm_l), Some(arm_r), Some(leg_l), Some(leg_r)) = (limb("ArmL"), limb("ArmR"), limb("LegL"), limb("LegR"))
    else {
        warn!("bean model: limbs missing");
        return;
    };
    let (Some(hand_l), Some(hand_r), Some(eye_l), Some(eye_r), Some(pupil_l), Some(pupil_r)) = (
        node("HandL"),
        node("HandR"),
        node("EyeL"),
        node("EyeR"),
        node("PupilL"),
        node("PupilR"),
    ) else {
        warn!("bean model: hands or eyes missing");
        return;
    };
    anim.arm_base = [(arm_l.1.x, arm_l.1.z), (arm_r.1.x, arm_r.1.z)];
    rig.parts = Some(Parts {
        limbs: [arm_l, arm_r, leg_l, leg_r],
        hands: [hand_l.0, hand_r.0],
        eyes: [eye_l.0, eye_r.0],
        pupils: [(pupil_l.0, pupil_l.1.translation), (pupil_r.0, pupil_r.1.translation)],
        body,
        belly,
        shoes,
        body_mat,
        belly_mat,
        shoe_mat,
    });
    dress.worn = None;
}

impl Paints {
    fn get(
        &mut self,
        key: String,
        materials: &mut Assets<StandardMaterial>,
        make: impl FnOnce() -> StandardMaterial,
    ) -> Handle<StandardMaterial> {
        self.mats.entry(key).or_insert_with(|| materials.add(make())).clone()
    }
}

/// Paints and dresses each bean as its player is now: colour, outfit, a crown for a game won, a tail.
pub fn dress_beans(
    mut commands: Commands,
    session: Res<Session>,
    map: Option<Res<Map>>,
    mut beans: Query<(&PlayerId, &BeanColor, &Rig, &mut Dress)>,
    assets: Res<AssetServer>,
    mut paints: ResMut<Paints>,
    mut wardrobe: ResMut<Wardrobe>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (id, color, rig, mut dress) in &mut beans {
        let Some(parts) = &rig.parts else { continue };
        let player = session
            .lobby
            .as_ref()
            .and_then(|l| l.players.iter().find(|p| p.id == id.0));
        let outfit = player.map(|p| p.outfit).unwrap_or_default();
        let crown = player.is_some_and(|p| p.crowns > 0);
        let tail = map
            .as_ref()
            .and_then(|m| m.deco.get(&id.0))
            .is_some_and(|d| d.tail == Some(true));
        let worn = (color.0, outfit, crown, tail);
        if dress.worn == Some(worn) {
            continue;
        }
        dress.worn = Some(worn);
        for e in core::mem::take(&mut dress.parts) {
            commands.entity(e).try_despawn();
        }
        dress.wiggles.clear();
        dress.tail.clear();

        let suit = tint_of(color.0);
        let template = materials.get(&parts.body_mat).cloned().unwrap_or_default();
        // Soft plastic with a faint clearcoat.
        let body = paints.get(format!("body {suit}"), &mut materials, || StandardMaterial {
            base_color: suit_base(suit),
            perceptual_roughness: 0.5,
            clearcoat: 0.3,
            clearcoat_perceptual_roughness: 0.35,
            ..template
        });
        let belly_template = materials.get(&parts.belly_mat).cloned().unwrap_or_default();
        // The belly patch: the suit colour washed towards white, or a colour of its own.
        let (belly_key, belly_color) = match outfit.belly {
            Some(t) => (format!("belly {}", t.hex()), hex(t.hex())),
            None => (format!("belly washed {suit}"), suit_base(suit).mix(&Color::WHITE, 0.62)),
        };
        let belly = paints.get(belly_key, &mut materials, || StandardMaterial {
            base_color: belly_color,
            perceptual_roughness: 0.55,
            ..belly_template
        });
        if suit == RAINBOW {
            for (h, is_belly) in [(&body, false), (&belly, true)] {
                if (outfit.belly.is_none() || !is_belly) && !paints.rainbow.iter().any(|(r, _)| r == h) {
                    paints.rainbow.push((h.clone(), is_belly));
                }
            }
        }
        for e in &parts.body {
            commands.entity(*e).insert(MeshMaterial3d(body.clone()));
        }
        for e in &parts.belly {
            commands.entity(*e).insert(MeshMaterial3d(belly.clone()));
        }
        if let Some(t) = outfit.shoes {
            let shoe = paints.get(format!("shoe {}", t.hex()), &mut materials, || StandardMaterial {
                base_color: hex(t.hex()),
                perceptual_roughness: 0.5,
                ..default()
            });
            for e in &parts.shoes {
                commands.entity(*e).insert(MeshMaterial3d(shoe.clone()));
            }
        } else {
            for e in &parts.shoes {
                commands.entity(*e).insert(MeshMaterial3d(parts.shoe_mat.clone()));
            }
        }

        let hat = make_hat(outfit.hat, outfit.hat_color.map(|t| t.hex()));
        let crown_lift = hat.as_ref().map_or(0.0, |h| h.crown_lift);
        let mut wiggles = Vec::new();
        for (acc, shadows) in [(hat, true), (make_glasses(outfit.glasses), false)] {
            if let Some(a) = acc {
                let e = wardrobe.spawn(
                    &mut commands,
                    rig.model,
                    &a.root,
                    &body,
                    shadows,
                    &mut wiggles,
                    &mut meshes,
                    &mut materials,
                );
                dress.parts.push(e);
            }
        }
        dress.wiggles = wiggles;
        if crown {
            let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/crown.glb"));
            // The band (radius 0.5 in the model) rests on the head where it is 0.31 m from the axis.
            let e = commands
                .spawn((
                    WorldAssetRoot(scene),
                    Transform::from_xyz(0.0, 1.465 + crown_lift, -0.01)
                        .with_rotation(Quat::from_rotation_x(-0.06))
                        .with_scale(Vec3::splat(CROWN_SCALE)),
                    Visibility::default(),
                    ChildOf(rig.model),
                ))
                .id();
            dress.parts.push(e);
        }
        if tail {
            let fur = paints.get("tail".into(), &mut materials, || StandardMaterial {
                base_color: hex("#ff9f1c"),
                perceptual_roughness: 0.7,
                ..default()
            });
            let tip = paints.get("tail tip".into(), &mut materials, || StandardMaterial {
                base_color: hex("#fff4d6"),
                perceptual_roughness: 0.8,
                ..default()
            });
            let mut parent = commands
                .spawn((
                    Transform::from_xyz(0.0, 0.45, -0.52),
                    Visibility::default(),
                    ChildOf(rig.model),
                ))
                .id();
            dress.parts.push(parent);
            for i in 0..5 {
                let r = 0.17 - i as f32 * 0.02;
                let pos = if i > 0 { Vec3::new(0.0, 0.1, -0.15) } else { Vec3::ZERO };
                parent = commands
                    .spawn((
                        Mesh3d(meshes.add(Sphere::new(r).mesh().uv(14, 10))),
                        MeshMaterial3d(if i == 4 { tip.clone() } else { fur.clone() }),
                        Transform::from_translation(pos),
                        Visibility::default(),
                        ChildOf(parent),
                    ))
                    .id();
                dress.tail.push(parent);
            }
        }
    }
}

/// The rainbow suit: its colour runs round the colour wheel.
fn tick_rainbow(
    time: Res<Time<Real>>,
    paints: Res<Paints>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<u32>>,
) {
    if paints.rainbow.is_empty() {
        return;
    }
    // In steps of 3°, about 18 a second: a material changed is uploaded again, not with every frame.
    const STEP: f32 = 3.0;
    let step = ((time.elapsed_secs() * 0.15).fract() * 360.0 / STEP) as u32;
    if last.replace(step) == Some(step) {
        return;
    }
    let hue = Color::hsl(step as f32 * STEP, 0.85, 0.6);
    for (h, belly) in &paints.rainbow {
        if let Some(mut m) = materials.get_mut(h) {
            m.base_color = if *belly { hue.mix(&Color::WHITE, 0.62) } else { hue };
        }
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

/// Where each bean is drawn: the own one between its last two ticks, the others as interpolated.
pub fn place_beans(
    fixed: Res<Time<Fixed>>,
    map: Option<Res<Map>>,
    own: Query<(&BodyFull, &PrevPos, &mut Transform, &mut Visibility), (With<Predicted>, With<BeanView>)>,
    mut others: OtherBeans,
) {
    let a = fixed.overstep_fraction();
    // A bean drawn between ticks (or tipped over) may poke into a wall or the floor: out of it, as the
    // simulation would put it.
    let out = |p: Vec3, tilt: f32, dir: f32, size: f32| match map.as_ref() {
        Some(m) => fb_sim::physics::push_out(
            &m.world,
            fb_sim::math::V3::new(p.x as f64, p.y as f64, p.z as f64),
            tilt as f64,
            dir as f64,
            size as f64,
        )
        .as_vec3(),
        None => p,
    };
    for (full, prev, mut tf, mut vis) in own {
        let b = &full.body;
        let p = prev.0.as_vec3().lerp(b.pos.as_vec3(), a);
        tf.translation = if b.state == BodyState::Portal {
            p
        } else {
            out(p, b.tilt as f32, b.tilt_dir as f32, b.size as f32)
        };
        tf.rotation = Quat::from_rotation_y(b.yaw as f32);
        // Inside a portal: out of sight, gliding to the other end.
        vis.set_if_neq(if b.state == BodyState::Portal {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    for (id, p, mut tf, mut vis) in &mut others {
        tf.translation = if p.anim == Anim::Portal {
            p.pos
        } else {
            out(p.pos, p.tilt, p.tilt_dir, p.size)
        };
        tf.rotation = Quat::from_rotation_y(p.yaw);
        // Finished or out: gone at once (the last snapshots may still carry the bean).
        let gone = map.as_ref().is_some_and(|m| m.gone(id.0)) || p.anim == Anim::Portal;
        vis.set_if_neq(if gone {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
}

/// The pose of the model's body node as the animation left it last.
fn model_affine(root: &Transform, out: &crate::bean::Out) -> bevy::math::Affine3A {
    let root = Transform {
        scale: Vec3::splat(out.grow.max(0.5)),
        ..*root
    };
    let pivot = Transform::from_xyz(0.0, PIVOT_Y, 0.0).with_rotation(out.pivot);
    let model = Transform::from_xyz(0.0, -PIVOT_Y + out.lift, 0.0)
        .with_rotation(Quat::from_euler(EulerRot::XYZ, out.lean, out.twist, out.roll))
        .with_scale(out.squash.max(Vec3::splat(0.1)));
    root.compute_affine() * pivot.compute_affine() * model.compute_affine()
}

pub fn animate_beans(
    time: Res<Time<Real>>,
    map: Option<Res<Map>>,
    session: Res<Session>,
    mut cues: MessageReader<Cue>,
    mut beans: Query<(
        &PlayerId,
        &mut BeanAnim,
        &Rig,
        &Dress,
        &mut Transform,
        Option<&BodyFull>,
        Option<&RemotePose>,
        Option<&Hold>,
        Has<Predicted>,
    )>,
    mut parts: Query<&mut Transform, (Without<BeanAnim>, Without<Wiggle>)>,
    mut wiggles: Query<(&Wiggle, &Base, &mut Transform), Without<BeanAnim>>,
    mut vis: Query<&mut Visibility, Without<BeanAnim>>,
    mut aura_mats: Query<&mut MeshMaterial3d<StandardMaterial>, Without<BeanAnim>>,
    paints: Res<Paints>,
    mut scratch: Local<(Vec<Cue>, HashMap<u32, (Vec3, f32)>)>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    // (Kept from frame to frame: no list and map allocated every frame.)
    let (cue_list, drawn) = &mut *scratch;
    cue_list.clear();
    cue_list.extend(cues.read().copied());
    let cues = &*cue_list;
    // Where every bean is drawn (for arms reaching for the one held).
    drawn.clear();
    drawn.extend(
        beans
            .iter()
            .map(|(id, a, .., tf, _, _, _, _)| (id.0, (tf.translation, a.out.grow))),
    );
    let podium = map.as_ref().is_some_and(|m| m.round.kind == ArenaKind::Podium);
    for (id, mut anim, rig, dress, mut root, full, pose, hold, own) in &mut beans {
        for c in cues {
            match *c {
                Cue::Emote { id: who, e } if who == id.0 => anim.play_emote(e),
                Cue::Finish(who) if who == id.0 => anim.react(Expr::Laugh, 3.0),
                Cue::Ko { id: who, out } if who == id.0 => {
                    if out {
                        anim.react(Expr::Cry, 3.0);
                    } else {
                        anim.react(Expr::Scared, 1.2);
                    }
                }
                Cue::Bonus(who) if who == id.0 => anim.react(Expr::Grin, 1.5),
                _ => {}
            }
        }
        let hold = hold.copied().unwrap_or_default();
        let (vel, anim_code, tilt, tilt_dir, size, power, mut impact) = match (own, full, pose) {
            (true, Some(f), _) => {
                let b = &f.body;
                (
                    b.vel.as_vec3(),
                    Anim::of(b, hold.target.is_some(), hold.reaching),
                    b.tilt as f32,
                    b.tilt_dir as f32,
                    b.size as f32,
                    b.power,
                    0.0,
                )
            }
            (_, _, Some(p)) => {
                // Velocity from the drawn path, smoothed: steady cycles instead of per-frame jitter.
                let fresh = anim.drawn_at.is_none_or(|at| at.distance(root.translation) > 3.0);
                let v = if fresh || dt <= 0.0 {
                    Vec3::ZERO
                } else {
                    let v = (root.translation - anim.drawn_at.unwrap_or_default()) / dt;
                    if v.length() > 40.0 { Vec3::ZERO } else { v }
                };
                anim.drawn_vel = if fresh {
                    Vec3::ZERO
                } else {
                    anim.drawn_vel.lerp(v, 1.0 - (-dt * 14.0).exp())
                };
                (anim.drawn_vel, p.anim, p.tilt, p.tilt_dir, p.size, p.power, 0.0)
            }
            _ => continue,
        };
        anim.drawn_at = Some(root.translation);
        if let Some(f) = full.filter(|_| own) {
            let grounded = f.body.grounded;
            if grounded && !anim.was_grounded && f.body.land_impact > 0.3 {
                impact = f.body.land_impact as f32;
            }
            anim.was_grounded = grounded;
        }
        // Both hands on the near side of the held bean, about its middle (in the model's space).
        let grab_at = hold.target.and_then(|h| drawn.get(&h)).map(|&(at, hs)| {
            let mut aim = (root.translation - at).with_y(0.0);
            let d = aim.length();
            if d > 1e-3 {
                aim *= 0.42 * hs / d;
            }
            let world = Vec3::new(aim.x + at.x, at.y + 0.85 * hs, aim.z + at.z);
            (model_affine(&root, &anim.out).inverse().transform_point3(world), hs)
        });
        let pose = podium.then(|| {
            let place = session
                .standings
                .iter()
                .find(|s| s.id == id.0)
                .and_then(|s| (s.place as usize).checked_sub(1));
            podium_pose(place, session.standings.len().max(1))
        });
        let (yaw, _, _) = root.rotation.to_euler(EulerRot::YXZ);
        anim.animate(
            dt,
            &Frame {
                vel,
                anim: anim_code,
                t,
                land_impact: impact,
                tilt,
                tilt_dir,
                yaw,
                grab_at,
                size,
                power,
                pose,
            },
        );
        let out = anim.out;
        root.scale = Vec3::splat(out.grow);
        if let Ok(mut tf) = parts.get_mut(rig.pivot) {
            tf.rotation = out.pivot;
        }
        if let Ok(mut tf) = parts.get_mut(rig.model) {
            tf.translation.y = -PIVOT_Y + out.lift;
            tf.rotation = Quat::from_euler(EulerRot::XYZ, out.lean, out.twist, out.roll);
            tf.scale = out.squash;
        }
        if let Some(p) = &rig.parts {
            for (i, ((e, base), (x, z))) in p.limbs.iter().zip(out.limbs).enumerate() {
                if let Ok(mut tf) = parts.get_mut(*e) {
                    tf.rotation = Quat::from_euler(EulerRot::XYZ, base.x + x, base.y, base.z + z);
                    if i < 2 {
                        tf.scale.y = out.stretch[i];
                    }
                }
            }
            for (i, h) in p.hands.iter().enumerate() {
                if let Ok(mut tf) = parts.get_mut(*h) {
                    tf.scale.y = 1.0 / out.stretch[i];
                }
            }
            for e in p.eyes {
                if let Ok(mut tf) = parts.get_mut(e) {
                    tf.scale.y = out.eye_open;
                }
            }
            for (i, (e, base)) in p.pupils.iter().enumerate() {
                if let Ok(mut tf) = parts.get_mut(*e) {
                    tf.scale = Vec3::splat(out.pupil);
                    tf.translation = *base + out.pupil_roll[i].extend(0.0);
                }
            }
        }
        for (i, e) in rig.tears.iter().enumerate() {
            if let Ok(mut v) = vis.get_mut(*e) {
                v.set_if_neq(if out.crying {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            if out.crying
                && let Ok(mut tf) = parts.get_mut(*e)
            {
                // Drops run down from under each eye and drip off.
                let side = if i % 2 == 1 { 1.0 } else { -1.0 };
                let f = (t * 1.3 + i as f32 * 0.37).fract();
                tf.translation = Vec3::new(side * (0.12 + f * 0.02), 1.15 - f * 0.25, 0.52 - f * 0.05);
                tf.scale = Vec3::splat(0.6 + (f * core::f32::consts::PI).sin() * 0.5);
            }
        }
        if let Ok(mut v) = vis.get_mut(rig.aura) {
            v.set_if_neq(if out.aura.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
        if let Some(s) = out.aura {
            if let Ok(mut tf) = parts.get_mut(rig.aura) {
                tf.scale = Vec3::splat(s);
            }
            if let (Ok(mut m), Some(h)) = (aura_mats.get_mut(rig.aura), paints.aura.get(power as usize))
                && m.0 != *h
            {
                m.0 = h.clone();
            }
        }
        for (e, (ry, rx)) in dress.tail.iter().zip(out.tail) {
            if let Ok(mut tf) = parts.get_mut(*e) {
                tf.rotation = Quat::from_euler(EulerRot::XYZ, rx, ry, 0.0);
            }
        }
        let mut rotor = anim.rotor;
        for e in &dress.wiggles {
            if let Ok((w, base, mut tf)) = wiggles.get_mut(*e) {
                wiggle(*w, &base.0, &mut tf, t, out.speed, &mut rotor, dt);
            }
        }
        anim.rotor = rotor;
    }
}
