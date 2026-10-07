//! Meshes of map primitives: boxes with rounded edges, cylinders whose first segment faces +z, UV spheres; a few
//! levels of detail with fewer segments, switched by size on screen; the millimetre lifts that keep
//! overlapping primitives from z-fighting; and static geometry merged into one mesh per cell and material.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues, VertexFormat};
use bevy::prelude::*;
use fb_sim::scene::PrimKind;

/// Distance bands B0…B6 (by size on screen); a level of detail spans one or more of them.
pub const BANDS: usize = 7;
const SPHERE_SEG: [(u32, u32); BANDS] = [(32, 20), (32, 20), (20, 12), (20, 12), (10, 7), (10, 7), (10, 7)];
const CYL_SEG: [u32; BANDS] = [48, 48, 24, 24, 12, 12, 12];
/// Boxes are rounded up to this band, plain from it on.
const BOX_PLAIN: usize = 4;
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

/// A cylinder whose first segment faces +z: hexagonal tiles line up.
pub fn cylinder(r: f32, h: f32, seg: u32) -> Mesh {
    Cylinder::new(r, h)
        .mesh()
        .resolution(seg)
        .build()
        .rotated_by(Quat::from_rotation_y(-core::f32::consts::FRAC_PI_2))
}

fn cyl_seg(dims: [f64; 3], band: usize) -> u32 {
    let seg = if dims[2] > 0.0 { dims[2] as u32 } else { 48 };
    seg.min(seg.min(8).max(CYL_SEG[band]))
}

/// What sets a band's mesh apart (bands that come out the same are one level, one mesh).
pub fn level_id(kind: PrimKind, dims: [f64; 3], band: usize) -> u32 {
    match kind {
        PrimKind::Box => u32::from(band >= BOX_PLAIN),
        PrimKind::Cyl => cyl_seg(dims, band),
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
        Some(Preset::Medium) => 0.9,
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
        PrimKind::Box => {
            let size = Vec3::new(f(dims[0]), f(dims[1]), f(dims[2]));
            let r = 0.25f32.min(size.x / 4.0).min(size.y / 4.0).min(size.z / 4.0);
            if band < BOX_PLAIN {
                rounded_box(size, 2, r)
            } else {
                Cuboid::from_size(size).into()
            }
        }
        PrimKind::Cyl => cylinder(f(dims[0]), f(dims[1]), cyl_seg(dims, band)),
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
/// frame (at world scale) in UV_1 and UV_0.x. (The value itself is unused.)
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
    out.retain(|g| g.len() >= 2);
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
            (PrimKind::Box, [4.0, 1.0, 2.0], 2),
            (PrimKind::Sphere, [1.0, 0.0, 0.0], 3),
            (PrimKind::Cyl, [1.0, 2.0, 0.0], 3),
            (PrimKind::Cyl, [1.0, 0.5, 6.0], 1),
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
        // Levels: boxes of four times the size switch apart; single-level cylinders of any size go together.
        let class = |kind, dims| level_class(&prim_levels(kind, dims));
        assert_ne!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Box, [4.0; 3]));
        assert_eq!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Box, [1.1, 1.0, 1.0]));
        assert_eq!(
            class(PrimKind::Cyl, [1.0, 0.5, 6.0]),
            class(PrimKind::Cyl, [5.0, 0.5, 6.0])
        );
        assert_ne!(class(PrimKind::Box, [1.0; 3]), class(PrimKind::Sphere, [0.9, 0.0, 0.0]));
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
