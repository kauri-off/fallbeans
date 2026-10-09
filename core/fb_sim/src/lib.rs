//! Deterministic simulation core: no Bevy, f64 everywhere, maths only through `m`.
pub mod beans;
pub mod bonus;
pub mod bots;
pub mod builder;
pub mod collider;
pub mod course;
pub mod looks;
pub mod map;
pub mod math;
pub mod nav;
pub mod nodes;
pub mod physics;
pub mod props;
pub mod scene;
pub mod world;

pub use fb_shared::m;
pub use math::{Affine, V3};
