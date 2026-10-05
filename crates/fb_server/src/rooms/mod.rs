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

/// A random number from the system (room codes, PINs, seeds: not simulation).
pub fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("system randomness");
    u32::from_le_bytes(b)
}
