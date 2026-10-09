//! Replicated components: the round, each bean's identity, its full state (owner) and its pose (others).
use bevy::math::Curve;
use bevy::prelude::*;
use fb_shared::PlayerId;
use fb_shared::game::{ArenaKind, FallBehaviour, MapId};
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyState, Power};
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// Whose bean an entity is.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BeanId(pub PlayerId);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeanColor(pub u8);

/// The arena of a room (its lobby, a round, the podium): clients build the same map from it. `zero_tick` is
/// the tick of sim time 0; it moves only when a dev command warps or pauses the room's clock.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Round {
    /// Changes with every new arena of the room (`fb_proto::ArenaInfo::id`).
    pub arena: u32,
    pub kind: ArenaKind,
    pub map: MapId,
    pub seed: u32,
    pub zero_tick: i64,
    /// What a fall does here (a server run with `--respawn` keeps survival rounds respawning).
    pub fall: FallBehaviour,
    /// `World::hash(true)` of the server's build: clients compare theirs.
    pub static_hash: String,
}

impl Round {
    pub fn arena_tick(&self, tick: Tick) -> i64 {
        tick.0 as i64 - self.zero_tick
    }

    /// The same map (a new zero tick is the same arena, shifted).
    pub fn same_arena(&self, o: &Round) -> bool {
        self.arena == o.arena && self.map == o.map && self.seed == o.seed
    }
}

/// Full state of the bean, sent to its owner only (it predicts it and rolls back on a mismatch). On the
/// wire exactly (`wire.rs`), all but the landing impact.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(from = "crate::wire::Full", into = "crate::wire::Full")]
pub struct BodyFull {
    pub body: Body,
    /// Bumped on every respawn: views snap instead of smoothing.
    pub teleports: u32,
    /// The checkpoint reached and the spawn: the client predicts where a fall puts the bean back.
    pub checkpoint: Option<u16>,
    pub spawn: u16,
}

/// Rolls back on any difference in what the physics step reads. Server and client run the same code on
/// the same bits and the state travels exactly, so a difference is never float noise: it is the client
/// having seen something else (another bean where it was a moment ago, an input that came late). Left
/// alone below a threshold it grows until it crosses it, a second later, as a bigger correction.
/// Not compared: the landing impact (drawn only, f32 on the wire) and the size (set from the bonus at
/// the start of every step). Floats compare by their bits: a NaN equals itself (`!=` would roll back on
/// every snapshot) and −0 differs from +0 (the next step may not treat them alike).
pub fn body_differs(a: &BodyFull, b: &BodyFull) -> bool {
    let (x, y) = (&a.body, &b.body);
    let f = |a: f64, b: f64| a.to_bits() != b.to_bits();
    let v = |a: V3, b: V3| a.to_array().map(f64::to_bits) != b.to_array().map(f64::to_bits);
    a.teleports != b.teleports
        || a.checkpoint != b.checkpoint
        || x.actor != y.actor
        || v(x.pos, y.pos)
        || v(x.vel, y.vel)
        || f(x.yaw, y.yaw)
        || x.grounded != y.grounded
        || x.ground_col != y.ground_col
        || x.state != y.state
        || f(x.state_t, y.state_t)
        || f(x.coyote, y.coyote)
        || f(x.jump_buf, y.jump_buf)
        || f(x.slow_until, y.slow_until)
        || f(x.slow_k, y.slow_k)
        || f(x.tilt, y.tilt)
        || f(x.tilt_dir, y.tilt_dir)
        || x.power != y.power
        || f(x.power_until, y.power_until)
        || v(x.climb_to, y.climb_to)
}

/// What a bean is doing, for its animation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anim {
    #[default]
    Idle,
    Air,
    Dive,
    Stun,
    Grab,
    Slide,
    Tumble,
    Getup,
    Reach,
    Climb,
    ClimbOver,
    /// Inside a portal: not drawn.
    Portal,
    Ladder,
}

impl Anim {
    pub const ALL: [Anim; 13] = [
        Anim::Idle,
        Anim::Air,
        Anim::Dive,
        Anim::Stun,
        Anim::Grab,
        Anim::Slide,
        Anim::Tumble,
        Anim::Getup,
        Anim::Reach,
        Anim::Climb,
        Anim::ClimbOver,
        Anim::Portal,
        Anim::Ladder,
    ];

    /// The body's state first, then in the air, then what the hands do.
    pub fn of(b: &Body, grabbing: bool, reaching: bool) -> Anim {
        match b.state {
            BodyState::Portal => Anim::Portal,
            BodyState::Ladder => Anim::Ladder,
            BodyState::Tumble => Anim::Tumble,
            BodyState::Getup => Anim::Getup,
            BodyState::Stun => Anim::Stun,
            BodyState::Dive => Anim::Dive,
            BodyState::Slide => Anim::Slide,
            BodyState::Climb if b.climbing_over() => Anim::ClimbOver,
            BodyState::Climb => Anim::Climb,
            _ if !b.grounded => Anim::Air,
            _ if grabbing => Anim::Grab,
            _ if reaching => Anim::Reach,
            _ => Anim::Idle,
        }
    }
}

/// Whom a bean holds and whether it reaches out with nobody in hand (decided by the server only): the
/// arms of every bean, the own one's included.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Hold {
    pub target: Option<PlayerId>,
    pub reaching: bool,
}

/// What everybody else sees of a bean (interpolated). On the wire about 20 bytes: position and velocity in centimetres, angles in u16/u8.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(from = "crate::wire::Pose", into = "crate::wire::Pose")]
pub struct RemotePose {
    pub pos: Vec3,
    pub yaw: f32,
    pub tilt: f32,
    pub tilt_dir: f32,
    pub anim: Anim,
    pub power: Option<Power>,
    pub size: f32,
    pub vel: Vec2,
    pub teleports: u32,
}

impl RemotePose {
    pub fn of(b: &BodyFull, hold: &Hold) -> Self {
        let body = &b.body;
        Self {
            pos: body.pos.as_vec3(),
            yaw: body.yaw as f32,
            tilt: body.tilt as f32,
            tilt_dir: body.tilt_dir as f32,
            anim: Anim::of(body, hold.target.is_some(), hold.reaching),
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
