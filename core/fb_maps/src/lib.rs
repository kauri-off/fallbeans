//! Every map: one module per map (the race's sections in `race`), and the registry.
#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value
)]
use fb_shared::game::RACE_SECTIONS_DEFAULT;
use fb_sim::map::{MapDef, MapId};

pub mod director;
pub mod hex_a_gone;
pub mod jump_club;
pub mod lobby;
pub mod plate_drop;
pub mod podium;
pub mod race;
pub mod roll_out;
pub mod star_fall;
pub mod tail_tag;
mod util;
pub mod wall_rush;

/// Every game map, in the order of the list in the menu (the lobby and the podium are not games).
pub static GAMES: &[&dyn MapDef] = &[
    race::of(RACE_SECTIONS_DEFAULT),
    &jump_club::JumpClub,
    &roll_out::RollOut,
    &wall_rush::WallRush,
    &tail_tag::TailTag,
    &hex_a_gone::HexAGone,
    &plate_drop::PlateDrop,
    &star_fall::StarFall,
];

/// Every map, the lobby and the podium included.
pub static MAPS: &[&dyn MapDef] = &[
    race::of(RACE_SECTIONS_DEFAULT),
    &jump_club::JumpClub,
    &roll_out::RollOut,
    &wall_rush::WallRush,
    &tail_tag::TailTag,
    &hex_a_gone::HexAGone,
    &plate_drop::PlateDrop,
    &star_fall::StarFall,
    &lobby::Lobby,
    &podium::Podium,
];

/// The map of this id (the race of its default length).
pub fn by_id(id: MapId) -> &'static dyn MapDef {
    match id {
        MapId::Race => race::of(RACE_SECTIONS_DEFAULT),
        MapId::JumpClub => &jump_club::JumpClub,
        MapId::RollOut => &roll_out::RollOut,
        MapId::WallRush => &wall_rush::WallRush,
        MapId::TailTag => &tail_tag::TailTag,
        MapId::HexAGone => &hex_a_gone::HexAGone,
        MapId::PlateDrop => &plate_drop::PlateDrop,
        MapId::StarFall => &star_fall::StarFall,
        MapId::Lobby => &lobby::Lobby,
        MapId::Podium => &podium::Podium,
    }
}

/// The map of this id with this many sections (the race's length; other maps ignore it).
pub fn get(map: MapId, sections: u8) -> &'static dyn MapDef {
    match map {
        MapId::Race => race::of(sections),
        _ => by_id(map),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_are_the_games_then_the_lobby_and_the_podium() {
        let ids = |list: &[&dyn MapDef]| list.iter().map(|m| m.meta().id).collect::<Vec<_>>();
        let games = ids(GAMES);
        let mut all = games;
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
