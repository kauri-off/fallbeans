use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use fb_shared::input::InputFrame;
use serde::{Deserialize, Serialize};

/// One tick of a player's input as Lightyear carries it (the same three bytes as `InputFrame`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct FbInput {
    pub mx: i8,
    pub mz: i8,
    pub buttons: u8,
}

impl MapEntities for FbInput {
    fn map_entities<M: EntityMapper>(&mut self, _: &mut M) {}
}

impl From<FbInput> for InputFrame {
    fn from(i: FbInput) -> Self {
        InputFrame {
            mx: i.mx,
            mz: i.mz,
            buttons: i.buttons,
        }
    }
}

impl From<InputFrame> for FbInput {
    fn from(f: InputFrame) -> Self {
        FbInput {
            mx: f.mx,
            mz: f.mz,
            buttons: f.buttons,
        }
    }
}
