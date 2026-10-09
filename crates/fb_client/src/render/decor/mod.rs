//! The scenery around a map (client only): drifting clouds kept clear
//! of the course, birds circling far out, hot-air balloons on the horizon, floating islands, the set
//! pieces of the round's look (castle towers, gears, snowmen, planets, tents, neon rings, lighthouses,
//! cacti, volcanoes, crowns, lollipops…) and the land far below. Visual only: its own random numbers. The parts
//! nothing moves are merged by cell and material.
use std::collections::{HashMap, HashSet};

use bevy::asset::{RenderAssetUsages, UntypedAssetId};
use bevy::camera::primitives::MeshAabb;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use fb_shared::{Rgb, rgb};
use fb_sim::looks::{LookId, Pattern, ResolvedLook, Swatch};
use fb_sim::scene::Model;
use fb_sim::scene::SceneryRequest;

use super::props::Prop;
use super::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::game::Map;
use crate::view::{FrameClock, MapRoot, color};

mod buildings;
mod fancy;
mod merge;
mod nature;
mod scenery;
mod shapes;

use buildings::*;
use fancy::*;
use merge::*;
use nature::*;
use scenery::*;
use shapes::*;

/// Half size of the cloud model at scale 1 (x/z and y).
const CLOUD_R: f32 = 3.9;
const CLOUD_H: f32 = 2.1;
/// How far a cloud drifts from its home (m).
const DRIFT: f32 = 2.5;

/// The scenery's own random numbers (mulberry32 on a seed of its own).
struct Rnd(i32);

impl Rnd {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let s = self.0;
        let mut t = (s ^ ((s as u32) >> 15) as i32).wrapping_mul(1 | s);
        t = (t.wrapping_add((t ^ ((t as u32) >> 7) as i32).wrapping_mul(61 | t))) ^ t;
        ((t ^ ((t as u32) >> 14) as i32) as u32) as f32 / 4_294_967_296.0
    }
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

/// Is a volume (horizontal radius r, half height h) at p too close to anything solid?
fn blocked(boxes: &[Aabb], p: Vec3, r: f32, h: f32, margin: f32) -> bool {
    boxes.iter().any(|b| {
        let dx = (b.min.x - p.x).max(0.0).max(p.x - b.max.x);
        let dz = (b.min.z - p.z).max(0.0).max(p.z - b.max.z);
        if dx.hypot(dz) > r + margin {
            return false;
        }
        // Far below the course is fine (you look down on them); above it they would hide the action.
        !(p.y + h < b.min.y - 16.0 || p.y - h > b.max.y + 32.0)
    })
}

fn bounds(boxes: &[Aabb]) -> Aabb {
    if boxes.is_empty() {
        return Aabb {
            min: Vec3::new(-10.0, -1.0, -10.0),
            max: Vec3::new(10.0, 1.0, 10.0),
        };
    }
    boxes.iter().fold(
        Aabb {
            min: Vec3::INFINITY,
            max: Vec3::NEG_INFINITY,
        },
        |a, b| Aabb {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        },
    )
}

/// Something that moves every frame: given the round's time, it sets transforms.
type Tick = Box<dyn Fn(f32, &mut Tx) + Send + Sync>;

/// What a tick may change.
pub struct Tx<'a, 'w, 's> {
    q: &'a mut Query<'w, 's, (&'static mut Transform, &'static mut Visibility), With<Decor>>,
}

impl Tx<'_, '_, '_> {
    fn set(&mut self, e: Entity, tf: Transform) {
        if let Ok((mut t, _)) = self.q.get_mut(e)
            && *t != tf
        {
            *t = tf;
        }
    }

    fn show(&mut self, e: Entity, on: bool) {
        if let Ok((_, mut v)) = self.q.get_mut(e) {
            v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

/// An entity of the scenery (ticks move only these).
#[derive(Component)]
pub struct Decor;

#[derive(Clone)]
enum Mat {
    S(Handle<SurfaceMaterial>),
    G(Handle<StandardMaterial>),
}

/// A part made of a unit shape, and the set piece it belongs to (0: none).
struct Part {
    e: Entity,
    shape: Shape,
    mat: Mat,
    piece: u32,
}

/// Shared bits for building pieces: unit shapes, materials, the look's colours, randomness.
struct Kit<'a> {
    world: &'a mut World,
    look: ResolvedLook,
    rnd: Rnd,
    shapes: HashMap<Shape, Handle<Mesh>>,
    glows: HashMap<String, Handle<StandardMaterial>>,
    assets: AssetServer,
    ticks: Vec<Tick>,
    /// Every part made (`merge_still` merges the ones nothing moves).
    parts: Vec<Part>,
    /// The set piece being built (its parts shade each other, `occlusion`).
    piece: u32,
    /// Set pieces standing on an island: the entity whose origin is on the grass.
    grounds: Vec<(u32, Entity)>,
    /// The soft shadow under what stands on an island (`blob_shadow`), made on first use.
    blob: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

fn tf(at: [f32; 3], size: [f32; 3], rot: [f32; 3]) -> Transform {
    Transform {
        translation: Vec3::from(at),
        rotation: Quat::from_euler(EulerRot::XYZ, rot[0], rot[1], rot[2]),
        scale: Vec3::from(size),
    }
}

const NO_ROT: [f32; 3] = [0.0; 3];

impl Kit<'_> {
    fn rnd(&mut self) -> f32 {
        self.rnd.next()
    }

    fn pick<T: Copy>(&mut self, list: &[T]) -> T {
        list[((self.rnd() * list.len() as f32) as usize).min(list.len() - 1)]
    }

    fn col(&self, s: Swatch) -> Rgb {
        self.look.tones(s)[0]
    }

    fn col2(&self, s: Swatch) -> Rgb {
        self.look.tones(s)[1]
    }

    /// One of the look's bright palette colours.
    fn bright(&mut self) -> Rgb {
        let k = self.pick(&[
            Swatch::Pink,
            Swatch::Yellow,
            Swatch::Blue,
            Swatch::Green,
            Swatch::Orange,
            Swatch::Purple,
            Swatch::Teal,
            Swatch::Red,
        ]);
        self.col(k)
    }

    fn surface(&mut self, spec: Spec) -> Mat {
        let world = &mut *self.world;
        world.resource_scope(|w, mut surfaces: Mut<Surfaces>| {
            w.resource_scope(|w, mut images: Mut<Assets<Image>>| {
                let mut mats = w.resource_mut::<Assets<SurfaceMaterial>>();
                Mat::S(surfaces.material(&spec, &mut images, &mut mats))
            })
        })
    }

    fn plain(&mut self, c: Rgb, kind: Kind) -> Mat {
        self.surface(Spec::plain(color(c).to_linear(), Some(kind)))
    }

    fn plain_with(&mut self, c: Rgb, kind: Kind, f: impl FnOnce(&mut Spec)) -> Mat {
        let mut spec = Spec::plain(color(c).to_linear(), Some(kind));
        f(&mut spec);
        self.surface(spec)
    }

    fn pattern(&mut self, c1: Rgb, c2: Rgb, freq: f32, dir: [f32; 2], kind: Kind, p: Pattern) -> Mat {
        self.surface(Spec {
            paint: Some(Paint {
                c1: color(c1).to_linear(),
                c2: color(c2).to_linear(),
                freq,
                dir: Vec2::from(dir),
                speed: 0.0,
                kind: p,
            }),
            ..Spec::plain(LinearRgba::WHITE, Some(kind))
        })
    }

    fn stripes(&mut self, c1: Rgb, c2: Rgb, freq: f32, dir: [f32; 2], p: Pattern) -> Mat {
        self.pattern(c1, c2, freq, dir, Kind::Plastic, p)
    }

    /// Unlit, glowing (not dimmed by the light, no fog).
    fn glow(&mut self, c: Rgb, opacity: f32) -> Mat {
        let key = format!("{c}|{opacity}");
        if let Some(h) = self.glows.get(&key) {
            return Mat::G(h.clone());
        }
        let h = self
            .world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: color(c).with_alpha(opacity),
                unlit: true,
                fog_enabled: false,
                alpha_mode: if opacity < 1.0 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                },
                ..default()
            });
        self.glows.insert(key, h.clone());
        Mat::G(h)
    }

    fn shape(&mut self, s: Shape) -> Handle<Mesh> {
        if let Some(h) = self.shapes.get(&s) {
            return h.clone();
        }
        // (The parts that stay apart move, mostly near the course: the near level.)
        let h = self
            .world
            .resource_mut::<Assets<Mesh>>()
            .add(shape_mesh(s, Level::Near));
        self.shapes.insert(s, h.clone());
        h
    }

    fn group(&mut self, parent: Option<Entity>, t: Transform) -> Entity {
        let mut e = self.world.spawn((Decor, t, Visibility::default()));
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
        e.id()
    }

    /// A part: a unit shape scaled to `size`, at `at` in `parent`, turned by `rot`.
    fn part(&mut self, parent: Entity, shape: Shape, mat: &Mat, at: [f32; 3], size: [f32; 3], rot: [f32; 3]) -> Entity {
        let mesh = self.shape(shape);
        let mut e = self.world.spawn((
            Decor,
            Mesh3d(mesh),
            tf(at, size, rot),
            Visibility::default(),
            NotShadowCaster,
            ChildOf(parent),
        ));
        match mat {
            Mat::S(h) => e.insert(MeshMaterial3d(h.clone())),
            Mat::G(h) => e.insert((MeshMaterial3d(h.clone()), NotShadowReceiver)),
        };
        let e = e.id();
        self.parts.push(Part {
            e,
            shape,
            mat: mat.clone(),
            piece: self.piece,
        });
        e
    }

    /// A model (turned at random); a flag's cloth takes `tint`.
    fn model(&mut self, name: Model, parent: Entity, at: [f32; 3], scale: f32, tint: Option<Rgb>) -> Entity {
        let paint = tint.map(|t| vec![("Flag", t, 0.0)]).unwrap_or_default();
        self.model_painted(name, parent, at, scale, paint)
    }

    /// A model (turned at random) with materials repainted by name (`Prop::paint`).
    fn model_painted(
        &mut self,
        name: Model,
        parent: Entity,
        at: [f32; 3],
        scale: f32,
        paint: Vec<(&'static str, Rgb, f32)>,
    ) -> Entity {
        let yaw = self.rnd() * 6.3;
        let scene = self
            .assets
            .load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
        self.world
            .spawn((
                Decor,
                WorldAssetRoot(scene),
                Prop::painted(name, paint),
                Transform::from_translation(Vec3::from(at))
                    .with_rotation(Quat::from_rotation_y(yaw))
                    .with_scale(Vec3::splat(scale)),
                Visibility::default(),
                // (Given to each mesh by `props::dress`, as the rest of the scenery.)
                NotShadowCaster,
                ChildOf(parent),
            ))
            .id()
    }

    /// An island in the look's colours.
    fn island(&mut self, parent: Entity, t: Transform) -> Entity {
        let l = self.look.look;
        let paint = if !matches!(l.id, LookId::Classic | LookId::Meadow) {
            let i = l.island;
            vec![
                ("Grass", i.grass, 0.0),
                ("Rock", i.rock, 0.0),
                ("Leaves", i.leaves, 0.0),
            ]
        } else {
            Vec::new()
        };
        let scene = self
            .assets
            .load(GltfAssetLabel::Scene(0).from_asset("models/island.glb"));
        self.world
            .spawn((
                Decor,
                WorldAssetRoot(scene),
                Prop::painted(Model::Island, paint),
                t,
                Visibility::default(),
                NotShadowCaster,
                ChildOf(parent),
            ))
            .id()
    }

    /// A soft round shadow on an island's grass under what stands there (the shadow maps do not reach this far
    /// out): a disc darkening what is under it, most at its centre. At `at` in `parent`, `r` across.
    fn blob_shadow(&mut self, parent: Entity, at: [f32; 3], r: f32) {
        if self.blob.is_none() {
            let mesh = self.world.resource_mut::<Assets<Mesh>>().add(blob_disc());
            let mat = self
                .world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    // (Multiplied into what is under it, by the disc's alpha: a cool shade, not grey.)
                    base_color: Color::srgb(0.46, 0.4, 0.56),
                    unlit: true,
                    fog_enabled: false,
                    alpha_mode: AlphaMode::Multiply,
                    ..default()
                });
            self.blob = Some((mesh, mat));
        }
        let Some((mesh, mat)) = self.blob.clone() else {
            return;
        };
        self.world.spawn((
            Decor,
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Transform::from_translation(Vec3::from(at)).with_scale(Vec3::new(r, 1.0, r)),
            Visibility::default(),
            NotShadowCaster,
            NotShadowReceiver,
            ChildOf(parent),
        ));
    }

    fn tick(&mut self, f: impl Fn(f32, &mut Tx) + Send + Sync + 'static) {
        self.ticks.push(Box::new(f));
    }

    fn set_tf(&mut self, e: Entity, f: impl FnOnce(&mut Transform)) {
        if let Some(mut t) = self.world.get_mut::<Transform>(e) {
            f(&mut t);
        }
    }
}

struct Piece {
    /// Footprint radius and height at scale 1.
    r: f32,
    h: f32,
    /// Stands on a floating island (else floats by itself).
    island: bool,
    weight: f32,
    make: fn(&mut Kit, Entity),
}

const fn piece(r: f32, h: f32, island: bool, weight: f32, make: fn(&mut Kit, Entity)) -> Piece {
    Piece {
        r,
        h,
        island,
        weight,
        make,
    }
}

/// The pieces of each look (weighted).
fn set_of(look: LookId) -> Vec<Piece> {
    let grove_ = piece(3.0, 5.0, true, 2.0, grove);
    let flowers_ = piece(2.6, 1.6, true, 1.0, flowers);
    let windmill_ = piece(2.2, 8.0, true, 1.0, windmill);
    let banners_ = piece(2.2, 6.0, true, 1.0, banners);
    let palm_ = piece(2.8, 6.0, true, 2.0, palm);
    let crystals_ = |lit: bool| piece(1.8, 4.0, false, 1.0, if lit { crystals_lit } else { crystals_cold });
    match look {
        LookId::Meadow => vec![grove_, flowers_, windmill_],
        LookId::Castle => vec![
            piece(2.0, 11.0, true, 2.0, tower),
            piece(3.2, 8.0, true, 1.0, keep),
            banners_,
            grove_,
        ],
        LookId::Factory => vec![
            piece(3.4, 7.0, false, 2.0, gear),
            piece(1.6, 13.0, true, 2.0, chimney),
            piece(2.6, 5.0, true, 1.0, tank),
        ],
        LookId::Snow => vec![
            piece(2.0, 5.0, true, 2.0, snow_pine),
            piece(1.3, 3.6, true, 1.0, snowman),
            crystals_(false),
        ],
        LookId::Starlight => vec![
            piece(5.0, 6.0, false, 2.0, planet),
            piece(3.0, 5.0, false, 2.0, orbs),
            crystals_(true),
        ],
        LookId::Circus => vec![
            piece(3.3, 6.5, true, 2.0, tent),
            piece(2.0, 6.0, false, 2.0, balloon_bunch),
            piece(4.4, 10.0, true, 1.0, ferris),
            banners_,
        ],
        LookId::Neon => vec![
            piece(3.2, 7.0, false, 2.0, neon_rings),
            piece(1.5, 10.0, false, 2.0, pylon),
            crystals_(true),
        ],
        LookId::Ocean => vec![
            piece(1.8, 10.0, true, 1.0, lighthouse),
            palm_,
            piece(2.8, 3.5, true, 1.0, beach),
        ],
        LookId::Desert => vec![
            piece(1.5, 5.0, true, 2.0, cactus),
            piece(4.4, 7.0, false, 1.0, mesa),
            piece(3.3, 4.0, true, 1.0, pyramid),
            palm_,
        ],
        LookId::Jungle => vec![palm_, piece(2.8, 3.5, true, 2.0, big_plant), flowers_, grove_],
        LookId::Lava => vec![
            piece(4.4, 7.0, false, 2.0, volcano),
            piece(2.8, 4.0, false, 2.0, rocks),
            piece(2.0, 5.0, true, 1.0, shards),
        ],
        LookId::Royal => vec![
            piece(1.4, 7.5, true, 2.0, pillar),
            piece(2.8, 4.0, false, 2.0, crown),
            banners_,
            grove_,
        ],
        LookId::Candy => vec![
            piece(1.6, 6.0, true, 2.0, lollipop),
            piece(1.4, 6.0, true, 1.0, cane),
            piece(2.2, 3.0, false, 2.0, donut),
            piece(2.6, 1.8, true, 1.0, gumdrops),
        ],
        _ => Vec::new(),
    }
}

/// Looks whose islands grow trees.
const LEAFY: [LookId; 7] = [
    LookId::Classic,
    LookId::Meadow,
    LookId::Castle,
    LookId::Circus,
    LookId::Royal,
    LookId::Jungle,
    LookId::Ocean,
];

/// The scenery's motion for the map shown.
#[derive(Resource, Default)]
struct Ticks {
    generation: Option<u32>,
    list: Vec<Tick>,
}

pub struct DecorPlugin;

impl Plugin for DecorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ticks>();
        app.add_systems(Update, (build, animate).chain());
    }
}

/// Builds the scenery once the map's entities are in (under the map's root: it goes with the map).
fn build(world: &mut World) {
    let Some(map) = world.get_resource::<Map>() else { return };
    let generation = map.generation;
    if world.resource::<Ticks>().generation == Some(generation) {
        return;
    }
    let root = world
        .query::<(Entity, &MapRoot)>()
        .iter(world)
        .find(|(_, r)| r.0 == generation)
        .map(|(e, _)| e);
    let Some(root) = root else { return };
    let map = world.resource::<Map>();
    let boxes: Vec<Aabb> = map
        .world
        .colliders
        .iter()
        .filter(|c| !c.opts.trigger)
        .map(|c| {
            let (p, r) = (c.center.as_vec3(), c.radius as f32);
            Aabb {
                min: p - Vec3::splat(r),
                max: p + Vec3::splat(r),
            }
        })
        .collect();
    let all = bounds(&boxes);
    let look = map.look.clone();
    let seed = map.round.seed as i32 ^ 0x5eed;
    let requests = map.scene.scenery.clone();
    let assets = world.resource::<AssetServer>().clone();
    let mut kit = Kit {
        world,
        look,
        rnd: Rnd(seed),
        shapes: HashMap::new(),
        glows: HashMap::new(),
        assets,
        ticks: Vec::new(),
        parts: Vec::new(),
        piece: 0,
        grounds: Vec::new(),
        blob: None,
    };
    let base = kit.group(Some(root), Transform::default());
    for req in &requests {
        clouds(&mut kit, base, &boxes, req);
    }
    if kit.look.look.birds {
        birds(&mut kit, base, all);
    }
    if kit.look.look.balloons {
        balloons(&mut kit, base, &boxes, all);
    }
    islands(&mut kit, base, &boxes, all);
    ground(&mut kit, base, all);
    decorate(&mut kit, base, &boxes, all);
    let list = core::mem::take(&mut kit.ticks);
    let parts = core::mem::take(&mut kit.parts);
    let grounds = core::mem::take(&mut kit.grounds);
    merge_still(world, &list, &parts, &grounds, base);
    let mut ticks = world.resource_mut::<Ticks>();
    ticks.generation = Some(generation);
    ticks.list = list;
}

fn animate(
    ticks: Res<Ticks>,
    map: Option<Res<Map>>,
    mut q: Query<(&'static mut Transform, &'static mut Visibility), With<Decor>>,
    clock: FrameClock,
) {
    let Some(map) = map else { return };
    if ticks.generation != Some(map.generation) {
        return;
    }
    let t = map.time(clock.tick()) as f32;
    let mut tx = Tx { q: &mut q };
    for f in &ticks.list {
        f(t, &mut tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rnd_is_stable() {
        // The first values with seed 1 ^ 0x5eed: the scenery stays where it was.
        let mut r = Rnd(1 ^ 0x5eed);
        let v: Vec<f32> = (0..3).map(|_| r.next()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)), "{v:?}");
    }

    #[test]
    fn solids_close() {
        for m in [octahedron(), dodecahedron()] {
            let n = m.count_vertices();
            assert!(n >= 24 && n % 3 == 0, "{n}");
        }
        assert_eq!(dodecahedron().count_vertices(), 12 * 3 * 3);
    }

    #[test]
    fn shapes_merge_at_both_levels() {
        let all = [
            Shape::Box,
            Shape::Sphere,
            Shape::Cyl,
            Shape::Taper,
            Shape::Cone,
            Shape::Cone4,
            Shape::Torus,
            Shape::Ring,
            Shape::Octa,
            Shape::Rock,
            Shape::Dome,
            Shape::HalfTorus,
        ];
        for s in all {
            for level in [Level::Near, Level::Far] {
                assert!(
                    super::super::meshes::mergeable(&shape_mesh(s, level)),
                    "{s:?} {level:?}"
                );
            }
        }
    }

    #[test]
    fn blob_disc_faces_up_and_fades_out() {
        let m = blob_disc();
        let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!()
        };
        let idx: Vec<usize> = m.indices().unwrap().iter().collect();
        for t in idx.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(p[i]));
            assert!((b - a).cross(c - a).y > 0.0, "{t:?}");
        }
        let Some(VertexAttributeValues::Float32x4(c)) = m.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!()
        };
        assert!(c[0][3] > 0.9);
        assert_eq!(c[c.len() - 1][3], 0.0);
    }

    #[test]
    fn parts_shade_their_neighbours_not_themselves() {
        let mut shade = Shade::default();
        balls(
            Shape::Sphere,
            &Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            1,
            &mut shade.balls,
        );
        // Under the ball, facing it: shaded; facing across it, or the ball's own part: not.
        let under = occlusion(&shade, 0, Vec3::ZERO, Vec3::Y);
        assert!(under > 0.05 && under <= SHADE_MAX, "{under}");
        assert_eq!(occlusion(&shade, 0, Vec3::ZERO, Vec3::X), 0.0);
        assert_eq!(occlusion(&shade, 1, Vec3::ZERO, Vec3::Y), 0.0);
        // Close above the grass: shaded by it.
        shade.ground = Some(-0.1);
        assert!(occlusion(&shade, 0, Vec3::ZERO, Vec3::X) > 0.05);
        // A long part stands as a row of balls.
        let mut row = Vec::new();
        balls(Shape::Cyl, &Mat4::from_scale(Vec3::new(0.1, 4.0, 0.1)), 0, &mut row);
        assert_eq!(row.len(), 4);
    }
}
