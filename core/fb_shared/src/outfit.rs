//! What a bean wears besides its suit colour (visual only).
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hat {
    #[default]
    None,
    Cap,
    Beanie,
    Party,
    Tophat,
    Cowboy,
    Viking,
    Propeller,
    Bunny,
    Cat,
    Horns,
    Halo,
    Flower,
    Antenna,
}

pub const HATS: [Hat; 14] = [
    Hat::None,
    Hat::Cap,
    Hat::Beanie,
    Hat::Party,
    Hat::Tophat,
    Hat::Cowboy,
    Hat::Viking,
    Hat::Propeller,
    Hat::Bunny,
    Hat::Cat,
    Hat::Horns,
    Hat::Halo,
    Hat::Flower,
    Hat::Antenna,
];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Glasses {
    #[default]
    None,
    Round,
    Shades,
    Hearts,
    Monocle,
    Visor,
}

pub const GLASSES: [Glasses; 6] = [
    Glasses::None,
    Glasses::Round,
    Glasses::Shades,
    Glasses::Hearts,
    Glasses::Monocle,
    Glasses::Visor,
];

/// Colours of the hat, the belly and the shoes (`TINTS[tint as usize]`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    White,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    Blue,
    Teal,
    Green,
    Brown,
    Grey,
    Black,
}

pub const TINTS: [&str; 12] = [
    "#ffffff", "#ffd23f", "#ff8a3d", "#ff3b3b", "#ff5fa2", "#a66bff", "#3fa9ff", "#39e0d0", "#4fdc6a", "#8b5a2b",
    "#9ea3b0", "#2b2b33",
];

pub const TINT_LIST: [Tint; 12] = [
    Tint::White,
    Tint::Yellow,
    Tint::Orange,
    Tint::Red,
    Tint::Pink,
    Tint::Purple,
    Tint::Blue,
    Tint::Teal,
    Tint::Green,
    Tint::Brown,
    Tint::Grey,
    Tint::Black,
];

impl Tint {
    pub fn hex(self) -> &'static str {
        TINTS[self as usize]
    }
}

/// None of a tint leaves that part its own default colour.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outfit {
    pub hat: Hat,
    pub hat_color: Option<Tint>,
    pub glasses: Glasses,
    pub belly: Option<Tint>,
    pub shoes: Option<Tint>,
}

/// A bot's outfit: picked from its id, so the same bot always looks the same.
pub fn bot_outfit(id: u32) -> Outfit {
    let h = |n: u32| {
        let x = (id.wrapping_mul(31).wrapping_add(n + 1) ^ 0x5bd1_e995).wrapping_mul(0x9e37_79b1);
        let y = (x ^ (x >> 15)).wrapping_mul(0x85eb_ca77);
        y ^ (y >> 13)
    };
    let pick = |n: u32, len: usize| h(n) as usize % len;
    Outfit {
        hat: if h(0).is_multiple_of(3) {
            Hat::None
        } else {
            HATS[pick(1, HATS.len())]
        },
        hat_color: if h(2) % 2 == 1 {
            None
        } else {
            Some(TINT_LIST[pick(3, TINTS.len())])
        },
        glasses: if !h(4).is_multiple_of(3) {
            Glasses::None
        } else {
            GLASSES[pick(5, GLASSES.len())]
        },
        belly: None,
        shoes: if h(6) % 2 == 1 {
            None
        } else {
            Some(TINT_LIST[pick(7, TINTS.len())])
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_outfits_match_ts() {
        let o = |hat, hat_color, glasses, shoes| Outfit {
            hat,
            hat_color,
            glasses,
            belly: None,
            shoes,
        };
        assert_eq!(
            bot_outfit(1),
            o(Hat::Halo, Some(Tint::Brown), Glasses::None, Some(Tint::Red))
        );
        assert_eq!(
            bot_outfit(2),
            o(Hat::Cap, Some(Tint::Pink), Glasses::Shades, Some(Tint::Grey))
        );
        assert_eq!(bot_outfit(5), o(Hat::None, None, Glasses::Hearts, None));
        assert_eq!(
            bot_outfit(9),
            o(Hat::Viking, Some(Tint::Black), Glasses::Monocle, Some(Tint::Teal))
        );
    }
}
