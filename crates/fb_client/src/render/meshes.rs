//! Meshes of map primitives as `view.ts` built them: boxes with rounded edges (three's
//! `RoundedBoxGeometry`), cylinders whose first segment faces +z, UV spheres; each level of detail
//! with fewer segments, and a millimetre lift so primitives sharing a top face never z-fight.
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use fb_sim::scene::PrimKind;

/// Levels of detail (M0…M6, `lod.ts`).
pub const LEVELS: usize = 7;
const SPHERE_SEG: [(u32, u32); LEVELS] = [(32, 20), (26, 16), (20, 12), (16, 10), (12, 8), (10, 7), (8, 6)];
const CYL_SEG: [u32; LEVELS] = [48, 36, 24, 18, 14, 10, 8];
const BOX_SEG: [u32; LEVELS] = [2, 1, 1, 1, 0, 0, 0];
/// Rounding radius per level: flatter bevels before the plain box.
const BOX_ROUND: [f32; LEVELS] = [1.0, 1.0, 0.75, 0.5, 0.0, 0.0, 0.0];
/// Height of one lift step (m); primitives take one of a few in turn.
pub const LIFT: f32 = 0.0025;
pub const LIFTS: u32 = 6;

/// A box with rounded edges and corners (three's `RoundedBoxGeometry`): `segments` per rounded edge.
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

/// A cylinder as three.js builds it (the first segment faces +z): hexagonal tiles line up.
pub fn cylinder(r: f32, h: f32, seg: u32) -> Mesh {
    Cylinder::new(r, h)
        .mesh()
        .resolution(seg)
        .build()
        .rotated_by(Quat::from_rotation_y(-core::f32::consts::FRAC_PI_2))
}

fn cyl_seg(dims: [f64; 3], level: usize) -> u32 {
    let seg = if dims[2] > 0.0 { dims[2] as u32 } else { 48 };
    seg.min(seg.min(8).max(CYL_SEG[level]))
}

/// What sets a level's mesh apart (levels that come out the same share one mesh).
pub fn level_id(kind: PrimKind, dims: [f64; 3], level: usize) -> u32 {
    match kind {
        PrimKind::Box => level.min(4) as u32,
        PrimKind::Cyl => cyl_seg(dims, level),
        PrimKind::Sphere => SPHERE_SEG[level].0,
    }
}

/// Screen-size thresholds (bounding radius / (distance · tan(fov/2))) where level i goes to i + 1.
const THRESHOLDS: [f32; LEVELS - 1] = [0.16, 0.095, 0.056, 0.033, 0.02, 0.012];
/// The cross-fade band around each threshold (ratio of the distance).
const HYST: f32 = 1.08;

/// The bounding radius of a primitive.
pub fn radius(kind: PrimKind, dims: [f64; 3]) -> f32 {
    let [a, b, c] = dims.map(|v| v as f32);
    match kind {
        PrimKind::Box => Vec3::new(a, b, c).length() / 2.0,
        PrimKind::Cyl => a.hypot(b / 2.0),
        PrimKind::Sphere => a,
    }
}

/// The camera distances a level is shown between, for something of bounding radius r (with a cross-fade
/// band). `k` = 1 / (tan(fov/2) · bias).
pub fn range_for(r: f32, k: f32, level: usize) -> VisibilityRange {
    span(r, k, level, level)
}

/// Levels `first`…`last` shown as one.
fn span(r: f32, k: f32, first: usize, last: usize) -> VisibilityRange {
    let at = |i: usize| r * k / THRESHOLDS[i];
    let start = if first == 0 { 0.0 } else { at(first - 1) };
    let end = if last == LEVELS - 1 { f32::MAX } else { at(last) };
    let band = |d: f32| {
        if d == 0.0 || d == f32::MAX {
            d..d
        } else {
            d / HYST..d * HYST
        }
    };
    VisibilityRange {
        start_margin: band(start),
        end_margin: band(end),
        use_aabb: false,
    }
}

/// `k` for the field of view and the preset (further detail on better presets).
pub fn lod_k(fov_deg: f32, preset: Option<super::quality::Preset>) -> f32 {
    use super::quality::Preset;
    let bias = match preset {
        Some(Preset::High) | None => 1.0,
        Some(Preset::Medium) => 0.75,
        Some(Preset::Low) => 0.6,
    };
    1.0 / ((fov_deg.to_radians() / 2.0).tan() * bias)
}

/// The levels a primitive is drawn at and the distances each is shown between: levels that come out the
/// same are merged.
pub fn level_ranges(kind: PrimKind, dims: [f64; 3], k: f32) -> Vec<(usize, VisibilityRange)> {
    let r = radius(kind, dims);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for level in 0..LEVELS {
        match out.last_mut() {
            Some(last) if level_id(kind, dims, last.0) == level_id(kind, dims, level) => last.1 = level,
            _ => out.push((level, level)),
        }
    }
    out.into_iter().map(|(a, b)| (a, span(r, k, a, b))).collect()
}

/// A primitive's mesh at a level of detail, lifted `lift` steps.
pub fn prim(kind: PrimKind, dims: [f64; 3], level: usize, lift: u32) -> Mesh {
    let f = |v: f64| v as f32;
    let mesh = match kind {
        PrimKind::Box => {
            let size = Vec3::new(f(dims[0]), f(dims[1]), f(dims[2]));
            let r = 0.25f32.min(size.x / 4.0).min(size.y / 4.0).min(size.z / 4.0);
            rounded_box(size, BOX_SEG[level], r * BOX_ROUND[level])
        }
        PrimKind::Cyl => cylinder(f(dims[0]), f(dims[1]), cyl_seg(dims, level)),
        PrimKind::Sphere => {
            let (w, h) = SPHERE_SEG[level];
            Sphere::new(f(dims[0])).mesh().uv(w, h)
        }
    };
    if lift == 0 {
        mesh
    } else {
        mesh.translated_by(Vec3::Y * lift as f32 * LIFT)
    }
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
}
