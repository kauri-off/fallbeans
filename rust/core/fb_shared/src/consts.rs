/// Bumped on every change to a replicated component or message (`fb_net`): old clients cannot connect.
pub const PROTOCOL_VERSION: u32 = 15;

pub const MAX_PLAYERS: usize = 8;

/// Simulation: fixed 120 Hz steps on the server and in client prediction.
pub const TICK_RATE: u32 = 120;
pub const DT: f64 = 1.0 / TICK_RATE as f64;
/// Server snapshots every 4 ticks (30 Hz).
pub const SNAPSHOT_EVERY: u32 = 4;
/// Bot brains decide at 20 Hz.
pub const BOT_EVERY: u32 = 6;
/// Emotes 1…EMOTES: wave, dance, laugh, cry, fright.
pub const EMOTES: u32 = 5;
/// Missing input: the last one is kept this many ticks (never its jump or dive), then idle.
pub const INPUT_HOLD: u32 = 60;

pub const INTRO_S: f64 = 6.0;
pub const RESULTS_S: f64 = 8.0;

pub const BEAN_COLORS: [&str; 8] = [
    "#ff5fa2", "#3fa9ff", "#ffd23f", "#4fdc6a", "#a66bff", "#ff8a3d", "#39e0d0", "#ffffff",
];

#[inline]
pub fn tick_to_time(tick: i64) -> f64 {
    tick as f64 * DT
}
