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

use core::fmt;

use bevy::ecs::entity::Entity;
use fb_proto::{MapEventMsg, Pid, ServerMsg};
use fb_shared::TICK_RATE;
use fb_shared::input::InputFrame;

/// A connection: the network layer's link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub Entity);

/// A player's identity: the same person on every connection they make (`auth::Auth::identity`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(String);

impl Uid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Uid {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl From<String> for Uid {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

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

/// Server ticks in seconds.
pub fn secs(ticks: u64) -> f64 {
    ticks as f64 / TICK_RATE as f64
}

/// A warning that backs off while it keeps coming: logged at once, then at most after 1 s, 2 s, 4 s … (up to
/// 256 s); quiet for a whole step and it starts over.
#[derive(Clone, Debug, Default)]
pub struct Backoff {
    /// Time (s) before which it is only counted.
    next: f64,
    step: u32,
    hushed: u32,
}

impl Backoff {
    fn period(step: u32) -> f64 {
        f64::from(1u32 << step.min(8))
    }

    /// It happened at `now` (s, any origin): Some(times it was left out since the last line) when to log it.
    pub fn hit(&mut self, now: f64) -> Option<u32> {
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

/// Messages counted per second of server ticks.
#[derive(Clone, Debug, Default)]
pub struct RateWindow {
    start: u64,
    count: u32,
}

impl RateWindow {
    /// One more at server tick `now`: how many there were in its window so far, this one included.
    pub fn hit(&mut self, now: u64) -> u32 {
        if now.saturating_sub(self.start) > ticks(1.0) {
            self.start = now;
            self.count = 0;
        }
        self.count += 1;
        self.count
    }
}
