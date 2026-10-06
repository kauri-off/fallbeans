//! Meshes of map primitives: boxes with rounded edges, cylinders whose first segment faces +z, UV spheres; a few
//! levels of detail with fewer segments, switched by size on screen; and the millimetre lifts that keep
//! overlapping primitives from z-fighting.
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
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
        let at = |i: usize| self.r * k / THRESHOLDS[i.min(BANDS - 2)];
        let start = if self.first == 0 { 0.0 } else { at(self.first - 1) };
        let end = if self.last >= BANDS - 1 {
            f32::MAX
        } else {
            at(self.last)
        };
        VisibilityRange::abrupt(start, end.max(start))
    }
}

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
    mut bands: Query<(&LodBand, &mut VisibilityRange)>,
) {
    let k = lod_k(display.fov, quality.map(|q| q.preset));
    if *last == Some(k) {
        return;
    }
    *last = Some(k);
    for (band, mut range) in &mut bands {
        let next = band.range(k);
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
}
