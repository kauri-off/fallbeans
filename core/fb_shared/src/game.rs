//! What a game is (port of `shared/game.ts`): its description, genre, and how a round treats beans.
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

#[derive(Clone, Copy, Debug)]
pub struct GameMeta {
    pub id: &'static str,
    pub title: &'static str,
    pub genre: Genre,
    pub desc: &'static str,
    pub goal: &'static str,
    /// Round length in seconds (60–180).
    pub duration: f64,
    pub min_players: Option<u32>,
    /// Grab (Q / right mouse) does something special in this game.
    pub grab: bool,
    /// A big, busy map: planned as the last round of a game when possible.
    pub finale: bool,
}

impl GameMeta {
    /// A game description with the optional parts left out (fill them with struct update syntax).
    pub const fn new(
        id: &'static str,
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
            min_players: None,
            grab: false,
            finale: false,
        }
    }

    /// What `GameMetaSchema` checks in TS; empty when the description is fine.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let id_ok = self.id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !id_ok {
            out.push("meta.id: lowercase letters, digits and dashes".to_string());
        }
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
        if self.min_players.is_some_and(|n| !(1..=8).contains(&n)) {
            out.push("meta.minPlayers: 1…8".to_string());
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
