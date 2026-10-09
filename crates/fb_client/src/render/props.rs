//! The map's models: materials get surfaces by name, and props move (pennants, fans, stars, bumpers, trees).
use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::gltf::GltfMaterialName;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use fb_shared::Rgb;
use fb_sim::scene::Model;

use super::cloth::{ClothMaterial, Cloths, Cut, WIND, gust, pennant_bounds};
use super::lod::{Levels, ModelLods, PlacedMesh};
use super::surface::{Kind, SurfaceKit, SurfaceMaterial};
use super::vfx::{Burst, Pool};
use crate::game::Map;
use crate::view::{FrameClock, MainCamera};

/// A map model: what it is, its tint, and the phase of its motion (from where it stands).
#[derive(Component)]
pub struct Prop {
    pub name: Model,
    pub tint: Option<Rgb>,
    pub phase: f32,
    /// Materials repainted by name: colour, and how much of it glows (clouds of a tinted sky).
    pub paint: Vec<(&'static str, Rgb, f32)>,
    /// A special's piece: its special moves it, and it gets no level-of-detail copies.
    pub special: bool,
    /// Scenery or a special's piece: its own transform is left alone (stars and mushrooms of the map
    /// bob and squash; the scenery's are placed and scaled by `decor`).
    still: bool,
}

impl Prop {
    pub fn new(name: Model, tint: Option<Rgb>, x: f64, z: f64) -> Self {
        // (x and z of the placement: the node's own position.)
        let phase = if name == Model::Mushroom {
            ((x * 3.1 + z * 1.3) % core::f64::consts::TAU) as f32
        } else {
            ((x * 12.9898 + z * 78.233) % core::f64::consts::TAU) as f32
        };
        Self {
            name,
            tint,
            phase,
            paint: Vec::new(),
            special: false,
            still: false,
        }
    }

    pub fn special(name: Model) -> Self {
        Self {
            special: true,
            ..Self::painted(name, Vec::new())
        }
    }

    /// A model of the scenery: no motion of its own, materials repainted by name.
    pub fn painted(name: Model, paint: Vec<(&'static str, Rgb, f32)>) -> Self {
        Self {
            name,
            tint: None,
            phase: 0.0,
            paint,
            special: false,
            still: true,
        }
    }
}

/// A part of a prop that moves, with its pose as modelled.
#[derive(Component)]
struct Moving {
    prop: Entity,
    rest: Transform,
}

/// A mesh `dress` is done with: a model's, now on its surface, or one of no model (beans, bonuses…), which
/// is not looked at again.
#[derive(Component)]
struct Dressed;

/// A tree swaying with the wind over its foot: its pose as placed, the phase of its own sway, how far it
/// tips (rad), its turn among the frames it moves in (`SWAY_EVERY`), whether it is off its pose.
#[derive(Component)]
struct Sway {
    rest: Quat,
    phase: f32,
    amount: f32,
    slot: u32,
    moved: bool,
}

/// Trees sway within this distance of the camera (m), fading out over the last stretch.
const SWAY_NEAR: f32 = 45.0;
const SWAY_FAR: f32 = 75.0;
/// A tree moves every this many frames (its sway is slow: no steps show), the trees taking turns.
const SWAY_EVERY: u32 = 3;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelLods>();
        app.add_plugins(super::cloth::ClothPlugin);
        app.add_systems(Update, (dress, find_moving, animate, sway).chain());
        app.add_systems(Update, (super::lod::finish_levels, super::meshes::refresh_bands));
        app.add_systems(Update, (twinkle_stars, bumpers.after(animate)));
    }
}

fn prop_of(mut e: Entity, parents: &Query<&ChildOf>, props: &Query<&Prop>) -> Option<Entity> {
    for _ in 0..12 {
        if props.contains(e) {
            return Some(e);
        }
        e = parents.get(e).ok()?.parent();
    }
    None
}

/// A model mesh as the glTF loader spawns it.
type GltfMesh = (
    Entity,
    &'static MeshMaterial3d<StandardMaterial>,
    Option<&'static GltfMaterialName>,
    Option<&'static Name>,
);

/// The props' entity tree: what is a prop, and where its meshes are.
#[derive(SystemParam)]
struct PropTree<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    props: Query<'w, 's, &'static Prop>,
    shadowless: Query<'w, 's, (), (With<Prop>, With<NotShadowCaster>)>,
    shapes: Query<'w, 's, (&'static Mesh3d, &'static Transform, &'static ChildOf)>,
    transforms: Query<'w, 's, &'static Transform>,
}

/// The surface materials made, by glTF material and look.
type Made = HashMap<(AssetId<StandardMaterial>, String), Handle<SurfaceMaterial>>;

/// What model meshes are dressed in, and the surface materials made so far.
#[derive(SystemParam)]
struct Dressing<'w, 's> {
    standard: Res<'w, Assets<StandardMaterial>>,
    surfaces: SurfaceKit<'w>,
    cloths: ResMut<'w, Cloths>,
    cloth_mats: ResMut<'w, Assets<ClothMaterial>>,
    done: Local<'s, Made>,
}

/// Model meshes as they appear: the standard material from the glTF becomes a surface material. A model whose
/// root casts no shadow (scenery) gives that to each of its meshes: the component is not inherited.
fn dress(
    mut commands: Commands,
    meshes: Query<GltfMesh, Without<Dressed>>,
    tree: PropTree,
    mut dressing: Dressing,
    mut levels: Levels,
) {
    if !meshes.is_empty() {
        // (Materials of a model unloaded between maps come back under new ids: forget the old ones.)
        dressing.done.retain(|(id, _), _| dressing.standard.contains(*id));
        dressing
            .cloths
            .retain(|id| dressing.done.values().any(|h| h.id() == id));
    }
    for (e, mat, mat_name, name) in &meshes {
        let Some(p) = prop_of(e, &tree.parents, &tree.props) else {
            commands.entity(e).try_insert(Dressed);
            continue;
        };
        let Some(base) = dressing.standard.get(&mat.0) else {
            continue;
        };
        let prop = tree.props.get(p).ok();
        let part = name.map(|n| n.as_str()).unwrap_or("");
        let mat_name = mat_name.map(|n| n.0.as_str()).unwrap_or("");
        let tinted = part.starts_with("Pennant") || part.starts_with("MushCap");
        let tint = prop.and_then(|p| p.tint).filter(|_| tinted);
        let paint = prop.and_then(|p| p.paint.iter().find(|(n, _, _)| *n == mat_name));
        let key = format!("{tint:?} {paint:?}");
        let h = dressing
            .done
            .entry((mat.0.id(), key))
            .or_insert_with(|| {
                let mut base = base.clone();
                let kind = match tint {
                    Some(_) if part.starts_with("Pennant") => Some(Kind::Cloth),
                    Some(_) => Some(Kind::Glossy),
                    None => Kind::of_model(mat_name),
                };
                if let Some(t) = tint {
                    base.base_color = crate::view::color(t);
                    base.base_color_texture = None;
                }
                if let Some((_, c, glow)) = paint {
                    let c = crate::view::color(*c);
                    base.base_color = c;
                    base.base_color_texture = None;
                    if *glow > 0.0 {
                        base.emissive = c.to_linear() * *glow;
                    }
                }
                if let Some(k) = kind
                    && !matches!(k, Kind::Metal | Kind::Gold)
                    && base.metallic > 0.0
                {
                    base.metallic = 0.0;
                }
                dressing.surfaces.material_from(base, kind, mat_name == "Glint")
            })
            .clone();
        let shadow = !tree.shadowless.contains(p);
        // A pennant waves: the model's flat one gives way to a cloth of its cut sewn round the pole, in its
        // surface's cloth twin, with bounds that hold the swing.
        if part.starts_with("Pennant")
            && let Some(c) = dressing
                .cloths
                .pennant(&h, &dressing.surfaces.materials, &mut dressing.cloth_mats)
            && let Some(cloth) = dressing.cloths.mesh(cut_of(prop))
        {
            let mut pennant = commands.entity(e);
            pennant.remove::<MeshMaterial3d<StandardMaterial>>().insert((
                Mesh3d(cloth),
                MeshMaterial3d(c),
                pennant_bounds(),
                Dressed,
            ));
            if !shadow {
                pennant.insert(NotShadowCaster);
            }
            continue;
        }
        let mut dressed = commands.entity(e);
        dressed
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(h.clone()), Dressed));
        if !shadow {
            dressed.insert(NotShadowCaster);
        }
        // Levels of detail for what stands still (moving parts would leave their copies behind).
        let moving = part.starts_with("Pennant") || part.starts_with("FanBlades") || prop.is_some_and(|p| p.special);
        if !moving && let Ok((mesh, tf, parent)) = tree.shapes.get(e) {
            // (The levels switch by the size on screen: the scales of the mesh and all above it count.
            // The global transform is not propagated yet for a scene just spawned.)
            let mut scale = tf.scale.abs().max_element();
            let mut up = Some(parent.parent());
            while let Some(a) = up {
                if let Ok(t) = tree.transforms.get(a) {
                    scale *= t.scale.abs().max_element();
                }
                up = tree.parents.get(a).ok().map(ChildOf::parent);
            }
            let placed = PlacedMesh {
                e,
                parent: parent.parent(),
                tf: *tf,
                mesh: &mesh.0,
                material: &h,
                scale,
                shadow,
            };
            levels.add(&mut commands, placed);
        }
    }
}

/// The cut of a flag's cloth: swallowtails over the scenery, the map's flags one or the other by where they
/// stand.
fn cut_of(prop: Option<&Prop>) -> Cut {
    match prop {
        Some(p) if p.still || (p.phase * 5.0).sin() > 0.0 => Cut::Swallowtail,
        _ => Cut::Pennant,
    }
}

type NewlyNamed = (Added<Name>, Without<Moving>);

/// The parts that move (by node name), once the model's scene is in; the trees that sway.
fn find_moving(
    mut commands: Commands,
    named: Query<(Entity, &Name, &Transform), NewlyNamed>,
    parents: Query<&ChildOf>,
    props: Query<&Prop>,
    trees: Query<(Entity, &Prop, &Transform), Added<Prop>>,
    mut slots: Local<u32>,
) {
    for (e, p, tf) in &trees {
        let amount = match p.name {
            Model::Tree => 0.03,
            Model::Pine => 0.022,
            _ => continue,
        };
        // (The map's props stand at the origin of their piece and have a phase; the scenery's are placed.)
        let phase = p.phase + tf.translation.x * 1.7 + tf.translation.z * 2.3;
        *slots = slots.wrapping_add(1);
        commands.entity(e).insert(Sway {
            rest: tf.rotation,
            phase,
            amount,
            slot: *slots,
            moved: false,
        });
    }
    for (e, name, tf) in &named {
        if !matches!(name.as_str(), "Pennant" | "FanBlades") {
            continue;
        }
        if let Some(p) = prop_of(e, &parents, &props) {
            commands.entity(e).insert(Moving { prop: p, rest: *tf });
        }
    }
}

/// Props that move as a whole (not by their parts, not swaying).
type Whole = (Without<Moving>, Without<Sway>);

fn animate(
    map: Option<Res<Map>>,
    mut props: Query<(&Prop, &mut Transform), Whole>,
    mut parts: Query<(&Moving, &Name, &mut Transform), Without<Prop>>,
    clock: FrameClock,
) {
    let Some(map) = map else { return };
    let t = map.time(clock.tick()) as f32;
    for (p, mut tf) in &mut props {
        if p.still {
            continue;
        }
        let ph = p.phase;
        match p.name {
            Model::Star => {
                tf.rotation = Quat::from_rotation_y(t * 1.4 + ph);
                tf.translation.y = (t * 1.8 + ph).sin() * 0.2;
            }
            Model::Mushroom => {
                let k = (t * 5.0 + ph).sin().max(0.0).powi(6);
                tf.scale = Vec3::new(1.0 + k * 0.05, 1.0 - k * 0.06, 1.0 + k * 0.05);
            }
            _ => {}
        }
    }
    for (m, name, mut tf) in &mut parts {
        let Ok((p, _)) = props.get(m.prop) else { continue };
        let ph = p.phase;
        match name.as_str() {
            "Pennant" => {
                // (The cloth waves by itself: the pennant only swings a little about the pole.)
                let a = (t * 1.3 + ph).sin() * 0.07 + (t * 3.4 + ph * 2.0).sin() * 0.025;
                tf.rotation = m.rest.rotation * Quat::from_rotation_y(a);
            }
            "FanBlades" => {
                tf.rotation = m.rest.rotation * Quat::from_rotation_z(t * 6.0 + ph);
            }
            _ => {}
        }
    }
}

/// Trees near the camera tip over their foot with the wind: downwind with the gusts (on the cloth's
/// clock: the flags wave with the same gusts), a little across it.
fn sway(
    time: Res<Time>,
    camera: Query<&GlobalTransform, With<MainCamera>>,
    mut trees: Query<(&mut Sway, &ChildOf, &GlobalTransform, &mut Transform)>,
    placed: Query<&GlobalTransform, Without<Sway>>,
    mut frame: Local<u32>,
) {
    let Ok(cam) = camera.single() else { return };
    let eye = cam.translation();
    let t = time.elapsed_secs_wrapped();
    *frame = frame.wrapping_add(1);
    // Turning about these world axes tips the top downwind, and across the wind.
    let down = Vec3::new(WIND.y, 0.0, -WIND.x);
    let across = Vec3::new(WIND.x, 0.0, WIND.y);
    for (mut s, parent, at, mut tf) in &mut trees {
        if (*frame).wrapping_add(s.slot) % SWAY_EVERY != 0 {
            continue;
        }
        let p = at.translation();
        let near = ((SWAY_FAR - p.distance(eye)) / (SWAY_FAR - SWAY_NEAR)).clamp(0.0, 1.0);
        if near <= 0.0 {
            if s.moved {
                tf.rotation = s.rest;
                s.moved = false;
            }
            continue;
        }
        let a = s.amount * near;
        let lean = a * gust(t, Vec2::new(p.x, p.z)) * (0.65 + 0.35 * (t * 1.7 + s.phase).sin());
        let side = a * 0.35 * (t * 1.1 + s.phase * 1.3).sin();
        let tilt = Quat::from_axis_angle(down, lean) * Quat::from_axis_angle(across, side);
        // (The tilt is in the world: brought into the parent's frame.)
        let up = placed
            .get(parent.parent())
            .map_or(Quat::IDENTITY, GlobalTransform::rotation);
        tf.rotation = up.inverse() * tilt * up * s.rest;
        s.moved = true;
    }
}

/// The map's stars twinkle (`vfx.rs`): a shell of flaring points about each.
fn twinkle_stars(mut commands: Commands, stars: Query<(Entity, &Prop), Added<Prop>>, pool: Option<Res<Pool>>) {
    let Some(pool) = pool else { return };
    for (e, p) in &stars {
        if p.name == Model::Star && !p.still {
            commands.entity(e).with_child(pool.twinkle());
        }
    }
}

/// How long a bumper wobbles after a bean touches it (s).
const BUMP_S: f32 = 0.6;

/// Per bumper: last touch time (s) and whether a bean touches it now; drives its jelly squash and wobble.
fn bumpers(
    time: Res<Time>,
    mut props: Query<(Entity, &Prop, &GlobalTransform, &mut Transform), Whole>,
    beans: Query<&GlobalTransform, With<crate::beans::BeanView>>,
    mut hits: Local<HashMap<Entity, (f32, bool)>>,
    mut bursts: MessageWriter<Burst>,
) {
    let now = time.elapsed_secs();
    hits.retain(|e, _| props.contains(*e));
    for (e, p, at, mut tf) in &mut props {
        if p.name != Model::Bumper || p.still {
            continue;
        }
        // (Its collider: 0.9 of its scale round and 1.9 high, from its foot.)
        let foot = at.translation();
        let s = at.scale().x.abs().max(0.1);
        let touch = beans.iter().map(GlobalTransform::translation).find(|b| {
            let d = Vec2::new(b.x - foot.x, b.z - foot.z).length();
            d < 0.9 * s + 0.55 && b.y > foot.y - 0.4 && b.y < foot.y + 1.9 * s + 0.3
        });
        let (last, was) = hits.entry(e).or_insert((-BUMP_S, false));
        if let Some(b) = touch
            && !*was
            && now - *last > 0.25
        {
            *last = now;
            let out = Vec2::new(b.x - foot.x, b.z - foot.z).normalize_or_zero() * 0.9 * s;
            let y = b.y.clamp(foot.y + 0.2, foot.y + 0.2 + 1.5 * s);
            bursts.write(Burst {
                kind: super::vfx::Kind::Ring,
                at: Vec3::new(foot.x + out.x, y, foot.z + out.y),
                color: LinearRgba::rgb(1.0, 0.9, 0.6),
                size: s,
            });
        }
        *was = touch.is_some();
        let k = now - *last;
        if (0.0..BUMP_S).contains(&k) {
            // Squashed at once, ringing back out.
            let a = 0.16 * (-k * 7.0).exp() * (k * 28.0).cos();
            tf.scale = Vec3::new(1.0 + a, 1.0 - a, 1.0 + a);
        } else if tf.scale != Vec3::ONE {
            tf.scale = Vec3::ONE;
        }
    }
}
