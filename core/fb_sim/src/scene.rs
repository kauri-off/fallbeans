//! What the client draws for a map: filled by the same build code that makes the colliders.
use std::sync::Arc;

use crate::math::V3;
use crate::nodes::NodeId;
use crate::world::World;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrimKind {
    Box,
    Cyl,
    Sphere,
}

/// Two-colour palette (base, accent).
pub type Palette = [&'static str; 2];

pub mod pal {
    use super::Palette;
    /// One colour for both tones.
    pub const fn hex(c: &'static str) -> Palette {
        [c, c]
    }
    pub const BLUE: Palette = ["#7ccfff", "#9bdcff"];
    pub const PURPLE: Palette = ["#a98bff", "#bca4ff"];
    pub const PINK: Palette = ["#ff8cc8", "#ffa6d6"];
    pub const YELLOW: Palette = ["#ffd84a", "#ffe47a"];
    pub const GREEN: Palette = ["#6fe08a", "#8ceaa2"];
    pub const WHITE: Palette = ["#f4f1ff", "#ffffff"];
    pub const ORANGE: Palette = ["#ff9f4a", "#ffb673"];
    pub const RED: Palette = ["#ff6070", "#ff8490"];
    pub const TEAL: Palette = ["#39e0d0", "#6ff0e4"];
}

#[derive(Clone, Debug)]
pub enum SceneItem {
    Prim {
        node: NodeId,
        kind: PrimKind,
        /// box: sx, sy, sz · cyl: r, h, segments · sphere: r.
        dims: [f64; 3],
        pal: Palette,
        /// Pattern frequency (None: the default, 0.25 per metre).
        freq: Option<f64>,
        /// Surface finish and pattern (None: the default for the shape and the map's style).
        surface: Option<&'static str>,
        pattern: Option<&'static str>,
    },
    Model {
        node: NodeId,
        name: &'static str,
        /// Colour of the part a prop has for it (a flag's pennant, a mushroom's cap).
        tint: Option<&'static str>,
    },
    /// Something only the client draws (portal rings, glass panes…): pieces of its parts, placed in the
    /// node's frame; with a look, they follow the map's state.
    Special {
        node: NodeId,
        kind: &'static str,
        parts: Vec<Part>,
        pieces: Vec<Piece>,
        look: Option<Look>,
    },
}

/// Shape of a special's part. Flat ones lie in the x/y plane facing +z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Form {
    Box([f64; 3]),
    /// r, h, segments.
    Cyl([f64; 3]),
    Sphere(f64),
    /// Major and minor radius, round the z axis.
    Torus(f64, f64),
    /// Inner and outer radius.
    Ring(f64, f64),
    /// A portal's sash: a half disc of radius r, round side at the origin (the rim), flat side at x = r.
    Sash(f64),
    /// A portal's disc: a spiral in its colour (the way in).
    Swirl(f64),
    /// A one-way exit's disc: rings flowing out in its colour.
    Rings(f64),
    /// An arrow pointing +y (laid on the ground with its piece's rotation).
    Arrow,
    Plane(f64, f64),
    /// A board with a text (an emoji) on its colour: w, h, text.
    Label(f64, f64, &'static str),
    Model(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Finish {
    Matte,
    Glossy,
    Metal,
    Glass,
    /// Lit from within (lamps).
    Glow,
    /// Unlit, both sides (confetti, portal discs).
    Flat,
    /// Unlit and added to what is behind, like light (flashes).
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub form: Form,
    /// The colours at tone 0 and tone 1 (`#rrggbb` or `#rrggbbaa`).
    pub colors: [&'static str; 2],
    pub finish: Finish,
    /// Surface of a lit finish (None: plastic), as a primitive's.
    pub surface: Option<&'static str>,
    /// The palette whose first tone is the first colour, repainted by the round's look.
    pub pal: Option<Palette>,
}

impl Part {
    pub const fn new(form: Form, color: &'static str, finish: Finish) -> Self {
        Self {
            form,
            colors: [color, color],
            finish,
            surface: None,
            pal: None,
        }
    }

    pub const fn toned(form: Form, from: &'static str, to: &'static str, finish: Finish) -> Self {
        Self {
            form,
            colors: [from, to],
            finish,
            surface: None,
            pal: None,
        }
    }

    pub const fn on(self, surface: &'static str) -> Self {
        Self {
            surface: Some(surface),
            ..self
        }
    }

    /// Tone 0 is the palette's first colour as the round's look paints it.
    pub const fn painted(self, pal: Palette) -> Self {
        Self {
            colors: [pal[0], self.colors[1]],
            pal: Some(pal),
            ..self
        }
    }
}

/// A lamp over a gate or a portal: red while shut, green (tone 1) while open.
pub const fn lamp_part(r: f64) -> Part {
    Part::toned(Form::Sphere(r), "#ff6070", "#4fdc6a", Finish::Glow)
}

/// One drawn piece of a part at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub part: u8,
    pub pos: V3,
    /// Euler angles, XYZ order.
    pub rot: V3,
    /// 0 or less: hidden.
    pub scale: f64,
    /// Times the scale along each axis.
    pub axes: V3,
    /// 0…1: from the part's first colour to its second; below 0: darker.
    pub tone: f64,
    /// Times the colour's own opacity.
    pub alpha: f64,
}

impl Piece {
    pub const fn at(part: u8, x: f64, y: f64, z: f64) -> Self {
        Self {
            part,
            pos: V3::new(x, y, z),
            rot: V3::ZERO,
            scale: 1.0,
            axes: V3::ONE,
            tone: 0.0,
            alpha: 1.0,
        }
    }

    pub const fn rot(self, x: f64, y: f64, z: f64) -> Self {
        Self {
            rot: V3::new(x, y, z),
            ..self
        }
    }

    pub const fn scale(self, scale: f64) -> Self {
        Self { scale, ..self }
    }

    pub const fn axes(self, x: f64, y: f64, z: f64) -> Self {
        Self {
            axes: V3::new(x, y, z),
            ..self
        }
    }

    pub const fn tone(self, tone: f64) -> Self {
        Self { tone, ..self }
    }

    pub const fn alpha(self, alpha: f64) -> Self {
        Self { alpha, ..self }
    }
}

/// A map primitive turned towards another colour (plates about to drop): k from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub node: NodeId,
    pub to: &'static str,
    pub k: f64,
}

/// What a special shows at a moment.
#[derive(Debug, Default)]
pub struct LookOut {
    pub pieces: Vec<Piece>,
    pub tints: Vec<Tint>,
}

/// Fills a special's pieces for sim time t from the map's state (client only, every frame).
#[derive(Clone)]
pub struct Look(pub Arc<dyn Fn(&World, f64, &mut LookOut) + Send + Sync>);

impl core::fmt::Debug for Look {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str("Look")
    }
}

#[derive(Clone, Debug)]
pub struct SceneryRequest {
    pub cx: f64,
    pub cz: f64,
    pub spread: f64,
    pub clouds: u32,
    pub y_min: f64,
    pub y_max: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SceneDesc {
    pub items: Vec<SceneItem>,
    pub scenery: Vec<SceneryRequest>,
}
