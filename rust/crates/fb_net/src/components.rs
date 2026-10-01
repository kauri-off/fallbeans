//! Replicated components: the round, each bean's identity, its full state (owner) and its pose (others).
use bevy::math::Curve;
use bevy::prelude::*;
use fb_sim::physics::Body;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// The server's id of a player in the room (small, sequential; not the network id).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeanColor(pub u8);

/// The round being played: clients build the same map from it. `zero_tick` is the tick of sim time 0.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Round {
    pub map: String,
    pub seed: u32,
    pub zero_tick: u32,
    pub number: u32,
    /// `World::hash(true)` of the server's build: clients compare theirs.
    pub static_hash: String,
}

impl Round {
    pub fn arena_tick(&self, tick: Tick) -> i64 {
        tick.0 as i64 - self.zero_tick as i64
    }
}

/// Full state of the bean, sent to its owner only (it predicts it and rolls back on a mismatch). On the
/// wire as the TS `FULL_BYTES`: position, velocity and yaw exact, timers and angles in f32.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(from = "crate::wire::Full", into = "crate::wire::Full")]
pub struct BodyFull {
    pub body: Body,
    /// Bumped on every respawn: views snap instead of smoothing.
    pub teleports: u32,
}

/// Rolls back only when the server disagrees beyond float noise.
pub fn body_differs(a: &BodyFull, b: &BodyFull) -> bool {
    let (x, y) = (&a.body, &b.body);
    a.teleports != b.teleports
        || x.state != y.state
        || x.power != y.power
        || x.ground_col != y.ground_col
        || (x.pos - y.pos).length_squared() > 1e-6
        || (x.vel - y.vel).length_squared() > 1e-4
        || (x.state_t - y.state_t).abs() > 1e-4
        || (x.tilt - y.tilt).abs() > 1e-3
}

/// What everybody else sees of a bean (interpolated). On the wire about 20 bytes (TS:
/// `REMOTE_BYTES` = 22): position and velocity in centimetres, angles in u16/u8.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(from = "crate::wire::Pose", into = "crate::wire::Pose")]
pub struct RemotePose {
    pub pos: Vec3,
    pub yaw: f32,
    pub tilt: f32,
    pub tilt_dir: f32,
    pub state: u8,
    pub power: u8,
    pub size: f32,
    pub vel: Vec2,
    pub teleports: u32,
}

impl RemotePose {
    pub fn of(b: &BodyFull) -> Self {
        let body = &b.body;
        Self {
            pos: body.pos.as_vec3(),
            yaw: body.yaw as f32,
            tilt: body.tilt as f32,
            tilt_dir: body.tilt_dir as f32,
            state: body.state as u8,
            power: body.power,
            size: body.size as f32,
            vel: Vec2::new(body.vel.x as f32, body.vel.z as f32),
            teleports: b.teleports,
        }
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI;
    a + d * t
}

impl Ease for RemotePose {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| {
            if start.teleports != end.teleports {
                return if t < 1.0 { start } else { end };
            }
            RemotePose {
                pos: start.pos.lerp(end.pos, t),
                yaw: lerp_angle(start.yaw, end.yaw, t),
                tilt: start.tilt + (end.tilt - start.tilt) * t,
                tilt_dir: lerp_angle(start.tilt_dir, end.tilt_dir, t),
                vel: start.vel.lerp(end.vel, t),
                size: start.size + (end.size - start.size) * t,
                ..if t < 0.5 { start } else { end }
            }
        })
    }
}
