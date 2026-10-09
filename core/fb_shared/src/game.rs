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
    DoorDash,
    HammerSwing,
    BallHill,
    HiddenBridge,
    DrumRoll,
    JumpClub,
    RollOut,
    WallRush,
    TailTag,
    HexAGone,
    CrownPeak,
    PlateDrop,
    PortalPanic,
    BouncePark,
    CliffClimb,
    FrostSky,
    StarFall,
    Lobby,
    Podium,
}

impl MapId {
    pub const ALL: [MapId; 19] = [
        MapId::DoorDash,
        MapId::HammerSwing,
        MapId::BallHill,
        MapId::HiddenBridge,
        MapId::DrumRoll,
        MapId::JumpClub,
        MapId::RollOut,
        MapId::WallRush,
        MapId::TailTag,
        MapId::HexAGone,
        MapId::CrownPeak,
        MapId::PlateDrop,
        MapId::PortalPanic,
        MapId::BouncePark,
        MapId::CliffClimb,
        MapId::FrostSky,
        MapId::StarFall,
        MapId::Lobby,
        MapId::Podium,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            MapId::DoorDash => "door-dash",
            MapId::HammerSwing => "hammer-swing",
            MapId::BallHill => "ball-hill",
            MapId::HiddenBridge => "hidden-bridge",
            MapId::DrumRoll => "drum-roll",
            MapId::JumpClub => "jump-club",
            MapId::RollOut => "roll-out",
            MapId::WallRush => "wall-rush",
            MapId::TailTag => "tail-tag",
            MapId::HexAGone => "hex-a-gone",
            MapId::CrownPeak => "crown-peak",
            MapId::PlateDrop => "plate-drop",
            MapId::PortalPanic => "portal-panic",
            MapId::BouncePark => "bounce-park",
            MapId::CliffClimb => "cliff-climb",
            MapId::FrostSky => "frost-sky",
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
