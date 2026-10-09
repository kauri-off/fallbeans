//! World-space ambient occlusion, soft and a few metres short, in place of SSAO (whole-target only: `quality.rs`).
//! Baked into static vertices (UV_0.y) off the main thread; moving beans and parts add capsule shade (`gather`).
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::math::Affine3A;
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};
use fb_sim::scene::Model;
use fb_sim::scene::PrimKind;

use super::meshes::{self, edge_radius};
use super::quality::{Preset, Quality};
use super::surface::{SurfaceMaterial, Surfaces};
use crate::beans::{BeanView, Rig};
use crate::view::MainCamera;

pub struct AoPlugin;

impl Plugin for AoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BakedAo>();
        app.init_resource::<Movers>();
        app.init_resource::<ModelParts>();
        app.add_systems(Update, finish_bakes);
        app.add_systems(PostUpdate, (tag_grounded, gather.after(TransformSystems::Propagate)));
    }
}

// ---------------------------------------------------------------- the solids of the map

/// How far the baked occlusion reaches (m): further off, nothing hides the sky.
pub const REACH: f32 = 2.5;
/// Samples along each direction, ever further out (`REACH` · (i / STEPS)²), and their weights: the near ones
/// count more.
const STEPS: usize = 4;
const STEP_WEIGHT: [f32; STEPS] = [1.0, 0.8, 0.6, 0.4];
/// How strongly the samples darken (where a wall meets the floor comes out at about a half), and the most any
/// vertex is darkened by: soft and toy-like, never black.
const GAIN: f32 = 0.65;
const MOST: f32 = 0.6;
/// A vertex this deep in another solid is hidden in it: that solid does not count for it (no dark bleeding from
/// where pieces overlap, along the triangles that reach out of it).
const EMBED: f32 = 0.03;
/// Side of the cells the solids are looked up by (m).
const GRID: f32 = 4.0;
/// A solid that would fill more cells than this (a ground hundreds of metres wide) is left out.
const MOST_CELLS: i64 = 200_000;
/// How much of the sky a model's part can hide (foliage, posts and caps are no solid walls), and how round its
/// box is taken, of its thinnest half size.
const MODEL_STRENGTH: f32 = 0.75;
const MODEL_ROUND: f32 = 0.6;

/// A shape in its own frame, at world size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Half size, and the radius of its rounded edges.
    Box(Vec3, f32),
    /// Radius, half height and the radius of its rounded rims; round the y axis.
    Cyl(f32, f32, f32),
    Ball(f32),
}

impl Shape {
    /// The signed distance from a point in its frame to its surface.
    fn distance(&self, p: Vec3) -> f32 {
        match *self {
            Shape::Box(half, r) => {
                let q = p.abs() - (half - Vec3::splat(r));
                q.max(Vec3::ZERO).length() + q.max_element().min(0.0) - r
            }
            Shape::Cyl(radius, half_h, r) => {
                let d = Vec2::new(Vec2::new(p.x, p.z).length() - (radius - r), p.y.abs() - (half_h - r));
                d.max(Vec2::ZERO).length() + d.max_element().min(0.0) - r
            }
            Shape::Ball(radius) => p.length() - radius,
        }
    }

    /// Half the size of its bounding box.
    fn half(&self) -> Vec3 {
        match *self {
            Shape::Box(half, _) => half,
            Shape::Cyl(r, h, _) => Vec3::new(r, h, r),
            Shape::Ball(r) => Vec3::splat(r),
        }
    }

    /// Stretched by a placement's scale (a cylinder's radius by the wider of x and z, a ball's by the widest).
    fn scaled(self, s: Vec3) -> Shape {
        match self {
            Shape::Box(half, r) => {
                let half = half * s;
                Shape::Box(half, (r * s.min_element()).min(half.min_element()).max(0.0))
            }
            Shape::Cyl(radius, h, r) => {
                let (radius, h) = (radius * s.x.max(s.z), h * s.y);
                Shape::Cyl(radius, h, (r * s.min_element()).min(radius.min(h)).max(0.0))
            }
            Shape::Ball(radius) => Shape::Ball(radius * s.max_element()),
        }
    }

    fn bits(&self) -> [u32; 5] {
        match *self {
            Shape::Box(h, r) => [0, h.x.to_bits(), h.y.to_bits(), h.z.to_bits(), r.to_bits()],
            Shape::Cyl(a, b, r) => [1, a.to_bits(), b.to_bits(), r.to_bits(), 0],
            Shape::Ball(a) => [2, a.to_bits(), 0, 0, 0],
        }
    }
}

/// A map primitive's shape (as `meshes::prim` rounds it), in its own frame.
pub fn prim_shape(kind: PrimKind, dims: [f64; 3]) -> Shape {
    let [a, b, c] = dims.map(|v| v as f32);
    match kind {
        PrimKind::Box => Shape::Box(Vec3::new(a, b, c) / 2.0, edge_radius(a.min(b).min(c))),
        PrimKind::Cyl => Shape::Cyl(a, b / 2.0, edge_radius(b.min(2.0 * a))),
        PrimKind::Sphere => Shape::Ball(a),
    }
}

/// A model's part (a box of half size `half`, `model_parts`) as a blob: rounded well into its box.
pub fn part_shape(half: Vec3) -> Shape {
    Shape::Box(half, MODEL_ROUND * half.min_element())
}

/// A shape placed in the world that hides the sky from what is near it.
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    /// From the world into its frame (no scale: the shape is at world size).
    to_local: Affine3A,
    shape: Shape,
    /// How much of the sky it can hide (1: all of what it covers).
    strength: f32,
    /// Its world bounds.
    lo: Vec3,
    hi: Vec3,
}

impl Solid {
    /// `shape` placed by `world` (its scale stretches the shape); None if that is degenerate.
    pub fn new(shape: Shape, world: &Mat4, strength: f32) -> Option<Solid> {
        let (s, rot, at) = world.to_scale_rotation_translation();
        let s = s.abs();
        if !s.is_finite() || !at.is_finite() || !rot.is_finite() || s.cmple(Vec3::splat(1e-4)).any() {
            return None;
        }
        let shape = shape.scaled(s);
        let place = Affine3A::from_rotation_translation(rot.normalize(), at);
        let half = shape.half();
        let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for i in 0..8u32 {
            let side = |bit: u32, h: f32| if i & bit == 0 { -h } else { h };
            let p = place.transform_point3(Vec3::new(side(1, half.x), side(2, half.y), side(4, half.z)));
            lo = lo.min(p);
            hi = hi.max(p);
        }
        Some(Solid {
            to_local: place.inverse(),
            shape,
            strength,
            lo,
            hi,
        })
    }

    /// The signed distance from a world point to its surface.
    fn distance(&self, p: Vec3) -> f32 {
        self.shape.distance(self.to_local.transform_point3(p))
    }

    /// How far a world point is from its bounds (0 inside them): never more than the distance.
    fn bound_distance(&self, p: Vec3) -> f32 {
        (self.lo - p).max(p - self.hi).max(Vec3::ZERO).length()
    }
}

fn cell_of(p: Vec3) -> IVec3 {
    (p / GRID).floor().as_ivec3()
}

/// The solids of a map, looked up by cell, and a fingerprint of all of them (the bakes are kept by it).
pub struct Solids {
    list: Vec<Solid>,
    cells: HashMap<IVec3, Vec<u32>>,
    pub key: u64,
}

impl Solids {
    pub fn new(list: Vec<Solid>) -> Self {
        let mut cells: HashMap<IVec3, Vec<u32>> = HashMap::new();
        let mut h = DefaultHasher::new();
        list.len().hash(&mut h);
        let reach = Vec3::splat(2.0 * REACH);
        for (i, s) in list.iter().enumerate() {
            s.to_local.to_cols_array().map(f32::to_bits).hash(&mut h);
            s.shape.bits().hash(&mut h);
            s.strength.to_bits().hash(&mut h);
            // (In every cell it can shade a point of.)
            let (lo, hi) = (cell_of(s.lo - reach), cell_of(s.hi + reach));
            let d = (hi - lo + IVec3::ONE).max(IVec3::ZERO);
            if i64::from(d.x) * i64::from(d.y) * i64::from(d.z) > MOST_CELLS {
                continue;
            }
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        cells.entry(IVec3::new(x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Self {
            list,
            cells,
            key: h.finish(),
        }
    }

    /// The solids that may shade a point.
    fn near(&self, p: Vec3) -> &[u32] {
        self.cells.get(&cell_of(p)).map(Vec::as_slice).unwrap_or_default()
    }
}

/// Sky hidden from `p` (normal `n`) by the solids except `own`, 0 to `MOST`; coplanar tiles must not shade each other.
pub fn occlusion(solids: &Solids, own: Option<u32>, p: Vec3, n: Vec3, near: &mut Vec<u32>) -> f32 {
    near.clear();
    for &i in solids.near(p) {
        let Some(s) = solids.list.get(i as usize) else { continue };
        // (A sample at most `REACH` out is shaded only by what is closer to it than that.)
        if Some(i) == own || s.bound_distance(p) >= 2.0 * REACH {
            continue;
        }
        let d = s.distance(p);
        if d > -EMBED && d < 2.0 * REACH {
            near.push(i);
        }
    }
    if near.is_empty() {
        return 0.0;
    }
    let n = n.normalize_or(Vec3::Y);
    let (t, b) = n.any_orthonormal_pair();
    let all = [
        n,
        (n + t).normalize(),
        (n - t).normalize(),
        (n + b).normalize(),
        (n - b).normalize(),
    ];
    let (mut occ, mut total) = (0.0, 0.0);
    for (j, dir) in all.iter().enumerate() {
        let lean = dir.dot(n);
        let wd = if j == 0 { 2.0 } else { 1.0 };
        for (i, w) in STEP_WEIGHT.iter().enumerate() {
            let f = (i + 1) as f32 / STEPS as f32;
            let h = REACH * f * f;
            let s = p + *dir * h;
            let free = h * lean;
            let mut most = 0.0f32;
            for solid in near.iter().filter_map(|&k| solids.list.get(k as usize)) {
                let d = solid.distance(s);
                most = most.max(solid.strength * ((free - d) / free).clamp(0.0, 1.0));
            }
            occ += wd * w * most;
            total += wd * w;
        }
    }
    (occ / total * GAIN).min(MOST)
}

/// The parts of a model as boxes in its frame (each mesh's bounds under its node: the box's frame and half size),
/// from its glTF.
fn model_parts(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Vec<(Mat4, Vec3)> {
    let kids: HashSet<AssetId<GltfNode>> = gltf
        .nodes
        .iter()
        .filter_map(|h| nodes.get(h))
        .flat_map(|n| n.children.iter().map(Handle::id))
        .collect();
    let mut stack: Vec<(AssetId<GltfNode>, Mat4)> = gltf
        .nodes
        .iter()
        .map(Handle::id)
        .filter(|id| !kids.contains(id))
        .map(|id| (id, Mat4::IDENTITY))
        .collect();
    let mut out = Vec::new();
    // (A node is visited once per path to it; glTF nodes have one parent at most.)
    while let Some((id, up)) = stack.pop() {
        let Some(node) = nodes.get(id) else { continue };
        let m = up * node.transform.to_matrix();
        if let Some(mesh) = node.mesh.as_ref().and_then(|h| gltf_meshes.get(h)) {
            for p in &mesh.primitives {
                if let Some(aabb) = meshes.get(&p.mesh).and_then(MeshAabb::compute_aabb) {
                    out.push((m * Mat4::from_translation(aabb.center.into()), aabb.half_extents.into()));
                }
            }
        }
        stack.extend(node.children.iter().map(|c| (c.id(), m)));
        if out.len() > 64 {
            break;
        }
    }
    out
}

/// The models' parts by name (`model_parts`), made once each is loaded.
#[derive(Resource, Default)]
pub struct ModelParts(HashMap<Model, Vec<(Mat4, Vec3)>>);

/// The solids of a placed model's parts (`world`: its node's), the thin ones left out.
pub fn model_solids(parts: &[(Mat4, Vec3)], world: &Mat4, out: &mut Vec<Solid>) {
    for (m, half) in parts {
        if half.min_element() < 0.04 || half.max_element() < 0.15 {
            continue;
        }
        if let Some(s) = Solid::new(part_shape(*half), &(*world * *m), MODEL_STRENGTH) {
            out.push(s);
        }
    }
}

/// What `view.rs` builds the map's occlusion with.
#[derive(SystemParam)]
pub struct AoKit<'w> {
    pub baked: ResMut<'w, BakedAo>,
    pub movers: ResMut<'w, Movers>,
    parts: ResMut<'w, ModelParts>,
    gltfs: Res<'w, Assets<Gltf>>,
    nodes: Res<'w, Assets<GltfNode>>,
    gltf_meshes: Res<'w, Assets<GltfMesh>>,
}

impl AoKit<'_> {
    /// The parts of a model (`model_parts`); none while its glTF is not in (the warm-up loads them all first).
    pub fn model(&mut self, name: Model, assets: &AssetServer, meshes: &Assets<Mesh>) -> Vec<(Mat4, Vec3)> {
        if let Some(p) = self.parts.0.get(&name) {
            return p.clone();
        }
        let handle: Handle<Gltf> = assets.load(format!("models/{name}.glb"));
        let Some(gltf) = self.gltfs.get(&handle) else {
            return Vec::new();
        };
        let parts = model_parts(gltf, &self.nodes, &self.gltf_meshes, meshes);
        if !parts.is_empty() {
            self.parts.0.insert(name, parts.clone());
        }
        parts
    }
}

// ---------------------------------------------------------------- cutting faces finer

/// Most passes `refine` makes (each halves the edges still too long).
const PASSES: usize = 12;

/// How long the edges of a level of detail may be (m), by the band it starts at: the occlusion lives in the
/// vertices.
pub fn cut_length(band: usize) -> f32 {
    match band {
        0 | 1 => 0.8,
        2 | 3 => 1.6,
        _ => 3.2,
    }
}

fn mean<const N: usize>(a: [f32; N], b: [f32; N]) -> [f32; N] {
    core::array::from_fn(|i| (a[i] + b[i]) / 2.0)
}

fn extend<T: Copy>(v: &mut Vec<T>, splits: &[(u32, u32)], f: impl Fn(T, T) -> T) {
    for &(a, b) in splits {
        let x = f(v[a as usize], v[b as usize]);
        v.push(x);
    }
}

/// Splits triangles `idx` at edge midpoints until no edge exceeds `most`, appending to `pos`; stops at `limit` points.
fn split_edges(pos: &mut Vec<Vec3>, idx: &mut Vec<u32>, most: f32, limit: usize) -> Vec<(u32, u32)> {
    let most2 = most * most;
    let mut splits: Vec<(u32, u32)> = Vec::new();
    for _ in 0..PASSES {
        if pos.len() >= limit {
            break;
        }
        let mut mids: HashMap<(u32, u32), u32> = HashMap::new();
        let mut out = Vec::with_capacity(idx.len() * 2);
        for t in idx.as_chunks::<3>().0 {
            let (a, b, c) = (t[0], t[1], t[2]);
            let mut mid = |x: u32, y: u32| -> Option<u32> {
                let (px, py) = (pos[x as usize], pos[y as usize]);
                if px.distance_squared(py) <= most2 {
                    return None;
                }
                let key = (x.min(y), x.max(y));
                Some(*mids.entry(key).or_insert_with(|| {
                    pos.push((px + py) / 2.0);
                    splits.push(key);
                    (pos.len() - 1) as u32
                }))
            };
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            match (ab, bc, ca) {
                (None, None, None) => out.extend_from_slice(&[a, b, c]),
                (Some(ab), None, None) => out.extend_from_slice(&[a, ab, c, ab, b, c]),
                (None, Some(bc), None) => out.extend_from_slice(&[a, b, bc, a, bc, c]),
                (None, None, Some(ca)) => out.extend_from_slice(&[a, b, ca, ca, b, c]),
                (Some(ab), Some(bc), None) => out.extend_from_slice(&[ab, b, bc, a, ab, bc, a, bc, c]),
                (None, Some(bc), Some(ca)) => out.extend_from_slice(&[bc, c, ca, a, b, bc, a, bc, ca]),
                (Some(ab), None, Some(ca)) => out.extend_from_slice(&[a, ab, ca, ab, b, c, ab, c, ca]),
                (Some(ab), Some(bc), Some(ca)) => {
                    out.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
                }
            }
        }
        let done = mids.is_empty();
        *idx = out;
        if done {
            break;
        }
    }
    splits
}

/// Splits a mesh's triangles until no world edge exceeds `most` (m); false if it is not indexed float triangles.
pub fn refine(mesh: &mut Mesh, world: &Mat4, most: f32) -> bool {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return false;
    }
    let Ok(VertexAttributeValues::Float32x3(p)) = mesh.try_attribute(Mesh::ATTRIBUTE_POSITION) else {
        return false;
    };
    let count = p.len();
    let mut pos: Vec<Vec3> = p.iter().map(|v| world.transform_vector3(Vec3::from(*v))).collect();
    let floats = mesh.try_attributes().is_ok_and(|mut all| {
        all.all(|(_, v)| {
            v.len() == count
                && matches!(
                    v,
                    VertexAttributeValues::Float32(_)
                        | VertexAttributeValues::Float32x2(_)
                        | VertexAttributeValues::Float32x3(_)
                        | VertexAttributeValues::Float32x4(_)
                )
        })
    });
    let mut idx: Vec<u32> = match mesh.try_indices_option() {
        Ok(Some(ix)) if floats => ix.iter().map(|i| i as u32).collect(),
        _ => return false,
    };
    if idx.iter().any(|&i| i as usize >= count) {
        return false;
    }
    let splits = split_edges(&mut pos, &mut idx, most, count * 8 + 50_000);
    if splits.is_empty() {
        return true;
    }
    let Ok(all) = mesh.try_attributes_mut() else {
        return false;
    };
    for (attr, values) in all {
        let unit = attr.id == Mesh::ATTRIBUTE_NORMAL.id;
        match values {
            VertexAttributeValues::Float32(v) => extend(v, &splits, |a, b| (a + b) / 2.0),
            VertexAttributeValues::Float32x2(v) => extend(v, &splits, mean),
            VertexAttributeValues::Float32x3(v) => extend(v, &splits, |a, b| {
                let m = mean(a, b);
                if unit {
                    Vec3::from(m).normalize_or(Vec3::Y).to_array()
                } else {
                    m
                }
            }),
            VertexAttributeValues::Float32x4(v) => extend(v, &splits, mean),
            _ => {}
        }
    }
    mesh.insert_indices(Indices::U32(idx));
    true
}

// ---------------------------------------------------------------- the bakes

/// A piece of a merged mesh to bake: its mesh at the level, where it is placed (the lift in it), the solid it is
/// itself (it does not shade itself) and what its occlusion is kept by (`BakedAo`).
pub struct Piece {
    pub mesh: Arc<Mesh>,
    pub world: Mat4,
    pub own: Option<u32>,
    pub key: u64,
}

/// A merged mesh to bake again with its occlusion: its pieces in their order, the point its positions are taken
/// from (`meshes::merge`), and the band its level of detail starts at.
pub struct Bake {
    pub pieces: Vec<Piece>,
    pub origin: Vec3,
    pub band: usize,
}

/// What a piece's occlusion is kept by: the map's solids, the piece's mesh at its level, its placement.
pub fn piece_key(solids: &Solids, level: impl Hash, world: &Mat4, band: usize) -> u64 {
    let mut h = DefaultHasher::new();
    solids.key.hash(&mut h);
    level.hash(&mut h);
    world.to_cols_array().map(f32::to_bits).hash(&mut h);
    cut_length(band).to_bits().hash(&mut h);
    h.finish()
}

/// A merged mesh with its occlusion, and the occlusion of the pieces worked out for it (not kept yet).
type Baked = (Mesh, Vec<(u64, Arc<[u8]>)>);

/// The occlusion of each vertex of a piece placed by `world`, 0…255.
fn shade_piece(mesh: &Mesh, world: &Mat4, solids: &Solids, own: Option<u32>, near: &mut Vec<u32>) -> Option<Vec<u8>> {
    let (Ok(VertexAttributeValues::Float32x3(pos)), Ok(VertexAttributeValues::Float32x3(nrm))) = (
        mesh.try_attribute(Mesh::ATTRIBUTE_POSITION),
        mesh.try_attribute(Mesh::ATTRIBUTE_NORMAL),
    ) else {
        return None;
    };
    let normal = Mat3::from_mat4(*world).inverse().transpose();
    Some(
        pos.iter()
            .zip(nrm)
            .map(|(p, n)| {
                let p = world.transform_point3(Vec3::from(*p));
                let n = (normal * Vec3::from(*n)).normalize_or(Vec3::Y);
                (occlusion(solids, own, p, n, near) * 255.0).round() as u8
            })
            .collect(),
    )
}

/// A bake (off the main thread): each piece cut finer and shaded (or given what was kept of it), merged again,
/// the occlusion in UV_0.y.
fn run(bake: Bake, solids: &Solids, kept: Vec<Option<Arc<[u8]>>>) -> Option<Baked> {
    let most = cut_length(bake.band);
    let mut made: Vec<(Mesh, Mat4)> = Vec::with_capacity(bake.pieces.len());
    let mut values: Vec<u8> = Vec::new();
    let mut fresh = Vec::new();
    let mut near = Vec::new();
    for (piece, kept) in bake.pieces.iter().zip(kept) {
        let mut mesh = (*piece.mesh).clone();
        if !refine(&mut mesh, &piece.world, most) {
            return None;
        }
        let n = mesh.count_vertices();
        match kept.filter(|k| k.len() == n) {
            Some(k) => values.extend_from_slice(&k),
            None => {
                let shade = shade_piece(&mesh, &piece.world, solids, piece.own, &mut near)?;
                values.extend_from_slice(&shade);
                fresh.push((piece.key, Arc::from(shade)));
            }
        }
        made.push((mesh, piece.world));
    }
    let pieces: Vec<(&Mesh, Mat4)> = made.iter().map(|(m, w)| (m, *w)).collect();
    let mut merged = meshes::merge(&pieces, bake.origin, true)?;
    if let Ok(VertexAttributeValues::Float32x2(uv)) = merged.try_attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        for (uv, v) in uv.iter_mut().zip(&values) {
            uv[1] = f32::from(*v) / 255.0;
        }
    }
    Some((merged, fresh))
}

/// How many bytes of baked occlusion are kept at most (a map takes some hundreds of kilobytes).
const KEEP_BYTES: usize = 48 << 20;

/// The baked occlusion: what is kept of it by piece, and the bakes running.
#[derive(Resource, Default)]
pub struct BakedAo {
    /// The occlusion of each vertex of a piece cut finer, by `piece_key`.
    done: HashMap<u64, Arc<[u8]>>,
    bytes: usize,
    /// Each for the mesh it will replace (gone meanwhile: its map is).
    running: Vec<(AssetId<Mesh>, Task<Option<Baked>>)>,
}

impl BakedAo {
    /// Bakes still running (the warm-up waits for them at its end).
    pub fn baking(&self) -> usize {
        self.running.len()
    }

    /// Bakes a merged mesh again with its occlusion; once done it replaces `mesh`.
    pub fn start(&mut self, mesh: AssetId<Mesh>, bake: Bake, solids: Arc<Solids>) {
        let kept: Vec<Option<Arc<[u8]>>> = bake.pieces.iter().map(|p| self.done.get(&p.key).cloned()).collect();
        let pool = AsyncComputeTaskPool::get();
        let task = pool.spawn(async move { run(bake, &solids, kept) });
        self.running.push((mesh, task));
    }

    fn keep(&mut self, key: u64, values: Arc<[u8]>) {
        if self.bytes + values.len() > KEEP_BYTES {
            self.done.clear();
            self.bytes = 0;
        }
        self.bytes += values.len();
        if let Some(old) = self.done.insert(key, values) {
            self.bytes -= old.len();
        }
    }
}

/// Finished bakes replace their meshes (the same layout: no pipeline changes) and are kept.
fn finish_bakes(mut ao: ResMut<BakedAo>, mut meshes: ResMut<Assets<Mesh>>) {
    let ao = &mut *ao;
    let mut i = 0;
    while i < ao.running.len() {
        if !ao.running[i].1.is_finished() {
            i += 1;
            continue;
        }
        let (id, task) = ao.running.swap_remove(i);
        let Some((mesh, fresh)) = block_on(task) else {
            continue;
        };
        for (key, values) in fresh {
            ao.keep(key, values);
        }
        if let Some(mut m) = meshes.get_mut(id) {
            *m = mesh;
        }
    }
}

// ---------------------------------------------------------------- what moves

/// A capsule (a ball where its ends meet): its ends and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: Vec3,
    pub b: Vec3,
    pub r: f32,
}

/// Thinner than this (m), a moving part shades nothing worth the texels.
const MIN_RADIUS: f32 = 0.12;

/// Capsules standing in for a box of half size `half` placed by `m` (in its holder's frame): one along a bar or
/// a squat box, two or three side by side across a plate; never thicker than the box, which would shade itself.
pub fn box_capsules(m: &Mat4, half: Vec3, out: &mut Vec<Capsule>) {
    let axes = [m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate()];
    let ext = [
        axes[0].length() * half.x,
        axes[1].length() * half.y,
        axes[2].length() * half.z,
    ];
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| ext[b].total_cmp(&ext[a]));
    let [long, mid, thin] = order;
    let r = ext[thin];
    if !r.is_finite() || r < MIN_RADIUS {
        return;
    }
    let along = axes[long].normalize_or_zero() * (ext[long] - r).max(0.0);
    let across = axes[mid].normalize_or_zero() * (ext[mid] - r).max(0.0);
    let n = if ext[mid] < 1.6 * r {
        1
    } else if ext[mid] < 3.0 * r {
        2
    } else {
        3
    };
    let c = m.w_axis.truncate();
    for k in 0..n {
        let f = if n == 1 {
            0.0
        } else {
            2.0 * k as f32 / (n - 1) as f32 - 1.0
        };
        let o = c + across * f;
        out.push(Capsule {
            a: o - along,
            b: o + along,
            r,
        });
    }
}

/// Capsules standing in for a map primitive, in its own frame.
pub fn prim_capsules(kind: PrimKind, dims: [f64; 3], out: &mut Vec<Capsule>) {
    let [a, b, c] = dims.map(|v| v as f32);
    match kind {
        PrimKind::Box => box_capsules(&Mat4::IDENTITY, Vec3::new(a, b, c) / 2.0, out),
        // A post: along its axis. A disc: as a plate a little inside it.
        PrimKind::Cyl if b / 2.0 >= a => {
            if a >= MIN_RADIUS {
                let h = Vec3::Y * (b / 2.0 - a);
                out.push(Capsule { a: -h, b: h, r: a });
            }
        }
        PrimKind::Cyl => box_capsules(&Mat4::IDENTITY, Vec3::new(a * 0.85, b / 2.0, a * 0.85), out),
        PrimKind::Sphere => {
            if a >= MIN_RADIUS {
                out.push(Capsule {
                    a: Vec3::ZERO,
                    b: Vec3::ZERO,
                    r: a,
                });
            }
        }
    }
}

/// Capsules standing in for a model's parts (`model_parts`), in its frame: four at most.
pub fn model_capsules(parts: &[(Mat4, Vec3)], out: &mut Vec<Capsule>) {
    let start = out.len();
    for (m, half) in parts {
        box_capsules(m, *half, out);
    }
    out.truncate(start + 4);
}

/// The map's moving parts that shade what is near them: each entity (it follows its node) with a capsule in its
/// frame. (The beans are found each frame.)
#[derive(Resource, Default)]
pub struct Movers {
    list: Vec<(Entity, Capsule)>,
}

impl Movers {
    /// Those of the map just drawn (the ones of a map gone are gone with their entities).
    pub fn set(&mut self, list: Vec<(Entity, Capsule)>) {
        self.list = list;
    }
}

/// Texels of the occluder texture: the anchor and the count, then two per capsule (`occluder` in surface.wgsl).
const TEXELS: u32 = 128;
const MOST_OCCLUDERS: usize = (TEXELS as usize - 1) / 2;
/// How many go to the shader (the nearest to the camera) on Low and on High, and how far off they may be (m).
const OCCLUDERS_LOW: usize = 16;
const OCCLUDERS_HIGH: usize = 48;
const OCCLUDER_FAR: f32 = 60.0;
/// How far a capsule's shade reaches, in its radii (`OCCLUDER_REACH` in surface.wgsl).
const OCCLUDER_REACH: f32 = 3.5;
/// The grid the anchor snaps to (m): its multiples are exact in half floats out to 8 km, and the capsules are
/// kept near it (precision of 1/64 m within 16 m of it, 1/32 within 64).
const ANCHOR: f32 = 4.0;

/// The texture the moving occluders go to the shader in (half floats: the formats it can read without a filter
/// are not all in a bindless table's).
pub fn occluder_image() -> Image {
    Image::new_fill(
        Extent3d {
            width: TEXELS,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0; 8],
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A half float, rounded to the nearest (beyond its range: its largest).
fn half(x: f32) -> u16 {
    let sign = ((x.to_bits() >> 16) & 0x8000) as u16;
    let a = x.abs();
    if a.is_nan() || a >= 65504.0 {
        return sign | 0x7bff;
    }
    let b = a.to_bits();
    // (Under 2⁻¹⁴: subnormal, in steps of 2⁻²⁴.)
    if b < 0x3880_0000 {
        return sign | (a * 16_777_216.0).round() as u16;
    }
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    let mut h = ((exp as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h as u16
}

fn texel(out: &mut Vec<u8>, v: Vec4) {
    for c in v.to_array() {
        out.extend_from_slice(&half(c).to_le_bytes());
    }
}

/// The point of a segment nearest to `p`.
fn nearest(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-8)).clamp(0.0, 1.0);
    a + ab * t
}

/// The scale a placement gives a radius (the widest of its axes).
fn radius_scale(gt: &GlobalTransform) -> f32 {
    let m = gt.affine().matrix3;
    m.x_axis.length().max(m.y_axis.length()).max(m.z_axis.length())
}

/// What may shade the map from up close: the beans, the moving parts, as placed, and the camera.
#[derive(SystemParam)]
struct Occluders<'w, 's> {
    movers: Res<'w, Movers>,
    camera: Query<'w, 's, &'static GlobalTransform, With<MainCamera>>,
    beans: Query<'w, 's, &'static Rig, With<BeanView>>,
    placed: Query<'w, 's, (&'static GlobalTransform, &'static InheritedVisibility)>,
}

/// Each frame: the beans and the moving parts nearest to the camera go to the occluder texture as capsules
/// (positions from an anchor near the camera, `ANCHOR`), if anything changed.
fn gather(
    quality: Option<Res<Quality>>,
    occ: Occluders,
    surfaces: Res<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut near: Local<Vec<(f32, Capsule)>>,
    mut sent: Local<Vec<u8>>,
) {
    let Some(texture) = surfaces.occluder_texture() else {
        return;
    };
    let eye = occ.camera.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    near.clear();
    let mut consider = |cap: Capsule| {
        let d = nearest(eye, cap.a, cap.b).distance(eye) - cap.r;
        if d < OCCLUDER_FAR && cap.r.is_finite() && cap.a.is_finite() && cap.b.is_finite() {
            near.push((d, cap));
        }
    };
    // A bean: its two spheres (`fb_sim::physics`), as its model is posed (tumbles, squash, a giant's size).
    let [low, high] = fb_sim::physics::SPHERES.map(|y| Vec3::new(0.0, y as f32, 0.0));
    for rig in &occ.beans {
        let Ok((gt, shown)) = occ.placed.get(rig.model) else {
            continue;
        };
        if shown.get() {
            consider(Capsule {
                a: gt.transform_point(low),
                b: gt.transform_point(high),
                r: fb_sim::physics::R as f32 * radius_scale(gt),
            });
        }
    }
    for (e, cap) in &occ.movers.list {
        let Ok((gt, shown)) = occ.placed.get(*e) else { continue };
        if shown.get() {
            consider(Capsule {
                a: gt.transform_point(cap.a),
                b: gt.transform_point(cap.b),
                r: cap.r * radius_scale(gt),
            });
        }
    }
    let most = match quality.map(|q| q.preset) {
        Some(Preset::Low) => OCCLUDERS_LOW,
        _ => OCCLUDERS_HIGH,
    }
    .min(MOST_OCCLUDERS);
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(most);
    let anchor = (eye / ANCHOR).round() * ANCHOR;
    let mut data = Vec::with_capacity(TEXELS as usize * 8);
    texel(&mut data, anchor.extend(near.len() as f32));
    // Each: its centre and how far it reaches from it (what is further is skipped at one texel's cost), then half
    // its axis and its radius.
    for (_, c) in near.iter() {
        let (centre, axis) = ((c.a + c.b) / 2.0, (c.b - c.a) / 2.0);
        texel(
            &mut data,
            (centre - anchor).extend(axis.length() + c.r * OCCLUDER_REACH),
        );
        texel(&mut data, axis.extend(c.r));
    }
    data.resize(TEXELS as usize * 8, 0);
    if *sent == data {
        return;
    }
    // (The same size: the texture is written over, the materials' bind groups stay good.)
    if let Some(mut image) = images.get_mut(texture) {
        image.data = Some(data.clone());
    }
    *sent = data;
}

// ---------------------------------------------------------------- the foot of what stands on the ground

/// A model standing on the ground at this height (m): its meshes get it in their tag, and the surface shader
/// shades their foot (`ground_sky` in surface.wgsl).
#[derive(Component, Clone, Copy, Debug)]
pub struct Grounded(pub f32);

/// The models that stand on the ground they are placed on (not those that float or hang).
pub const GROUNDED: [Model; 7] = [
    Model::Tree,
    Model::Pine,
    Model::Mushroom,
    Model::Cone,
    Model::Flag,
    Model::Bumper,
    Model::Finish,
];

/// The tag of a mesh standing on the ground at `y`: its bits, the lowest set (0 is no ground).
fn ground_tag(y: f32) -> u32 {
    y.to_bits() | 1
}

type Untagged = (Added<MeshMaterial3d<SurfaceMaterial>>, Without<MeshTag>);

/// Meshes on a surface that appear under a `Grounded` model (its own, its levels of detail) get its tag.
fn tag_grounded(
    mut commands: Commands,
    fresh: Query<(Entity, &ChildOf), Untagged>,
    parents: Query<&ChildOf>,
    grounded: Query<&Grounded>,
) {
    for (e, up) in &fresh {
        let mut at = Some(up.parent());
        for _ in 0..12 {
            let Some(a) = at else { break };
            if let Ok(g) = grounded.get(a) {
                commands.entity(e).try_insert(MeshTag(ground_tag(g.0)));
                break;
            }
            at = parents.get(a).ok().map(ChildOf::parent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box primitive of `size` at `at`.
    fn boxed(size: [f64; 3], at: Vec3) -> Solid {
        let s = Solid::new(prim_shape(PrimKind::Box, size), &Mat4::from_translation(at), 1.0);
        s.unwrap()
    }

    fn floor() -> Solid {
        // 20 × 1 × 20, its top at y = 0.
        boxed([20.0, 1.0, 20.0], Vec3::new(0.0, -0.5, 0.0))
    }

    fn wall() -> Solid {
        // 1 × 4 × 20 standing on the floor, its face at x = 0.
        boxed([1.0, 4.0, 20.0], Vec3::new(-0.5, 2.0, 0.0))
    }

    fn occ(solids: &Solids, own: Option<u32>, p: Vec3, n: Vec3) -> f32 {
        occlusion(solids, own, p, n, &mut Vec::new())
    }

    #[test]
    fn shapes_measure_their_distance() {
        let b = Shape::Box(Vec3::new(2.0, 1.0, 1.0), 0.0);
        assert!((b.distance(Vec3::new(3.0, 0.0, 0.0)) - 1.0).abs() < 1e-5);
        assert!((b.distance(Vec3::ZERO) + 1.0).abs() < 1e-5);
        // A rounded corner is further than the sharp one.
        let r = Shape::Box(Vec3::ONE, 0.5);
        assert!(r.distance(Vec3::splat(1.0)) > 0.1);
        assert!(r.distance(Vec3::new(1.0, 0.0, 0.0)).abs() < 1e-5);
        let c = Shape::Cyl(1.0, 2.0, 0.1);
        assert!((c.distance(Vec3::new(3.0, 0.0, 0.0)) - 2.0).abs() < 1e-5);
        assert!((c.distance(Vec3::new(0.0, 3.0, 0.0)) - 1.0).abs() < 1e-5);
        assert!((Shape::Ball(1.0).distance(Vec3::new(0.0, 0.0, 3.0)) - 2.0).abs() < 1e-5);
        // Placed: turned and moved, scaled.
        let m = Mat4::from_scale_rotation_translation(Vec3::splat(2.0), Quat::from_rotation_y(1.0), Vec3::X * 10.0);
        let s = Solid::new(Shape::Ball(1.0), &m, 1.0).unwrap();
        assert!((s.distance(Vec3::new(10.0, 5.0, 0.0)) - 3.0).abs() < 1e-4);
        assert!(s.bound_distance(Vec3::new(10.0, 5.0, 0.0)) <= 3.0 + 1e-4);
        let flat = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
        assert!(Solid::new(Shape::Ball(1.0), &flat, 1.0).is_none());
    }

    #[test]
    fn open_floors_are_open_and_corners_shaded() {
        let solids = Solids::new(vec![floor(), wall()]);
        // Out in the open, and the floor's own top: nothing.
        assert_eq!(occ(&solids, Some(0), Vec3::new(8.0, 0.0, 0.0), Vec3::Y), 0.0);
        // At the wall's foot: about a half, never more than the most.
        let corner = occ(&solids, Some(0), Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
        assert!(corner > 0.35 && corner <= MOST, "{corner}");
        // Fading away from it.
        let mut last = corner;
        for x in [0.3, 0.8, 1.5, 3.0] {
            let o = occ(&solids, Some(0), Vec3::new(x, 0.0, 0.0), Vec3::Y);
            assert!(o < last, "{x}: {o} after {last}");
            last = o;
        }
        assert!(last < 0.05);
        // The wall's face low down is shaded by the floor; high up, little.
        let low = occ(&solids, Some(1), Vec3::new(0.0, 0.1, 0.0), Vec3::X);
        let high = occ(&solids, Some(1), Vec3::new(0.0, 3.5, 0.0), Vec3::X);
        assert!(low > 0.2 && high < low / 3.0, "{low} {high}");
    }

    #[test]
    fn neighbours_in_one_plane_and_buried_points_do_not_shade() {
        // Two tiles side by side, tops level: no seam darkened.
        let tile = |x: f32| boxed([4.0, 0.5, 4.0], Vec3::new(x, -0.25, 0.0));
        let solids = Solids::new(vec![tile(-2.0), tile(2.0)]);
        let seam = occ(&solids, Some(0), Vec3::new(-0.5, 0.0, 0.0), Vec3::Y);
        assert!(seam < 0.02, "{seam}");
        // A point of the floor deep inside a block sunk into it is not darkened by it.
        let sunk = boxed([1.0, 4.0, 1.0], Vec3::new(-0.5, 1.5, 0.0));
        let buried = occ(
            &Solids::new(vec![floor(), sunk]),
            Some(0),
            Vec3::new(-0.5, 0.0, 0.0),
            Vec3::Y,
        );
        assert_eq!(buried, 0.0);
        let solids = Solids::new(vec![floor(), wall()]);
        // A weaker solid shades less.
        let soft = Solid {
            strength: 0.5,
            ..wall()
        };
        let weak = occ(
            &Solids::new(vec![floor(), soft]),
            Some(0),
            Vec3::new(0.3, 0.0, 0.0),
            Vec3::Y,
        );
        let full = occ(&solids, Some(0), Vec3::new(0.3, 0.0, 0.0), Vec3::Y);
        assert!(weak < full * 0.75, "{weak} {full}");
    }

    #[test]
    fn under_a_platform_is_shaded() {
        let deck = boxed([6.0, 0.5, 6.0], Vec3::new(0.0, 1.75, 0.0));
        let solids = Solids::new(vec![floor(), deck]);
        let under = occ(&solids, Some(0), Vec3::ZERO, Vec3::Y);
        assert!(under > 0.1, "{under}");
        // Its underside, facing the floor 1.5 m below.
        let above = occ(&solids, Some(1), Vec3::new(0.0, 1.5, 0.0), Vec3::NEG_Y);
        assert!(above > 0.1, "{above}");
        // The same solids, the same fingerprint; another, another.
        assert_eq!(Solids::new(vec![floor(), deck]).key, solids.key);
        assert_ne!(Solids::new(vec![floor()]).key, solids.key);
    }

    #[test]
    fn refining_cuts_long_edges_without_cracks() {
        // A tetrahedron with shared vertices, 6 m across.
        let corners = [Vec3::ZERO, Vec3::X * 6.0, Vec3::Z * 6.0, Vec3::Y * 6.0];
        let uv = vec![[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, corners.map(|c| c.to_array()).to_vec())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 4])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
            .with_inserted_indices(Indices::U32(vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3]));
        assert!(refine(&mut mesh, &Mat4::IDENTITY, 1.0));
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!()
        };
        let pos: Vec<Vec3> = pos.iter().map(|v| Vec3::from(*v)).collect();
        let idx: Vec<usize> = mesh.indices().unwrap().iter().collect();
        assert!(pos.len() > 20);
        assert_eq!(mesh.count_vertices(), pos.len());
        let mut edges: HashMap<(usize, usize), i32> = HashMap::new();
        for t in idx.as_chunks::<3>().0 {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                assert!(pos[a].distance(pos[b]) <= 1.0 + 1e-4, "{} → {}", pos[a], pos[b]);
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        // Closed: every edge once each way.
        for (&(a, b), &n) in &edges {
            assert_eq!(n, 1);
            assert_eq!(edges.get(&(b, a)), Some(&1), "a crack at {a} {b}");
        }
        // The UVs of a new vertex are its edge's ends' mean: here they follow x and z over 6 m.
        let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!()
        };
        for (p, uv) in pos.iter().zip(uv) {
            if p.y.abs() < 1e-5 {
                assert!((uv[0] - p.x / 6.0).abs() < 1e-4 && (uv[1] - p.z / 6.0).abs() < 1e-4);
            }
        }
        // Scaled up by its placement: cut finer.
        let mut small = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let n = small.count_vertices();
        assert!(refine(&mut small, &Mat4::IDENTITY, 2.0));
        assert_eq!(small.count_vertices(), n);
        assert!(refine(&mut small, &Mat4::from_scale(Vec3::splat(4.0)), 2.0));
        assert!(small.count_vertices() > n);
    }

    #[test]
    fn half_floats_round_and_keep_the_anchor_exact() {
        assert_eq!(half(0.0), 0);
        assert_eq!(half(1.0), 0x3c00);
        assert_eq!(half(0.5), 0x3800);
        assert_eq!(half(-2.0), 0xc000);
        assert_eq!(half(65504.0), 0x7bff);
        assert_eq!(half(1e9), 0x7bff);
        assert_eq!(half(1.0 + 1.0 / 2048.0), 0x3c00, "ties to even");
        assert_eq!(half(1.0 + 3.0 / 2048.0), 0x3c02);
        // Decoded: multiples of the anchor's grid out to 8 km are exact.
        let decode = |h: u16| {
            let (s, e, m) = (i32::from(h >> 15), i32::from((h >> 10) & 0x1f), f32::from(h & 0x3ff));
            let v = if e == 0 {
                m * 2f32.powi(-24)
            } else {
                (1.0 + m / 1024.0) * 2f32.powi(e - 15)
            };
            if s == 1 { -v } else { v }
        };
        for x in [-8188.0f32, -300.0, 4.0, 1024.0, 4096.0, 8188.0] {
            assert_eq!(decode(half(x)), x);
        }
        assert!((decode(half(13.37)) - 13.37).abs() < 1.0 / 128.0);
        assert!((decode(half(3e-6)) - 3e-6).abs() < 1e-7);
    }

    #[test]
    fn capsules_stand_in_for_bars_balls_and_plates() {
        let mut out = Vec::new();
        // A cube: one ball.
        box_capsules(&Mat4::IDENTITY, Vec3::splat(0.5), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].a, out[0].b, out[0].r), (Vec3::ZERO, Vec3::ZERO, 0.5));
        // A bar along x: one capsule, ends inside the bar.
        out.clear();
        box_capsules(&Mat4::IDENTITY, Vec3::new(4.0, 0.3, 0.3), &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].r - 0.3).abs() < 1e-5 && (out[0].b.x - 3.7).abs() < 1e-5 && out[0].a.x < 0.0);
        // A door (a plate) turned and moved: three, as thick as it, inside it.
        out.clear();
        let m = Mat4::from_rotation_translation(Quat::from_rotation_y(0.5), Vec3::new(1.0, 2.0, 3.0));
        box_capsules(&m, Vec3::new(1.5, 2.0, 0.2), &mut out);
        assert_eq!(out.len(), 3);
        let to_box = m.inverse();
        for c in &out {
            assert!((c.r - 0.2).abs() < 1e-5);
            for p in [c.a, c.b] {
                let q = to_box.transform_point3(p).abs();
                assert!(q.x <= 1.5 - 0.2 + 1e-4 && q.y <= 2.0 - 0.2 + 1e-4 && q.z < 1e-4, "{q}");
            }
        }
        // Too thin: nothing. A post; a disc; a ball.
        out.clear();
        box_capsules(&Mat4::IDENTITY, Vec3::new(2.0, 0.05, 2.0), &mut out);
        assert!(out.is_empty());
        prim_capsules(PrimKind::Cyl, [0.3, 4.0, 0.0], &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].b.y - 1.7).abs() < 1e-5);
        out.clear();
        prim_capsules(PrimKind::Cyl, [3.0, 0.5, 0.0], &mut out);
        assert_eq!(out.len(), 3);
        out.clear();
        prim_capsules(PrimKind::Sphere, [1.0, 0.0, 0.0], &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn ground_tags_are_never_zero() {
        assert_ne!(ground_tag(0.0), 0);
        assert!((f32::from_bits(ground_tag(2.5)) - 2.5).abs() < 1e-6);
        assert!((f32::from_bits(ground_tag(-7.0)) + 7.0).abs() < 1e-6);
    }
}
