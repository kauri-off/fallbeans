/// Bumped on every change to a replicated component or message (`fb_net`): old clients cannot connect.
pub const PROTOCOL_VERSION: u32 = 19;

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
