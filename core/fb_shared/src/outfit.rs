//! What a bean wears besides its suit colour (visual only).
use serde::{Deserialize, Serialize};

use crate::{PlayerId, Rgb, rgb};

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

/// Colours of the hat, the belly and the shoes.
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

impl Tint {
    pub const ALL: [Tint; 12] = [
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

    pub const fn rgb(self) -> Rgb {
        match self {
            Tint::White => rgb(0xffffff),
            Tint::Yellow => rgb(0xffd23f),
            Tint::Orange => rgb(0xff8a3d),
            Tint::Red => rgb(0xff3b3b),
            Tint::Pink => rgb(0xff5fa2),
            Tint::Purple => rgb(0xa66bff),
            Tint::Blue => rgb(0x3fa9ff),
            Tint::Teal => rgb(0x39e0d0),
            Tint::Green => rgb(0x4fdc6a),
            Tint::Brown => rgb(0x8b5a2b),
            Tint::Grey => rgb(0x9ea3b0),
            Tint::Black => rgb(0x2b2b33),
        }
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
pub fn bot_outfit(id: PlayerId) -> Outfit {
    let h = |n: u32| {
        let x = (id.0.wrapping_mul(31).wrapping_add(n + 1) ^ 0x5bd1_e995).wrapping_mul(0x9e37_79b1);
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
            Some(Tint::ALL[pick(3, Tint::ALL.len())])
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
            Some(Tint::ALL[pick(7, Tint::ALL.len())])
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_outfits_are_stable() {
        let o = |hat, hat_color, glasses, shoes| Outfit {
            hat,
            hat_color,
            glasses,
            belly: None,
            shoes,
        };
        assert_eq!(
            bot_outfit(PlayerId(1)),
            o(Hat::Halo, Some(Tint::Brown), Glasses::None, Some(Tint::Red))
        );
        assert_eq!(
            bot_outfit(PlayerId(2)),
            o(Hat::Cap, Some(Tint::Pink), Glasses::Shades, Some(Tint::Grey))
        );
        assert_eq!(bot_outfit(PlayerId(5)), o(Hat::None, None, Glasses::Hearts, None));
        assert_eq!(
            bot_outfit(PlayerId(9)),
            o(Hat::Viking, Some(Tint::Black), Glasses::Monocle, Some(Tint::Teal))
        );
    }
}
