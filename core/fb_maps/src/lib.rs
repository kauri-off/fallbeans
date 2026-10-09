//! Every map: one module per map, and the registry.
use fb_sim::map::{MapDef, MapId};

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

pub fn by_id(id: MapId) -> &'static dyn MapDef {
    match id {
        MapId::DoorDash => &door_dash::DoorDash,
        MapId::HammerSwing => &hammer_swing::HammerSwing,
        MapId::BallHill => &ball_hill::BallHill,
        MapId::HiddenBridge => &hidden_bridge::HiddenBridge,
        MapId::DrumRoll => &drum_roll::DrumRoll,
        MapId::JumpClub => &jump_club::JumpClub,
        MapId::RollOut => &roll_out::RollOut,
        MapId::WallRush => &wall_rush::WallRush,
        MapId::TailTag => &tail_tag::TailTag,
        MapId::HexAGone => &hex_a_gone::HexAGone,
        MapId::CrownPeak => &crown_peak::CrownPeak,
        MapId::PlateDrop => &plate_drop::PlateDrop,
        MapId::PortalPanic => &portal_panic::PortalPanic,
        MapId::BouncePark => &bounce_park::BouncePark,
        MapId::CliffClimb => &cliff_climb::CliffClimb,
        MapId::FrostSky => &frost_sky::FrostSky,
        MapId::StarFall => &star_fall::StarFall,
        MapId::Lobby => &lobby::Lobby,
        MapId::Podium => &podium::Podium,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_are_the_games_then_the_lobby_and_the_podium() {
        let ids = |list: &[&dyn MapDef]| list.iter().map(|m| m.meta().id).collect::<Vec<_>>();
        let games = ids(GAMES);
        let mut all = games.clone();
        all.extend([MapId::Lobby, MapId::Podium]);
        assert_eq!(ids(MAPS), all);
        assert_eq!(all, MapId::ALL, "every map id has its map, in the order of MapId");
        for id in all {
            assert_eq!(by_id(id).meta().id, id);
            assert_eq!(MapId::parse(id.as_str()), Some(id));
            assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{id}\""));
        }
    }
}
