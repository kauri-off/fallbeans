//! The scenery around a map (client only): drifting clouds kept clear
//! of the course, birds circling far out, hot-air balloons on the horizon, floating islands, the set
//! pieces of the round's look (castle towers, gears, snowmen, planets, tents, neon rings, lighthouses,
//! cacti, volcanoes, crowns, lollipops…) and the land far below. Visual only: its own random numbers. The parts
//! nothing moves are merged by cell and material.
use std::collections::{HashMap, HashSet};

use bevy::asset::{RenderAssetUsages, UntypedAssetId};
use bevy::camera::primitives::MeshAabb;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use fb_sim::looks::{Pattern, ResolvedLook};
use fb_sim::scene::SceneryRequest;

use super::props::Prop;
use super::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::game::Map;
use crate::view::{MapRoot, frame_tick, hex};

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

// ---------------------------------------------------------------- shapes

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Shape {
    Box,
    Sphere,
    Cyl,
    Taper,
    Cone,
    Cone4,
    Torus,
    Ring,
    Octa,
    Rock,
    Dome,
    HalfTorus,
}

/// A mesh of flat triangles (each its own normal).
fn flat(tris: &[[Vec3; 3]]) -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    for t in tris {
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or(Vec3::Y);
        for v in t {
            pos.push(v.to_array());
            nrm.push(n.to_array());
        }
    }
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
}

/// Faces wound outwards (a convex shape around the origin).
fn outward(mut t: [Vec3; 3]) -> [Vec3; 3] {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    if n.dot(t[0] + t[1] + t[2]) < 0.0 {
        t.swap(1, 2);
    }
    t
}

fn octahedron() -> Mesh {
    let v = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    let mut tris = Vec::new();
    for x in [0, 1] {
        for y in [2, 3] {
            for z in [4, 5] {
                tris.push(outward([v[x], v[y], v[z]]));
            }
        }
    }
    flat(&tris)
}

/// A dodecahedron of radius 1: twelve flat pentagons.
fn dodecahedron() -> Mesh {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let r = 1.0 / t;
    let mut v: Vec<Vec3> = Vec::new();
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                v.push(Vec3::new(x, y, z));
            }
        }
    }
    for a in [-r, r] {
        for b in [-t, t] {
            v.push(Vec3::new(0.0, a, b));
            v.push(Vec3::new(a, b, 0.0));
            v.push(Vec3::new(b, 0.0, a));
        }
    }
    let v: Vec<Vec3> = v.into_iter().map(|p| p.normalize()).collect();
    // Faces: the pentagons around each of the twelve icosahedron directions.
    let mut tris = Vec::new();
    let ico: Vec<Vec3> = {
        let mut d = Vec::new();
        for a in [-1.0, 1.0] {
            for b in [-t, t] {
                d.push(Vec3::new(0.0, b, a).normalize());
                d.push(Vec3::new(a, 0.0, b).normalize());
                d.push(Vec3::new(b, a, 0.0).normalize());
            }
        }
        d
    };
    for n in ico {
        let mut ring: Vec<Vec3> = v.iter().copied().filter(|p| p.dot(n) > 0.7).collect();
        let c = ring.iter().copied().sum::<Vec3>() / ring.len() as f32;
        let u = (ring[0] - c).normalize();
        let w = n.cross(u);
        ring.sort_by(|a, b| {
            let fa = (*a - c).dot(w).atan2((*a - c).dot(u));
            let fb = (*b - c).dot(w).atan2((*b - c).dot(u));
            fa.total_cmp(&fb)
        });
        for i in 1..ring.len() - 1 {
            tris.push(outward([ring[0], ring[i], ring[i + 1]]));
        }
    }
    flat(&tris)
}

/// A torus round the z axis (standing in the x/y plane); `arc` of it.
fn torus(radius: f32, tube: f32, radial: u32, tubular: u32, arc: f32) -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    for j in 0..=radial {
        for i in 0..=tubular {
            let u = i as f32 / tubular as f32 * arc;
            let v = j as f32 / radial as f32 * core::f32::consts::TAU;
            let p = Vec3::new(
                (radius + tube * v.cos()) * u.cos(),
                (radius + tube * v.cos()) * u.sin(),
                tube * v.sin(),
            );
            let c = Vec3::new(radius * u.cos(), radius * u.sin(), 0.0);
            pos.push(p.to_array());
            nrm.push((p - c).normalize().to_array());
        }
    }
    let mut idx = Vec::new();
    for j in 1..=radial {
        for i in 1..=tubular {
            let a = (tubular + 1) * j + i - 1;
            let b = (tubular + 1) * (j - 1) + i - 1;
            let c = (tubular + 1) * (j - 1) + i;
            let d = (tubular + 1) * j + i;
            idx.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// The upper half of a unit sphere (a dome), flat side down.
fn dome() -> Mesh {
    let (w, h) = (20u32, 10u32);
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    for j in 0..=h {
        let th = j as f32 / h as f32 * core::f32::consts::FRAC_PI_2;
        for i in 0..=w {
            let ph = i as f32 / w as f32 * core::f32::consts::TAU;
            let p = Vec3::new(-ph.cos() * th.sin(), th.cos(), ph.sin() * th.sin());
            pos.push(p.to_array());
            nrm.push(p.to_array());
        }
    }
    let mut idx = Vec::new();
    for j in 0..h {
        for i in 0..w {
            let a = j * (w + 1) + i;
            let b = a + w + 1;
            idx.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
        }
    }
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

fn shape_mesh(s: Shape) -> Mesh {
    match s {
        Shape::Box => Cuboid::new(1.0, 1.0, 1.0).into(),
        Shape::Sphere => Sphere::new(1.0).mesh().uv(20, 14),
        Shape::Cyl => Cylinder::new(1.0, 1.0).mesh().resolution(20).build(),
        Shape::Taper => ConicalFrustum {
            radius_top: 0.62,
            radius_bottom: 1.0,
            height: 1.0,
        }
        .mesh()
        .resolution(20)
        .build(),
        Shape::Cone => Cone::new(1.0, 1.0).mesh().resolution(20).build(),
        Shape::Cone4 => Cone::new(1.0, 1.0).mesh().resolution(4).build(),
        Shape::Torus => torus(1.0, 0.3, 12, 36, core::f32::consts::TAU),
        Shape::Ring => torus(1.0, 0.07, 8, 56, core::f32::consts::TAU),
        Shape::Octa => octahedron(),
        Shape::Rock => dodecahedron(),
        Shape::Dome => dome(),
        Shape::HalfTorus => torus(0.7, 0.28, 12, 24, core::f32::consts::PI),
    }
}

// ---------------------------------------------------------------- building

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
    parts: Vec<(Entity, Shape, Mat)>,
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

    fn col(&self, k: &str) -> String {
        let i = fb_sim::looks::PAL_KEYS.iter().position(|p| *p == k).unwrap_or(0);
        self.look.palette[i][0].clone()
    }

    fn col2(&self, k: &str) -> String {
        let i = fb_sim::looks::PAL_KEYS.iter().position(|p| *p == k).unwrap_or(0);
        self.look.palette[i][1].clone()
    }

    /// One of the look's bright palette colours.
    fn bright(&mut self) -> String {
        let k = self.pick(&["pink", "yellow", "blue", "green", "orange", "purple", "teal", "red"]);
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

    fn plain(&mut self, c: &str, kind: Kind) -> Mat {
        self.surface(Spec::plain(hex(c).to_linear(), Some(kind)))
    }

    fn plain_with(&mut self, c: &str, kind: Kind, f: impl FnOnce(&mut Spec)) -> Mat {
        let mut spec = Spec::plain(hex(c).to_linear(), Some(kind));
        f(&mut spec);
        self.surface(spec)
    }

    fn pattern(&mut self, c1: &str, c2: &str, freq: f32, dir: [f32; 2], kind: Kind, p: Pattern) -> Mat {
        self.surface(Spec {
            paint: Some(Paint {
                c1: hex(c1).to_linear(),
                c2: hex(c2).to_linear(),
                freq,
                dir: Vec2::from(dir),
                speed: 0.0,
                kind: p,
            }),
            ..Spec::plain(LinearRgba::WHITE, Some(kind))
        })
    }

    fn stripes(&mut self, c1: &str, c2: &str, freq: f32, dir: [f32; 2], p: Pattern) -> Mat {
        self.pattern(c1, c2, freq, dir, Kind::Plastic, p)
    }

    /// Unlit, glowing (not dimmed by the light, no fog).
    fn glow(&mut self, c: &str, opacity: f32) -> Mat {
        let key = format!("{c}|{opacity}");
        if let Some(h) = self.glows.get(&key) {
            return Mat::G(h.clone());
        }
        let h = self
            .world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: hex(c).with_alpha(opacity),
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
        let h = self.world.resource_mut::<Assets<Mesh>>().add(shape_mesh(s));
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
        self.parts.push((e, shape, mat.clone()));
        e
    }

    /// A model (turned at random); a flag's cloth takes `tint`.
    fn model(&mut self, name: &'static str, parent: Entity, at: [f32; 3], scale: f32, tint: Option<String>) -> Entity {
        let yaw = self.rnd() * 6.3;
        let scene = self
            .assets
            .load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
        let paint = tint.map(|t| vec![("Flag", t, 0.0)]).unwrap_or_default();
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
        let paint = if l.id != "classic" && l.id != "meadow" {
            let i = l.island;
            vec![
                ("Grass", i.grass.to_string(), 0.0),
                ("Rock", i.rock.to_string(), 0.0),
                ("Leaves", i.leaves.to_string(), 0.0),
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
                Prop::painted("island", paint),
                t,
                Visibility::default(),
                NotShadowCaster,
                ChildOf(parent),
            ))
            .id()
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

// ---------------------------------------------------------------- pieces

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

fn flora(k: &mut Kit, g: Entity) {
    let n = 2 + (k.rnd() * 2.0) as usize;
    for i in 0..n {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 1.2 + k.rnd() * 1.2 } else { 0.0 };
        let name = k.pick(&["tree", "pine", "tree", "mushroom"]);
        let s = 0.55 + k.rnd() * 0.3;
        k.model(name, g, [a.cos() * r, 0.0, a.sin() * r], s, None);
    }
}

fn grove(k: &mut Kit, g: Entity) {
    flora(k, g);
}

fn flowers(k: &mut Kit, g: Entity) {
    let stem = k.plain("#4fae4a", Kind::Leaf);
    let yellow = k.col("yellow");
    let heart = k.plain(&yellow, Kind::Plastic);
    for _ in 0..9 {
        let a = k.rnd() * 6.3;
        let r = 0.4 + k.rnd() * 2.2;
        let h = 0.6 + k.rnd() * 0.9;
        let (x, z) = (a.cos() * r, a.sin() * r);
        k.part(g, Shape::Cyl, &stem, [x, h / 2.0, z], [0.05, h, 0.05], NO_ROT);
        let c = k.bright();
        let head = k.plain(&c, Kind::Fabric);
        for p in 0..5 {
            let pa = p as f32 / 5.0 * core::f32::consts::TAU;
            k.part(
                g,
                Shape::Sphere,
                &head,
                [x + pa.cos() * 0.2, h, z + pa.sin() * 0.2],
                [0.18, 0.07, 0.18],
                NO_ROT,
            );
        }
        k.part(g, Shape::Sphere, &heart, [x, h + 0.03, z], [0.11, 0.08, 0.11], NO_ROT);
    }
}

fn windmill(k: &mut Kit, g: Entity) {
    let white = k.col("white");
    let red = k.col("red");
    let pink = k.col("pink");
    let yellow = k.col("yellow");
    let wall = k.plain(&white, Kind::Wood);
    k.part(g, Shape::Taper, &wall, [0.0, 2.6, 0.0], [1.0, 5.2, 1.0], NO_ROT);
    let roof = k.plain(&red, Kind::Wood);
    k.part(g, Shape::Cone, &roof, [0.0, 5.9, 0.0], [1.1, 1.6, 1.1], NO_ROT);
    let base = Transform::from_xyz(0.0, 4.6, 0.75);
    let hub = k.group(Some(g), base);
    let sail = k.stripes(&white, &pink, 2.2, [1.0, 0.0], Pattern::Stripes);
    for i in 0..4 {
        let arm = k.group(
            Some(hub),
            Transform::from_rotation(Quat::from_rotation_z(i as f32 * core::f32::consts::FRAC_PI_2)),
        );
        k.part(arm, Shape::Box, &sail, [0.0, 1.7, 0.0], [0.55, 3.0, 0.06], NO_ROT);
    }
    let y = k.plain(&yellow, Kind::Plastic);
    k.part(hub, Shape::Sphere, &y, [0.0, 0.0, 0.05], [0.25, 0.25, 0.25], NO_ROT);
    let sp = 0.6 + k.rnd() * 0.6;
    k.tick(move |t, tx| tx.set(hub, base.with_rotation(Quat::from_rotation_z(t * sp))));
}

fn tower(k: &mut Kit, g: Entity) {
    let stone = k.stripes("#d8d2c6", "#bdb5a8", 1.6, [0.0, 1.0], Pattern::Checker);
    k.part(g, Shape::Cyl, &stone, [0.0, 3.5, 0.0], [1.4, 7.0, 1.4], NO_ROT);
    for i in 0..8 {
        let a = i as f32 / 8.0 * core::f32::consts::TAU;
        k.part(
            g,
            Shape::Box,
            &stone,
            [a.cos() * 1.35, 7.25, a.sin() * 1.35],
            [0.45, 0.5, 0.45],
            [0.0, -a, 0.0],
        );
    }
    let roof = k.pick(&["red", "blue", "purple"]);
    let (c1, c2) = (k.col(roof), k.col2(roof));
    let rm = k.stripes(&c1, &c2, 2.0, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Cone, &rm, [0.0, 8.8, 0.0], [1.65, 2.6, 1.65], NO_ROT);
    let door = k.plain("#3a3048", Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 4.2, 1.38], [0.35, 0.7, 0.1], NO_ROT);
    let y = k.col("yellow");
    k.model("flag", g, [0.0, 9.9, 0.0], 0.45, Some(y));
}

fn keep(k: &mut Kit, g: Entity) {
    let stone = k.stripes("#d8d2c6", "#c4bcae", 1.4, [1.0, 1.0], Pattern::Checker);
    k.part(g, Shape::Box, &stone, [0.0, 2.4, 0.0], [4.0, 4.8, 4.0], NO_ROT);
    let blue = k.col("blue");
    let roof = k.plain(&blue, Kind::Plastic);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            k.part(
                g,
                Shape::Cyl,
                &stone,
                [sx * 2.0, 3.0, sz * 2.0],
                [0.7, 6.0, 0.7],
                NO_ROT,
            );
            k.part(
                g,
                Shape::Cone,
                &roof,
                [sx * 2.0, 6.8, sz * 2.0],
                [0.85, 1.6, 0.85],
                NO_ROT,
            );
        }
    }
    let (r, y) = (k.col("red"), k.col("yellow"));
    let banner = k.stripes(&r, &y, 1.8, [1.0, 0.0], Pattern::Chevron);
    k.part(g, Shape::Box, &banner, [0.0, 3.2, 2.03], [1.3, 2.4, 0.05], NO_ROT);
    let door = k.plain("#3a3048", Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 0.8, 2.02], [1.0, 1.6, 0.05], NO_ROT);
}

fn banners(k: &mut Kit, g: Entity) {
    let pole = k.plain_with("#d8c090", Kind::Gold, |s| {
        s.metallic = Some(0.7);
        s.roughness = Some(0.3);
    });
    let ph = k.rnd() * 6.0;
    let white = k.col("white");
    let mut cloths = Vec::new();
    for i in 0..3 {
        let x = (i as f32 - 1.0) * 1.6;
        k.part(g, Shape::Cyl, &pole, [x, 2.8, 0.0], [0.07, 5.6, 0.07], NO_ROT);
        k.part(g, Shape::Sphere, &pole, [x, 5.7, 0.0], [0.16, 0.16, 0.16], NO_ROT);
        let c = k.bright();
        let base = Transform::from_xyz(x, 5.3, 0.1);
        let cloth = k.group(Some(g), base);
        let m = k.stripes(&c, &white, 1.6, [0.0, 1.0], Pattern::Chevron);
        k.part(cloth, Shape::Box, &m, [0.0, -1.3, 0.0], [0.9, 2.6, 0.04], NO_ROT);
        cloths.push((cloth, base));
    }
    k.tick(move |t, tx| {
        for (i, (c, base)) in cloths.iter().enumerate() {
            let a = (t * 1.3 + ph + i as f32).sin() * 0.08;
            tx.set(*c, base.with_rotation(Quat::from_rotation_x(a)));
        }
    });
}

fn gear(k: &mut Kit, g: Entity) {
    let base = Transform::from_xyz(0.0, 3.5, 0.0);
    let wheel = k.group(Some(g), base);
    let (o, y) = (k.col("orange"), k.col("yellow"));
    let c = k.pick(&[o.as_str(), y.as_str(), "#9aa3b0"]).to_string();
    let metal = k.plain_with(&c, Kind::Metal, |s| {
        s.metallic = Some(0.6);
        s.roughness = Some(0.35);
    });
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(wheel, Shape::Cyl, &metal, [0.0; 3], [2.6, 0.6, 2.6], flat_x);
    for i in 0..12 {
        let a = i as f32 / 12.0 * core::f32::consts::TAU;
        k.part(
            wheel,
            Shape::Box,
            &metal,
            [a.cos() * 2.85, a.sin() * 2.85, 0.0],
            [0.7, 0.7, 0.6],
            [0.0, 0.0, a],
        );
    }
    let hub = k.plain("#4a4f5a", Kind::Metal);
    k.part(wheel, Shape::Cyl, &hub, [0.0; 3], [0.6, 0.9, 0.6], flat_x);
    let bolt = k.plain("#3a3f4a", Kind::Metal);
    for i in 0..4 {
        let a = i as f32 * 1.57;
        k.part(
            wheel,
            Shape::Cyl,
            &bolt,
            [a.cos() * 1.5, a.sin() * 1.5, 0.0],
            [0.35, 0.8, 0.35],
            flat_x,
        );
    }
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
    let sp = (if k.rnd() < 0.5 { -1.0 } else { 1.0 }) * (0.3 + k.rnd() * 0.4);
    k.tick(move |t, tx| tx.set(wheel, base.with_rotation(Quat::from_rotation_z(t * sp))));
}

/// Puffs of smoke rising from a spout (chimneys, volcanoes).
fn puffs(k: &mut Kit, g: Entity, mat: &Mat, y0: f32, rise: f32, drift: Vec3, size: (f32, f32), rate: f32) {
    let list: Vec<Entity> = (0..4)
        .map(|_| k.part(g, Shape::Sphere, mat, [0.0, y0, 0.0], [size.0; 3], NO_ROT))
        .collect();
    let ph = k.rnd() * 4.0;
    k.tick(move |t, tx| {
        for (i, p) in list.iter().enumerate() {
            let f = (t * rate + ph + i as f32 / list.len() as f32).rem_euclid(1.0);
            tx.set(
                *p,
                Transform::from_translation(Vec3::new(drift.x * f, y0 + f * rise, drift.z * f))
                    .with_scale(Vec3::splat(size.0 + f * size.1)),
            );
            tx.show(*p, f < 0.95);
        }
    });
}

fn chimney(k: &mut Kit, g: Entity) {
    let (r, w) = (k.col("red"), k.col("white"));
    let m = k.stripes(&r, &w, 0.45, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Taper, &m, [0.0, 5.0, 0.0], [0.9, 10.0, 0.9], NO_ROT);
    let top = k.plain("#4a4f5a", Kind::Metal);
    k.part(g, Shape::Cyl, &top, [0.0, 10.1, 0.0], [0.62, 0.4, 0.62], NO_ROT);
    let smoke = k.plain_with("#e8e4de", Kind::Cloud, |s| {
        s.color.alpha = 0.75;
        s.alpha = AlphaMode::Blend;
    });
    puffs(k, g, &smoke, 10.5, 5.0, Vec3::new(1.6, 0.0, 0.6), (0.5, 1.4), 0.25);
}

fn tank(k: &mut Kit, g: Entity) {
    let metal = k.plain_with("#aeb6c2", Kind::Metal, |s| {
        s.metallic = Some(0.6);
        s.roughness = Some(0.3);
    });
    let y = k.col("yellow");
    let band = k.stripes(&y, "#3a3f4a", 1.4, [1.0, 1.0], Pattern::Chevron);
    k.part(g, Shape::Cyl, &band, [0.0, 0.4, 0.0], [2.0, 0.8, 2.0], NO_ROT);
    k.part(g, Shape::Cyl, &metal, [0.0, 2.2, 0.0], [1.9, 2.8, 1.9], NO_ROT);
    k.part(g, Shape::Dome, &metal, [0.0, 3.6, 0.0], [1.9, 0.9, 1.9], NO_ROT);
    let teal = k.col("teal");
    let pipe = k.plain(&teal, Kind::Metal);
    k.part(
        g,
        Shape::Cyl,
        &pipe,
        [2.2, 2.6, 0.0],
        [0.25, 2.2, 0.25],
        [0.0, 0.0, core::f32::consts::FRAC_PI_2],
    );
    k.part(g, Shape::Cyl, &pipe, [3.2, 1.6, 0.0], [0.25, 2.2, 0.25], NO_ROT);
}

fn snow_pine(k: &mut Kit, g: Entity) {
    let snow = k.plain("#ffffff", Kind::Cloth);
    for i in 0..2 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 1.4 } else { 0.0 };
        let s = 0.7 + k.rnd() * 0.4;
        let (x, z) = (a.cos() * r, a.sin() * r);
        k.model("pine", g, [x, 0.0, z], s, None);
        k.part(
            g,
            Shape::Cone,
            &snow,
            [x, 4.25 * s, z],
            [0.5 * s, 0.7 * s, 0.5 * s],
            NO_ROT,
        );
        k.part(
            g,
            Shape::Cone,
            &snow,
            [x, 3.35 * s, z],
            [0.85 * s, 0.35 * s, 0.85 * s],
            NO_ROT,
        );
        k.part(
            g,
            Shape::Cone,
            &snow,
            [x, 2.45 * s, z],
            [1.2 * s, 0.35 * s, 1.2 * s],
            NO_ROT,
        );
    }
}

fn snowman(k: &mut Kit, g: Entity) {
    let snow = k.plain("#ffffff", Kind::Cloth);
    k.part(g, Shape::Sphere, &snow, [0.0, 0.9, 0.0], [1.0, 0.95, 1.0], NO_ROT);
    k.part(g, Shape::Sphere, &snow, [0.0, 2.2, 0.0], [0.72, 0.7, 0.72], NO_ROT);
    k.part(g, Shape::Sphere, &snow, [0.0, 3.15, 0.0], [0.52, 0.5, 0.52], NO_ROT);
    let coal = k.plain("#2a2a33", Kind::Plastic);
    for sx in [-1.0, 1.0] {
        k.part(g, Shape::Sphere, &coal, [sx * 0.18, 3.28, 0.44], [0.06; 3], NO_ROT);
    }
    for i in 0..3 {
        k.part(
            g,
            Shape::Sphere,
            &coal,
            [0.0, 2.5 - i as f32 * 0.3, 0.68],
            [0.07; 3],
            NO_ROT,
        );
    }
    let nose = k.plain("#ff8a3d", Kind::Plastic);
    k.part(
        g,
        Shape::Cone,
        &nose,
        [0.0, 3.15, 0.62],
        [0.08, 0.45, 0.08],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    k.part(g, Shape::Cyl, &coal, [0.0, 3.62, 0.0], [0.55, 0.06, 0.55], NO_ROT);
    k.part(g, Shape::Cyl, &coal, [0.0, 3.9, 0.0], [0.36, 0.55, 0.36], NO_ROT);
    let red = k.col("red");
    let scarf = k.plain(&red, Kind::Fabric);
    k.part(
        g,
        Shape::Torus,
        &scarf,
        [0.0, 2.72, 0.0],
        [0.5, 0.5, 0.35],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

fn crystals(k: &mut Kit, g: Entity, glowing: bool) {
    let base = Transform::from_xyz(0.0, 2.0, 0.0);
    let spin = k.group(Some(g), base);
    let n = 3 + (k.rnd() * 3.0) as usize;
    for i in 0..n {
        let mat = if glowing {
            let c = k.bright();
            k.glow(&c, 0.9)
        } else {
            k.plain_with("#cdeeff", Kind::Ice, |s| {
                s.roughness = Some(0.35);
                s.emissive = hex("#9fd8ff").to_linear() * 0.25;
            })
        };
        let a = i as f32 / n as f32 * core::f32::consts::TAU;
        let r = if i > 0 { 0.8 } else { 0.0 };
        let s = if i > 0 { 0.35 + k.rnd() * 0.25 } else { 0.55 };
        let tilt = (k.rnd() - 0.5) * 0.5;
        k.part(
            spin,
            Shape::Octa,
            &mat,
            [a.cos() * r, 0.0, a.sin() * r],
            [s, s * 2.6, s],
            [0.0, a, tilt],
        );
    }
    let sp = 0.2 + k.rnd() * 0.3;
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            Transform::from_xyz(0.0, 2.0 + (t * 0.8 + ph).sin() * 0.4, 0.0)
                .with_rotation(Quat::from_rotation_y(t * sp)),
        );
    });
}

fn crystals_cold(k: &mut Kit, g: Entity) {
    crystals(k, g, false);
}

fn crystals_lit(k: &mut Kit, g: Entity) {
    crystals(k, g, true);
}

fn planet(k: &mut Kit, g: Entity) {
    let body = k.group(Some(g), Transform::from_xyz(0.0, 3.0, 0.0));
    let c = k.bright();
    let white = k.col("white");
    let m = k.stripes(&c, &white, 0.9, [0.0, 1.0], Pattern::Waves);
    k.part(body, Shape::Sphere, &m, [0.0; 3], [2.4; 3], NO_ROT);
    let tilt = k.group(
        Some(body),
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.5, 0.0, 0.3)),
    );
    let c1 = k.bright();
    let g1 = k.glow(&c1, 0.85);
    let g2 = k.glow(&white, 0.6);
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(tilt, Shape::Ring, &g1, [0.0; 3], [3.8, 3.8, 1.0], flat_x);
    k.part(tilt, Shape::Ring, &g2, [0.0; 3], [4.4, 4.4, 1.0], flat_x);
    let sp = 0.1 + k.rnd() * 0.15;
    k.tick(move |t, tx| {
        tx.set(
            body,
            Transform::from_xyz(0.0, 3.0, 0.0).with_rotation(Quat::from_rotation_y(t * sp)),
        )
    });
}

fn orbs(k: &mut Kit, g: Entity) {
    let n = 3 + (k.rnd() * 3.0) as usize;
    let mut list = Vec::new();
    for _ in 0..n {
        let c = k.bright();
        let o = k.group(Some(g), Transform::default());
        let a = k.glow(&c, 1.0);
        let b = k.glow(&c, 0.25);
        k.part(o, Shape::Sphere, &a, [0.0; 3], [0.45; 3], NO_ROT);
        k.part(o, Shape::Sphere, &b, [0.0; 3], [0.8; 3], NO_ROT);
        let (x, z, y, ph) = (
            (k.rnd() - 0.5) * 4.0,
            (k.rnd() - 0.5) * 4.0,
            1.0 + k.rnd() * 3.0,
            k.rnd() * 6.0,
        );
        list.push((o, x, y, z, ph));
    }
    k.tick(move |t, tx| {
        for (o, x, y, z, ph) in &list {
            tx.set(
                *o,
                Transform::from_xyz(x + (t * 0.4 + ph).sin() * 0.4, y + (t * 0.9 + ph).sin() * 0.5, *z),
            );
        }
    });
}

fn tent(k: &mut Kit, g: Entity) {
    let (a, b) = k.pick(&[
        ("red", "white"),
        ("blue", "yellow"),
        ("pink", "white"),
        ("purple", "yellow"),
    ]);
    let (ca, cb) = (k.col(a), k.col(b));
    let cloth = k.pattern(&ca, &cb, 2.4, [1.0, 0.0], Kind::Cloth, Pattern::Stripes);
    k.part(g, Shape::Cyl, &cloth, [0.0, 1.3, 0.0], [2.8, 2.6, 2.8], NO_ROT);
    k.part(g, Shape::Cone, &cloth, [0.0, 3.9, 0.0], [3.1, 2.6, 3.1], NO_ROT);
    let door = k.plain("#3a2040", Kind::Plastic);
    k.part(g, Shape::Box, &door, [0.0, 0.9, 2.72], [1.2, 1.8, 0.2], NO_ROT);
    let y = k.col("yellow");
    let gold = k.plain(&y, Kind::Gold);
    k.part(g, Shape::Cyl, &gold, [0.0, 5.4, 0.0], [0.06, 0.8, 0.06], NO_ROT);
    k.model("flag", g, [0.0, 5.2, 0.0], 0.35, Some(y));
    let (ma, mb) = (k.plain(&ca, Kind::Plastic), k.plain(&cb, Kind::Plastic));
    for i in 0..12 {
        let aa = i as f32 / 12.0 * core::f32::consts::TAU;
        let m = if i % 2 == 1 { &ma } else { &mb };
        k.part(
            g,
            Shape::Sphere,
            m,
            [aa.cos() * 2.95, 2.62, aa.sin() * 2.95],
            [0.22; 3],
            NO_ROT,
        );
    }
}

fn balloon_bunch(k: &mut Kit, g: Entity) {
    let sway = k.group(Some(g), Transform::default());
    let string = k.plain("#ffffff", Kind::Fabric);
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let r = 0.6 + k.rnd() * 0.6;
        let (x, z) = (a.cos() * r, a.sin() * r);
        let y = 3.6 + k.rnd() * 1.4;
        let c = k.bright();
        let m = k.plain_with(&c, Kind::Rubber, |s| s.roughness = Some(0.25));
        k.part(sway, Shape::Sphere, &m, [x, y, z], [0.55, 0.68, 0.55], NO_ROT);
        let v = Vec3::new(x, y, z);
        let s = k.part(
            sway,
            Shape::Cyl,
            &string,
            (v / 2.0).to_array(),
            [0.015, v.length(), 0.015],
            NO_ROT,
        );
        k.set_tf(s, |t| t.rotation = Quat::from_rotation_arc(Vec3::Y, v.normalize()));
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            sway,
            Transform::from_rotation(Quat::from_euler(
                EulerRot::XYZ,
                (t * 0.7 + ph).sin() * 0.08,
                t * 0.1,
                (t * 0.6 + ph).cos() * 0.08,
            )),
        )
    });
}

fn ferris(k: &mut Kit, g: Entity) {
    let white = k.col("white");
    let metal = k.plain_with(&white, Kind::Metal, |s| s.metallic = Some(0.4));
    for sz in [-0.6, 0.6] {
        for sx in [-1.0f32, 1.0] {
            k.part(
                g,
                Shape::Box,
                &metal,
                [sx * 1.4, 2.6, sz],
                [0.25, 5.6, 0.25],
                [0.0, 0.0, sx * -0.25],
            );
        }
    }
    let base = Transform::from_xyz(0.0, 5.2, 0.0);
    let wheel = k.group(Some(g), base);
    let pink = k.col("pink");
    let rim = k.plain(&pink, Kind::Metal);
    k.part(wheel, Shape::Ring, &rim, [0.0; 3], [3.8; 3], NO_ROT);
    k.part(wheel, Shape::Ring, &rim, [0.0; 3], [1.2; 3], NO_ROT);
    let mut cabins = Vec::new();
    for i in 0..8 {
        let a = i as f32 / 8.0 * core::f32::consts::TAU;
        k.part(
            wheel,
            Shape::Box,
            &metal,
            [a.cos() * 1.9, a.sin() * 1.9, 0.0],
            [3.8, 0.1, 0.1],
            [0.0, 0.0, a],
        );
        let at = Transform::from_xyz(a.cos() * 3.8, a.sin() * 3.8, 0.0);
        let cab = k.group(Some(wheel), at);
        let c = k.bright();
        let m = k.plain(&c, Kind::Plastic);
        k.part(cab, Shape::Box, &m, [0.0, -0.45, 0.0], [0.7, 0.6, 0.7], NO_ROT);
        k.part(cab, Shape::Box, &metal, [0.0, -0.05, 0.0], [0.8, 0.08, 0.8], NO_ROT);
        cabins.push((cab, at));
    }
    let sp = 0.15 + k.rnd() * 0.1;
    k.tick(move |t, tx| {
        tx.set(wheel, base.with_rotation(Quat::from_rotation_z(t * sp)));
        for (c, at) in &cabins {
            tx.set(*c, at.with_rotation(Quat::from_rotation_z(-t * sp)));
        }
    });
}

fn neon_rings(k: &mut Kit, g: Entity) {
    let mut rings = Vec::new();
    for i in 0..3 {
        let c = k.bright();
        let m = k.glow(&c, 1.0);
        let s = 2.8 - i as f32 * 0.6;
        rings.push((k.part(g, Shape::Ring, &m, [0.0, 3.5, 0.0], [s; 3], NO_ROT), s));
    }
    let white = k.col("white");
    let w = k.glow(&white, 1.0);
    k.part(g, Shape::Sphere, &w, [0.0, 3.5, 0.0], [0.4; 3], NO_ROT);
    let sp = 0.4 + k.rnd() * 0.5;
    k.tick(move |t, tx| {
        for (i, (r, s)) in rings.iter().enumerate() {
            let i = i as f32;
            tx.set(
                *r,
                tf(
                    [0.0, 3.5, 0.0],
                    [*s; 3],
                    [t * sp * (i + 1.0) * 0.6, t * sp * (1.4 - i * 0.3), i],
                ),
            );
        }
    });
}

fn pylon(k: &mut Kit, g: Entity) {
    let dark = k.plain_with("#1d1438", Kind::Metal, |s| {
        s.metallic = Some(0.5);
        s.roughness = Some(0.3);
    });
    k.part(g, Shape::Box, &dark, [0.0, 4.0, 0.0], [1.1, 8.0, 1.1], NO_ROT);
    let c = k.bright();
    let edge = k.glow(&c, 1.0);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            k.part(
                g,
                Shape::Box,
                &edge,
                [sx * 0.57, 4.0, sz * 0.57],
                [0.08, 8.0, 0.08],
                NO_ROT,
            );
        }
    }
    let band = k.glow(&c, 0.8);
    for i in 0..4 {
        k.part(
            g,
            Shape::Box,
            &band,
            [0.0, 1.0 + i as f32 * 2.0, 0.0],
            [1.16, 0.08, 1.16],
            NO_ROT,
        );
    }
    let c2 = k.bright();
    let tm = k.glow(&c2, 1.0);
    let top = k.part(g, Shape::Octa, &tm, [0.0, 9.3, 0.0], [0.7, 0.9, 0.7], NO_ROT);
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            top,
            tf(
                [0.0, 9.3 + (t * 1.5 + ph).sin() * 0.25, 0.0],
                [0.7, 0.9, 0.7],
                [0.0, t * 1.2, 0.0],
            ),
        )
    });
}

fn lighthouse(k: &mut Kit, g: Entity) {
    let red = k.col("red");
    let m = k.stripes(&red, "#ffffff", 0.55, [0.0, 1.0], Pattern::Stripes);
    k.part(g, Shape::Taper, &m, [0.0, 3.5, 0.0], [1.2, 7.0, 1.2], NO_ROT);
    let metal = k.plain("#3a3f4a", Kind::Metal);
    k.part(g, Shape::Cyl, &metal, [0.0, 7.1, 0.0], [1.05, 0.2, 1.05], NO_ROT);
    let glass = k.plain_with("#ffffff", Kind::Glass, |s| {
        s.color.alpha = 0.5;
        s.alpha = AlphaMode::Blend;
    });
    k.part(g, Shape::Cyl, &glass, [0.0, 7.7, 0.0], [0.6, 1.0, 0.6], NO_ROT);
    let lm = k.glow("#fff2a0", 1.0);
    let lamp = k.part(g, Shape::Sphere, &lm, [0.0, 7.7, 0.0], [0.35; 3], NO_ROT);
    let roof = k.plain(&red, Kind::Plastic);
    k.part(g, Shape::Cone, &roof, [0.0, 8.6, 0.0], [0.8, 0.9, 0.8], NO_ROT);
    let beam = k.group(Some(g), Transform::from_xyz(0.0, 7.7, 0.0));
    let bm = k.glow("#fff6c0", 0.18);
    k.part(
        beam,
        Shape::Cone,
        &bm,
        [0.0, 0.0, 4.0],
        [0.9, 8.0, 0.9],
        [-core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    k.tick(move |t, tx| {
        tx.set(
            beam,
            Transform::from_xyz(0.0, 7.7, 0.0).with_rotation(Quat::from_rotation_y(t * 0.8)),
        );
        tx.set(
            lamp,
            Transform::from_xyz(0.0, 7.7, 0.0).with_scale(Vec3::splat(0.33 + (t * 4.0).sin() * 0.04)),
        );
    });
}

fn palm(k: &mut Kit, g: Entity) {
    let bark = k.stripes("#b88a5a", "#9a6f45", 3.0, [0.0, 1.0], Pattern::Stripes);
    let lean = (k.rnd() - 0.5) * 0.5;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for i in 0..7 {
        let f = i as f32;
        x += lean * 0.3 * (f / 3.0);
        k.part(
            g,
            Shape::Cyl,
            &bark,
            [x, y + 0.4, 0.0],
            [0.26 - f * 0.015, 0.82, 0.26 - f * 0.015],
            [0.0, 0.0, -lean * 0.3 * (f / 3.0)],
        );
        y += 0.78;
    }
    let lc = if k.look.look.id == "jungle" {
        "#2fae4a"
    } else {
        "#4fcf5a"
    };
    let leaf = k.plain(lc, Kind::Leaf);
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let l = k.group(
            Some(g),
            Transform::from_xyz(x, y, 0.0).with_rotation(Quat::from_rotation_y(a)),
        );
        k.part(
            l,
            Shape::Sphere,
            &leaf,
            [1.3, -0.35, 0.0],
            [1.4, 0.08, 0.35],
            [0.0, 0.0, -0.45],
        );
    }
    let nut = k.plain("#7a5030", Kind::Plastic);
    for i in 0..3 {
        let a = i as f32 * 2.1;
        k.part(
            g,
            Shape::Sphere,
            &nut,
            [x + a.cos() * 0.3, y - 0.3, a.sin() * 0.3],
            [0.2; 3],
            NO_ROT,
        );
    }
}

fn beach(k: &mut Kit, g: Entity) {
    let c = k.bright();
    let pole = k.plain("#ffffff", Kind::Plastic);
    k.part(g, Shape::Cyl, &pole, [0.0, 1.3, 0.0], [0.05, 2.6, 0.05], NO_ROT);
    let shade = k.pattern(&c, "#ffffff", 3.0, [1.0, 0.0], Kind::Cloth, Pattern::Stripes);
    k.part(g, Shape::Cone, &shade, [0.0, 2.75, 0.0], [1.6, 0.6, 1.6], NO_ROT);
    let c2 = k.bright();
    let towel = k.stripes(&c2, "#ffffff", 2.0, [1.0, 0.0], Pattern::Stripes);
    k.part(g, Shape::Box, &towel, [0.6, 0.03, 1.2], [1.0, 0.04, 1.9], NO_ROT);
    let c3 = k.bright();
    let ball = k.pattern(&c3, "#ffffff", 2.2, [1.0, 0.0], Kind::Rubber, Pattern::Stripes);
    k.part(g, Shape::Sphere, &ball, [-1.3, 0.35, 0.8], [0.35; 3], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

fn cactus(k: &mut Kit, g: Entity) {
    let green = k.stripes("#5a9a4a", "#6aae56", 5.0, [1.0, 0.0], Pattern::Stripes);
    let h = 3.2 + k.rnd() * 1.5;
    k.part(g, Shape::Cyl, &green, [0.0, h / 2.0, 0.0], [0.45, h, 0.45], NO_ROT);
    k.part(g, Shape::Sphere, &green, [0.0, h, 0.0], [0.45; 3], NO_ROT);
    for side in [-1.0, 1.0] {
        let ay = 1.2 + k.rnd() * 1.2;
        let up = 0.8 + k.rnd() * 0.8;
        k.part(
            g,
            Shape::Cyl,
            &green,
            [side * 0.6, ay, 0.0],
            [0.26, 0.7, 0.26],
            [0.0, 0.0, core::f32::consts::FRAC_PI_2],
        );
        k.part(
            g,
            Shape::Cyl,
            &green,
            [side * 0.9, ay + up / 2.0, 0.0],
            [0.26, up, 0.26],
            NO_ROT,
        );
        k.part(g, Shape::Sphere, &green, [side * 0.9, ay + up, 0.0], [0.26; 3], NO_ROT);
    }
    let pink = k.col("pink");
    let flower = k.plain(&pink, Kind::Fabric);
    k.part(g, Shape::Sphere, &flower, [0.0, h + 0.4, 0.0], [0.2, 0.15, 0.2], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

fn mesa(k: &mut Kit, g: Entity) {
    let layers = ["#c8764a", "#e0a070", "#b8603a", "#e8b888", "#a8503a"];
    let mut y = 0.0;
    for (i, c) in layers.iter().enumerate() {
        let r = 4.0 - i as f32 * 0.35 - k.rnd() * 0.2;
        let h = 0.8 + k.rnd() * 0.6;
        let m = k.plain(c, Kind::Rock);
        k.part(g, Shape::Cyl, &m, [0.0, y - h / 2.0, 0.0], [r, h, r], NO_ROT);
        y -= h;
    }
    let rock = k.plain("#a8503a", Kind::Rock);
    k.part(
        g,
        Shape::Cone,
        &rock,
        [0.0, y - 1.2, 0.0],
        [3.0, 2.4, 3.0],
        [core::f32::consts::PI, 0.0, 0.0],
    );
    let red = k.col("red");
    k.model("flag", g, [0.0; 3], 0.5, Some(red));
}

fn pyramid(k: &mut Kit, g: Entity) {
    let sand = k.stripes("#f0d090", "#e0bc78", 1.6, [0.0, 1.0], Pattern::Stripes);
    let q = core::f32::consts::FRAC_PI_4;
    k.part(g, Shape::Cone4, &sand, [0.0, 1.8, 0.0], [3.0, 3.6, 3.0], [0.0, q, 0.0]);
    let y = k.col("yellow");
    let gold = k.plain_with(&y, Kind::Gold, |s| {
        s.metallic = Some(0.7);
        s.roughness = Some(0.3);
    });
    k.part(g, Shape::Cone4, &gold, [0.0, 3.35, 0.0], [0.5, 0.6, 0.5], [0.0, q, 0.0]);
}

fn big_plant(k: &mut Kit, g: Entity) {
    let leaf = k.stripes("#2f9f3f", "#48b858", 2.0, [1.0, 0.0], Pattern::Stripes);
    for i in 0..7 {
        let yaw = i as f32 / 7.0 * core::f32::consts::TAU + k.rnd() * 0.3;
        let l = k.group(Some(g), Transform::from_rotation(Quat::from_rotation_y(yaw)));
        let tilt = 0.5 + k.rnd() * 0.3;
        k.part(
            l,
            Shape::Sphere,
            &leaf,
            [1.1, 0.9, 0.0],
            [1.3, 0.1, 0.45],
            [0.0, 0.0, tilt],
        );
    }
    let c = k.bright();
    let petal = k.plain(&c, Kind::Fabric);
    for p in 0..6 {
        let a = p as f32 / 6.0 * core::f32::consts::TAU;
        k.part(
            g,
            Shape::Sphere,
            &petal,
            [a.cos() * 0.45, 2.2, a.sin() * 0.45],
            [0.42, 0.1, 0.25],
            [0.0, -a, 0.25],
        );
    }
    let y = k.col("yellow");
    let heart = k.plain(&y, Kind::Plastic);
    k.part(g, Shape::Sphere, &heart, [0.0, 2.25, 0.0], [0.22, 0.18, 0.22], NO_ROT);
    let stem = k.plain("#3f8f3a", Kind::Leaf);
    k.part(g, Shape::Cyl, &stem, [0.0, 1.1, 0.0], [0.08, 2.2, 0.08], NO_ROT);
}

fn volcano(k: &mut Kit, g: Entity) {
    let rock = k.plain("#3a2a2a", Kind::Rock);
    k.part(g, Shape::Taper, &rock, [0.0, 1.6, 0.0], [4.0, 3.2, 4.0], NO_ROT);
    k.part(
        g,
        Shape::Cone,
        &rock,
        [0.0, -1.5, 0.0],
        [4.0, 3.0, 4.0],
        [core::f32::consts::PI, 0.0, 0.0],
    );
    let crater = k.glow("#ff7a2a", 1.0);
    k.part(g, Shape::Cyl, &crater, [0.0, 3.22, 0.0], [2.3, 0.05, 2.3], NO_ROT);
    let lava = k.glow("#ffb030", 0.9);
    for _ in 0..3 {
        let a = k.rnd() * 6.3;
        k.part(
            g,
            Shape::Box,
            &lava,
            [a.cos() * 2.95, 1.6, a.sin() * 2.95],
            [0.25, 3.0, 0.05],
            [0.3, -a + core::f32::consts::FRAC_PI_2, 0.0],
        );
    }
    let smoke = k.plain_with("#5a4a4a", Kind::Cloud, |s| {
        s.color.alpha = 0.6;
        s.alpha = AlphaMode::Blend;
    });
    puffs(k, g, &smoke, 3.4, 7.0, Vec3::new(2.0, 0.0, -1.0), (0.8, 2.2), 0.18);
}

fn rocks(k: &mut Kit, g: Entity) {
    let dark = k.plain("#2e2426", Kind::Rock);
    let first = k.plain("#4a2e2a", Kind::Rock);
    let ember = k.glow("#ff8a3a", 1.0);
    let mut list = Vec::new();
    for i in 0..4 {
        let at = [(k.rnd() - 0.5) * 4.0, 1.0 + k.rnd() * 3.0, (k.rnd() - 0.5) * 4.0];
        let size = [0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9];
        let m = if i > 0 { &dark } else { &first };
        let o = k.part(g, Shape::Rock, m, at, size, NO_ROT);
        k.part(o, Shape::Octa, &ember, [0.0; 3], [0.3; 3], NO_ROT);
        let ph = k.rnd() * 6.0;
        list.push((o, at, size, ph));
    }
    k.tick(move |t, tx| {
        for (o, at, size, ph) in &list {
            tx.set(
                *o,
                tf(
                    [at[0], at[1] + (t * 0.5 + ph).sin() * 0.3, at[2]],
                    *size,
                    [0.0, t * 0.1 + ph, 0.0],
                ),
            );
        }
    });
}

fn shards(k: &mut Kit, g: Entity) {
    let obsidian = k.plain_with("#241a2a", Kind::Glass, |s| {
        s.roughness = Some(0.15);
        s.metallic = Some(0.3);
    });
    for i in 0..5 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 0.6 + k.rnd() * 1.2 } else { 0.0 };
        let h = 1.5 + k.rnd() * 3.0;
        let rx = (k.rnd() - 0.5) * 0.3;
        let rz = (k.rnd() - 0.5) * 0.3;
        k.part(
            g,
            Shape::Cone4,
            &obsidian,
            [a.cos() * r, h / 2.0, a.sin() * r],
            [0.4, h, 0.4],
            [rx, a, rz],
        );
    }
    let glow = k.glow("#ff7a2a", 0.8);
    k.part(g, Shape::Sphere, &glow, [0.0, 0.1, 0.0], [1.4, 0.1, 1.4], NO_ROT);
}

fn pillar(k: &mut Kit, g: Entity) {
    let gold = k.plain_with("#f2c14e", Kind::Gold, |s| {
        s.metallic = Some(0.8);
        s.roughness = Some(0.25);
    });
    let marble = k.stripes("#fff8ee", "#efe6f6", 2.0, [1.0, 1.0], Pattern::Waves);
    k.part(g, Shape::Box, &marble, [0.0, 0.3, 0.0], [1.8, 0.6, 1.8], NO_ROT);
    k.part(g, Shape::Cyl, &marble, [0.0, 3.4, 0.0], [0.55, 5.6, 0.55], NO_ROT);
    for i in 0..3 {
        k.part(
            g,
            Shape::Torus,
            &gold,
            [0.0, 1.0 + i as f32 * 2.2, 0.0],
            [0.58; 3],
            [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
        );
    }
    k.part(g, Shape::Box, &marble, [0.0, 6.35, 0.0], [1.4, 0.3, 1.4], NO_ROT);
    let orb = k.part(g, Shape::Sphere, &gold, [0.0, 7.0, 0.0], [0.45; 3], NO_ROT);
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            orb,
            Transform::from_xyz(0.0, 7.1 + (t * 1.2 + ph).sin() * 0.15, 0.0).with_scale(Vec3::splat(0.45)),
        )
    });
}

fn crown(k: &mut Kit, g: Entity) {
    let spin = k.group(
        Some(g),
        Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(0.15)),
    );
    let gold = k.plain_with("#ffcf3f", Kind::Gold, |s| {
        s.metallic = Some(0.85);
        s.roughness = Some(0.2);
    });
    k.part(spin, Shape::Cyl, &gold, [0.0; 3], [2.0, 0.7, 2.0], NO_ROT);
    let red = k.col("red");
    let velvet = k.plain(&red, Kind::Fabric);
    k.part(spin, Shape::Cyl, &velvet, [0.0, 0.1, 0.0], [1.9, 0.72, 1.9], NO_ROT);
    let (blue, pink, teal) = (k.col("blue"), k.col("pink"), k.col("teal"));
    let (gb, gp, gt) = (k.glow(&blue, 1.0), k.glow(&pink, 1.0), k.glow(&teal, 1.0));
    for i in 0..7 {
        let a = i as f32 / 7.0 * core::f32::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        k.part(
            spin,
            Shape::Cone,
            &gold,
            [c * 1.85, 0.95, s * 1.85],
            [0.35, 1.2, 0.35],
            NO_ROT,
        );
        let gem = if i % 2 == 1 { &gb } else { &gp };
        k.part(spin, Shape::Sphere, gem, [c * 1.85, 1.6, s * 1.85], [0.16; 3], NO_ROT);
        k.part(
            spin,
            Shape::Octa,
            &gt,
            [c * 2.02, 0.0, s * 2.02],
            [0.14, 0.2, 0.14],
            NO_ROT,
        );
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            tf(
                [0.0, 2.0 + (t * 0.6 + ph).sin() * 0.4, 0.0],
                [1.0; 3],
                [0.15, t * 0.25, 0.0],
            ),
        )
    });
}

fn lollipop(k: &mut Kit, g: Entity) {
    let h = 3.5 + k.rnd() * 1.5;
    let stick = k.plain("#ffffff", Kind::Plastic);
    k.part(g, Shape::Cyl, &stick, [0.0, h / 2.0, 0.0], [0.09, h, 0.09], NO_ROT);
    let c = k.bright();
    let p = k.pick(&[Pattern::Waves, Pattern::Stripes, Pattern::Chevron]);
    let candy = k.pattern(&c, "#ffffff", 2.6, [1.0, 1.0], Kind::Glossy, p);
    let at = [0.0, h + 1.2, 0.0];
    let disc = k.part(
        g,
        Shape::Cyl,
        &candy,
        at,
        [1.3, 0.35, 1.3],
        [core::f32::consts::FRAC_PI_2, 0.0, 0.0],
    );
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            disc,
            tf(
                at,
                [1.3, 0.35, 1.3],
                [core::f32::consts::FRAC_PI_2, (t * 0.5 + ph).sin() * 0.4, 0.0],
            ),
        )
    });
}

fn cane(k: &mut Kit, g: Entity) {
    let red = k.col("red");
    let stripes = k.pattern(&red, "#ffffff", 2.2, [1.0, 1.6], Kind::Glossy, Pattern::Stripes);
    k.part(g, Shape::Cyl, &stripes, [0.0, 2.2, 0.0], [0.28, 4.4, 0.28], NO_ROT);
    k.part(g, Shape::HalfTorus, &stripes, [0.7, 4.4, 0.0], [1.0; 3], NO_ROT);
    let yaw = k.rnd() * 6.3;
    k.set_tf(g, |t| t.rotation = Quat::from_rotation_y(yaw));
}

fn donut(k: &mut Kit, g: Entity) {
    let tilt = 0.4 + k.rnd() * 0.5;
    let spin = k.group(
        Some(g),
        Transform::from_xyz(0.0, 1.5, 0.0).with_rotation(Quat::from_rotation_x(tilt)),
    );
    let dough = k.plain("#e8b878", Kind::Plastic);
    let flat_x = [core::f32::consts::FRAC_PI_2, 0.0, 0.0];
    k.part(spin, Shape::Torus, &dough, [0.0; 3], [1.4; 3], flat_x);
    let c = k.bright();
    let icing = k.plain_with(&c, Kind::Glossy, |s| s.roughness = Some(0.3));
    k.part(spin, Shape::Torus, &icing, [0.0, 0.14, 0.0], [1.42, 1.42, 1.2], flat_x);
    for _ in 0..9 {
        let a = k.rnd() * 6.3;
        let r = 1.1 + k.rnd() * 0.6;
        let c = k.bright();
        let m = k.plain(&c, Kind::Plastic);
        let yaw = k.rnd() * 3.0;
        k.part(
            spin,
            Shape::Box,
            &m,
            [a.cos() * r, 0.55, a.sin() * r],
            [0.06, 0.06, 0.2],
            [0.0, yaw, 0.0],
        );
    }
    let ph = k.rnd() * 6.0;
    k.tick(move |t, tx| {
        tx.set(
            spin,
            tf(
                [0.0, 1.5 + (t * 0.7 + ph).sin() * 0.3, 0.0],
                [1.0; 3],
                [tilt, 0.0, t * 0.3],
            ),
        )
    });
}

fn gumdrops(k: &mut Kit, g: Entity) {
    for i in 0..6 {
        let a = k.rnd() * 6.3;
        let r = if i > 0 { 0.8 + k.rnd() * 1.6 } else { 0.0 };
        let s = 0.45 + k.rnd() * 0.4;
        let c = k.bright();
        let m = k.plain_with(&c, Kind::Glossy, |sp| sp.roughness = Some(0.2));
        k.part(
            g,
            Shape::Dome,
            &m,
            [a.cos() * r, 0.0, a.sin() * r],
            [s, s * 1.5, s],
            NO_ROT,
        );
    }
}

/// The pieces of each look (weighted).
fn set_of(look: &str) -> Vec<Piece> {
    let grove_ = piece(3.0, 5.0, true, 2.0, grove);
    let flowers_ = piece(2.6, 1.6, true, 1.0, flowers);
    let windmill_ = piece(2.2, 8.0, true, 1.0, windmill);
    let banners_ = piece(2.2, 6.0, true, 1.0, banners);
    let palm_ = piece(2.8, 6.0, true, 2.0, palm);
    let crystals_ = |lit: bool| piece(1.8, 4.0, false, 1.0, if lit { crystals_lit } else { crystals_cold });
    match look {
        "meadow" => vec![grove_, flowers_, windmill_],
        "castle" => vec![
            piece(2.0, 11.0, true, 2.0, tower),
            piece(3.2, 8.0, true, 1.0, keep),
            banners_,
            grove_,
        ],
        "factory" => vec![
            piece(3.4, 7.0, false, 2.0, gear),
            piece(1.6, 13.0, true, 2.0, chimney),
            piece(2.6, 5.0, true, 1.0, tank),
        ],
        "snow" => vec![
            piece(2.0, 5.0, true, 2.0, snow_pine),
            piece(1.3, 3.6, true, 1.0, snowman),
            crystals_(false),
        ],
        "starlight" => vec![
            piece(5.0, 6.0, false, 2.0, planet),
            piece(3.0, 5.0, false, 2.0, orbs),
            crystals_(true),
        ],
        "circus" => vec![
            piece(3.3, 6.5, true, 2.0, tent),
            piece(2.0, 6.0, false, 2.0, balloon_bunch),
            piece(4.4, 10.0, true, 1.0, ferris),
            banners_,
        ],
        "neon" => vec![
            piece(3.2, 7.0, false, 2.0, neon_rings),
            piece(1.5, 10.0, false, 2.0, pylon),
            crystals_(true),
        ],
        "ocean" => vec![
            piece(1.8, 10.0, true, 1.0, lighthouse),
            palm_,
            piece(2.8, 3.5, true, 1.0, beach),
        ],
        "desert" => vec![
            piece(1.5, 5.0, true, 2.0, cactus),
            piece(4.4, 7.0, false, 1.0, mesa),
            piece(3.3, 4.0, true, 1.0, pyramid),
            palm_,
        ],
        "jungle" => vec![palm_, piece(2.8, 3.5, true, 2.0, big_plant), flowers_, grove_],
        "lava" => vec![
            piece(4.4, 7.0, false, 2.0, volcano),
            piece(2.8, 4.0, false, 2.0, rocks),
            piece(2.0, 5.0, true, 1.0, shards),
        ],
        "royal" => vec![
            piece(1.4, 7.5, true, 2.0, pillar),
            piece(2.8, 4.0, false, 2.0, crown),
            banners_,
            grove_,
        ],
        "candy" => vec![
            piece(1.6, 6.0, true, 2.0, lollipop),
            piece(1.4, 6.0, true, 1.0, cane),
            piece(2.2, 3.0, false, 2.0, donut),
            piece(2.6, 1.8, true, 1.0, gumdrops),
        ],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------- the scenery

/// Looks whose islands grow trees.
const LEAFY: [&str; 7] = ["classic", "meadow", "castle", "circus", "royal", "jungle", "ocean"];

fn islands(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let count = 7;
    let flora = ["tree", "pine", "mushroom", "tree", "pine"];
    for n in 0..count {
        let scale = 0.8 + k.rnd() * 0.9;
        let mut pos = None;
        for _ in 0..24 {
            let a = n as f32 / count as f32 * core::f32::consts::TAU + k.rnd() * 0.8;
            let d = reach * 0.6 + 18.0 + k.rnd() * 40.0;
            let c = Vec3::new(cx + a.cos() * d, all.min.y - 10.0 - k.rnd() * 22.0, cz + a.sin() * d);
            if !blocked(boxes, c, 4.5 * scale + 2.0, 7.0 * scale, 8.0) {
                pos = Some(c);
                break;
            }
        }
        let Some(home) = pos else { continue };
        let yaw = k.rnd() * 6.3;
        let base = Transform::from_translation(home)
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::splat(scale));
        let g = k.group(Some(root), base);
        k.island(g, Transform::default());
        let trees = if LEAFY.contains(&k.look.look.id) {
            1 + (k.rnd() * 3.0) as usize
        } else {
            0
        };
        for i in 0..trees {
            let name = flora[((k.rnd() * flora.len() as f32) as usize).min(flora.len() - 1)];
            let a = k.rnd() * 6.3;
            let r = if i == 0 && trees == 1 { 0.0 } else { 1.0 + k.rnd() * 1.6 };
            let f = k.model(name, g, [a.cos() * r, 0.45, a.sin() * r], 1.0, None);
            let (fy, fs) = (k.rnd() * 6.3, 0.55 + k.rnd() * 0.35);
            k.set_tf(f, |t| {
                t.rotation = Quat::from_rotation_y(fy);
                t.scale = Vec3::splat(fs);
            });
        }
        let ph = k.rnd() * 50.0;
        k.tick(move |t, tx| {
            let mut b = base;
            b.translation.y = home.y + (t * 0.25 + ph).sin() * 0.7;
            b.rotation = Quat::from_rotation_y(yaw + (t * 0.05 + ph).sin() * 0.15);
            tx.set(g, b);
        });
    }
}

fn clouds(k: &mut Kit, root: Entity, boxes: &[Aabb], req: &SceneryRequest) {
    let cloud = k.look.look.sky.cloud;
    let paint = if cloud.eq_ignore_ascii_case("#ffffff") {
        Vec::new()
    } else {
        vec![("Cloud", cloud.to_string(), 0.5)]
    };
    for _ in 0..req.clouds {
        for _ in 0..40 {
            let a = k.rnd() * core::f32::consts::TAU;
            let d = req.spread as f32 * (0.55 + k.rnd() * 0.9);
            let home = Vec3::new(
                req.cx as f32 + a.cos() * d,
                req.y_min as f32 + k.rnd() * (req.y_max - req.y_min) as f32,
                req.cz as f32 + a.sin() * d * 1.2,
            );
            let scale = 1.4 + k.rnd() * 2.8;
            if blocked(boxes, home, CLOUD_R * scale + DRIFT, CLOUD_H * scale, 10.0) {
                continue;
            }
            let (yaw, spin, ph, w) = (
                k.rnd() * 6.3,
                (k.rnd() - 0.5) * 0.04,
                k.rnd() * 100.0,
                0.05 + k.rnd() * 0.07,
            );
            let scene = k.assets.load(GltfAssetLabel::Scene(0).from_asset("models/cloud.glb"));
            let e = k
                .world
                .spawn((
                    Decor,
                    WorldAssetRoot(scene),
                    Prop::painted("cloud", paint.clone()),
                    Transform::from_translation(home).with_scale(Vec3::splat(scale)),
                    Visibility::default(),
                    NotShadowCaster,
                    ChildOf(root),
                ))
                .id();
            k.tick(move |t, tx| {
                let p = Vec3::new(
                    home.x + (t * w + ph).sin() * DRIFT,
                    home.y + (t * w * 2.3 + ph * 1.7).sin() * 0.6,
                    home.z + (t * w * 0.8 + ph).cos() * DRIFT,
                );
                let breathe = 1.0 + (t * 0.35 + ph).sin() * 0.035;
                tx.set(
                    e,
                    Transform {
                        translation: p,
                        rotation: Quat::from_rotation_y(yaw + t * spin),
                        scale: Vec3::new(scale * breathe, scale * (2.0 - breathe), scale * breathe),
                    },
                );
            });
            break;
        }
    }
}

/// A few small flocks circling well outside the course.
fn birds(k: &mut Kit, root: Entity, all: Aabb) {
    let (flocks, per) = (2, 5);
    let mat = k.plain_with("#4b3d7a", Kind::Fabric, |s| s.roughness = Some(0.7));
    let body = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(0.22).mesh().uv(10, 8).scaled_by(Vec3::new(0.8, 0.7, 1.6)));
    let wing = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Cuboid::new(0.9, 0.04, 0.34)).translated_by(Vec3::new(0.45, 0.0, 0.0)));
    let Mat::S(m) = mat else { return };
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let mut list = Vec::new();
    for f in 0..flocks {
        let r = reach + 25.0 + k.rnd() * 30.0;
        let y = all.max.y + 10.0 + k.rnd() * 14.0;
        let speed = (0.06 + k.rnd() * 0.05) * if f % 2 == 1 { -1.0 } else { 1.0 };
        let a0 = k.rnd() * 6.3;
        for i in 0..per {
            let back = i as f32 * 1.3;
            let side = (if i % 2 == 1 { 1.0 } else { -1.0 }) * (i as f32 / 2.0).ceil() * 1.1;
            let ph = k.rnd() * 6.0;
            let mut spawn = |mesh: &Handle<Mesh>| {
                k.world
                    .spawn((
                        Decor,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(m.clone()),
                        Transform::default(),
                        Visibility::default(),
                        NotShadowCaster,
                        ChildOf(root),
                    ))
                    .id()
            };
            let parts = [spawn(&body), spawn(&wing), spawn(&wing)];
            list.push((parts, r, y, speed, a0, back, side, ph));
        }
    }
    k.tick(move |t, tx| {
        for (parts, r, y, speed, a0, back, side, ph) in &list {
            let a = a0 + t * speed;
            let dir = speed.signum();
            let aa = a - back / r * dir;
            let rr = r + side;
            let p = Vec3::new(
                cx + aa.cos() * rr,
                y + (t * 0.7 + ph).sin() * 0.8 + back * 0.15,
                cz + aa.sin() * rr,
            );
            // Heading along the circle.
            let yaw = (-aa.sin() * dir).atan2(aa.cos() * dir);
            let m = Transform::from_translation(p).with_rotation(Quat::from_euler(
                EulerRot::XYZ,
                0.0,
                yaw,
                (t * 0.5 + ph).sin() * 0.15 - dir * 0.25,
            ));
            tx.set(parts[0], m);
            let flap = (t * 9.0 + ph).sin() * 0.7 + 0.1;
            tx.set(
                parts[1],
                m.mul_transform(Transform::from_rotation(Quat::from_rotation_z(flap))),
            );
            let right =
                Transform::from_rotation(Quat::from_rotation_y(core::f32::consts::PI) * Quat::from_rotation_z(flap));
            tx.set(parts[2], m.mul_transform(right));
        }
    });
}

/// Striped hot-air balloons far out, slowly rising, sinking and turning.
fn balloons(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let pals = [
        ("#ff8cc8", "#ffffff"),
        ("#ffd84a", "#ff9f4a"),
        ("#7ccfff", "#ffffff"),
        ("#a98bff", "#ffd84a"),
        ("#6fe08a", "#ffffff"),
    ];
    let cx = (all.min.x + all.max.x) / 2.0;
    let cz = (all.min.z + all.max.z) / 2.0;
    let reach = (all.max.x - all.min.x).hypot(all.max.z - all.min.z) / 2.0;
    let basket = k.plain("#b07a4a", Kind::Wood);
    let rope = k.plain("#6b4f3a", Kind::Fabric);
    let count = 4;
    let envelope = k
        .world
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(3.2).mesh().uv(28, 18).scaled_by(Vec3::new(1.0, 1.15, 1.0)));
    let frustum = |top: f32, bottom: f32, h: f32, seg: u32| {
        ConicalFrustum {
            radius_top: top,
            radius_bottom: bottom,
            height: h,
        }
        .mesh()
        .resolution(seg)
        .build()
    };
    let sk = k.world.resource_mut::<Assets<Mesh>>().add(frustum(1.1, 0.7, 1.2, 18));
    let bk = k.world.resource_mut::<Assets<Mesh>>().add(frustum(0.75, 0.6, 0.8, 12));
    for n in 0..count {
        let mut pos = None;
        for _ in 0..20 {
            let a = n as f32 / count as f32 * core::f32::consts::TAU + k.rnd() * 1.2;
            let d = reach + 45.0 + k.rnd() * 50.0;
            let c = Vec3::new(cx + a.cos() * d, all.max.y + 4.0 + k.rnd() * 22.0, cz + a.sin() * d);
            if !blocked(boxes, c, 6.0, 6.0, 10.0) {
                pos = Some(c);
                break;
            }
        }
        let Some(home) = pos else { continue };
        let (c1, c2) = pals[((k.rnd() * pals.len() as f32) as usize).min(pals.len() - 1)];
        let g = k.group(Some(root), Transform::from_translation(home));
        let env = k.pattern(c1, c2, 0.9, [1.0, 0.0], Kind::Fabric, Pattern::Stripes);
        let skirt = k.plain(c2, Kind::Fabric);
        if let Mat::S(h) = &env {
            k.world.spawn((
                Decor,
                Mesh3d(envelope.clone()),
                MeshMaterial3d(h.clone()),
                Transform::from_xyz(0.0, 5.2, 0.0),
                Visibility::default(),
                NotShadowCaster,
                ChildOf(g),
            ));
        }
        for (mesh, mat, y) in [(sk.clone(), &skirt, 1.9), (bk.clone(), &basket, 0.0)] {
            if let Mat::S(h) = mat {
                k.world.spawn((
                    Decor,
                    Mesh3d(mesh),
                    MeshMaterial3d(h.clone()),
                    Transform::from_xyz(0.0, y, 0.0),
                    Visibility::default(),
                    NotShadowCaster,
                    ChildOf(g),
                ));
            }
        }
        for i in 0..4 {
            let a = i as f32 / 4.0 * core::f32::consts::TAU + core::f32::consts::FRAC_PI_4;
            k.part(
                g,
                Shape::Cyl,
                &rope,
                [a.cos() * 0.65, 0.95, a.sin() * 0.65],
                [0.03, 1.4, 0.03],
                NO_ROT,
            );
        }
        let ph = k.rnd() * 50.0;
        let spin = (k.rnd() - 0.5) * 0.08;
        k.tick(move |t, tx| {
            tx.set(
                g,
                Transform::from_xyz(
                    home.x + (t * 0.03 + ph).sin() * 4.0,
                    home.y + (t * 0.11 + ph).sin() * 2.5,
                    home.z + (t * 0.025 + ph).cos() * 4.0,
                )
                .with_rotation(Quat::from_euler(
                    EulerRot::XYZ,
                    (t * 0.4 + ph).sin() * 0.03,
                    t * spin,
                    (t * 0.33 + ph).cos() * 0.03,
                )),
            )
        });
    }
}

/// The land far below the course, in the look's colours.
fn ground(k: &mut Kit, root: Entity, all: Aabb) {
    let Some(gr) = k.look.look.ground else { return };
    let kind = if gr.glow { Kind::Glossy } else { Kind::Padded };
    let mut spec = Spec {
        paint: Some(Paint {
            c1: hex(gr.c1).to_linear(),
            c2: hex(gr.c2).to_linear(),
            freq: gr.freq as f32,
            dir: Vec2::new(1.0, 0.6),
            speed: gr.speed as f32,
            kind: gr.kind,
        }),
        ..Spec::plain(LinearRgba::WHITE, Some(kind))
    };
    if gr.glow {
        spec.emissive = hex(gr.c1).to_linear() * 0.8;
    }
    let Mat::S(m) = k.surface(spec) else { return };
    let disc = k.world.resource_mut::<Assets<Mesh>>().add(
        Circle::new(900.0)
            .mesh()
            .resolution(72)
            .build()
            .rotated_by(Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2)),
    );
    k.world.spawn((
        Decor,
        Mesh3d(disc),
        MeshMaterial3d(m),
        Transform::from_xyz(
            (all.min.x + all.max.x) / 2.0,
            all.min.y - 70.0,
            (all.min.z + all.max.z) / 2.0,
        ),
        Visibility::default(),
        NotShadowCaster,
        NotShadowReceiver,
        ChildOf(root),
    ));
}

/// Places the look's pieces round the course.
fn decorate(k: &mut Kit, root: Entity, boxes: &[Aabb], all: Aabb) {
    let set = set_of(k.look.look.id);
    if set.is_empty() {
        return;
    }
    let total: f32 = set.iter().map(|p| p.weight).sum();
    let mut placed: Vec<(Vec3, f32)> = Vec::new();
    let w = all.max.x - all.min.x;
    let d = all.max.z - all.min.z;
    // More for bigger maps (long races), within a budget.
    let count = (12.0 + (w + d) / 14.0).min(26.0).round() as usize;
    for _ in 0..count {
        let mut x = k.rnd() * total;
        let piece = set
            .iter()
            .find(|p| {
                x -= p.weight;
                x <= 0.0
            })
            .unwrap_or(&set[0]);
        let s = 0.8 + k.rnd() * 0.6;
        let r = piece.r * s + if piece.island { 1.0 } else { 0.0 };
        for _ in 0..30 {
            let p = Vec3::new(
                all.min.x - 40.0 + k.rnd() * (w + 80.0),
                if piece.island {
                    all.min.y - 24.0 + k.rnd() * (all.max.y - all.min.y + 20.0)
                } else {
                    all.min.y - 16.0 + k.rnd() * (all.max.y - all.min.y + 26.0)
                },
                all.min.z - 40.0 + k.rnd() * (d + 80.0),
            );
            // (An island's rock reaches 7 m below it.)
            let (mid, half) = if piece.island {
                (p.y + (piece.h * s - 7.0) / 2.0, (piece.h * s + 7.0) / 2.0)
            } else {
                (p.y + piece.h * s / 2.0, piece.h * s / 2.0)
            };
            if blocked(boxes, Vec3::new(p.x, mid, p.z), r + 2.0, half, 8.0) {
                continue;
            }
            if placed
                .iter()
                .any(|(q, qr)| (q.x - p.x).hypot(q.z - p.z) < qr + r + 2.0 && (q.y - p.y).abs() < 12.0)
            {
                continue;
            }
            placed.push((p, r));
            let at = k.group(Some(root), Transform::from_translation(p));
            let obj = k.group(Some(at), Transform::default());
            (piece.make)(k, obj);
            if piece.island {
                k.island(at, Transform::from_scale(Vec3::splat(s * 0.85)));
            }
            let spin = if piece.island { 0.0 } else { k.rnd() * 6.3 };
            let lift = if piece.island { 0.4 * s * 0.85 } else { 0.0 };
            k.set_tf(obj, |t| {
                t.translation.y += lift;
                t.scale *= s;
                t.rotation = Quat::from_rotation_y(spin) * t.rotation;
            });
            break;
        }
    }
}

// ---------------------------------------------------------------- merging

/// Times the ticks are tried at to see what they move (s).
const TRIES: [f32; 6] = [0.0, 0.37, 1.9, 5.3, 13.7, 41.1];

/// The scenery's entities its ticks move, turn, show or hide: each tick is tried at a few times, and what
/// they changed is put back.
fn ticked(world: &mut World, ticks: &[Tick]) -> HashSet<Entity> {
    let mut read = world.query_filtered::<(Entity, &Transform, &Visibility), With<Decor>>();
    let before: HashMap<Entity, (Transform, Visibility)> = read.iter(world).map(|(e, t, v)| (e, (*t, *v))).collect();
    let mut write = world.query_filtered::<(&'static mut Transform, &'static mut Visibility), With<Decor>>();
    let mut moved = HashSet::new();
    for t in TRIES {
        {
            let mut q = write.query_mut(world);
            let mut tx = Tx { q: &mut q };
            for f in ticks {
                f(t, &mut tx);
            }
        }
        for (e, tf, v) in read.iter(world) {
            if before.get(&e).is_some_and(|(t0, v0)| t0 != tf || v0 != v) {
                moved.insert(e);
            }
        }
    }
    for e in &moved {
        if let (Some((t, v)), Ok(mut e)) = (before.get(e), world.get_entity_mut(*e)) {
            e.insert((*t, *v));
        }
    }
    moved
}

/// The parts nothing moves are drawn merged, one mesh per cell and material (`meshes::merge`): the set pieces
/// are hundreds of small parts.
fn merge_still(world: &mut World, ticks: &[Tick], parts: &[(Entity, Shape, Mat)], base: Entity) {
    use super::meshes;
    let moving = ticked(world, ticks);
    // World matrices by entity (None: it, or something above it, moves or is hidden).
    let mut placed: HashMap<Entity, Option<Mat4>> = HashMap::new();
    fn place(
        world: &World,
        e: Entity,
        moving: &HashSet<Entity>,
        placed: &mut HashMap<Entity, Option<Mat4>>,
    ) -> Option<Mat4> {
        if let Some(m) = placed.get(&e) {
            return *m;
        }
        let local = world.get::<Transform>(e).map(Transform::to_matrix);
        let m = if moving.contains(&e) || world.get::<Visibility>(e) == Some(&Visibility::Hidden) {
            None
        } else {
            match world.get::<ChildOf>(e) {
                Some(up) => place(world, up.parent(), moving, placed).zip(local).map(|(p, l)| p * l),
                None => local,
            }
        };
        placed.insert(e, m);
        m
    }
    let mut materials: HashMap<UntypedAssetId, usize> = HashMap::new();
    let mut candidates = Vec::with_capacity(parts.len());
    let mut worlds = Vec::with_capacity(parts.len());
    for (e, _, mat) in parts {
        let m = place(world, *e, &moving, &mut placed);
        // (A part something hangs from goes with what hangs from it: left alone.)
        let leaf = world.get::<Children>(*e).is_none_or(|c| c.is_empty());
        // (See-through ones are sorted one by one.)
        let (id, opaque) = match mat {
            Mat::S(h) => (
                h.id().untyped(),
                world
                    .resource::<Assets<SurfaceMaterial>>()
                    .get(h)
                    .is_some_and(|m| m.base.alpha_mode == AlphaMode::Opaque),
            ),
            Mat::G(h) => (
                h.id().untyped(),
                world
                    .resource::<Assets<StandardMaterial>>()
                    .get(h)
                    .is_some_and(|m| m.alpha_mode == AlphaMode::Opaque),
            ),
        };
        let n = materials.len();
        candidates.push(meshes::Candidate {
            at: m.map_or(Vec3::ZERO, |m| m.w_axis.truncate()),
            material: *materials.entry(id).or_insert(n),
            class: u64::from(matches!(mat, Mat::G(_))),
            still: leaf && opaque && m.is_some_and(|m| meshes::frame(&m).is_some()),
        });
        worlds.push(m.unwrap_or(Mat4::IDENTITY));
    }
    let mut cpu: HashMap<Shape, Mesh> = HashMap::new();
    let mut gone = 0;
    let groups = meshes::groups(&candidates);
    for group in &groups {
        let members: Vec<(Entity, Shape, &Mat, Mat4)> = group
            .iter()
            .filter_map(|&i| Some((parts.get(i)?, *worlds.get(i)?)))
            .map(|((e, s, mat), m)| (*e, *s, mat, m))
            .collect();
        let Some(&(_, _, mat, _)) = members.first() else {
            continue;
        };
        let (lo, hi) = members
            .iter()
            .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), p| {
                let at = p.3.w_axis.truncate();
                (lo.min(at), hi.max(at))
            });
        let origin = (lo + hi) / 2.0;
        for p in &members {
            cpu.entry(p.1).or_insert_with(|| shape_mesh(p.1));
        }
        let pieces: Vec<(&Mesh, Mat4)> = members.iter().filter_map(|p| cpu.get(&p.1).map(|m| (m, p.3))).collect();
        let Some(mesh) = meshes::merge(&pieces, origin, matches!(mat, Mat::S(_))) else {
            continue;
        };
        let aabb = mesh.compute_aabb();
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        {
            let mut e = world.spawn((
                Mesh3d(mesh),
                Transform::from_translation(origin),
                Visibility::default(),
                NotShadowCaster,
                ChildOf(base),
            ));
            match mat {
                Mat::S(h) => e.insert(MeshMaterial3d(h.clone())),
                Mat::G(h) => e.insert((MeshMaterial3d(h.clone()), NotShadowReceiver)),
            };
            if let Some(aabb) = aabb {
                e.insert(aabb);
            }
        }
        for p in &members {
            world.despawn(p.0);
        }
        gone += members.len();
    }
    debug!(
        "scenery: {gone} of {} parts merged into {} meshes",
        parts.len(),
        groups.len()
    );
}

// ---------------------------------------------------------------- systems

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
        .filter(|c| !c.trigger)
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
    merge_still(world, &list, &parts, base);
    let mut ticks = world.resource_mut::<Ticks>();
    ticks.generation = Some(generation);
    ticks.list = list;
}

fn animate(
    ticks: Res<Ticks>,
    map: Option<Res<Map>>,
    timeline: Res<lightyear::prelude::LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut q: Query<(&'static mut Transform, &'static mut Visibility), With<Decor>>,
) {
    let Some(map) = map else { return };
    if ticks.generation != Some(map.generation) {
        return;
    }
    let t = map.time(frame_tick(&timeline, &fixed)) as f32;
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
}
