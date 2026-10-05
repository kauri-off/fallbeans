//! Every map: one module per map, and the registry.
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

/// Every game map, in the order of the list in the menu (the lobby and the podium are not games).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_are_the_games_then_the_lobby_and_the_podium() {
        let ids = |list: &[&dyn MapDef]| list.iter().map(|m| m.meta().id).collect::<Vec<_>>();
        let games = ids(GAMES);
        let mut all = games.clone();
        all.extend(["lobby", "podium"]);
        assert_eq!(ids(MAPS), all);
        let mut uniq = all.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), all.len(), "a map id is used twice");
        for id in all {
            assert_eq!(by_id(id).map(|m| m.meta().id), Some(id));
        }
        assert_eq!(lobby::META.id, "lobby");
        assert_eq!(podium::META.id, "podium");
    }
}
