//! The only maths the simulation may use: libm (same bits on every OS) and helpers that give the same bits
//! in every build.
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
#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}
/// IEEE square root is exact on every platform.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// Length of (a, b): no scaling, the game's numbers are far from overflow.
#[inline]
pub fn hypot(a: f64, b: f64) -> f64 {
    sqrt(a * a + b * b)
}

#[inline]
pub fn hypot3(a: f64, b: f64, c: f64) -> f64 {
    sqrt(a * a + b * b + c * c)
}

/// −1, 0 or 1 by the sign of `x` (unlike `f64::signum`, 0 stays 0; NaN stays NaN).
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

/// The larger of `a` and `b`, with +0 winning a ±0 tie (`f64::max` may return either operand there, so two
/// builds need not agree). A NaN operand is ignored, as `f64::max` does.
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

/// The smaller of `a` and `b`, with −0 winning a ±0 tie. A NaN operand is ignored, as `f64::min` does.
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

/// [`max`] and [`min`] as methods (`f64::max`/`f64::min` are disallowed in the simulation): `x.at_least(0.0)`
/// is `max(x, 0.0)`, `x.at_most(1.0)` is `min(x, 1.0)`.
pub trait MinMax {
    fn at_least(self, lo: f64) -> f64;
    fn at_most(self, hi: f64) -> f64;
}

impl MinMax for f64 {
    #[inline]
    fn at_least(self, lo: f64) -> f64 {
        max(self, lo)
    }
    #[inline]
    fn at_most(self, hi: f64) -> f64 {
        min(self, hi)
    }
}

/// `x` limited to [lo, hi] (`max(lo, min(hi, x))`).
#[inline]
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    max(lo, min(hi, x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
        assert_eq!(hypot3(1.0, 2.0, 2.0), 3.0);
    }

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn max_min_settle_zero_ties() {
        // ±0 ties, both orders: max gives +0, min gives −0.
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            assert_eq!(bits(max(a, b)), bits(0.0));
            assert_eq!(bits(min(a, b)), bits(-0.0));
            assert_eq!(bits(a.at_least(b)), bits(0.0));
            assert_eq!(bits(a.at_most(b)), bits(-0.0));
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
