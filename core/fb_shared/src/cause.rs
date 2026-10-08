//! What knocks a bean off.
use serde::{Deserialize, Serialize};

/// A collider that knocks beans off (`ColliderOpts::tag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hazard {
    Hammer,
    Rotor,
    Ball,
    Bumper,
    Wall,
    Pusher,
    Drum,
    Peg,
    Block,
    Glove,
    Gate,
}

impl Hazard {
    pub fn name(self) -> &'static str {
        match self {
            Hazard::Hammer => "hammer",
            Hazard::Rotor => "rotor",
            Hazard::Ball => "ball",
            Hazard::Bumper => "bumper",
            Hazard::Wall => "wall",
            Hazard::Pusher => "pusher",
            Hazard::Drum => "drum",
            Hazard::Peg => "peg",
            Hazard::Block => "block",
            Hazard::Glove => "glove",
            Hazard::Gate => "gate",
        }
    }
}

/// What knocked a bean off the course.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cause {
    Fall,
    Shortcut,
    Tackle,
    Grab,
    /// A dev command.
    Dev,
    Hazard(Hazard),
}

impl Cause {
    /// Its name in the journal and the state hash.
    pub fn name(self) -> &'static str {
        match self {
            Cause::Fall => "fall",
            Cause::Shortcut => "shortcut",
            Cause::Tackle => "tackle",
            Cause::Grab => "grab",
            Cause::Dev => "dev",
            Cause::Hazard(h) => h.name(),
        }
    }
}

impl core::fmt::Display for Cause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
