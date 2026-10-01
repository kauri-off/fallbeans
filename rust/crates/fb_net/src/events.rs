use serde::{Deserialize, Serialize};

/// Map events with the tick they happened at (clients apply them on that tick).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum MapEventKind {
    Bonus { i: u32, id: u32, at: f64 },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MapEventMsg {
    pub round: u32,
    pub tick: u32,
    pub ev: MapEventKind,
}

pub struct MapEventsChannel;
