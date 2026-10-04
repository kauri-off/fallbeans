//! Vector and matrix operations in exactly the order three.js does them (results match the TS build).
use crate::m;
pub use glam::DVec3 as V3;

/// Column-major 4×4 matrix, laid out like three.js `Matrix4.elements`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M4(pub [f64; 16]);

impl Default for M4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl M4 {
    pub const IDENTITY: M4 = M4([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);

    /// `Matrix4.compose(position, quaternion from Euler XYZ, scale)`.
    pub fn compose(p: V3, rot: V3, s: V3) -> M4 {
        let (c1, c2, c3) = (m::cos(rot.x / 2.0), m::cos(rot.y / 2.0), m::cos(rot.z / 2.0));
        let (s1, s2, s3) = (m::sin(rot.x / 2.0), m::sin(rot.y / 2.0), m::sin(rot.z / 2.0));
        let x = s1 * c2 * c3 + c1 * s2 * s3;
        let y = c1 * s2 * c3 - s1 * c2 * s3;
        let z = c1 * c2 * s3 + s1 * s2 * c3;
        let w = c1 * c2 * c3 - s1 * s2 * s3;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        M4([
            (1.0 - (yy + zz)) * s.x,
            (xy + wz) * s.x,
            (xz - wy) * s.x,
            0.0,
            (xy - wz) * s.y,
            (1.0 - (xx + zz)) * s.y,
            (yz + wx) * s.y,
            0.0,
            (xz + wy) * s.z,
            (yz - wx) * s.z,
            (1.0 - (xx + yy)) * s.z,
            0.0,
            p.x,
            p.y,
            p.z,
            1.0,
        ])
    }

    /// `multiplyMatrices(self, b)`.
    pub fn mul(&self, b: &M4) -> M4 {
        let a = &self.0;
        let b = &b.0;
        let mut t = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                t[col * 4 + row] = a[row] * b[col * 4]
                    + a[4 + row] * b[col * 4 + 1]
                    + a[8 + row] * b[col * 4 + 2]
                    + a[12 + row] * b[col * 4 + 3];
            }
        }
        M4(t)
    }

    /// `Matrix4.invert()`.
    pub fn invert(&self) -> M4 {
        let te = &self.0;
        let (n11, n21, n31, n41) = (te[0], te[1], te[2], te[3]);
        let (n12, n22, n32, n42) = (te[4], te[5], te[6], te[7]);
        let (n13, n23, n33, n43) = (te[8], te[9], te[10], te[11]);
        let (n14, n24, n34, n44) = (te[12], te[13], te[14], te[15]);
        let t1 = n11 * n22 - n21 * n12;
        let t2 = n11 * n32 - n31 * n12;
        let t3 = n11 * n42 - n41 * n12;
        let t4 = n21 * n32 - n31 * n22;
        let t5 = n21 * n42 - n41 * n22;
        let t6 = n31 * n42 - n41 * n32;
        let t7 = n13 * n24 - n23 * n14;
        let t8 = n13 * n34 - n33 * n14;
        let t9 = n13 * n44 - n43 * n14;
        let t10 = n23 * n34 - n33 * n24;
        let t11 = n23 * n44 - n43 * n24;
        let t12 = n33 * n44 - n43 * n34;
        let det = t1 * t12 - t2 * t11 + t3 * t10 + t4 * t9 - t5 * t8 + t6 * t7;
        if det == 0.0 {
            return M4([0.0; 16]);
        }
        let d = 1.0 / det;
        M4([
            (n22 * t12 - n32 * t11 + n42 * t10) * d,
            (n31 * t11 - n21 * t12 - n41 * t10) * d,
            (n24 * t6 - n34 * t5 + n44 * t4) * d,
            (n33 * t5 - n23 * t6 - n43 * t4) * d,
            (n32 * t9 - n12 * t12 - n42 * t8) * d,
            (n11 * t12 - n31 * t9 + n41 * t8) * d,
            (n34 * t3 - n14 * t6 - n44 * t2) * d,
            (n13 * t6 - n33 * t3 + n43 * t2) * d,
            (n12 * t11 - n22 * t9 + n42 * t7) * d,
            (n21 * t9 - n11 * t11 - n41 * t7) * d,
            (n14 * t5 - n24 * t3 + n44 * t1) * d,
            (n23 * t3 - n13 * t5 - n43 * t1) * d,
            (n22 * t8 - n12 * t10 - n32 * t7) * d,
            (n11 * t10 - n21 * t8 + n31 * t7) * d,
            (n24 * t2 - n14 * t4 - n34 * t1) * d,
            (n13 * t4 - n23 * t2 + n33 * t1) * d,
        ])
    }

    /// `Vector3.applyMatrix4`.
    #[inline]
    pub fn apply_point(&self, v: V3) -> V3 {
        let e = &self.0;
        let w = 1.0 / (e[3] * v.x + e[7] * v.y + e[11] * v.z + e[15]);
        V3::new(
            (e[0] * v.x + e[4] * v.y + e[8] * v.z + e[12]) * w,
            (e[1] * v.x + e[5] * v.y + e[9] * v.z + e[13]) * w,
            (e[2] * v.x + e[6] * v.y + e[10] * v.z + e[14]) * w,
        )
    }

    /// `Vector3.transformDirection` (normalized).
    #[inline]
    pub fn transform_dir(&self, v: V3) -> V3 {
        let e = &self.0;
        normalize(V3::new(
            e[0] * v.x + e[4] * v.y + e[8] * v.z,
            e[1] * v.x + e[5] * v.y + e[9] * v.z,
            e[2] * v.x + e[6] * v.y + e[10] * v.z,
        ))
    }

    #[inline]
    pub fn position(&self) -> V3 {
        V3::new(self.0[12], self.0[13], self.0[14])
    }
}

#[inline]
pub fn len(v: V3) -> f64 {
    m::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
}

#[inline]
pub fn len_sq(v: V3) -> f64 {
    v.x * v.x + v.y * v.y + v.z * v.z
}

#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

#[inline]
pub fn dist_sq(a: V3, b: V3) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    dx * dx + dy * dy + dz * dz
}

/// `divideScalar`: three multiplies by the reciprocal.
#[inline]
pub fn div_s(v: V3, s: f64) -> V3 {
    let k = 1.0 / s;
    V3::new(v.x * k, v.y * k, v.z * k)
}

#[inline]
pub fn normalize(v: V3) -> V3 {
    let l = len(v);
    div_s(v, if l == 0.0 || l.is_nan() { 1.0 } else { l })
}

/// `addScaledVector`.
#[inline]
pub fn add_scaled(a: V3, v: V3, s: f64) -> V3 {
    V3::new(a.x + v.x * s, a.y + v.y * s, a.z + v.z * s)
}

#[inline]
pub fn lerp(a: V3, b: V3, k: f64) -> V3 {
    V3::new(a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k, a.z + (b.z - a.z) * k)
}
