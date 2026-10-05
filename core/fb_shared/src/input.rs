use serde::{Deserialize, Serialize};

pub const BTN_JUMP: u8 = 1;
pub const BTN_DIVE: u8 = 2;
pub const BTN_GRAB: u8 = 4;

/// One simulation tick of player input: world-space stick quantized to −127…127, buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InputFrame {
    pub mx: i8,
    pub mz: i8,
    pub buttons: u8,
}

impl InputFrame {
    pub const IDLE: InputFrame = InputFrame {
        mx: 0,
        mz: 0,
        buttons: 0,
    };

    /// A stick of at most unit length, quantized and clamped as the server takes it (see `clamped`).
    pub fn from_stick(x: f64, z: f64, buttons: u8) -> Self {
        Self {
            mx: quantize_axis(x),
            mz: quantize_axis(z),
            buttons: buttons & 7,
        }
        .clamped()
    }

    /// The stick clamped to unit length (127), as the server applies it to every frame. Idempotent: a
    /// client sending clamped frames predicts with exactly the input the server uses. (Rounding a
    /// rescaled stick may leave it a hair over 127; scaling again would then move it once more.)
    pub fn clamped(self) -> Self {
        const MAX: i32 = 127 * 127;
        let len2 = |x: i8, z: i8| i32::from(x) * i32::from(x) + i32::from(z) * i32::from(z);
        let (mut mx, mut mz) = (self.mx, self.mz);
        if len2(mx, mz) > MAX {
            let (x, z) = (f64::from(mx), f64::from(mz));
            let s = 127.0 / crate::m::hypot(x, z);
            mx = (x * s).round().clamp(-127.0, 127.0) as i8;
            mz = (z * s).round().clamp(-127.0, 127.0) as i8;
            while len2(mx, mz) > MAX {
                if mx.unsigned_abs() >= mz.unsigned_abs() {
                    mx -= mx.signum();
                } else {
                    mz -= mz.signum();
                }
            }
        }
        Self {
            mx,
            mz,
            buttons: self.buttons & 7,
        }
    }

    pub fn jump(self) -> bool {
        self.buttons & BTN_JUMP != 0
    }
    pub fn dive(self) -> bool {
        self.buttons & BTN_DIVE != 0
    }
    pub fn grab(self) -> bool {
        self.buttons & BTN_GRAB != 0
    }
}

pub fn quantize_axis(v: f64) -> i8 {
    let q = (v * 127.0).round();
    if q.is_nan() { 0 } else { q.clamp(-127.0, 127.0) as i8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping_is_idempotent_and_within_reach() {
        for mx in i8::MIN..=i8::MAX {
            for mz in i8::MIN..=i8::MAX {
                let c = InputFrame { mx, mz, buttons: 0 }.clamped();
                assert_eq!(c.clamped(), c, "{mx} {mz}");
                let l2 = i32::from(c.mx).pow(2) + i32::from(c.mz).pow(2);
                assert!(l2 <= 127 * 127, "{mx} {mz} -> {c:?}");
                if i32::from(mx).pow(2) + i32::from(mz).pow(2) <= 127 * 127 {
                    assert_eq!((c.mx, c.mz), (mx, mz), "a stick within reach is kept");
                }
            }
        }
        // The stick the stress run caught: 127.7 long, sent as is, moved by the server.
        assert_eq!(InputFrame::from_stick(83.0 / 127.0, -97.0 / 127.0, 0).clamped().mz, -96);
    }
}
