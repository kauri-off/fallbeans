//! Meshes of map primitives: soft, toy-like boxes and cylinders (rounded edges and corners, a lip round broad
//! slabs; the cylinders' first segment faces +z), UV spheres; a few levels of detail with fewer segments,
//! switched by size on screen; the millimetre lifts that keep overlapping primitives from z-fighting; and static
//! geometry merged into one mesh per cell and material.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues, VertexFormat};
use bevy::prelude::*;
use core::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use fb_sim::scene::PrimKind;

/// Distance bands B0…B6 (by size on screen); a level of detail spans one or more of them.
pub const BANDS: usize = 7;
const SPHERE_SEG: [(u32, u32); BANDS] = [(32, 20), (32, 20), (20, 12), (20, 12), (10, 7), (10, 7), (10, 7)];
const CYL_SEG: [u32; BANDS] = [48, 48, 24, 24, 12, 12, 12];
/// Segments per rounded quarter (edges, corners, rims) by band, at most: 1 is a chamfer.
const ARCS: [u32; BANDS] = [4, 4, 2, 2, 1, 1, 1];
/// Cylinders of this many sides or fewer are polygons with rounded corners (hexagonal tiles, rungs).
const POLYGON: u32 = 8;
/// Height of one lift step (m).
pub const LIFT: f32 = 0.0025;
const LIFTS: u32 = 6;

/// A box with rounded edges and corners: `segments` per rounded edge.
pub fn rounded_box(size: Vec3, segments: u32, radius: f32) -> Mesh {
    if segments == 0 || radius <= 0.0 {
        return Cuboid::from_size(size).into();
    }
    let total = segments * 2 + 1;
    let radius = radius.min(size.x / 2.0).min(size.y / 2.0).min(size.z / 2.0);
    let inner = size / 2.0 - Vec3::splat(radius);
    let half_seg = 0.5 / total as f32;
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    // Each face of a unit box as a grid: its normal axis and the two axes it spans (right-handed).
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let base = pos.len() as u32;
        for j in 0..=total {
            for i in 0..=total {
                let a = i as f32 / total as f32 - 0.5;
                let b = j as f32 / total as f32 - 0.5;
                let p = n * 0.5 + u * a + v * b;
                let normal = (p - p.signum() * half_seg).normalize();
                pos.push((inner * p.signum() + normal * radius).to_array());
                nrm.push(normal.to_array());
            }
        }
        let row = total + 1;
        for j in 0..total {
            for i in 0..total {
                let a = base + j * row + i;
                let b = a + 1;
                let c = a + row;
                let d = c + 1;
                idx.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
    }
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

// ---------------------------------------------------------------- soft boxes and cylinders
//
// Both are an outline seen from above (a rounded rectangle, a rounded polygon, a circle) swept along a profile
// from the bottom up: each point of the profile is the outline moved in by its inset, at its height. Moving a
// rounded outline in by no more than its corners' radius keeps it a rounded outline with the same normals,
// so the normals of the sides are the outline's tilted by the profile's.

/// A point of a profile: how far in from the outline (m), its height, and its normal in the (out, up) plane.
#[derive(Clone, Copy, Debug)]
struct Step {
    inset: f32,
    y: f32,
    n: Vec2,
}

/// The sides of a soft piece of height `h`, centred on y = 0.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rim {
    h: f32,
    /// The radii of the rounded top and bottom edges.
    top: f32,
    bottom: f32,
    /// A lip round the top: how far it stands out over the body below (0: none), how far its underside is
    /// below the top, and the radius of its lower edge.
    lip: f32,
    band: f32,
    curl: f32,
}

/// How round the edges along a piece's thinnest side `a` are (m): soft on small pieces, nearly a half round on
/// thin plates, capped on big ones (the seams between the pieces of a floor stay narrow).
pub fn edge_radius(a: f32) -> f32 {
    (0.45 * a).min(0.1 + 0.15 * a).min(0.3)
}

/// The rim of a piece of height `h` with edges of radius `e`, its insets within `room` (what the outline's
/// corners allow); with a lip (its shaded underside a darker band under the top) round a broad slab, but not at
/// a chamfer's detail.
fn rim(h: f32, e: f32, room: f32, slab: bool, arcs: u32) -> Rim {
    let e = e.min(room);
    let plain = Rim {
        h,
        top: e,
        bottom: e,
        lip: 0.0,
        band: 0.0,
        curl: 0.0,
    };
    if !slab || arcs < 2 || h < 0.45 {
        return plain;
    }
    let lip = (0.03 + 0.25 * e).min(0.1);
    let bottom = (0.6 * e).min(room - lip);
    let band = (e + (0.15 * h).clamp(0.08, 0.25)).min(0.6 * h);
    let curl = (0.5 * lip).min(0.04).min(0.4 * (band - e));
    if bottom < 0.01 || curl < 0.005 {
        return plain;
    }
    Rim {
        h,
        top: e,
        bottom,
        lip,
        band,
        curl,
    }
}

/// A quarter round (or another arc) of a profile: centre (inset, y), radius, normal angles from and to.
fn arc(out: &mut Vec<Step>, centre: Vec2, r: f32, from: f32, to: f32, segs: u32) {
    for s in 0..=segs {
        let n = Vec2::from_angle(from + (to - from) * s as f32 / segs as f32);
        out.push(Step {
            inset: centre.x - r * n.x,
            y: centre.y + r * n.y,
            n,
        });
    }
}

/// A rim's profile from the bottom up, as runs of smooth steps; runs meet at creases (under the lip).
fn profile(rim: &Rim, arcs: u32) -> Vec<Vec<Step>> {
    let (lo, hi) = (-rim.h / 2.0, rim.h / 2.0);
    let mut runs = Vec::with_capacity(2);
    let mut run = Vec::new();
    arc(
        &mut run,
        Vec2::new(rim.lip + rim.bottom, lo + rim.bottom),
        rim.bottom,
        -FRAC_PI_2,
        0.0,
        arcs,
    );
    if rim.lip > 0.0 {
        // Up the body to under the lip; out along its underside and round its edge.
        let y = hi - rim.band;
        run.push(Step {
            inset: rim.lip,
            y,
            n: Vec2::X,
        });
        runs.push(core::mem::take(&mut run));
        run.push(Step {
            inset: rim.lip,
            y,
            n: Vec2::NEG_Y,
        });
        arc(
            &mut run,
            Vec2::new(rim.curl, y + rim.curl),
            rim.curl,
            -FRAC_PI_2,
            0.0,
            arcs.div_ceil(2),
        );
    }
    let top = rim.top;
    arc(&mut run, Vec2::new(top, hi - top), top, 0.0, FRAC_PI_2, arcs);
    runs.push(run);
    runs
}

/// The direction (x, z) at angle `a` round the y axis, from +z towards −x (as Bevy's cylinder turned so that its
/// first segment faces +z).
fn dir(a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(-s, c)
}

/// An outline: points (x, z) with their outward normals, round the y axis by that angle.
type Outline = Vec<(Vec2, Vec2)>;

/// A rectangle of half size `half` with corners of radius `r`.
fn rect_outline(half: Vec2, r: f32, segs: u32) -> Outline {
    let mut out = Vec::with_capacity(4 * (segs as usize + 1));
    for k in 0..4 {
        let a = k as f32 * FRAC_PI_2;
        let centre = (half - Vec2::splat(r)) * dir(a + FRAC_PI_4).signum();
        for s in 0..=segs {
            let n = dir(a + FRAC_PI_2 * s as f32 / segs as f32);
            out.push((centre + n * r, n));
        }
    }
    out
}

/// A regular polygon of `sides` with its corners on a circle of `radius`, the first at +z, rounded by `r`.
fn polygon_outline(sides: u32, radius: f32, r: f32, segs: u32) -> Outline {
    let half = PI / sides as f32;
    let reach = radius - r / half.cos();
    let mut out = Vec::with_capacity((sides * (segs + 1)) as usize);
    for k in 0..sides {
        let a = k as f32 * 2.0 * half;
        let centre = dir(a) * reach;
        for s in 0..=segs {
            let n = dir(a - half + 2.0 * half * s as f32 / segs as f32);
            out.push((centre + n * r, n));
        }
    }
    out
}

/// A circle of `seg` points, the first at +z, shaded smooth.
fn circle_outline(seg: u32, radius: f32) -> Outline {
    (0..seg)
        .map(|k| {
            let n = dir(k as f32 * TAU / seg as f32);
            (n * radius, n)
        })
        .collect()
}

/// An outline swept along a profile, closed by flat caps (fans round the centre) at its first and last steps.
fn sweep(outline: &[(Vec2, Vec2)], runs: &[Vec<Step>]) -> Mesh {
    let n = outline.len() as u32;
    let steps: usize = runs.iter().map(Vec::len).sum();
    let count = steps * outline.len() + 2;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut idx: Vec<u32> = Vec::with_capacity((steps + 1) * outline.len() * 6);
    for run in runs {
        let first = pos.len() as u32;
        for s in run {
            for &(p, d) in outline {
                let q = p - d * s.inset;
                pos.push([q.x, s.y, q.y]);
                nrm.push([d.x * s.n.x, s.n.y, d.y * s.n.x]);
            }
        }
        for j in 0..(run.len() as u32).saturating_sub(1) {
            let row = first + j * n;
            for i in 0..n {
                let (a, b) = (row + i, row + (i + 1) % n);
                let (c, d) = (a + n, b + n);
                idx.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
    }
    let lo = runs.first().and_then(|r| r.first()).map_or(0.0, |s| s.y);
    let hi = runs.last().and_then(|r| r.last()).map_or(0.0, |s| s.y);
    let top = (pos.len() as u32).saturating_sub(n);
    for (ring, y, up) in [(0, lo, false), (top, hi, true)] {
        let centre = pos.len() as u32;
        pos.push([0.0, y, 0.0]);
        nrm.push([0.0, if up { 1.0 } else { -1.0 }, 0.0]);
        for i in 0..n {
            let (a, b) = (ring + i, ring + (i + 1) % n);
            idx.extend_from_slice(&if up { [centre, b, a] } else { [centre, a, b] });
        }
    }
    let uv = vec![[0.0f32, 0.0]; pos.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// A soft box of `size`: edges rounded to the piece, vertical corners a little rounder, a lip round a broad slab;
/// `arcs` segments per rounded quarter. Its bounds are the box's.
pub fn soft_box(size: Vec3, arcs: u32) -> Mesh {
    if size.min_element() <= 1e-3 {
        return Cuboid::from_size(size).into();
    }
    let arcs = arcs.max(1);
    let foot = size.x.min(size.z);
    let e = edge_radius(size.min_element());
    let corner = (1.3 * e).min(0.45 * foot);
    let slab = size.y <= 0.75 * foot && foot >= 1.5;
    let sides = rim(size.y, e, corner / 1.3, slab, arcs);
    let outline = rect_outline(Vec2::new(size.x, size.z) / 2.0, corner, arcs);
    sweep(&outline, &profile(&sides, arcs))
}

/// A soft cylinder of `radius` and height `h`: rims rounded to the piece, a lip round a broad disc. Up to
/// `POLYGON` sides it is a polygon with rounded corners (the first at +z: hexagonal tiles line up; half the
/// segments, as their normals turn smoothly round them), smooth otherwise; `arcs` segments per rounded quarter.
pub fn soft_cylinder(radius: f32, h: f32, seg: u32, arcs: u32) -> Mesh {
    if radius <= 1e-3 || h <= 1e-3 || seg < 3 {
        return Cylinder::new(radius.max(1e-3), h.max(1e-3))
            .mesh()
            .resolution(seg.max(3))
            .build()
            .rotated_by(Quat::from_rotation_y(-FRAC_PI_2));
    }
    let arcs = arcs.max(1);
    let e = edge_radius(h.min(2.0 * radius));
    let slab = h <= 1.5 * radius && radius >= 0.75;
    let (outline, room) = if seg <= POLYGON {
        let corner = (1.3 * e).min(0.6 * radius * (PI / seg as f32).cos());
        (polygon_outline(seg, radius, corner, arcs.div_ceil(2)), corner / 1.3)
    } else {
        (circle_outline(seg, radius), 0.9 * radius)
    };
    sweep(&outline, &profile(&rim(h, e, room, slab, arcs), arcs))
}

/// The sides a cylinder was made with.
fn cyl_sides(dims: [f64; 3]) -> u32 {
    if dims[2] > 0.0 { dims[2] as u32 } else { 48 }
}

fn cyl_seg(dims: [f64; 3], band: usize) -> u32 {
    let seg = cyl_sides(dims);
    let base = seg.min(seg.min(POLYGON).max(CYL_SEG[band]));
    // (A round one, no sides asked for: a big disc gets more of them near, its rim's facets at most 0.6 m;
    // jump-club's 12 m floor showed 1.6 m ones.)
    if dims[2] > 0.0 || CYL_SEG[band] < 24 {
        return base;
    }
    let by_size = (core::f64::consts::TAU * dims[0] / 0.6).ceil() as u32;
    base.max(by_size.min(CYL_SEG[band] * 3))
}

/// A box's or cylinder's segments per rounded quarter at a band: fewer for small roundings (their normals still
/// turn smoothly round them), so that rungs and rails stay cheap.
fn prim_arcs(kind: PrimKind, dims: [f64; 3], band: usize) -> u32 {
    let [a, b, c] = dims.map(|v| v as f32);
    let e = match kind {
        PrimKind::Box => edge_radius(a.min(b).min(c)),
        PrimKind::Cyl => edge_radius(b.min(2.0 * a)),
        PrimKind::Sphere => return 0,
    };
    let most = if e >= 0.12 {
        4
    } else if e >= 0.05 {
        2
    } else {
        1
    };
    ARCS[band].min(most)
}

/// What sets a band's mesh apart (bands that come out the same are one level, one mesh).
pub fn level_id(kind: PrimKind, dims: [f64; 3], band: usize) -> u32 {
    match kind {
        PrimKind::Box => prim_arcs(kind, dims, band),
        PrimKind::Cyl => cyl_seg(dims, band) * 8 + prim_arcs(kind, dims, band),
        PrimKind::Sphere => SPHERE_SEG[band].0,
    }
}

/// Screen-size thresholds (bounding radius / (distance · tan(fov/2))) where band i goes to i + 1.
const THRESHOLDS: [f32; BANDS - 1] = [0.16, 0.095, 0.056, 0.033, 0.02, 0.012];

/// The bounding radius of a primitive.
pub fn radius(kind: PrimKind, dims: [f64; 3]) -> f32 {
    let [a, b, c] = dims.map(|v| v as f32);
    match kind {
        PrimKind::Box => Vec3::new(a, b, c).length() / 2.0,
        PrimKind::Cyl => a.hypot(b / 2.0),
        PrimKind::Sphere => a,
    }
}

/// Half the size of a primitive's local bounding box.
pub fn half_extents(kind: PrimKind, dims: [f64; 3]) -> Vec3 {
    let [a, b, c] = dims.map(|v| v as f32);
    match kind {
        PrimKind::Box => Vec3::new(a, b, c) / 2.0,
        PrimKind::Cyl => Vec3::new(a, b / 2.0, a),
        PrimKind::Sphere => Vec3::splat(a),
    }
}

/// A level of detail: drawn for bands `first..=last` of something of bounding radius `r`. Its distances follow
/// the field of view and the preset (`refresh_bands`).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct LodBand {
    pub r: f32,
    pub first: usize,
    pub last: usize,
}

impl LodBand {
    /// The camera distances it is shown between, switched abruptly: a cross-fade draws both levels in its band
    /// and takes another pipeline variant with a discard. `k` = 1 / (tan(fov/2) · bias).
    pub fn range(&self, k: f32) -> VisibilityRange {
        self.range_padded(k, 0.0)
    }

    /// `range` with `pad` added to the distances where the level starts and ends (`BandPad`).
    pub fn range_padded(&self, k: f32, pad: f32) -> VisibilityRange {
        let at = |i: usize| self.r * k / THRESHOLDS[i.min(BANDS - 2)] + pad;
        let start = if self.first == 0 { 0.0 } else { at(self.first - 1) };
        let end = if self.last >= BANDS - 1 {
            f32::MAX
        } else {
            at(self.last)
        };
        VisibilityRange::abrupt(start, end.max(start))
    }
}

/// A merged group's distance pad: how far its furthest piece is from the group's centre (where the distance
/// is measured), so that no piece is drawn coarser than it would be by itself.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct BandPad(pub f32);

/// `k` for the field of view and the preset (further detail on better presets).
pub fn lod_k(fov_deg: f32, preset: Option<super::quality::Preset>) -> f32 {
    use super::quality::Preset;
    // (High: the levels switch far enough out that the jump between them is a few pixels.)
    let bias = match preset {
        Some(Preset::High) => 1.6,
        None => 1.0,
        Some(Preset::Low) => 0.6,
    };
    1.0 / ((fov_deg.to_radians() / 2.0).tan() * bias)
}

/// When the field of view or the preset changes, the levels of detail placed so far move to its distances.
pub fn refresh_bands(
    display: Res<crate::settings::Display>,
    quality: Option<Res<super::quality::Quality>>,
    mut last: Local<Option<f32>>,
    mut bands: Query<(&LodBand, Option<&BandPad>, &mut VisibilityRange)>,
) {
    let k = lod_k(display.fov, quality.map(|q| q.preset));
    if *last == Some(k) {
        return;
    }
    *last = Some(k);
    for (band, pad, mut range) in &mut bands {
        let next = band.range_padded(k, pad.map_or(0.0, |p| p.0));
        if *range != next {
            *range = next;
        }
    }
}

/// The levels a primitive is drawn at: bands that come out the same are one level.
pub fn prim_levels(kind: PrimKind, dims: [f64; 3]) -> Vec<LodBand> {
    let r = radius(kind, dims);
    let mut out: Vec<LodBand> = Vec::new();
    for band in 0..BANDS {
        match out.last_mut() {
            Some(l) if level_id(kind, dims, l.first) == level_id(kind, dims, band) => l.last = band,
            _ => out.push(LodBand {
                r,
                first: band,
                last: band,
            }),
        }
    }
    out
}

/// A primitive's mesh at the level that starts at `band`.
pub fn prim(kind: PrimKind, dims: [f64; 3], band: usize) -> Mesh {
    let f = |v: f64| v as f32;
    match kind {
        PrimKind::Box => soft_box(
            Vec3::new(f(dims[0]), f(dims[1]), f(dims[2])),
            prim_arcs(kind, dims, band),
        ),
        PrimKind::Cyl => soft_cylinder(f(dims[0]), f(dims[1]), cyl_seg(dims, band), prim_arcs(kind, dims, band)),
        PrimKind::Sphere => {
            let (w, h) = SPHERE_SEG[band];
            Sphere::new(f(dims[0])).mesh().uv(w, h)
        }
    }
}

/// Lift steps (of `LIFT`) for primitives by their world bounds (min, max): any two that touch or overlap get
/// different steps, so faces they share never meet at one depth. (One with every step taken around it reuses
/// one.)
pub fn lifts(bounds: &[(Vec3, Vec3)]) -> Vec<u32> {
    const TOUCH: f32 = 0.001;
    let mut order: Vec<usize> = (0..bounds.len()).collect();
    order.sort_by(|&a, &b| bounds[a].0.x.total_cmp(&bounds[b].0.x));
    // Pairs that touch: a sweep along x.
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); bounds.len()];
    let mut open: Vec<usize> = Vec::new();
    for &i in &order {
        let (lo, hi) = bounds[i];
        open.retain(|&j| bounds[j].1.x + TOUCH >= lo.x);
        for &j in &open {
            let (a, b) = bounds[j];
            if a.y <= hi.y + TOUCH && lo.y <= b.y + TOUCH && a.z <= hi.z + TOUCH && lo.z <= b.z + TOUCH {
                near[i].push(j);
                near[j].push(i);
            }
        }
        open.push(i);
    }
    let mut out = vec![0u32; bounds.len()];
    for (i, around) in near.iter().enumerate() {
        let taken = around
            .iter()
            .filter(|&&j| j < i)
            .fold(0u32, |m, &j| m | (1u32 << out[j]));
        out[i] = (0..LIFTS)
            .find(|&s| (taken & (1u32 << s)) == 0)
            .unwrap_or(i as u32 % LIFTS);
    }
    out
}

// ---------------------------------------------------------------- merged static geometry

/// Side of the cells static geometry is merged in (m): few entities, yet frustum culling and the levels of
/// detail still tell near from far.
pub const CELL: f32 = 20.0;

/// Marks a merged mesh whose vertices carry the frames of the pieces they came from, for the surface shader's
/// object-space mapping (`OBJECT_FRAME` in surface.wgsl): the piece's rotation in COLOR, the position in its
/// frame (at world scale) in UV_1 and UV_0.x; UV_0.y is the sky hidden from the vertex, 0 until a bake writes it
/// (`ao.rs`, `decor.rs`). (The value itself is unused.)
pub const ATTRIBUTE_FRAME: MeshVertexAttribute =
    MeshVertexAttribute::new("Fb_ObjectFrame", 0x4642_4652_414d_4500, VertexFormat::Float32);

/// The frame the surface shader maps a placed piece in (its axes with the scale taken out) as a rotation; None
/// when the axes are not one (sheared by a parent's uneven scale, mirrored, degenerate).
pub fn frame(m: &Mat4) -> Option<Quat> {
    let [ax, ay, az] = [m.x_axis, m.y_axis, m.z_axis].map(|a| a.truncate().normalize_or_zero());
    let skew = ax.dot(ay).abs().max(ay.dot(az).abs()).max(ax.dot(az).abs());
    if ax == Vec3::ZERO || ay == Vec3::ZERO || az == Vec3::ZERO || skew > 1e-4 || ax.cross(ay).dot(az) <= 0.0 {
        return None;
    }
    Some(Quat::from_mat3(&Mat3::from_cols(ax, ay, az)).normalize())
}

/// A mesh `merge` takes: triangles with positions and normals, and nothing else than UVs.
pub fn mergeable(mesh: &Mesh) -> bool {
    let len = |id: MeshVertexAttribute| match mesh.try_attribute(id) {
        Ok(VertexAttributeValues::Float32x3(v)) => Some(v.len()),
        Ok(VertexAttributeValues::Float32x2(v)) => Some(v.len()),
        _ => None,
    };
    let n = len(Mesh::ATTRIBUTE_POSITION);
    let known = [
        Mesh::ATTRIBUTE_POSITION.id,
        Mesh::ATTRIBUTE_NORMAL.id,
        Mesh::ATTRIBUTE_UV_0.id,
    ];
    mesh.primitive_topology() == PrimitiveTopology::TriangleList
        && n.is_some()
        && len(Mesh::ATTRIBUTE_NORMAL) == n
        && mesh
            .try_attribute_option(Mesh::ATTRIBUTE_UV_0)
            .is_ok_and(|uv| uv.is_none() || len(Mesh::ATTRIBUTE_UV_0) == n)
        && mesh
            .try_attributes()
            .is_ok_and(|mut attrs| attrs.all(|(a, _)| known.contains(&a.id)))
}

/// Static pieces (a mesh and its world matrix each) baked into one mesh around `origin`: positions (less
/// `origin`) and normals in the world. With `frames` (surface materials), each vertex also carries its piece's
/// frame (`ATTRIBUTE_FRAME`) instead of its UVs; without, the UVs are kept. None: nothing to merge, or a piece
/// that `mergeable` or `frame` turns down.
pub fn merge(pieces: &[(&Mesh, Mat4)], origin: Vec3, frames: bool) -> Option<Mesh> {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    let mut uv0: Vec<[f32; 2]> = Vec::new();
    let mut uv1: Vec<[f32; 2]> = Vec::new();
    let mut rot: Vec<[f32; 4]> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for (mesh, m) in pieces {
        if !mergeable(mesh) {
            return None;
        }
        let q = frame(m)?;
        let (Ok(VertexAttributeValues::Float32x3(p)), Ok(VertexAttributeValues::Float32x3(n))) = (
            mesh.try_attribute(Mesh::ATTRIBUTE_POSITION),
            mesh.try_attribute(Mesh::ATTRIBUTE_NORMAL),
        ) else {
            return None;
        };
        let uv = match mesh.try_attribute(Mesh::ATTRIBUTE_UV_0) {
            Ok(VertexAttributeValues::Float32x2(v)) => Some(v),
            _ => None,
        };
        let normal = Mat3::from_mat4(*m).inverse().transpose();
        let at = m.w_axis.truncate();
        let base = pos.len() as u32;
        for (i, (v, vn)) in p.iter().zip(n).enumerate() {
            let w = m.transform_point3(Vec3::from(*v));
            pos.push((w - origin).to_array());
            nrm.push((normal * Vec3::from(*vn)).normalize_or(Vec3::Y).to_array());
            if frames {
                // (As the shader finds it for a piece of its own: the offset from its origin along its axes.)
                let o = q.inverse() * (w - at);
                uv1.push([o.x, o.y]);
                uv0.push([o.z, 0.0]);
                rot.push(q.to_array());
            } else {
                uv0.push(uv.and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]));
            }
        }
        match mesh.try_indices_option() {
            Ok(Some(ix)) => idx.extend(ix.iter().map(|i| base + i as u32)),
            _ => idx.extend(base..base + p.len() as u32),
        }
    }
    if pos.is_empty() {
        return None;
    }
    let count = pos.len();
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv0)
        .with_inserted_indices(Indices::U32(idx));
    if frames {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
        out.insert_attribute(Mesh::ATTRIBUTE_COLOR, rot);
        out.insert_attribute(ATTRIBUTE_FRAME, vec![0.0f32; count]);
    }
    Some(out)
}

/// What a primitive's levels of detail must share with another's for the two to be merged: where its levels
/// start, and (with more than one level) its size to within a factor of √2: a group switches levels at the
/// distances of its biggest piece.
pub fn level_class(levels: &[LodBand]) -> u64 {
    let starts = levels.iter().fold(0u64, |m, l| m | (1u64 << l.first));
    if levels.len() < 2 {
        return starts;
    }
    let r = levels.first().map_or(1.0, |l| l.r);
    let size = (r.max(1e-3).log2() * 2.0).floor() as i64;
    starts | ((size as u64) << 8)
}

/// A piece of static geometry that may be merged with others.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    /// Where it is (the centre of its world bounds).
    pub at: Vec3,
    /// What it is drawn with (an index of the caller's).
    pub material: usize,
    /// Whatever else must match (`level_class`, shadow flags).
    pub class: u64,
    /// Nothing moves, hides or recolours it, and it can be baked (`frame`).
    pub still: bool,
}

/// The cell of a point (`CELL`).
pub fn cell(p: Vec3) -> [i32; 3] {
    (p / CELL).floor().as_ivec3().to_array()
}

/// The candidates drawn merged, as groups (indices, in order): still ones in one cell with the same material
/// and class, two at least. The rest are drawn by themselves.
pub fn groups(candidates: &[Candidate]) -> Vec<Vec<usize>> {
    groups_of(candidates, 2)
}

/// `groups` of `least` candidates or more (one: every still piece gets a mesh of its own, which the baked
/// occlusion is written into, `ao.rs`).
pub fn groups_of(candidates: &[Candidate], least: usize) -> Vec<Vec<usize>> {
    let mut index: HashMap<([i32; 3], usize, u64), usize> = HashMap::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if !c.still {
            continue;
        }
        let g = *index.entry((cell(c.at), c.material, c.class)).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[g].push(i);
    }
    out.retain(|g| g.len() >= least);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_box_keeps_its_size() {
        let m = rounded_box(Vec3::new(4.0, 1.0, 2.0), 2, 0.25);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!()
        };
        let max = p.iter().fold(Vec3::splat(-9.0), |a, v| a.max(Vec3::from(*v)));
        let min = p.iter().fold(Vec3::splat(9.0), |a, v| a.min(Vec3::from(*v)));
        assert!((max - Vec3::new(2.0, 0.5, 1.0)).abs().max_element() < 1e-5, "{max}");
        assert!((min + Vec3::new(2.0, 0.5, 1.0)).abs().max_element() < 1e-5, "{min}");
    }

    #[test]
    fn levels_are_few_and_follow_on() {
        let k = lod_k(70.0, None);
        for (kind, dims, count) in [
            (PrimKind::Box, [4.0, 1.0, 2.0], 3),
            (PrimKind::Sphere, [1.0, 0.0, 0.0], 3),
            (PrimKind::Cyl, [1.0, 2.0, 0.0], 3),
            (PrimKind::Cyl, [1.0, 0.5, 6.0], 3),
            (PrimKind::Cyl, [0.045, 0.9, 8.0], 1),
            (PrimKind::Box, [0.11, 3.0, 0.11], 1),
        ] {
            let levels = prim_levels(kind, dims);
            assert_eq!(levels.len(), count, "{kind:?}");
            assert_eq!(levels[0].first, 0);
            assert_eq!(levels[levels.len() - 1].last, BANDS - 1);
            for pair in levels.windows(2) {
                let (a, b) = (pair[0].range(k), pair[1].range(k));
                assert!(a.is_abrupt() && b.is_abrupt());
                assert_eq!(
                    a.end_margin.start, b.start_margin.start,
                    "{kind:?}: a gap or an overlap"
                );
            }
        }
    }

    #[test]
    fn touching_primitives_lift_apart() {
        let b = |lo: [f32; 3], hi: [f32; 3]| (Vec3::from(lo), Vec3::from(hi));
        // A floor, two tiles side by side on it, one box far away.
        let l = lifts(&[
            b([-5.0, -1.0, -5.0], [5.0, 0.0, 5.0]),
            b([-1.0, 0.0, -1.0], [0.0, 0.2, 1.0]),
            b([0.0, 0.0, -1.0], [1.0, 0.2, 1.0]),
            b([20.0, 0.0, 0.0], [21.0, 1.0, 1.0]),
        ]);
        assert_ne!(l[0], l[1]);
        assert_ne!(l[0], l[2]);
        assert_ne!(l[1], l[2]);
        assert_eq!(l[3], 0);
        // More than six coplanar tiles over each other still get six different steps in a row.
        let stack: Vec<_> = (0..8).map(|_| b([0.0; 3], [1.0; 3])).collect();
        let l = lifts(&stack);
        for w in l.windows(6) {
            let mut s = w.to_vec();
            s.sort_unstable();
            s.dedup();
            assert_eq!(s.len(), 6, "{l:?}");
        }
    }

    fn v3(m: &Mesh, id: MeshVertexAttribute) -> Vec<Vec3> {
        match m.attribute(id) {
            Some(VertexAttributeValues::Float32x3(v)) => v.iter().map(|p| Vec3::from(*p)).collect(),
            _ => panic!("no {}", id.name),
        }
    }

    fn v2(m: &Mesh, id: MeshVertexAttribute) -> Vec<Vec2> {
        match m.attribute(id) {
            Some(VertexAttributeValues::Float32x2(v)) => v.iter().map(|p| Vec2::from(*p)).collect(),
            _ => panic!("no {}", id.name),
        }
    }

    /// A soft mesh's checks (mergeable, unit normals, no degenerate triangles, each turned the way its normals
    /// point); its bounds (min, max).
    fn check(what: &str, m: &Mesh) -> (Vec3, Vec3) {
        assert!(mergeable(m), "{what}: not mergeable");
        let p = v3(m, Mesh::ATTRIBUTE_POSITION);
        let n = v3(m, Mesh::ATTRIBUTE_NORMAL);
        assert_eq!(p.len(), n.len(), "{what}");
        for v in &n {
            assert!((v.length() - 1.0).abs() < 1e-4, "{what}: normal {v}");
        }
        let idx: Vec<usize> = m.indices().unwrap().iter().collect();
        assert!(!idx.is_empty() && idx.len().is_multiple_of(3), "{what}");
        for t in idx.as_chunks::<3>().0 {
            let [a, b, c] = [p[t[0]], p[t[1]], p[t[2]]];
            let face = (b - a).cross(c - a);
            assert!(face.length() > 1e-9, "{what}: degenerate {t:?}");
            assert!(
                face.dot(n[t[0]] + n[t[1]] + n[t[2]]) > 0.0,
                "{what}: turned inwards {t:?}"
            );
        }
        let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for v in &p {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        (lo, hi)
    }

    #[test]
    fn soft_boxes_are_sound_and_keep_the_colliders_bounds() {
        for size in [
            Vec3::new(4.0, 1.0, 2.0),
            Vec3::ONE,
            Vec3::new(0.3, 3.0, 0.3),
            Vec3::new(40.0, 2.0, 40.0),
            Vec3::new(8.0, 0.1, 8.0),
            Vec3::new(0.5, 4.0, 6.0),
            Vec3::new(2.6, 0.14, 0.12),
        ] {
            for arcs in [1, 2, 3, 4] {
                let what = format!("box {size} × {arcs}");
                let (lo, hi) = check(&what, &soft_box(size, arcs));
                assert!((hi - size / 2.0).abs().max_element() < 1e-4, "{what}: {hi}");
                assert!((lo + size / 2.0).abs().max_element() < 1e-4, "{what}: {lo}");
            }
        }
    }

    #[test]
    fn soft_cylinders_are_sound_and_keep_the_colliders_bounds() {
        for (r, h, seg) in [
            (13.0, 2.0, 48),
            (13.05, 0.1, 48),
            (0.3, 10.0, 48),
            (1.0, 2.0, 24),
            (0.75, 1.0, 32),
            (1.455, 0.5, 6),
            (1.0, 0.5, 8),
            (0.5, 0.3, 3),
        ] {
            for arcs in [1, 2, 3, 4] {
                let what = format!("cylinder {r} {h} {seg} × {arcs}");
                let (lo, hi) = check(&what, &soft_cylinder(r, h, seg, arcs));
                let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
                assert!(near(hi.y, h / 2.0) && near(lo.y, -h / 2.0), "{what}");
                let reach = Vec3::new(r, h, r) + 1e-4;
                assert!(hi.cmple(reach).all() && lo.cmpge(-reach).all(), "{what}");
                if seg % 4 == 0 && seg > POLYGON {
                    // Points on the axes: as wide as the collider.
                    assert!(near(hi.x, r) && near(lo.z, -r), "{what}: {lo} {hi}");
                }
            }
        }
        // A hexagon's flat sides (±x) are where they were; its rounded corners (±z) a little in.
        let (lo, hi) = check("hexagon", &soft_cylinder(1.5, 0.5, 6, 2));
        let apothem = 1.5 * (PI / 6.0).cos();
        assert!((hi.x - apothem).abs() < 1e-4, "{hi}");
        assert!((lo.x + apothem).abs() < 1e-4, "{lo}");
        assert!(hi.z < 1.5 && hi.z > 1.3, "{hi}");
    }

    #[test]
    fn broad_slabs_have_a_lip_and_thin_plates_none() {
        // (Faces looking down above the bottom: the lip's underside.)
        let under = |m: &Mesh, h: f32| {
            let p = v3(m, Mesh::ATTRIBUTE_POSITION);
            let n = v3(m, Mesh::ATTRIBUTE_NORMAL);
            p.iter().zip(&n).any(|(p, n)| n.y < -0.99 && p.y > -h / 2.0 + 0.05)
        };
        assert!(under(&soft_box(Vec3::new(6.0, 1.0, 6.0), 4), 1.0));
        assert!(under(&soft_cylinder(13.0, 2.0, 48, 2), 2.0));
        assert!(under(&soft_cylinder(1.455, 0.5, 6, 2), 0.5));
        // (Not at a chamfer's detail.)
        assert!(!under(&soft_box(Vec3::new(6.0, 1.0, 6.0), 1), 1.0));
        assert!(!under(&soft_box(Vec3::new(8.0, 0.2, 8.0), 4), 0.2));
        assert!(!under(&soft_box(Vec3::new(1.0, 3.0, 1.0), 4), 3.0));
        assert!(!under(&soft_cylinder(0.3, 10.0, 48, 4), 10.0));
    }

    #[test]
    fn every_level_of_every_primitive_is_sound() {
        for (kind, dims) in [
            (PrimKind::Box, [4.0, 1.0, 2.0]),
            (PrimKind::Box, [30.0, 1.0, 12.0]),
            (PrimKind::Cyl, [13.0, 2.0, 0.0]),
            (PrimKind::Cyl, [1.1, 3.0, 32.0]),
            (PrimKind::Cyl, [1.0, 0.5, 6.0]),
            (PrimKind::Cyl, [0.045, 0.9, 8.0]),
            (PrimKind::Box, [0.11, 3.0, 0.11]),
            (PrimKind::Cyl, [0.9, 0.8, 16.0]),
        ] {
            for level in prim_levels(kind, dims) {
                let m = prim(kind, dims, level.first);
                let (lo, hi) = check(&format!("{kind:?} {dims:?} from B{}", level.first), &m);
                let half = half_extents(kind, dims);
                assert!(hi.cmple(half + 1e-4).all() && lo.cmpge(-half - 1e-4).all());
                assert!((hi.y - half.y).abs() < 1e-4 && (lo.y + half.y).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn merging_bakes_the_transforms_and_keeps_each_frame() {
        let a: Mesh = Cuboid::new(1.0, 1.0, 1.0).into();
        let b = prim(PrimKind::Sphere, [0.5, 0.0, 0.0], 4);
        let turn = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.2);
        let m1 = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let m2 = Mat4::from_scale_rotation_translation(Vec3::new(2.0, 1.0, 3.0), turn, Vec3::new(0.0, 5.0, -4.0));
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let merged = merge(&[(&a, m1), (&b, m2)], origin, true).unwrap();
        let (na, nb) = (a.count_vertices(), b.count_vertices());
        assert_eq!(merged.count_vertices(), na + nb);
        let ia: Vec<usize> = a.indices().unwrap().iter().collect();
        let ib: Vec<usize> = b.indices().unwrap().iter().collect();
        let im: Vec<usize> = merged.indices().unwrap().iter().collect();
        assert_eq!(im.len(), ia.len() + ib.len());
        assert_eq!(
            im[ia.len()],
            ib[0] + na,
            "the second piece's indices follow the first's vertices"
        );
        let (pa, pb, pm) = (
            v3(&a, Mesh::ATTRIBUTE_POSITION),
            v3(&b, Mesh::ATTRIBUTE_POSITION),
            v3(&merged, Mesh::ATTRIBUTE_POSITION),
        );
        let (nrm_b, nm) = (v3(&b, Mesh::ATTRIBUTE_NORMAL), v3(&merged, Mesh::ATTRIBUTE_NORMAL));
        let close = |x: Vec3, y: Vec3| (x - y).abs().max_element() < 1e-4;
        assert!(close(pm[0], m1.transform_point3(pa[0]) - origin));
        for i in [0, nb / 2, nb - 1] {
            assert!(close(pm[na + i], m2.transform_point3(pb[i]) - origin), "vertex {i}");
            let n = (Mat3::from_mat4(m2).inverse().transpose() * nrm_b[i]).normalize();
            assert!(close(nm[na + i], n), "normal {i}");
        }
        // The frame: the rotation of each piece, and the position along its axes at world scale.
        let Some(VertexAttributeValues::Float32x4(rot)) = merged.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!()
        };
        let q = Quat::from_array(rot[na]);
        assert!(close(q * Vec3::X, turn * Vec3::X) && close(q * Vec3::Z, turn * Vec3::Z));
        assert!(close(Quat::from_array(rot[0]) * Vec3::X, Vec3::X));
        let (uv0, uv1) = (v2(&merged, Mesh::ATTRIBUTE_UV_0), v2(&merged, Mesh::ATTRIBUTE_UV_1));
        let i = nb / 3;
        let local = Vec3::new(uv1[na + i].x, uv1[na + i].y, uv0[na + i].x);
        assert!(close(local, pb[i] * Vec3::new(2.0, 1.0, 3.0)), "{local}");
        assert!(merged.contains_attribute(ATTRIBUTE_FRAME));
        // Without frames (plain materials): the UVs stay, no frame.
        let plain = merge(&[(&a, m1), (&a, m2)], Vec3::ZERO, false).unwrap();
        assert!(!plain.contains_attribute(ATTRIBUTE_FRAME) && !plain.contains_attribute(Mesh::ATTRIBUTE_COLOR));
        assert_eq!(
            v2(&plain, Mesh::ATTRIBUTE_UV_0)[na + 3],
            v2(&a, Mesh::ATTRIBUTE_UV_0)[3]
        );
        assert_eq!(plain.count_vertices(), 2 * na);
    }

    #[test]
    fn only_rotations_are_frames() {
        let r = Mat4::from_rotation_z(0.8);
        assert!(
            frame(&(Mat4::from_scale(Vec3::new(1.0, 2.0, 3.0)) * r)).is_none(),
            "sheared"
        );
        assert!(
            frame(&Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))).is_none(),
            "mirrored"
        );
        assert!(frame(&Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0))).is_none(), "flat");
        assert!(frame(&(r * Mat4::from_scale(Vec3::new(1.0, 2.0, 3.0)))).is_some());
        let a: Mesh = Cuboid::new(1.0, 1.0, 1.0).into();
        assert!(merge(&[(&a, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)))], Vec3::ZERO, false).is_none());
        let tangents = a.clone().with_inserted_attribute(
            Mesh::ATTRIBUTE_TANGENT,
            vec![[1.0f32, 0.0, 0.0, 1.0]; a.count_vertices()],
        );
        assert!(mergeable(&a) && !mergeable(&tangents));
        let one = merge(&[(&rounded_box(Vec3::ONE, 2, 0.1), Mat4::IDENTITY)], Vec3::ZERO, true);
        assert!(one.is_some_and(|m| m.contains_attribute(ATTRIBUTE_FRAME)));
    }

    #[test]
    fn groups_split_by_cell_material_class_and_stillness() {
        let c = |x: f32, material: usize, class: u64, still: bool| Candidate {
            at: Vec3::new(x, 1.0, 1.0),
            material,
            class,
            still,
        };
        let g = groups(&[
            c(1.0, 0, 0, true),
            c(2.0, 0, 0, true),
            c(3.0, 1, 0, true),
            c(4.0, 0, 0, false),
            c(CELL + 1.0, 0, 0, true),
            c(5.0, 0, 7, true),
            c(6.0, 0, 0, true),
            c(CELL + 2.0, 0, 0, true),
        ]);
        assert_eq!(g, vec![vec![0, 1, 6], vec![4, 7]]);
        // One at least: a still piece alone in its cell is a group of its own, a moving one none.
        let one = groups_of(&[c(1.0, 0, 0, true), c(2.0, 1, 0, true), c(3.0, 0, 0, false)], 1);
        assert_eq!(one, vec![vec![0], vec![1]]);
        // Levels: boxes of four times the size switch apart; single-level cylinders (rungs) of any size go
        // together; pieces whose levels start at the same bands, of a size, go together whatever their kind.
        let class = |kind, dims| level_class(&prim_levels(kind, dims));
        assert_ne!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Box, [4.0; 3]));
        assert_eq!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Box, [1.1, 1.0, 1.0]));
        assert_eq!(
            class(PrimKind::Cyl, [0.045, 0.9, 8.0]),
            class(PrimKind::Cyl, [0.04, 2.0, 8.0])
        );
        assert_ne!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Cyl, [0.045, 0.9, 8.0]));
        assert_eq!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Sphere, [0.9, 0.0, 0.0]));
    }

    #[test]
    fn padded_levels_start_and_end_further_out() {
        let k = lod_k(70.0, None);
        let levels = prim_levels(PrimKind::Sphere, [1.0, 0.0, 0.0]);
        let (a, b) = (levels[0].range_padded(k, 0.0), levels[0].range_padded(k, 5.0));
        assert_eq!(a.start_margin.start, 0.0);
        assert_eq!(b.start_margin.start, 0.0);
        assert!((b.end_margin.start - a.end_margin.start - 5.0).abs() < 1e-3);
        let last = levels[levels.len() - 1].range_padded(k, 5.0);
        assert_eq!(last.end_margin.start, f32::MAX);
        assert_eq!(levels[1].range_padded(k, 5.0).start_margin.start, b.end_margin.start);
    }
}
