//! glam's f64 types; use `euler_xyz` for angles, since glam's `sin_cos` calls the platform libm.
use crate::m;
pub use glam::{DAffine3 as Affine, DQuat, DVec3 as V3, dvec3 as v3};

/// Rotation by Euler angles about x, then y, then z (each about the axes already turned).
pub fn euler_xyz(r: V3) -> DQuat {
    let (c1, c2, c3) = (m::cos(r.x / 2.0), m::cos(r.y / 2.0), m::cos(r.z / 2.0));
    let (s1, s2, s3) = (m::sin(r.x / 2.0), m::sin(r.y / 2.0), m::sin(r.z / 2.0));
    DQuat::from_xyzw(
        s1 * c2 * c3 + c1 * s2 * s3,
        c1 * s2 * c3 - s1 * c2 * s3,
        c1 * c2 * s3 + s1 * s2 * c3,
        c1 * c2 * c3 - s1 * s2 * s3,
    )
}

/// A node's local transform: scaled, turned (Euler XYZ), then moved to `p`.
pub fn compose(p: V3, rot: V3, s: V3) -> Affine {
    Affine::from_scale_rotation_translation(s, euler_xyz(rot), p)
}

/// Horizontal distance between `a` and `b`.
#[inline]
pub fn dist_xz(a: V3, b: V3) -> f64 {
    m::hypot(a.x - b.x, a.z - b.z)
}
