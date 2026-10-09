//! Meshes of the scenery's shapes, each at two levels of detail.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Shape {
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

pub(super) fn octahedron() -> Mesh {
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
pub(super) fn dodecahedron() -> Mesh {
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

/// The upper half of a unit sphere (a dome), flat side down: `w` segments round, `h` up.
fn dome(w: u32, h: u32) -> Mesh {
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

/// A unit disc facing up whose vertex colours fade from opaque at the centre to clear at the rim (a blob
/// shadow's, `Kit::blob_shadow`).
pub(super) fn blob_disc() -> Mesh {
    const SEG: u32 = 28;
    // (Radius, opacity) of each ring after the centre: a soft falloff.
    const RINGS: [(f32, f32); 3] = [(0.45, 0.78), (0.75, 0.38), (1.0, 0.0)];
    let mut pos = vec![[0.0f32; 3]];
    let mut col = vec![[1.0f32, 1.0, 1.0, 0.95]];
    for (r, a) in RINGS {
        for i in 0..SEG {
            let t = i as f32 / SEG as f32 * core::f32::consts::TAU;
            pos.push([t.cos() * r, 0.0, t.sin() * r]);
            col.push([1.0, 1.0, 1.0, a]);
        }
    }
    // Vertex `i` of ring `k` (1-based; 0 is the centre).
    let at = |k: u32, i: u32| 1 + (k - 1) * SEG + i % SEG;
    let mut idx = Vec::new();
    for i in 0..SEG {
        // (Wound to face up.)
        idx.extend_from_slice(&[0, at(1, i + 1), at(1, i)]);
        for k in 1..RINGS.len() as u32 {
            let (a0, a1, b0, b1) = (at(k, i), at(k, i + 1), at(k + 1, i), at(k + 1, i + 1));
            idx.extend_from_slice(&[a0, b1, b0, a0, a1, b1]);
        }
    }
    let n = pos.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx))
}

/// The two levels of detail of the shapes: smooth silhouettes and rounded box edges near, the old segment
/// counts and plain boxes far.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Level {
    Near,
    Far,
}

/// Rounding of the boxes' edges near, in the unit box (a part's scale stretches it with the box).
const BOX_ROUND: f32 = 0.12;

pub(super) fn shape_mesh(s: Shape, level: Level) -> Mesh {
    let near = level == Level::Near;
    let seg = if near { 40 } else { 20 };
    match s {
        Shape::Box if near => crate::render::meshes::rounded_box(Vec3::ONE, 2, BOX_ROUND),
        Shape::Box => Cuboid::new(1.0, 1.0, 1.0).into(),
        Shape::Sphere if near => Sphere::new(1.0).mesh().uv(36, 22),
        Shape::Sphere => Sphere::new(1.0).mesh().uv(20, 14),
        Shape::Cyl => Cylinder::new(1.0, 1.0).mesh().resolution(seg).build(),
        Shape::Taper => ConicalFrustum {
            radius_top: 0.62,
            radius_bottom: 1.0,
            height: 1.0,
        }
        .mesh()
        .resolution(seg)
        .build(),
        Shape::Cone => Cone::new(1.0, 1.0).mesh().resolution(seg).build(),
        Shape::Cone4 => Cone::new(1.0, 1.0).mesh().resolution(4).build(),
        Shape::Torus if near => torus(1.0, 0.3, 16, 56, core::f32::consts::TAU),
        Shape::Torus => torus(1.0, 0.3, 12, 36, core::f32::consts::TAU),
        Shape::Ring if near => torus(1.0, 0.07, 10, 88, core::f32::consts::TAU),
        Shape::Ring => torus(1.0, 0.07, 8, 56, core::f32::consts::TAU),
        Shape::Octa => octahedron(),
        Shape::Rock => dodecahedron(),
        Shape::Dome if near => dome(36, 14),
        Shape::Dome => dome(20, 10),
        Shape::HalfTorus if near => torus(0.7, 0.28, 16, 36, core::f32::consts::PI),
        Shape::HalfTorus => torus(0.7, 0.28, 12, 24, core::f32::consts::PI),
    }
}

/// Whether a shape's two levels differ (the ones that do not are merged into one level).
pub(super) fn has_levels(s: Shape) -> bool {
    !matches!(s, Shape::Cone4 | Shape::Octa | Shape::Rock)
}

/// Half extents of a unit shape and the centre of its bulk (in the unit shape's space).
pub(super) fn bulk(s: Shape) -> (Vec3, Vec3) {
    match s {
        Shape::Box => (Vec3::splat(0.5), Vec3::ZERO),
        Shape::Sphere | Shape::Rock => (Vec3::ONE, Vec3::ZERO),
        Shape::Cyl | Shape::Taper => (Vec3::new(0.9, 0.5, 0.9), Vec3::ZERO),
        Shape::Cone | Shape::Cone4 => (Vec3::new(0.6, 0.4, 0.6), Vec3::new(0.0, -0.2, 0.0)),
        Shape::Octa => (Vec3::splat(0.6), Vec3::ZERO),
        Shape::Dome => (Vec3::new(0.85, 0.45, 0.85), Vec3::new(0.0, 0.38, 0.0)),
        // (Hollow: they hide too little of anything to count.)
        Shape::Torus | Shape::Ring | Shape::HalfTorus => (Vec3::ZERO, Vec3::ZERO),
    }
}
