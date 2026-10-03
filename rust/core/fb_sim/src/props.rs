use crate::m;
use crate::math::V3;

/// A rotor angle that starts at `start`, eases up to `w` rad/s over about `ease` s and keeps speeding up by `acc`.
#[derive(Clone, Copy, Debug)]
pub struct SpinUp {
    pub start: f64,
    pub w: f64,
    pub acc: f64,
    pub ease: f64,
}

impl SpinUp {
    pub fn new(start: f64, w: f64, acc: f64) -> Self {
        Self {
            start,
            w,
            acc,
            ease: 1.5,
        }
    }

    pub fn angle(&self, t: f64) -> f64 {
        if t <= 0.0 {
            self.start
        } else {
            self.start + (self.w * t * t) / (t + self.ease) + self.acc * t * t
        }
    }

    pub fn omega(&self, t: f64) -> f64 {
        if t <= 0.0 {
            0.2
        } else {
            let e = t + self.ease;
            (0.2f64).max((self.w * t * (t + 2.0 * self.ease)) / (e * e) + 2.0 * self.acc * t)
        }
    }
}

/// Seconds until a rotor arm sweeps over (x, z).
pub fn sweep_eta(x: f64, z: f64, angle: f64, omega: f64, arms: u32, cx: f64, cz: f64) -> f64 {
    let phi = m::atan2(-(z - cz), x - cx);
    let period = m::TAU / arms as f64;
    let mut d = (phi - angle) % period;
    if d < 0.0 {
        d += period;
    }
    if omega > 0.0 { d / omega } else { (period - d) / -omega }
}

/// Seconds until a rotor arm (half thickness `half`) first touches a bean at `pos`; negative while touching.
pub fn arm_contact_eta(pos: V3, angle: f64, omega: f64, arms: u32, cx: f64, cz: f64, half: f64) -> f64 {
    let r = m::hypot(pos.x - cx, pos.z - cz).max(0.5);
    let margin = (half + 0.55) / r / omega.abs();
    let eta = sweep_eta(pos.x, pos.z, angle, omega, arms, cx, cz);
    let period = (m::PI * 2.0) / arms as f64 / omega.abs();
    // Just passed: still touching until the arm clears the other side.
    if eta > period - margin {
        eta - period - margin
    } else {
        eta - margin
    }
}
