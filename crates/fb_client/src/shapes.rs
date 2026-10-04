//! Meshes built as three.js builds its geometries (the same vertex layout, orientation and winding), so
//! that parts ported from the TS client keep their numbers.
use core::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

#[derive(Default)]
struct Build {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl Build {
    fn vert(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.pos.push(p.to_array());
        self.nrm.push(n.normalize_or_zero().to_array());
        self.uv.push(uv.to_array());
        self.pos.len() as u32 - 1
    }

    fn mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
            .with_inserted_indices(Indices::U32(self.idx))
    }
}

/// `SphereGeometry(r, ws, hs, phiStart, phiLength, thetaStart, thetaLength)`.
pub fn sphere(r: f32, ws: u32, hs: u32, phi: (f32, f32), theta: (f32, f32)) -> Mesh {
    let mut b = Build::default();
    let theta_end = (theta.0 + theta.1).min(PI);
    let mut grid = Vec::new();
    for iy in 0..=hs {
        let v = iy as f32 / hs as f32;
        let mut row = Vec::new();
        for ix in 0..=ws {
            let u = ix as f32 / ws as f32;
            let (p, t) = (phi.0 + u * phi.1, theta.0 + v * theta.1);
            let p = Vec3::new(-r * p.cos() * t.sin(), r * t.cos(), r * p.sin() * t.sin());
            row.push(b.vert(p, p, Vec2::new(u, 1.0 - v)));
        }
        grid.push(row);
    }
    for iy in 0..hs as usize {
        for ix in 0..ws as usize {
            let (a, bb) = (grid[iy][ix + 1], grid[iy][ix]);
            let (c, d) = (grid[iy + 1][ix], grid[iy + 1][ix + 1]);
            if iy != 0 || theta.0 > 0.0 {
                b.idx.extend([a, bb, d]);
            }
            if iy != hs as usize - 1 || theta_end < PI {
                b.idx.extend([bb, c, d]);
            }
        }
    }
    b.mesh()
}

pub fn ball(r: f32) -> Mesh {
    sphere(r, 16, 12, (0.0, TAU), (0.0, PI))
}

/// The top half of a sphere squashed to height `h` (TS `dome`).
pub fn dome(r: f32, h: f32) -> Mesh {
    sphere(r, 28, 12, (0.0, TAU), (0.0, PI / 2.0)).scaled_by(Vec3::new(1.0, h / r, 1.0))
}

/// `CylinderGeometry(rt, rb, h, radial, 1, open, thetaStart, thetaLength)`.
pub fn cylinder(rt: f32, rb: f32, h: f32, radial: u32, open: bool, theta: (f32, f32)) -> Mesh {
    let mut b = Build::default();
    let half = h / 2.0;
    let slope = (rb - rt) / h;
    let mut rows = Vec::new();
    for y in 0..=1u32 {
        let v = y as f32;
        let r = v * (rb - rt) + rt;
        let mut row = Vec::new();
        for x in 0..=radial {
            let u = x as f32 / radial as f32;
            let t = u * theta.1 + theta.0;
            let (s, c) = t.sin_cos();
            let p = Vec3::new(r * s, -v * h + half, r * c);
            row.push(b.vert(p, Vec3::new(s, slope, c), Vec2::new(u, 1.0 - v)));
        }
        rows.push(row);
    }
    for x in 0..radial as usize {
        let (a, bb, c, d) = (rows[0][x], rows[1][x], rows[1][x + 1], rows[0][x + 1]);
        b.idx.extend([a, bb, d, bb, c, d]);
    }
    if !open {
        for top in [true, false] {
            let (r, sign) = if top { (rt, 1.0) } else { (rb, -1.0) };
            if r <= 0.0 {
                continue;
            }
            let n = Vec3::Y * sign;
            let centers: Vec<u32> = (0..radial)
                .map(|_| b.vert(Vec3::Y * half * sign, n, Vec2::splat(0.5)))
                .collect();
            let ring: Vec<u32> = (0..=radial)
                .map(|x| {
                    let t = x as f32 / radial as f32 * theta.1 + theta.0;
                    let (s, c) = t.sin_cos();
                    b.vert(
                        Vec3::new(r * s, half * sign, r * c),
                        n,
                        Vec2::new(c * 0.5 + 0.5, s * 0.5 * sign + 0.5),
                    )
                })
                .collect();
            for x in 0..radial as usize {
                let (c, i, j) = (centers[x], ring[x], ring[x + 1]);
                if top {
                    b.idx.extend([i, j, c]);
                } else {
                    b.idx.extend([j, i, c]);
                }
            }
        }
    }
    b.mesh()
}

pub fn cone(r: f32, h: f32, radial: u32) -> Mesh {
    cylinder(0.0, r, h, radial, false, (0.0, TAU))
}

/// `TorusGeometry(radius, tube, radial, tubular)`: the ring lies in the xy plane.
pub fn torus(radius: f32, tube: f32, radial: u32, tubular: u32) -> Mesh {
    let mut b = Build::default();
    for j in 0..=radial {
        for i in 0..=tubular {
            let u = i as f32 / tubular as f32 * TAU;
            let v = j as f32 / radial as f32 * TAU;
            let p = Vec3::new(
                (radius + tube * v.cos()) * u.cos(),
                (radius + tube * v.cos()) * u.sin(),
                tube * v.sin(),
            );
            let center = Vec3::new(radius * u.cos(), radius * u.sin(), 0.0);
            b.vert(
                p,
                p - center,
                Vec2::new(i as f32 / tubular as f32, j as f32 / radial as f32),
            );
        }
    }
    let w = tubular + 1;
    for j in 1..=radial {
        for i in 1..=tubular {
            let a = w * j + i - 1;
            let bb = w * (j - 1) + i - 1;
            let c = w * (j - 1) + i;
            let d = w * j + i;
            b.idx.extend([a, bb, d, bb, c, d]);
        }
    }
    b.mesh()
}

/// `LatheGeometry(points, segments)`: the profile (x = radius, y = height) turned round the y axis.
pub fn lathe(points: &[(f32, f32)], segments: u32) -> Mesh {
    let mut b = Build::default();
    let n = points.len();
    for i in 0..=segments {
        let phi = i as f32 / segments as f32 * TAU;
        let (s, c) = phi.sin_cos();
        for (j, &(x, y)) in points.iter().enumerate() {
            // Perpendicular to the profile's tangent, turned with it.
            let (prev, next) = (points[j.saturating_sub(1)], points[(j + 1).min(n - 1)]);
            let (dx, dy) = (next.0 - prev.0, next.1 - prev.1);
            let uv = Vec2::new(i as f32 / segments as f32, j as f32 / (n - 1) as f32);
            b.vert(Vec3::new(x * s, y, x * c), Vec3::new(dy * s, -dx, dy * c), uv);
        }
    }
    let n = n as u32;
    for i in 0..segments {
        for j in 0..n - 1 {
            let base = j + i * n;
            let (a, bb, c, d) = (base, base + n, base + n + 1, base + 1);
            b.idx.extend([a, bb, d, c, d, bb]);
        }
    }
    b.mesh()
}

/// `CircleGeometry(r, segments)`: a disc in the xy plane facing +z.
pub fn circle(r: f32, segments: u32) -> Mesh {
    let mut b = Build::default();
    b.vert(Vec3::ZERO, Vec3::Z, Vec2::splat(0.5));
    for i in 0..=segments {
        let t = i as f32 / segments as f32 * TAU;
        let (s, c) = t.sin_cos();
        b.vert(
            Vec3::new(r * c, r * s, 0.0),
            Vec3::Z,
            Vec2::new(c * 0.5 + 0.5, s * 0.5 + 0.5),
        );
    }
    for i in 1..=segments {
        b.idx.extend([i, i + 1, 0]);
    }
    b.mesh()
}

fn bezier(a: Vec2, b: Vec2, c: Vec2, d: Vec2, t: f32) -> Vec2 {
    let k = 1.0 - t;
    a * k * k * k + b * 3.0 * k * k * t + c * 3.0 * k * t * t + d * t * t * t
}

/// The heart of the heart glasses, extruded `depth` along +z (TS `heart()` through `ExtrudeGeometry`).
pub fn heart(depth: f32) -> Mesh {
    let v = Vec2::new;
    let curves = [
        [v(0.0, -0.09), v(-0.02, -0.06), v(-0.11, -0.02), v(-0.11, 0.035)],
        [v(-0.11, 0.035), v(-0.11, 0.1), v(-0.03, 0.11), v(0.0, 0.055)],
        [v(0.0, 0.055), v(0.03, 0.11), v(0.11, 0.1), v(0.11, 0.035)],
        [v(0.11, 0.035), v(0.11, -0.02), v(0.02, -0.06), v(0.0, -0.09)],
    ];
    let mut outline = Vec::new();
    for [a, b, c, d] in curves {
        for k in 0..10 {
            outline.push(bezier(a, b, c, d, k as f32 / 10.0));
        }
    }
    let mut b = Build::default();
    let n = outline.len() as u32;
    // Front (+z) and back faces: fans from a point every edge of the outline can see.
    for (z, nz) in [(depth, 1.0), (0.0, -1.0)] {
        let c = b.vert(Vec3::new(0.0, 0.0, z), Vec3::Z * nz, Vec2::splat(0.5));
        let ring: Vec<u32> = outline
            .iter()
            .map(|p| b.vert(p.extend(z), Vec3::Z * nz, *p + 0.5))
            .collect();
        for i in 0..n as usize {
            let (p, q) = (ring[i], ring[(i + 1) % n as usize]);
            // The outline runs clockwise seen from +z.
            if nz > 0.0 {
                b.idx.extend([c, q, p]);
            } else {
                b.idx.extend([c, p, q]);
            }
        }
    }
    for i in 0..n as usize {
        let (p, q) = (outline[i], outline[(i + 1) % n as usize]);
        let e = q - p;
        let nrm = Vec3::new(-e.y, e.x, 0.0);
        let a = b.vert(p.extend(0.0), nrm, Vec2::ZERO);
        let bb = b.vert(q.extend(0.0), nrm, Vec2::X);
        let c = b.vert(q.extend(depth), nrm, Vec2::ONE);
        let d = b.vert(p.extend(depth), nrm, Vec2::Y);
        b.idx.extend([a, c, bb, a, d, c]);
    }
    b.mesh()
}
