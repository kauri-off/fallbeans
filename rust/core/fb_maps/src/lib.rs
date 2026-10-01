use fb_sim::map::MapDef;

pub mod jump_club;

/// Every map, by id.
pub static MAPS: &[&dyn MapDef] = &[&jump_club::JumpClub];

pub fn by_id(id: &str) -> Option<&'static dyn MapDef> {
    MAPS.iter().copied().find(|m| m.meta().id == id)
}
