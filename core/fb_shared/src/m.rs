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

/// ToInt32 (`x | 0`) for |x| < 2^63 (beyond that the cast saturates where JS wraps); not finite: 0.
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

/// `Math.max(a, b)` on the sign of zero: +0 wins a ±0 tie (`f64::max` may return either operand there, so two
/// builds need not agree). A NaN operand is ignored, as `f64::max` does (JS would return NaN).
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else if b > a {
        b
    } else if a == b {
        // Equal and different only as ±0.
        if a.is_sign_negative() { b } else { a }
    } else if a.is_nan() {
        b
    } else {
        a
    }
}

/// `Math.min(a, b)` on the sign of zero: −0 wins a ±0 tie. A NaN operand is ignored, as `f64::min` does.
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else if b < a {
        b
    } else if a == b {
        if a.is_sign_positive() { b } else { a }
    } else if a.is_nan() {
        b
    } else {
        a
    }
}

/// [`max`] and [`min`] as methods, so `a.max(b).min(c)` keeps its shape as `a.max_js(b).min_js(c)`
/// (`f64::max`/`f64::min` are disallowed in the simulation).
pub trait MinMaxJs {
    fn max_js(self, b: f64) -> f64;
    fn min_js(self, b: f64) -> f64;
}

impl MinMaxJs for f64 {
    #[inline]
    fn max_js(self, b: f64) -> f64 {
        max(self, b)
    }
    #[inline]
    fn min_js(self, b: f64) -> f64 {
        min(self, b)
    }
}

/// `THREE.MathUtils.clamp`: `Math.max(lo, Math.min(hi, x))`.
#[inline]
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    max(lo, min(hi, x))
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

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn max_min_like_js() {
        // ±0 ties, both orders: Math.max gives +0, Math.min gives −0.
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            assert_eq!(bits(max(a, b)), bits(0.0));
            assert_eq!(bits(min(a, b)), bits(-0.0));
            assert_eq!(bits(a.max_js(b)), bits(0.0));
            assert_eq!(bits(a.min_js(b)), bits(-0.0));
        }
        assert_eq!(bits(max(-0.0, -0.0)), bits(-0.0));
        assert_eq!(bits(min(0.0, 0.0)), bits(0.0));
        // NaN on either side is ignored (as f64::max/min).
        assert_eq!(max(f64::NAN, 1.5), 1.5);
        assert_eq!(max(1.5, f64::NAN), 1.5);
        assert_eq!(min(f64::NAN, -2.0), -2.0);
        assert_eq!(min(-2.0, f64::NAN), -2.0);
        assert!(max(f64::NAN, f64::NAN).is_nan() && min(f64::NAN, f64::NAN).is_nan());
        // Ordinary values and infinities.
        assert_eq!(max(1.0, 2.0), 2.0);
        assert_eq!(max(2.0, 1.0), 2.0);
        assert_eq!(min(1.0, 2.0), 1.0);
        assert_eq!(min(2.0, 1.0), 1.0);
        assert_eq!(max(-3.5, -3.25), -3.25);
        assert_eq!(min(3.0, 3.0), 3.0);
        assert_eq!(max(f64::NEG_INFINITY, -1e308), -1e308);
        assert_eq!(max(f64::INFINITY, 1e308), f64::INFINITY);
        assert_eq!(min(f64::NEG_INFINITY, -1e308), f64::NEG_INFINITY);
        assert_eq!(min(f64::INFINITY, 1e308), 1e308);
        assert_eq!(bits(clamp(-0.0, 0.0, 1.0)), bits(0.0));
        assert_eq!(clamp(2.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-2.0, 0.0, 1.0), 0.0);
    }
}
