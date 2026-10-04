//! How a map looks (port of `sim/looks.ts`): its colours (the palettes its parts are painted with), the
//! patterns on them, the sky, sun, fog and ambient light, the land far below and the scenery around it.
//! Each map has a few looks (the first is its signature); a round picks one by its seed and shifts the
//! colours a little. Visual only: nothing here touches colliders or the map's layout.
use fb_shared::m;
use fb_shared::rng::Rng;

/// The palettes maps paint with, in the order of `scene::pal`.
pub const PAL_KEYS: [&str; 9] = [
    "blue", "purple", "pink", "yellow", "green", "white", "orange", "red", "teal",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern {
    Stripes,
    Checker,
    Dots,
    Chevron,
    Waves,
}

impl Pattern {
    pub fn of(name: &str) -> Option<Pattern> {
        Some(match name {
            "stripes" => Pattern::Stripes,
            "checker" => Pattern::Checker,
            "dots" => Pattern::Dots,
            "chevron" => Pattern::Chevron,
            "waves" => Pattern::Waves,
            _ => return None,
        })
    }

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
    pub top: &'static str,
    pub horizon: &'static str,
    /// Tint of the clouds.
    pub cloud: &'static str,
    /// Stars in the sky, 0…1 (night looks).
    pub stars: f64,
}

/// The sun (or moon): colour, strength, compass angle and height in degrees.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub color: &'static str,
    pub intensity: f64,
    pub azimuth: f64,
    pub elevation: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Hemi {
    pub sky: &'static str,
    pub ground: &'static str,
    pub intensity: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Fog {
    pub color: &'static str,
    pub near: f64,
    pub far: f64,
}

/// Specks in the air: colour and vertical drift (m/s; negative falls, like snow).
#[derive(Clone, Copy, Debug)]
pub struct Motes {
    pub color: &'static str,
    pub rise: f64,
}

/// The land far below the course; `glow` makes it shine (lava).
#[derive(Clone, Copy, Debug)]
pub struct Ground {
    pub c1: &'static str,
    pub c2: &'static str,
    pub kind: Pattern,
    pub freq: f64,
    pub speed: f64,
    pub glow: bool,
}

/// The floating islands: grass (top), rock (underside) and bush colours.
#[derive(Clone, Copy, Debug)]
pub struct Island {
    pub grass: &'static str,
    pub rock: &'static str,
    pub leaves: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub id: &'static str,
    /// Base colour of each palette (`PAL_KEYS` order; the lighter second tone is derived).
    pub colors: [&'static str; 9],
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
        id: "classic",
        colors: [
            "#7ccfff", "#a98bff", "#ff8cc8", "#ffd84a", "#6fe08a", "#f4f1ff", "#ff9f4a", "#ff6070", "#39e0d0",
        ],
        patterns: &[Pattern::Stripes],
        sky: Sky {
            top: "#6fb8ff",
            horizon: "#ffd9f2",
            cloud: "#ffffff",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff1dc",
            intensity: 2.2,
            azimuth: 38.0,
            elevation: 74.0,
        },
        hemi: Hemi {
            sky: "#cfe8ff",
            ground: "#b99be0",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#f1d4f7",
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.06,
        env: 0.3,
        motes: Motes {
            color: "#fff7e0",
            rise: 0.12,
        },
        ground: None,
        island: Island {
            grass: "#6fd46a",
            rock: "#9b7a5e",
            leaves: "#4fbf5a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "meadow",
        colors: [
            "#6ec3ff", "#b39cff", "#ff9ccf", "#ffe066", "#7fe07a", "#f7f5ea", "#ffae5c", "#ff6f6f", "#4fe0c4",
        ],
        patterns: &[Pattern::Dots, Pattern::Waves, Pattern::Stripes],
        sky: Sky {
            top: "#58aefc",
            horizon: "#e6f7ff",
            cloud: "#ffffff",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff4d6",
            intensity: 2.3,
            azimuth: 60.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: "#d8efff",
            ground: "#9fce8a",
            intensity: 0.95,
        },
        fog: Fog {
            color: "#e2f2fb",
            near: 130.0,
            far: 560.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.3,
        motes: Motes {
            color: "#fffbe0",
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: "#7ccf6a",
            c2: "#93dc7c",
            kind: Pattern::Waves,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#6fd46a",
            rock: "#9b7a5e",
            leaves: "#4fbf5a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "castle",
        colors: [
            "#6f8fd9", "#8e79c9", "#d98ab0", "#f2c94c", "#79b87a", "#e9e4da", "#e39a5b", "#d9534f", "#5bbfb0",
        ],
        patterns: &[Pattern::Checker, Pattern::Stripes, Pattern::Chevron],
        sky: Sky {
            top: "#6fa6e0",
            horizon: "#f6e3c8",
            cloud: "#fff8ee",
            stars: 0.0,
        },
        sun: Sun {
            color: "#ffe8c8",
            intensity: 2.3,
            azimuth: 20.0,
            elevation: 70.0,
        },
        hemi: Hemi {
            sky: "#dbe6ff",
            ground: "#a89a86",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#eee4d6",
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.02,
        env: 0.3,
        motes: Motes {
            color: "#fff2d8",
            rise: 0.1,
        },
        ground: Some(Ground {
            c1: "#6fae5a",
            c2: "#86c26c",
            kind: Pattern::Checker,
            freq: 0.012,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#78bf62",
            rock: "#8f8a86",
            leaves: "#5aa860",
        },
        birds: true,
        balloons: false,
    },
    Look {
        id: "factory",
        colors: [
            "#5f8fb8", "#7f7fa8", "#d07a8a", "#f5c542", "#7fa86a", "#d8dde3", "#f08a3a", "#e0523e", "#4fb5ac",
        ],
        patterns: &[Pattern::Chevron, Pattern::Stripes, Pattern::Checker],
        sky: Sky {
            top: "#7f9cbc",
            horizon: "#f2d6ae",
            cloud: "#e4ddd2",
            stars: 0.0,
        },
        sun: Sun {
            color: "#ffdcb0",
            intensity: 2.2,
            azimuth: 120.0,
            elevation: 66.0,
        },
        hemi: Hemi {
            sky: "#dfe6ee",
            ground: "#8a7a6a",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#e6d8c4",
            near: 90.0,
            far: 430.0,
        },
        exposure: 0.95,
        saturation: 0.98,
        env: 0.35,
        motes: Motes {
            color: "#ffe0b0",
            rise: 0.25,
        },
        ground: Some(Ground {
            c1: "#5b6270",
            c2: "#6a7280",
            kind: Pattern::Checker,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#8a8f78",
            rock: "#6b6660",
            leaves: "#7a8a5a",
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: "snow",
        colors: [
            "#8fd0ff", "#b7b0ff", "#ffb7d9", "#fff0a0", "#9fe0c0", "#ffffff", "#ffc38a", "#ff7f8f", "#8ff0ea",
        ],
        patterns: &[Pattern::Waves, Pattern::Dots, Pattern::Chevron],
        sky: Sky {
            top: "#8ec6f5",
            horizon: "#f4fbff",
            cloud: "#ffffff",
            stars: 0.0,
        },
        sun: Sun {
            color: "#f4f8ff",
            intensity: 2.0,
            azimuth: 200.0,
            elevation: 68.0,
        },
        hemi: Hemi {
            sky: "#e6f4ff",
            ground: "#c8d8f0",
            intensity: 1.0,
        },
        fog: Fog {
            color: "#eef6ff",
            near: 90.0,
            far: 420.0,
        },
        exposure: 0.92,
        saturation: 1.0,
        env: 0.35,
        motes: Motes {
            color: "#ffffff",
            rise: -1.1,
        },
        ground: Some(Ground {
            c1: "#f4f9ff",
            c2: "#dfeefa",
            kind: Pattern::Waves,
            freq: 0.015,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#f6fbff",
            rock: "#8a9bb0",
            leaves: "#e8f4ff",
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: "starlight",
        colors: [
            "#6f86ff", "#9a6bff", "#ff7ad9", "#ffd86b", "#5fe0a8", "#dfe6ff", "#ff9f6b", "#ff5f87", "#46e0e6",
        ],
        patterns: &[Pattern::Dots, Pattern::Checker, Pattern::Waves],
        sky: Sky {
            top: "#0b1238",
            horizon: "#46307a",
            cloud: "#5a4a8a",
            stars: 1.0,
        },
        sun: Sun {
            color: "#c8d4ff",
            intensity: 1.7,
            azimuth: 300.0,
            elevation: 70.0,
        },
        hemi: Hemi {
            sky: "#9fb0ff",
            ground: "#4a3a7e",
            intensity: 0.95,
        },
        fog: Fog {
            color: "#2e2860",
            near: 110.0,
            far: 480.0,
        },
        exposure: 1.0,
        saturation: 1.1,
        env: 0.4,
        motes: Motes {
            color: "#bfe8ff",
            rise: 0.08,
        },
        ground: Some(Ground {
            c1: "#1a1f4a",
            c2: "#283070",
            kind: Pattern::Dots,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#4a5aa0",
            rock: "#2e2a58",
            leaves: "#6a7ae0",
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: "circus",
        colors: [
            "#4fa3ff", "#a06bff", "#ff6fb0", "#ffd23f", "#4fdc6a", "#fff8ef", "#ff8a3d", "#ff4d5a", "#2fd3c4",
        ],
        patterns: &[Pattern::Stripes, Pattern::Dots, Pattern::Chevron],
        sky: Sky {
            top: "#58b9ff",
            horizon: "#ffe9c9",
            cloud: "#fffaf0",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff0d0",
            intensity: 2.3,
            azimuth: 80.0,
            elevation: 73.0,
        },
        hemi: Hemi {
            sky: "#cfe8ff",
            ground: "#b99be0",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#fbe8d6",
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.12,
        env: 0.3,
        motes: Motes {
            color: "#fff0c8",
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: "#ffe3b0",
            c2: "#ffd08a",
            kind: Pattern::Stripes,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#6fd46a",
            rock: "#9b7a5e",
            leaves: "#4fbf5a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "neon",
        colors: [
            "#3fd0ff", "#b45cff", "#ff4fcf", "#fff04f", "#4fff9f", "#e8e0ff", "#ff9a3f", "#ff4f6f", "#2ff5e0",
        ],
        patterns: &[Pattern::Checker, Pattern::Chevron, Pattern::Stripes],
        sky: Sky {
            top: "#1b0f3d",
            horizon: "#ff5fa2",
            cloud: "#7a3a8a",
            stars: 0.6,
        },
        sun: Sun {
            color: "#ffc0ec",
            intensity: 1.9,
            azimuth: 180.0,
            elevation: 66.0,
        },
        hemi: Hemi {
            sky: "#b0a0ff",
            ground: "#5a2a70",
            intensity: 0.95,
        },
        fog: Fog {
            color: "#5a2a6a",
            near: 100.0,
            far: 460.0,
        },
        exposure: 1.0,
        saturation: 1.15,
        env: 0.4,
        motes: Motes {
            color: "#ff9ff0",
            rise: 0.2,
        },
        ground: Some(Ground {
            c1: "#1a0f33",
            c2: "#44207a",
            kind: Pattern::Checker,
            freq: 0.04,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#3a2a7a",
            rock: "#20143e",
            leaves: "#ff4fcf",
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: "ocean",
        colors: [
            "#3fb6ff", "#8f8fff", "#ff9fbf", "#ffe27a", "#5fe0a0", "#fffaf0", "#ffb06a", "#ff7070", "#2fe0d0",
        ],
        patterns: &[Pattern::Waves, Pattern::Stripes, Pattern::Dots],
        sky: Sky {
            top: "#45b0ff",
            horizon: "#e0fbff",
            cloud: "#ffffff",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff6e0",
            intensity: 2.4,
            azimuth: 250.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: "#d0f0ff",
            ground: "#8fd0d8",
            intensity: 0.95,
        },
        fog: Fog {
            color: "#d8f4fc",
            near: 130.0,
            far: 560.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.35,
        motes: Motes {
            color: "#ffffff",
            rise: 0.1,
        },
        ground: Some(Ground {
            c1: "#1f9fd6",
            c2: "#37c1ec",
            kind: Pattern::Waves,
            freq: 0.035,
            speed: 0.4,
            glow: false,
        }),
        island: Island {
            grass: "#f2dca8",
            rock: "#b09070",
            leaves: "#4fcf6a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "desert",
        colors: [
            "#6fb6d9", "#b08fc9", "#e8a0a0", "#f2cf6b", "#a8c46a", "#f6ead2", "#e8944a", "#d8654a", "#5fc0a8",
        ],
        patterns: &[Pattern::Waves, Pattern::Chevron, Pattern::Stripes],
        sky: Sky {
            top: "#6cb2ea",
            horizon: "#ffe0b0",
            cloud: "#fff4e0",
            stars: 0.0,
        },
        sun: Sun {
            color: "#ffe2b8",
            intensity: 2.6,
            azimuth: 140.0,
            elevation: 76.0,
        },
        hemi: Hemi {
            sky: "#e0ecff",
            ground: "#d8b080",
            intensity: 0.85,
        },
        fog: Fog {
            color: "#f6e2c4",
            near: 110.0,
            far: 500.0,
        },
        exposure: 0.93,
        saturation: 1.04,
        env: 0.3,
        motes: Motes {
            color: "#ffe8c0",
            rise: 0.3,
        },
        ground: Some(Ground {
            c1: "#e8c690",
            c2: "#f0d6a8",
            kind: Pattern::Waves,
            freq: 0.012,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#e8c690",
            rock: "#b8764a",
            leaves: "#8fae5a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "jungle",
        colors: [
            "#5fb8e0", "#9f7fd9", "#ff8fb0", "#ffe05f", "#4fcf5f", "#f0f8e8", "#ffa04f", "#ff6050", "#3fd0a0",
        ],
        patterns: &[Pattern::Dots, Pattern::Waves, Pattern::Chevron],
        sky: Sky {
            top: "#62c0f8",
            horizon: "#e4ffdc",
            cloud: "#f8fff4",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff4c8",
            intensity: 2.2,
            azimuth: 330.0,
            elevation: 72.0,
        },
        hemi: Hemi {
            sky: "#dcffe8",
            ground: "#6a9a5a",
            intensity: 0.95,
        },
        fog: Fog {
            color: "#dff4dc",
            near: 100.0,
            far: 470.0,
        },
        exposure: 0.95,
        saturation: 1.1,
        env: 0.3,
        motes: Motes {
            color: "#f4ffc0",
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: "#2f8f4f",
            c2: "#3fa85f",
            kind: Pattern::Dots,
            freq: 0.03,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#4fbf4a",
            rock: "#7a5a3e",
            leaves: "#2f9f3f",
        },
        birds: true,
        balloons: false,
    },
    Look {
        id: "lava",
        colors: [
            "#6a7fb0", "#7a5a9a", "#e0708a", "#ffc84a", "#8aa06a", "#e8dcd0", "#ff7a2a", "#e8402a", "#4aa8a0",
        ],
        patterns: &[Pattern::Chevron, Pattern::Checker, Pattern::Waves],
        sky: Sky {
            top: "#3a2a3e",
            horizon: "#ff8a4a",
            cloud: "#6a4a4a",
            stars: 0.2,
        },
        sun: Sun {
            color: "#ffb888",
            intensity: 2.2,
            azimuth: 90.0,
            elevation: 68.0,
        },
        hemi: Hemi {
            sky: "#ffd0b0",
            ground: "#6a2a2a",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#8a4a3a",
            near: 90.0,
            far: 420.0,
        },
        exposure: 1.0,
        saturation: 1.06,
        env: 0.35,
        motes: Motes {
            color: "#ffa040",
            rise: 1.0,
        },
        ground: Some(Ground {
            c1: "#ff5a1a",
            c2: "#ffb030",
            kind: Pattern::Waves,
            freq: 0.03,
            speed: 0.25,
            glow: true,
        }),
        island: Island {
            grass: "#4a3a3a",
            rock: "#2a2020",
            leaves: "#8a4a2a",
        },
        birds: false,
        balloons: false,
    },
    Look {
        id: "royal",
        colors: [
            "#6fa8ff", "#9a6bff", "#ff8fc0", "#ffd23f", "#6fd08a", "#fff6e6", "#ffa04a", "#ff5a6a", "#4fd6c8",
        ],
        patterns: &[Pattern::Chevron, Pattern::Checker, Pattern::Stripes],
        sky: Sky {
            top: "#6a98e6",
            horizon: "#ffc89a",
            cloud: "#fff0e0",
            stars: 0.0,
        },
        sun: Sun {
            color: "#ffd8a8",
            intensity: 2.3,
            azimuth: 260.0,
            elevation: 67.0,
        },
        hemi: Hemi {
            sky: "#ffe8d8",
            ground: "#b88ac0",
            intensity: 0.9,
        },
        fog: Fog {
            color: "#f6d8c8",
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.95,
        saturation: 1.08,
        env: 0.35,
        motes: Motes {
            color: "#ffe8a0",
            rise: 0.2,
        },
        ground: Some(Ground {
            c1: "#9a6bd0",
            c2: "#b48ae0",
            kind: Pattern::Chevron,
            freq: 0.015,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#6fd46a",
            rock: "#9b7a5e",
            leaves: "#4fbf5a",
        },
        birds: true,
        balloons: true,
    },
    Look {
        id: "candy",
        colors: [
            "#8fd8ff", "#c8a8ff", "#ff9fd0", "#ffec8f", "#a8f0b0", "#fff6fb", "#ffc08f", "#ff7fa0", "#8ff0e0",
        ],
        patterns: &[Pattern::Dots, Pattern::Checker, Pattern::Stripes],
        sky: Sky {
            top: "#ff9fd6",
            horizon: "#fff0f8",
            cloud: "#fff0fb",
            stars: 0.0,
        },
        sun: Sun {
            color: "#fff0f4",
            intensity: 2.2,
            azimuth: 10.0,
            elevation: 74.0,
        },
        hemi: Hemi {
            sky: "#ffe8f6",
            ground: "#d8b0f0",
            intensity: 1.0,
        },
        fog: Fog {
            color: "#ffe4f2",
            near: 120.0,
            far: 520.0,
        },
        exposure: 0.93,
        saturation: 1.05,
        env: 0.3,
        motes: Motes {
            color: "#ffffff",
            rise: 0.15,
        },
        ground: Some(Ground {
            c1: "#ffc2e0",
            c2: "#ffffff",
            kind: Pattern::Checker,
            freq: 0.02,
            speed: 0.0,
            glow: false,
        }),
        island: Island {
            grass: "#ffb0d8",
            rock: "#c88a6a",
            leaves: "#a8f0b0",
        },
        birds: false,
        balloons: true,
    },
];

/// A look as a round uses it: its palettes (base and second tone, hex) and its pattern.
#[derive(Clone, Debug)]
pub struct ResolvedLook {
    pub look: &'static Look,
    pub palette: [[String; 2]; 9],
    pub pattern: Pattern,
}

/// Hue steps a round may shift a look by.
const HUE_STEPS: [f64; 5] = [-0.03, -0.015, 0.0, 0.015, 0.03];

pub fn by_id(id: &str) -> Option<&'static Look> {
    LOOKS.iter().find(|l| l.id == id)
}

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

fn parse_hex(hex: &str) -> [f64; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    [((v >> 16) & 255) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64].map(|c| srgb_to_linear(c / 255.0))
}

fn to_hex(c: [f64; 3]) -> String {
    let b = c.map(|x| m::round_js(m::clamp(linear_to_srgb(x) * 255.0, 0.0, 255.0)) as u32);
    format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
}

/// three.js `Color.getHSL` (in its linear working space).
fn hsl(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
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

/// three.js `Color.setHSL`.
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

/// three.js `Color.offsetHSL` on a hex colour.
pub fn shift(hex: &str, dh: f64, ds: f64, dl: f64) -> String {
    let [h, s, l] = hsl(parse_hex(hex));
    to_hex(from_hsl(h + dh, s + ds, l + dl))
}

/// A look with its palettes worked out and a hue shift (0 keeps its colours as designed).
pub fn resolve(look: &'static Look, hue: f64, pattern: Pattern) -> ResolvedLook {
    let palette = look.colors.map(|c| {
        let base = shift(c, hue, 0.0, 0.0);
        let second = shift(&base, 0.0, -0.04, 0.07);
        [base, second]
    });
    ResolvedLook { look, palette, pattern }
}

pub fn classic() -> ResolvedLook {
    resolve(&LOOKS[0], 0.0, Pattern::Stripes)
}

/// The look of a round: one of the map's looks (its signature one most often), shifted a little.
pub fn look_for(ids: &[&str], seed: u32) -> ResolvedLook {
    if ids.is_empty() {
        return classic();
    }
    let mut rng = Rng::new(seed ^ 0x100c_5eed);
    let id = if rng.next() < 0.55 || ids.len() == 1 {
        ids[0]
    } else {
        ids[1 + (rng.next() * (ids.len() - 1) as f64) as usize]
    };
    let look = by_id(id).unwrap_or(&LOOKS[0]);
    let hue = HUE_STEPS[(rng.next() * HUE_STEPS.len() as f64) as usize];
    let pattern = look.patterns[(rng.next() * look.patterns.len() as f64) as usize];
    resolve(look, hue, pattern)
}
