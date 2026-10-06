/// Bumped on every change to a replicated component or message (`fb_net`): old clients cannot connect.
pub const WIRE_VERSION: u32 = 21;

/// The simulation's fingerprint: FNV-1a of `fb_arena/tests/determinism.txt`, the recorded end states of full
/// rounds on every map. A change to the physics, a map or the bots changes a hash there (`cargo xtask check`
/// fails until it is re-blessed), and with it this. Line endings do not count (a CRLF checkout).
pub const SIM_FINGERPRINT: u32 = fnv1a_lines(include_bytes!("../../fb_arena/tests/determinism.txt"));

/// What client and server compare (session request and reply, `/health`): the wire version, then six digits
/// of the simulation's fingerprint (21_123456). A client and a server built from different simulation code
/// refuse to play together as surely as across a wire change: the client builds the map and predicts with
/// its own code. Never edited by hand: bump `WIRE_VERSION`, re-bless the determinism hashes.
pub const PROTOCOL_VERSION: u32 = WIRE_VERSION * 1_000_000 + SIM_FINGERPRINT % 1_000_000;

const fn fnv1a_lines(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
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

/// The suit colour that runs through the rainbow.
pub const RAINBOW: &str = "rainbow";
/// Bean colours (an index on the wire): hex, or RAINBOW. The first eight go to newcomers.
pub const COLORS: [&str; 13] = [
    "#ff5fa2", "#3fa9ff", "#ffd23f", "#4fdc6a", "#a66bff", "#ff8a3d", "#39e0d0", "#ffffff", "#ff3b3b", "#2b2b33",
    "#9ea3b0", "#8b5a2b", RAINBOW,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_carries_the_wire_version_and_the_simulation() {
        assert_eq!(PROTOCOL_VERSION / 1_000_000, WIRE_VERSION);
        assert_eq!(PROTOCOL_VERSION % 1_000_000, SIM_FINGERPRINT % 1_000_000);
        assert_eq!(fnv1a_lines(b"a 1\r\nb 2\r\n"), fnv1a_lines(b"a 1\nb 2\n"));
        assert_ne!(fnv1a_lines(b"a 1\n"), fnv1a_lines(b"a 2\n"));
    }
}
