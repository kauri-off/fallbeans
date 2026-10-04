//! Every map (port of `src/games`): one module per map, and the registry.
use fb_sim::map::MapDef;

pub mod ball_hill;
pub mod bounce_park;
pub mod cliff_climb;
pub mod crown_peak;
pub mod director;
pub mod door_dash;
pub mod drum_roll;
pub mod frost_sky;
pub mod hammer_swing;
pub mod hex_a_gone;
pub mod hidden_bridge;
pub mod jump_club;
pub mod lobby;
pub mod plate_drop;
pub mod podium;
pub mod portal_panic;
pub mod roll_out;
pub mod star_fall;
pub mod tail_tag;
mod util;
pub mod wall_rush;

/// Every game map, in the order of `src/games/index.ts` (the lobby and the podium are not games).
pub static GAMES: &[&dyn MapDef] = &[
    &door_dash::DoorDash,
    &hammer_swing::HammerSwing,
    &ball_hill::BallHill,
    &hidden_bridge::HiddenBridge,
    &drum_roll::DrumRoll,
    &jump_club::JumpClub,
    &roll_out::RollOut,
    &wall_rush::WallRush,
    &tail_tag::TailTag,
    &hex_a_gone::HexAGone,
    &crown_peak::CrownPeak,
    &plate_drop::PlateDrop,
    &portal_panic::PortalPanic,
    &bounce_park::BouncePark,
    &cliff_climb::CliffClimb,
    &frost_sky::FrostSky,
    &star_fall::StarFall,
];

/// Every map, the lobby and the podium included.
pub static MAPS: &[&dyn MapDef] = &[
    &door_dash::DoorDash,
    &hammer_swing::HammerSwing,
    &ball_hill::BallHill,
    &hidden_bridge::HiddenBridge,
    &drum_roll::DrumRoll,
    &jump_club::JumpClub,
    &roll_out::RollOut,
    &wall_rush::WallRush,
    &tail_tag::TailTag,
    &hex_a_gone::HexAGone,
    &crown_peak::CrownPeak,
    &plate_drop::PlateDrop,
    &portal_panic::PortalPanic,
    &bounce_park::BouncePark,
    &cliff_climb::CliffClimb,
    &frost_sky::FrostSky,
    &star_fall::StarFall,
    &lobby::Lobby,
    &podium::Podium,
];

pub fn by_id(id: &str) -> Option<&'static dyn MapDef> {
    MAPS.iter().copied().find(|m| m.meta().id == id)
}
