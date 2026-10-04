//! Shorthands for the options maps pass most often.
use fb_sim::builder::PrimOpts;
use fb_sim::math::V3;

/// No options.
pub fn o() -> PrimOpts {
    PrimOpts::default()
}

/// Drawn only (`noCollide`).
pub fn deco() -> PrimOpts {
    PrimOpts {
        no_collide: true,
        ..Default::default()
    }
}

pub fn freq(f: f64) -> PrimOpts {
    PrimOpts {
        freq: Some(f),
        ..Default::default()
    }
}

/// Moved by a mover (its collider is read every tick).
pub fn dynamic() -> PrimOpts {
    PrimOpts {
        dynamic: true,
        ..Default::default()
    }
}

pub fn rot(x: f64, y: f64, z: f64) -> PrimOpts {
    PrimOpts {
        rot: Some(V3::new(x, y, z)),
        ..Default::default()
    }
}
