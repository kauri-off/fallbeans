//! What the client draws for a map: filled by the same build code that makes the colliders.
use crate::nodes::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrimKind {
    Box,
    Cyl,
    Sphere,
}

/// Two-colour palette (base, accent), as `PAL` in TS.
pub type Palette = [&'static str; 2];

pub mod pal {
    use super::Palette;
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
        freq: f64,
    },
    Model {
        node: NodeId,
        name: &'static str,
    },
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
