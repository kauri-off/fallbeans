/// FNV-1a of `fb_arena/tests/determinism.txt`; changes when that file is re-blessed.
use crate::{Rgb, rgb};
pub const SIM_FINGERPRINT: u32 = fnv1a_lines(FNV_SEED, include_bytes!("../../fb_arena/tests/determinism.txt"));

pub const FNV_SEED: u32 = 0x811c_9dc5;

/// A time long before anything (s): «never» for a timer compared with the clock.
pub const NEVER: f64 = -1e9;

/// FNV-1a of a text file, `\r` skipped, continuing from `seed` (`FNV_SEED` to start).
pub const fn fnv1a_lines(seed: u32, bytes: &[u8]) -> u32 {
    let mut h = seed;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\r' {
            h = (h ^ bytes[i] as u32).wrapping_mul(0x0100_0193);
        }
        i += 1;
    }
    h
}

pub const MAX_PLAYERS: usize = 8;

/// Simulation: fixed 120 Hz steps on the server and in client prediction.
pub const TICK_RATE: u32 = 120;
pub const DT: f64 = 1.0 / TICK_RATE as f64;
/// Bot brains decide at 20 Hz (`fb_sim::bots::BOT_DT` is their step).
pub const BOT_EVERY: u32 = 6;
/// Emotes 1…EMOTES: wave, dance, laugh, cry, fright.
pub const EMOTES: u32 = 5;
/// Missing input: the last one is kept this many ticks (never its jump or dive), then idle.
pub const INPUT_HOLD: u32 = 60;

pub const INTRO_S: f64 = 6.0;
pub const RESULTS_S: f64 = 8.0;
pub const PRACTICE_RESULTS_S: f64 = 3.5;
pub const PODIUM_S: f64 = 20.0;
/// A player who lost the connection keeps their place in a game this long.
pub const RECONNECT_GRACE_S: f64 = 30.0;
pub const MAX_PRACTICE_ROOMS: usize = 3;
/// Rooms open at once (each simulates its own lobby or round).
pub const MAX_ROOMS: usize = 16;
/// A room nobody is in is closed after this long (its owner may be restarting the game).
pub const ROOM_EMPTY_S: f64 = 30.0;
/// Dev servers keep one room open under this id for the tools.
pub const DEV_ROOM_ID: &str = "dev";
pub const ROOM_TITLE_MAX: usize = 24;
pub const ROOM_PIN_DIGITS: usize = 4;
pub const CHAT_MAX: usize = 160;
pub const NAME_MAX: usize = 16;

/// A bean's suit colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suit {
    Color(Rgb),
    /// Runs through the rainbow.
    Rainbow,
}

/// Bean colours (an index on the wire). The first eight go to newcomers.
pub const COLORS: [Suit; 13] = [
    Suit::Color(rgb(0xff5fa2)),
    Suit::Color(rgb(0x3fa9ff)),
    Suit::Color(rgb(0xffd23f)),
    Suit::Color(rgb(0x4fdc6a)),
    Suit::Color(rgb(0xa66bff)),
    Suit::Color(rgb(0xff8a3d)),
    Suit::Color(rgb(0x39e0d0)),
    Suit::Color(rgb(0xffffff)),
    Suit::Color(rgb(0xff3b3b)),
    Suit::Color(rgb(0x2b2b33)),
    Suit::Color(rgb(0x9ea3b0)),
    Suit::Color(rgb(0x8b5a2b)),
    Suit::Rainbow,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_skip_carriage_returns() {
        let f = |b: &[u8]| fnv1a_lines(FNV_SEED, b);
        assert_eq!(f(b"a 1\r\nb 2\r\n"), f(b"a 1\nb 2\n"));
        assert_ne!(f(b"a 1\n"), f(b"a 2\n"));
    }
}
