//! Rooms and who is where, as plain Rust (no network): the network layer feeds connections, messages
//! and inputs in and sends what comes out (`Out`). Tests drive it the same way.
pub mod awards;
pub mod clock;
pub mod debug;
pub mod hub;
pub mod players;
pub mod room;
#[cfg(test)]
mod tests;

use fb_proto::{MapEventMsg, Pid, ServerMsg};
use fb_shared::TICK_RATE;
use fb_shared::input::InputFrame;

/// A connection (the network layer's link).
pub type ConnId = u64;

/// Something to send.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Msg(ConnId, ServerMsg),
    Event(ConnId, MapEventMsg),
    /// The connection stands for nobody any more: close it (after what was sent to it).
    Close(ConnId),
}

/// Where the players' inputs come from.
pub trait Inputs {
    /// Player `id`'s frame for server tick `tick`.
    fn frame(&mut self, id: Pid, conn: ConnId, tick: u32) -> InputFrame;
}

/// Nobody presses anything (warps, tests).
pub struct NoInputs;

impl Inputs for NoInputs {
    fn frame(&mut self, _: Pid, _: ConnId, _: u32) -> InputFrame {
        InputFrame::IDLE
    }
}

/// Seconds in server ticks.
pub fn ticks(s: f64) -> u64 {
    (s * TICK_RATE as f64).round() as u64
}

/// A warning that backs off while it keeps coming: logged at once, then at most after 1 s, 2 s, 4 s … (up to
/// 256 s); quiet for a whole step and it starts over.
#[derive(Clone, Debug, Default)]
pub struct Backoff {
    /// Server tick before which it is only counted.
    next: u64,
    step: u32,
    hushed: u32,
}

impl Backoff {
    fn period(step: u32) -> u64 {
        ticks(1.0) << step.min(8)
    }

    /// It happened at server tick `now`: Some(times it was left out since the last line) when to log it.
    pub fn hit(&mut self, now: u64) -> Option<u32> {
        if now < self.next {
            self.hushed += 1;
            return None;
        }
        if self.step > 0 && now >= self.next + Self::period(self.step) {
            self.step = 0;
        }
        self.next = now + Self::period(self.step);
        self.step += 1;
        Some(core::mem::take(&mut self.hushed))
    }
}

/// A random number from the system (room codes, PINs, seeds: not simulation).
pub fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("system randomness");
    u32::from_le_bytes(b)
}
