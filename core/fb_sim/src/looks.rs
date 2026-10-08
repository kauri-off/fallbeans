//! How a map looks: its colours (the palettes its parts are painted with), the
//! patterns on them, the sky, sun, fog and ambient light, the land far below and the scenery around it.
//! Each map has a few looks (the first is its signature); a round picks one by its seed and shifts the
//! colours a little. Visual only: nothing here touches colliders or the map's layout.
use fb_shared::m::{self, MinMax};
use fb_shared::rng::Rng;
use fb_shared::{Rgb, rgb};

use crate::scene::{Palette, pal};

/// The palettes maps paint with; a look gives each its own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Swatch {
    Blue,
    Purple,
    Pink,
    Yellow,
    Green,
    White,
    Orange,
    Red,
    Teal,
}

impl Swatch {
    pub const ALL: [Swatch; 9] = [
        Swatch::Blue,
        Swatch::Purple,
        Swatch::Pink,
        Swatch::Yellow,
        Swatch::Green,
        Swatch::White,
        Swatch::Orange,
        Swatch::Red,
        Swatch::Teal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Swatch::Blue => "blue",
            Swatch::Purple => "purple",
            Swatch::Pink => "pink",
            Swatch::Yellow => "yellow",
            Swatch::Green => "green",
            Swatch::White => "white",
            Swatch::Orange => "orange",
            Swatch::Red => "red",
            Swatch::Teal => "teal",
        }
    }

    /// The palette in the classic look (`scene::pal`).
    pub const fn classic(self) -> Palette {
        match self {
            Swatch::Blue => pal::BLUE,
            Swatch::Purple => pal::PURPLE,
            Swatch::Pink => pal::PINK,
            Swatch::Yellow => pal::YELLOW,
            Swatch::Green => pal::GREEN,
            Swatch::White => pal::WHITE,
            Swatch::Orange => pal::ORANGE,
            Swatch::Red => pal::RED,
            Swatch::Teal => pal::TEAL,
        }
    }

    /// The swatch a classic palette is, if it is one.
    pub fn of(p: Palette) -> Option<Swatch> {
        Swatch::ALL.into_iter().find(|s| s.classic() == p)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookId {
    Classic,
    Meadow,
    Castle,
    Factory,
    Snow,
    Starlight,
    Circus,
    Neon,
    Ocean,
    Desert,
    Jungle,
    Lava,
    Royal,
    Candy,
}

impl LookId {
    pub fn name(self) -> &'static str {
        match self {
            LookId::Classic => "classic",
            LookId::Meadow => "meadow",
            LookId::Castle => "castle",
            LookId::Factory => "factory",
            LookId::Snow => "snow",
            LookId::Starlight => "starlight",
            LookId::Circus => "circus",
            LookId::Neon => "neon",
            LookId::Ocean => "ocean",
            LookId::Desert => "desert",
            LookId::Jungle => "jungle",
            LookId::Lava => "lava",
            LookId::Royal => "royal",
            LookId::Candy => "candy",
        }
    }

    pub fn look(self) -> &'static Look {
        &LOOKS[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern {
    Stripes,
    Checker,
    Dots,
    Chevron,
    Waves,
}

impl Pattern {
    pub fn name(self) -> &'static str {
        match self {
            Pattern::Stripes => "stripes",
            Pattern::Checker => "checker",
            Pattern::Dots => "dots",
            Pattern::Chevron => "chevron",
            Pattern::Waves => "waves",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub top: Rgb,
    pub horizon: Rgb,
    /// Tint of the clouds.
    pub cloud: Rgb,
    /// Stars in the sky, 0…1 (night looks).
    pub stars: f64,
}

/// The sun (or moon): colour, strength, compass angle and height in degrees.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub color: Rgb,
    pub intensity: f64,
    pub azimuth: f64,
    pub elevation: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Hemi {
    pub sky: Rgb,
    pub ground: Rgb,
    pub intensity: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Fog {
    pub color: Rgb,
    pub near: f64,
    pub far: f64,
}

/// Specks in the air: colour and vertical drift (m/s; negative falls, like snow).
#[derive(Clone, Copy, Debug)]
pub struct Motes {
    pub color: Rgb,
    pub rise: f64,
}

/// The land far below the course; `glow` makes it shine (lava).
#[derive(Clone, Copy, Debug)]
pub struct Ground {
    pub c1: Rgb,
    pub c2: Rgb,
    pub kind: Pattern,
    pub freq: f64,
    pub speed: f64,
    pub glow: bool,
}

/// The floating islands: grass (top), rock (underside) and bush colours.
#[derive(Clone, Copy, Debug)]
pub struct Island {
    pub grass: Rgb,
    pub rock: Rgb,
    pub leaves: Rgb,
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub id: LookId,
    /// Base colour of each palette (`Swatch` order; the lighter second tone is derived).
    pub colors: [Rgb; 9],
    pub patterns: &'static [Pattern],
    pub sky: Sky,
    pub sun: Sun,
    pub hemi: Hemi,
    pub fog: Fog,
    pub exposure: f64,
    pub saturation: f64,
    /// Image-based light strength.
    pub env: f64,
    pub motes: Motes,
    pub ground: Option<Ground>,
    pub island: Island,
    /// Birds and hot-air balloons on the horizon.
    pub birds: bool,
    pub balloons: bool,
}

pub const LOOKS: &[Look] = &[
    Look {
        id: LookId::Classic,
        colors: [
            rgb(0x7ccfff),
            rgb(0xa98bff),
            rgb(0xff8cc8),
            rgb(0xffd84a),
            rgb(0x6fe08a),
            rgb(0xf4f1ff),
            rgb(0xff9f4a),
            rgb(0xff6070),
            rgb(0x39e0d0),
        ],
        patterns: &[Pattern::Stripes],
        sky: Sky {
            top: rgb(0x6fb8ff),
            horizon: rgb(0xffd9f2),
            cloud: rgb(0xffffff),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff1dc),
            intensity: 2.2,
            azimuth: 38.0,
            elevation: 74.0,
        },
        hemi: Hemi {
            sky: rgb(0xcfe8ff),
            ground: rgb(0xb99be0),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0xf1d4f7),
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.06,
        env: 0.3,
        motes: Motes {
            color: rgb(0xfff7e0),
            rise: 0.12,
        },
        ground: None,
        island: Island {
            grass: rgb(0x6fd46a),
            rock: rgb(0x9b7a5e),
            leaves: rgb(0x4fbf5a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Meadow,
        colors: [
            rgb(0x6ec3ff),
            rgb(0xb39cff),
            rgb(0xff9ccf),
            rgb(0xffe066),
            rgb(0x7fe07a),
            rgb(0xf7f5ea),
            rgb(0xffae5c),
            rgb(0xff6f6f),
            rgb(0x4fe0c4),
        ],
        patterns: &[Pattern::Dots, Pattern::Waves, Pattern::Stripes],
        sky: Sky {
            top: rgb(0x58aefc),
            horizon: rgb(0xe6f7ff),
            cloud: rgb(0xffffff),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff4d6),
            intensity: 2.3,
            azimuth: 60.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: rgb(0xd8efff),
            ground: rgb(0x9fce8a),
            intensity: 0.95,
        },
        fog: Fog {
            color: rgb(0xe2f2fb),
            near: 130.0,
            far: 560.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.3,
        motes: Motes {
            color: rgb(0xfffbe0),
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: rgb(0x7ccf6a),
            c2: rgb(0x93dc7c),
            kind: Pattern::Waves,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x6fd46a),
            rock: rgb(0x9b7a5e),
            leaves: rgb(0x4fbf5a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Castle,
        colors: [
            rgb(0x6f8fd9),
            rgb(0x8e79c9),
            rgb(0xd98ab0),
            rgb(0xf2c94c),
            rgb(0x79b87a),
            rgb(0xe9e4da),
            rgb(0xe39a5b),
            rgb(0xd9534f),
            rgb(0x5bbfb0),
        ],
        patterns: &[Pattern::Checker, Pattern::Stripes, Pattern::Chevron],
        sky: Sky {
            top: rgb(0x6fa6e0),
            horizon: rgb(0xf6e3c8),
            cloud: rgb(0xfff8ee),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xffe8c8),
            intensity: 2.3,
            azimuth: 20.0,
            elevation: 70.0,
        },
        hemi: Hemi {
            sky: rgb(0xdbe6ff),
            ground: rgb(0xa89a86),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0xeee4d6),
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.02,
        env: 0.3,
        motes: Motes {
            color: rgb(0xfff2d8),
            rise: 0.1,
        },
        ground: Some(Ground {
            c1: rgb(0x6fae5a),
            c2: rgb(0x86c26c),
            kind: Pattern::Checker,
            freq: 0.012,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x78bf62),
            rock: rgb(0x8f8a86),
            leaves: rgb(0x5aa860),
        },
        birds: true,
        balloons: false,
    },
    Look {
        id: LookId::Factory,
        colors: [
            rgb(0x5f8fb8),
            rgb(0x7f7fa8),
            rgb(0xd07a8a),
            rgb(0xf5c542),
            rgb(0x7fa86a),
            rgb(0xd8dde3),
            rgb(0xf08a3a),
            rgb(0xe0523e),
            rgb(0x4fb5ac),
        ],
        patterns: &[Pattern::Chevron, Pattern::Stripes, Pattern::Checker],
        sky: Sky {
            top: rgb(0x7f9cbc),
            horizon: rgb(0xf2d6ae),
            cloud: rgb(0xe4ddd2),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xffdcb0),
            intensity: 2.2,
            azimuth: 120.0,
            elevation: 66.0,
        },
        hemi: Hemi {
            sky: rgb(0xdfe6ee),
            ground: rgb(0x8a7a6a),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0xe6d8c4),
            near: 90.0,
            far: 430.0,
        },
        exposure: 0.95,
        saturation: 0.98,
        env: 0.35,
        motes: Motes {
            color: rgb(0xffe0b0),
            rise: 0.25,
        },
        ground: Some(Ground {
            c1: rgb(0x5b6270),
            c2: rgb(0x6a7280),
            kind: Pattern::Checker,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x8a8f78),
            rock: rgb(0x6b6660),
            leaves: rgb(0x7a8a5a),
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: LookId::Snow,
        colors: [
            rgb(0x8fd0ff),
            rgb(0xb7b0ff),
            rgb(0xffb7d9),
            rgb(0xfff0a0),
            rgb(0x9fe0c0),
            rgb(0xffffff),
            rgb(0xffc38a),
            rgb(0xff7f8f),
            rgb(0x8ff0ea),
        ],
        patterns: &[Pattern::Waves, Pattern::Dots, Pattern::Chevron],
        sky: Sky {
            top: rgb(0x8ec6f5),
            horizon: rgb(0xf4fbff),
            cloud: rgb(0xffffff),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xf4f8ff),
            intensity: 2.0,
            azimuth: 200.0,
            elevation: 68.0,
        },
        hemi: Hemi {
            sky: rgb(0xe6f4ff),
            ground: rgb(0xc8d8f0),
            intensity: 1.0,
        },
        fog: Fog {
            color: rgb(0xeef6ff),
            near: 90.0,
            far: 420.0,
        },
        exposure: 0.92,
        saturation: 1.0,
        env: 0.35,
        motes: Motes {
            color: rgb(0xffffff),
            rise: -1.1,
        },
        ground: Some(Ground {
            c1: rgb(0xf4f9ff),
            c2: rgb(0xdfeefa),
            kind: Pattern::Waves,
            freq: 0.015,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0xf6fbff),
            rock: rgb(0x8a9bb0),
            leaves: rgb(0xe8f4ff),
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: LookId::Starlight,
        colors: [
            rgb(0x6f86ff),
            rgb(0x9a6bff),
            rgb(0xff7ad9),
            rgb(0xffd86b),
            rgb(0x5fe0a8),
            rgb(0xdfe6ff),
            rgb(0xff9f6b),
            rgb(0xff5f87),
            rgb(0x46e0e6),
        ],
        patterns: &[Pattern::Dots, Pattern::Checker, Pattern::Waves],
        sky: Sky {
            top: rgb(0x0b1238),
            horizon: rgb(0x46307a),
            cloud: rgb(0x5a4a8a),
            stars: 1.0,
        },
        sun: Sun {
            color: rgb(0xc8d4ff),
            intensity: 1.7,
            azimuth: 300.0,
            elevation: 70.0,
        },
        hemi: Hemi {
            sky: rgb(0x9fb0ff),
            ground: rgb(0x4a3a7e),
            intensity: 0.95,
        },
        fog: Fog {
            color: rgb(0x2e2860),
            near: 110.0,
            far: 480.0,
        },
        exposure: 1.0,
        saturation: 1.1,
        env: 0.4,
        motes: Motes {
            color: rgb(0xbfe8ff),
            rise: 0.08,
        },
        ground: Some(Ground {
            c1: rgb(0x1a1f4a),
            c2: rgb(0x283070),
            kind: Pattern::Dots,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x4a5aa0),
            rock: rgb(0x2e2a58),
            leaves: rgb(0x6a7ae0),
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: LookId::Circus,
        colors: [
            rgb(0x4fa3ff),
            rgb(0xa06bff),
            rgb(0xff6fb0),
            rgb(0xffd23f),
            rgb(0x4fdc6a),
            rgb(0xfff8ef),
            rgb(0xff8a3d),
            rgb(0xff4d5a),
            rgb(0x2fd3c4),
        ],
        patterns: &[Pattern::Stripes, Pattern::Dots, Pattern::Chevron],
        sky: Sky {
            top: rgb(0x58b9ff),
            horizon: rgb(0xffe9c9),
            cloud: rgb(0xfffaf0),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff0d0),
            intensity: 2.3,
            azimuth: 80.0,
            elevation: 73.0,
        },
        hemi: Hemi {
            sky: rgb(0xcfe8ff),
            ground: rgb(0xb99be0),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0xfbe8d6),
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.12,
        env: 0.3,
        motes: Motes {
            color: rgb(0xfff0c8),
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: rgb(0xffe3b0),
            c2: rgb(0xffd08a),
            kind: Pattern::Stripes,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x6fd46a),
            rock: rgb(0x9b7a5e),
            leaves: rgb(0x4fbf5a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Neon,
        colors: [
            rgb(0x3fd0ff),
            rgb(0xb45cff),
            rgb(0xff4fcf),
            rgb(0xfff04f),
            rgb(0x4fff9f),
            rgb(0xe8e0ff),
            rgb(0xff9a3f),
            rgb(0xff4f6f),
            rgb(0x2ff5e0),
        ],
        patterns: &[Pattern::Checker, Pattern::Chevron, Pattern::Stripes],
        sky: Sky {
            top: rgb(0x1b0f3d),
            horizon: rgb(0xff5fa2),
            cloud: rgb(0x7a3a8a),
            stars: 0.6,
        },
        sun: Sun {
            color: rgb(0xffc0ec),
            intensity: 1.9,
            azimuth: 180.0,
            elevation: 66.0,
        },
        hemi: Hemi {
            sky: rgb(0xb0a0ff),
            ground: rgb(0x5a2a70),
            intensity: 0.95,
        },
        fog: Fog {
            color: rgb(0x5a2a6a),
            near: 100.0,
            far: 460.0,
        },
        exposure: 1.0,
        saturation: 1.15,
        env: 0.4,
        motes: Motes {
            color: rgb(0xff9ff0),
            rise: 0.2,
        },
        ground: Some(Ground {
            c1: rgb(0x1a0f33),
            c2: rgb(0x44207a),
            kind: Pattern::Checker,
            freq: 0.04,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x3a2a7a),
            rock: rgb(0x20143e),
            leaves: rgb(0xff4fcf),
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: LookId::Ocean,
        colors: [
            rgb(0x3fb6ff),
            rgb(0x8f8fff),
            rgb(0xff9fbf),
            rgb(0xffe27a),
            rgb(0x5fe0a0),
            rgb(0xfffaf0),
            rgb(0xffb06a),
            rgb(0xff7070),
            rgb(0x2fe0d0),
        ],
        patterns: &[Pattern::Waves, Pattern::Stripes, Pattern::Dots],
        sky: Sky {
            top: rgb(0x45b0ff),
            horizon: rgb(0xe0fbff),
            cloud: rgb(0xffffff),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff6e0),
            intensity: 2.4,
            azimuth: 250.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: rgb(0xd0f0ff),
            ground: rgb(0x8fd0d8),
            intensity: 0.95,
        },
        fog: Fog {
            color: rgb(0xd8f4fc),
            near: 130.0,
            far: 560.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.35,
        motes: Motes {
            color: rgb(0xffffff),
            rise: 0.1,
        },
        ground: Some(Ground {
            c1: rgb(0x1f9fd6),
            c2: rgb(0x37c1ec),
            kind: Pattern::Waves,
            freq: 0.035,
            speed: 0.4,
            glow: false,
        }),
        island: Island {
            grass: rgb(0xf2dca8),
            rock: rgb(0xb09070),
            leaves: rgb(0x4fcf6a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Desert,
        colors: [
            rgb(0x6fb6d9),
            rgb(0xb08fc9),
            rgb(0xe8a0a0),
            rgb(0xf2cf6b),
            rgb(0xa8c46a),
            rgb(0xf6ead2),
            rgb(0xe8944a),
            rgb(0xd8654a),
            rgb(0x5fc0a8),
        ],
        patterns: &[Pattern::Waves, Pattern::Chevron, Pattern::Stripes],
        sky: Sky {
            top: rgb(0x6cb2ea),
            horizon: rgb(0xffe0b0),
            cloud: rgb(0xfff4e0),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xffe2b8),
            intensity: 2.6,
            azimuth: 140.0,
            elevation: 76.0,
        },
        hemi: Hemi {
            sky: rgb(0xe0ecff),
            ground: rgb(0xd8b080),
            intensity: 0.85,
        },
        fog: Fog {
            color: rgb(0xf6e2c4),
            near: 110.0,
            far: 500.0,
        },
        exposure: 0.93,
        saturation: 1.04,
        env: 0.3,
        motes: Motes {
            color: rgb(0xffe8c0),
            rise: 0.3,
        },
        ground: Some(Ground {
            c1: rgb(0xe8c690),
            c2: rgb(0xf0d6a8),
            kind: Pattern::Waves,
            freq: 0.012,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0xe8c690),
            rock: rgb(0xb8764a),
            leaves: rgb(0x8fae5a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Jungle,
        colors: [
            rgb(0x5fb8e0),
            rgb(0x9f7fd9),
            rgb(0xff8fb0),
            rgb(0xffe05f),
            rgb(0x4fcf5f),
            rgb(0xf0f8e8),
            rgb(0xffa04f),
            rgb(0xff6050),
            rgb(0x3fd0a0),
        ],
        patterns: &[Pattern::Dots, Pattern::Waves, Pattern::Chevron],
        sky: Sky {
            top: rgb(0x62c0f8),
            horizon: rgb(0xe4ffdc),
            cloud: rgb(0xf8fff4),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff4c8),
            intensity: 2.2,
            azimuth: 330.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: rgb(0xdcffe8),
            ground: rgb(0x6a9a5a),
            intensity: 0.95,
        },
        fog: Fog {
            color: rgb(0xdff4dc),
            near: 100.0,
            far: 470.0,
        },
        exposure: 0.95,
        saturation: 1.1,
        env: 0.3,
        motes: Motes {
            color: rgb(0xf4ffc0),
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: rgb(0x2f8f4f),
            c2: rgb(0x3fa85f),
            kind: Pattern::Dots,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x4fbf4a),
            rock: rgb(0x7a5a3e),
            leaves: rgb(0x2f9f3f),
        },
        birds: true,
        balloons: false,
    },
    Look {
        id: LookId::Lava,
        colors: [
            rgb(0x6a7fb0),
            rgb(0x7a5a9a),
            rgb(0xe0708a),
            rgb(0xffc84a),
            rgb(0x8aa06a),
            rgb(0xe8dcd0),
            rgb(0xff7a2a),
            rgb(0xe8402a),
            rgb(0x4aa8a0),
        ],
        patterns: &[Pattern::Chevron, Pattern::Checker, Pattern::Waves],
        sky: Sky {
            top: rgb(0x3a2a3e),
            horizon: rgb(0xff8a4a),
            cloud: rgb(0x6a4a4a),
            stars: 0.2,
        },
        sun: Sun {
            color: rgb(0xffb888),
            intensity: 2.2,
            azimuth: 90.0,
            elevation: 68.0,
        },
        hemi: Hemi {
            sky: rgb(0xffd0b0),
            ground: rgb(0x6a2a2a),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0x8a4a3a),
            near: 90.0,
            far: 420.0,
        },
        exposure: 1.0,
        saturation: 1.06,
        env: 0.35,
        motes: Motes {
            color: rgb(0xffa040),
            rise: 1.0,
        },
        ground: Some(Ground {
            c1: rgb(0xff5a1a),
            c2: rgb(0xffb030),
            kind: Pattern::Waves,
            freq: 0.03,
            speed: 0.25,
            glow: true,
        }),
        island: Island {
            grass: rgb(0x4a3a3a),
            rock: rgb(0x2a2020),
            leaves: rgb(0x8a4a2a),
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: LookId::Royal,
        colors: [
            rgb(0x6fa8ff),
            rgb(0x9a6bff),
            rgb(0xff8fc0),
            rgb(0xffd23f),
            rgb(0x6fd08a),
            rgb(0xfff6e6),
            rgb(0xffa04a),
            rgb(0xff5a6a),
            rgb(0x4fd6c8),
        ],
        patterns: &[Pattern::Chevron, Pattern::Checker, Pattern::Stripes],
        sky: Sky {
            top: rgb(0x6a98e6),
            horizon: rgb(0xffc89a),
            cloud: rgb(0xfff0e0),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xffd8a8),
            intensity: 2.3,
            azimuth: 260.0,
            elevation: 67.0,
        },
        hemi: Hemi {
            sky: rgb(0xffe8d8),
            ground: rgb(0xb88ac0),
            intensity: 0.9,
        },
        fog: Fog {
            color: rgb(0xf6d8c8),
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.35,
        motes: Motes {
            color: rgb(0xffe8a0),
            rise: 0.2,
        },
        ground: Some(Ground {
            c1: rgb(0x9a6bd0),
            c2: rgb(0xb48ae0),
            kind: Pattern::Chevron,
            freq: 0.015,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0x6fd46a),
            rock: rgb(0x9b7a5e),
            leaves: rgb(0x4fbf5a),
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: LookId::Candy,
        colors: [
            rgb(0x8fd8ff),
            rgb(0xc8a8ff),
            rgb(0xff9fd0),
            rgb(0xffec8f),
            rgb(0xa8f0b0),
            rgb(0xfff6fb),
            rgb(0xffc08f),
            rgb(0xff7fa0),
            rgb(0x8ff0e0),
        ],
        patterns: &[Pattern::Dots, Pattern::Checker, Pattern::Stripes],
        sky: Sky {
            top: rgb(0xff9fd6),
            horizon: rgb(0xfff0f8),
            cloud: rgb(0xfff0fb),
            stars: 0.0,
        },
        sun: Sun {
            color: rgb(0xfff0f4),
            intensity: 2.2,
            azimuth: 10.0,
            elevation: 74.0,
        },
        hemi: Hemi {
            sky: rgb(0xffe8f6),
            ground: rgb(0xd8b0f0),
            intensity: 1.0,
        },
        fog: Fog {
            color: rgb(0xffe4f2),
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.93,
        saturation: 1.05,
        env: 0.3,
        motes: Motes {
            color: rgb(0xffffff),
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: rgb(0xffc2e0),
            c2: rgb(0xffffff),
            kind: Pattern::Checker,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: rgb(0xffb0d8),
            rock: rgb(0xc88a6a),
            leaves: rgb(0xa8f0b0),
        },
        birds: false,
        balloons: true,
    },
];

/// A look as a round uses it: its palettes (base and second tone, hex) and its pattern.
#[derive(Clone, Debug)]
pub struct ResolvedLook {
    pub look: &'static Look,
    pub palette: [[Rgb; 2]; 9],
    pub pattern: Pattern,
}

impl ResolvedLook {
    pub fn tones(&self, s: Swatch) -> Palette {
        self.palette[s as usize]
    }

    /// A map's palette as this look repaints it (other colours and the classic look keep theirs).
    pub fn repaint(&self, p: Palette) -> Option<Palette> {
        Swatch::of(p)
            .filter(|_| self.look.id != LookId::Classic)
            .map(|s| self.tones(s))
    }
}

/// Hue steps a round may shift a look by.
const HUE_STEPS: [f64; 5] = [-0.03, -0.015, 0.0, 0.015, 0.03];

fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 {
        c * 0.0773993808
    } else {
        m::pow(c * 0.9478672986 + 0.0521327014, 2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    if c < 0.0031308 {
        c * 12.92
    } else {
        1.055 * m::pow(c, 0.41666) - 0.055
    }
}

fn linear(c: Rgb) -> [f64; 3] {
    c.bytes().map(|b| srgb_to_linear(f64::from(b) / 255.0))
}

fn from_linear(c: [f64; 3]) -> Rgb {
    Rgb::from_bytes(c.map(|x| m::clamp(linear_to_srgb(x) * 255.0, 0.0, 255.0).round() as u8))
}

/// Hue, saturation and lightness of a linear colour.
fn hsl(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c;
    let max = r.at_least(g).at_least(b);
    let min = r.at_most(g).at_most(b);
    let l = (min + max) / 2.0;
    if min == max {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l <= 0.5 {
        d / (max + min)
    } else {
        d / (2.0 - max - min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn hue2rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * 6.0 * (2.0 / 3.0 - t);
    }
    p
}

/// A linear colour from hue, saturation and lightness.
fn from_hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = ((h % 1.0) + 1.0) % 1.0;
    let s = m::clamp(s, 0.0, 1.0);
    let l = m::clamp(l, 0.0, 1.0);
    if s == 0.0 {
        return [l, l, l];
    }
    let p = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let q = 2.0 * l - p;
    [
        hue2rgb(q, p, h + 1.0 / 3.0),
        hue2rgb(q, p, h),
        hue2rgb(q, p, h - 1.0 / 3.0),
    ]
}

/// A colour shifted in hue, saturation and lightness.
pub fn shift(c: Rgb, dh: f64, ds: f64, dl: f64) -> Rgb {
    let [h, s, l] = hsl(linear(c));
    from_linear(from_hsl(h + dh, s + ds, l + dl))
}

/// A look with its palettes worked out and a hue shift (0 keeps its colours as designed).
pub fn resolve(look: &'static Look, hue: f64, pattern: Pattern) -> ResolvedLook {
    let palette = look.colors.map(|c| {
        let base = shift(c, hue, 0.0, 0.0);
        let second = shift(base, 0.0, -0.04, 0.07);
        [base, second]
    });
    ResolvedLook { look, palette, pattern }
}

pub fn classic() -> ResolvedLook {
    resolve(&LOOKS[0], 0.0, Pattern::Stripes)
}

/// The look of a round: one of the map's looks (its signature one most often), shifted a little.
pub fn look_for(ids: &[LookId], seed: u32) -> ResolvedLook {
    if ids.is_empty() {
        return classic();
    }
    let mut rng = Rng::new(seed ^ 0x100c_5eed);
    let id = if rng.unit() < 0.55 || ids.len() == 1 {
        ids[0]
    } else {
        ids[1 + rng.index(ids.len() - 1)]
    };
    let look = id.look();
    let hue = HUE_STEPS[rng.index(HUE_STEPS.len())];
    let pattern = look.patterns[rng.index(look.patterns.len())];
    resolve(look, hue, pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_are_in_the_order_of_their_ids() {
        for (i, l) in LOOKS.iter().enumerate() {
            assert_eq!(l.id as usize, i, "{}", l.id.name());
        }
    }
}
