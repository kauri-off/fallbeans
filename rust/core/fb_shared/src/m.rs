//! The only maths the simulation may use: libm (same bits on every OS) and JS-exact helpers.
#![allow(clippy::disallowed_methods, clippy::excessive_precision)]

pub const PI: f64 = core::f64::consts::PI;
pub const TAU: f64 = core::f64::consts::TAU;
pub const SQRT2: f64 = core::f64::consts::SQRT_2;

#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
#[inline]
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}
#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}
/// `Math.pow` / `**` as JavaScriptCore computes them: a whole exponent up to 1000 by repeated squaring
/// (`x ** 3` is `x * (x * x)`, exactly), anything else by libm.
pub fn pow(x: f64, y: f64) -> f64 {
    if y.fract() == 0.0 && (0.0..=1000.0).contains(&y) {
        let mut n = y as u32;
        let (mut base, mut result) = (x, 1.0);
        while n != 0 {
            if n & 1 != 0 {
                result *= base;
            }
            base *= base;
            n >>= 1;
        }
        return result;
    }
    libm::pow(x, y)
}
/// IEEE square root is exact on every platform.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// `Math.hypot` as V8 computes it (scaled, Kahan-summed); Bun takes two arguments to the platform libm.
pub fn hypot_n(v: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &x in v {
        let a = x.abs();
        if a.is_nan() {
            nan = true;
        } else if a > max {
            max = a;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut comp = 0.0;
    for &x in v {
        let n = x.abs() / max;
        let summand = n * n - comp;
        let pre = sum + summand;
        comp = (pre - sum) - summand;
        sum = pre;
    }
    sqrt(sum) * max
}

#[inline]
pub fn hypot(a: f64, b: f64) -> f64 {
    hypot_n(&[a, b])
}

#[inline]
pub fn hypot3(a: f64, b: f64, c: f64) -> f64 {
    hypot_n(&[a, b, c])
}

/// `Math.sign` (0 stays 0, NaN stays NaN).
#[inline]
pub fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// `Math.round`: ties go towards +∞.
#[inline]
pub fn round_js(x: f64) -> f64 {
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    // Rounding up to zero from below gives −0 in JS (a stick of −0 turns a bean the other way round).
    if r == 0.0 && x < 0.0 { -0.0 } else { r }
}

/// ToInt32 of a finite number (`x | 0`).
#[inline]
pub fn to_i32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    (x.trunc() as i64) as i32
}

/// `Math.fround` (what a `Float32Array` keeps): IEEE rounding to f32, the same on every platform.
#[inline]
#[allow(clippy::disallowed_types)]
pub fn fround(x: f64) -> f64 {
    x as f32 as f64
}

#[inline]
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_like_v8() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
        assert_eq!(hypot3(1.0, 2.0, 2.0), 3.0);
        assert_eq!(hypot(1e-3, 0.7), 0.7000007142853498);
        assert_eq!(hypot3(0.1, 0.2, 0.3), 0.37416573867739417);
        assert_eq!(round_js(-0.5), 0.0);
        assert!(round_js(-0.3).is_sign_negative() && round_js(-0.0).is_sign_negative());
        assert!(round_js(0.3).is_sign_positive());
        assert_eq!(round_js(2.5), 3.0);
    }
}
