use core::fmt;

use serde::{Deserialize, Serialize};

/// A player's id in a room (small, sequential; not the network id): humans and bots alike.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<PlayerId> for u64 {
    fn from(id: PlayerId) -> Self {
        Self::from(id.0)
    }
}
