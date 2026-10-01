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

    pub fn from_stick(x: f64, z: f64, buttons: u8) -> Self {
        Self {
            mx: quantize_axis(x),
            mz: quantize_axis(z),
            buttons: buttons & 7,
        }
    }

    /// The stick clamped to unit length (what the server accepts).
    pub fn clamped(self) -> Self {
        let (x, z) = (self.mx as f64, self.mz as f64);
        let l = crate::m::hypot(x, z);
        let s = if l > 127.0 { 127.0 / l } else { 1.0 };
        Self {
            mx: crate::m::round_js(x * s).clamp(-127.0, 127.0) as i8,
            mz: crate::m::round_js(z * s).clamp(-127.0, 127.0) as i8,
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
    let q = crate::m::round_js(v * 127.0);
    if q.is_nan() { 0 } else { q.clamp(-127.0, 127.0) as i8 }
}
