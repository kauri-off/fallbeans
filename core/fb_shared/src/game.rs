//! What a game is: its description, genre, and how a round treats beans.
use core::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Genre {
    Race,
    Survival,
    Points,
}

impl Genre {
    pub fn label(self) -> &'static str {
        match self {
            Genre::Race => "Гонка",
            Genre::Survival => "Выживание",
            Genre::Points => "Очки",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Genre::Race => "race",
            Genre::Survival => "survival",
            Genre::Points => "points",
        }
    }
}

/// Every map: the games, the lobby and the podium. Its id is a variant index on the wire and the kebab-case
/// name (`as_str`) in text: files, command lines, JSON.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MapId {
    Race,
    JumpClub,
    RollOut,
    WallRush,
    TailTag,
    HexAGone,
    PlateDrop,
    StarFall,
    Lobby,
    Podium,
}

impl MapId {
    pub const ALL: [MapId; 10] = [
        MapId::Race,
        MapId::JumpClub,
        MapId::RollOut,
        MapId::WallRush,
        MapId::TailTag,
        MapId::HexAGone,
        MapId::PlateDrop,
        MapId::StarFall,
        MapId::Lobby,
        MapId::Podium,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            MapId::Race => "race",
            MapId::JumpClub => "jump-club",
            MapId::RollOut => "roll-out",
            MapId::WallRush => "wall-rush",
            MapId::TailTag => "tail-tag",
            MapId::HexAGone => "hex-a-gone",
            MapId::PlateDrop => "plate-drop",
            MapId::StarFall => "star-fall",
            MapId::Lobby => "lobby",
            MapId::Podium => "podium",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

impl fmt::Display for MapId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How many obstacle sections a race may have, and how many it has unless told.
pub const RACE_SECTIONS: core::ops::RangeInclusive<u8> = 3..=8;
pub const RACE_SECTIONS_DEFAULT: u8 = 6;
/// Map seeds (track codes) are below this.
pub const SEED_LIMIT: u32 = 1_000_000_000;

/// A round of a playlist: a game and, for the race, its length; a fixed seed replays one track.
/// In text: `hex-a-gone`, `race`, `race:5`, `race:5:123456`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub map: MapId,
    /// The race's obstacle sections (0 for other maps).
    pub sections: u8,
    pub seed: Option<u32>,
}

impl Pick {
    pub const fn map(map: MapId) -> Self {
        Self {
            map,
            sections: if matches!(map, MapId::Race) {
                RACE_SECTIONS_DEFAULT
            } else {
                0
            },
            seed: None,
        }
    }

    pub const fn race(sections: u8, seed: Option<u32>) -> Self {
        Self {
            map: MapId::Race,
            sections,
            seed,
        }
    }

    /// Within its bounds: sections only for the race, in `RACE_SECTIONS`; a seed below `SEED_LIMIT`.
    pub fn valid(&self) -> bool {
        let sections = if self.map == MapId::Race {
            RACE_SECTIONS.contains(&self.sections)
        } else {
            self.sections == 0
        };
        sections && self.seed.is_none_or(|s| s < SEED_LIMIT)
    }

    /// A race's track code to share (`6-123456789`: its sections and seed).
    pub fn code(&self) -> Option<String> {
        let seed = self.seed.filter(|_| self.map == MapId::Race)?;
        Some(format!("{}-{seed}", self.sections))
    }

    /// The race of a track code (`code`).
    pub fn from_code(s: &str) -> Option<Self> {
        let (n, seed) = s.trim().split_once('-')?;
        let pick = Self::race(n.parse().ok()?, Some(seed.parse().ok()?));
        pick.valid().then_some(pick)
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split(':');
        let map = MapId::parse(parts.next()?)?;
        let mut pick = Self::map(map);
        if let Some(n) = parts.next() {
            pick.sections = n.parse().ok()?;
        }
        if let Some(seed) = parts.next() {
            pick.seed = Some(seed.parse().ok()?);
        }
        (parts.next().is_none() && pick.valid()).then_some(pick)
    }
}

impl fmt::Display for Pick {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.map.as_str())?;
        if self.map == MapId::Race {
            write!(f, ":{}", self.sections)?;
        }
        if let Some(seed) = self.seed {
            write!(f, ":{seed}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GameMeta {
    pub id: MapId,
    pub title: &'static str,
    pub genre: Genre,
    pub desc: &'static str,
    pub goal: &'static str,
    /// Round length in seconds (60–180).
    pub duration: f64,
    pub min_players: u32,
    /// Grab (Q / right mouse) does something special in this game.
    pub grab: bool,
    /// A big, busy map: planned as the last round of a game when possible.
    pub finale: bool,
}

impl GameMeta {
    /// A game description with the optional parts left out (fill them with struct update syntax).
    pub const fn new(
        id: MapId,
        title: &'static str,
        genre: Genre,
        desc: &'static str,
        goal: &'static str,
        duration: f64,
    ) -> Self {
        Self {
            id,
            title,
            genre,
            desc,
            goal,
            duration,
            min_players: 1,
            grab: false,
            finale: false,
        }
    }

    /// A map that is not a game (the lobby, the podium): no description and no end.
    pub const fn place(id: MapId, title: &'static str) -> Self {
        Self::new(id, title, Genre::Points, "", "", f64::INFINITY)
    }

    /// What is wrong with the description; empty when it is fine.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let chars = |s: &str| s.chars().count();
        if !(2..=24).contains(&chars(self.title)) {
            out.push(format!("meta.title: {} characters (2…24)", chars(self.title)));
        }
        if !(10..=220).contains(&chars(self.desc)) {
            out.push(format!("meta.desc: {} characters (10…220)", chars(self.desc)));
        }
        if !(3..=40).contains(&chars(self.goal)) {
            out.push(format!("meta.goal: {} characters (3…40)", chars(self.goal)));
        }
        if self.duration.fract() != 0.0 || !(60.0..=180.0).contains(&self.duration) {
            out.push(format!("meta.duration: {} (a whole number, 60…180)", self.duration));
        }
        if !(1..=8).contains(&self.min_players) {
            out.push("meta.min_players: 1…8".to_string());
        }
        out
    }
}

/// What happens when a bean falls off: back to the last checkpoint, back to its spawn, or out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FallBehaviour {
    Checkpoint,
    Spawn,
    Out,
}

pub fn fall_behaviour(genre: Genre) -> FallBehaviour {
    match genre {
        Genre::Race => FallBehaviour::Checkpoint,
        Genre::Points => FallBehaviour::Spawn,
        Genre::Survival => FallBehaviour::Out,
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaKind {
    Lobby,
    Round,
    Podium,
}

/// Beans stand still before the start of a round (the intro) and on the podium.
pub fn can_move(kind: ArenaKind, t: f64) -> bool {
    kind == ArenaKind::Lobby || (kind == ArenaKind::Round && t >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_read_back_as_written() {
        for p in [
            Pick::map(MapId::HexAGone),
            Pick::map(MapId::Race),
            Pick::race(3, None),
            Pick::race(8, Some(SEED_LIMIT - 1)),
        ] {
            assert_eq!(Pick::parse(&p.to_string()), Some(p), "{p}");
        }
        assert_eq!(Pick::parse("race"), Some(Pick::race(RACE_SECTIONS_DEFAULT, None)));
        for bad in [
            "race:9",
            "race:2",
            "race:5:1000000000",
            "hex-a-gone:4",
            "race:5:1:2",
            "lobby:x",
        ] {
            assert_eq!(Pick::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn track_codes_name_a_race() {
        let p = Pick::race(5, Some(42));
        assert_eq!(p.code().as_deref(), Some("5-42"));
        assert_eq!(Pick::from_code(" 5-42 "), Some(p));
        assert_eq!(Pick::race(5, None).code(), None);
        assert_eq!(Pick::map(MapId::JumpClub).code(), None);
        for bad in ["5", "9-42", "5-x", "5-1000000000"] {
            assert_eq!(Pick::from_code(bad), None, "{bad}");
        }
    }
}
