//! The Lightyear protocol shared by server and client: replicated components, inputs, channels.
use core::time::Duration;

use bevy::ecs::entity::MapEntities;
use bevy::math::Curve;
use bevy::prelude::*;
use fb_shared::input::InputFrame;
use fb_shared::{PROTOCOL_VERSION, TICK_RATE};
use fb_sim::physics::Body;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_ID: u64 = 0xFB00_0000 + PROTOCOL_VERSION as u64;
/// Dev key for netcode's manual authentication (phase 0: no session endpoint yet).
pub const DEV_KEY: [u8; 32] = [0; 32];
pub const UDP_PORT: u16 = 5888;
pub const WS_PORT: u16 = 5889;
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE as u64);
/// Snapshots at 30 Hz, as the TS server.
pub const SEND_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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

/// Full state of the bean (its owner predicts it and rolls back on a mismatch).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BodyFull {
    pub body: Body,
    /// Bumped on every respawn: views snap instead of smoothing.
    pub teleports: u32,
}

/// Rolls back only when the server disagrees beyond float noise (other beans are drawn in the past).
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

/// What everybody else sees of a bean (interpolated).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
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

/// One tick of a player's input.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct FbInput {
    pub mx: i8,
    pub mz: i8,
    pub buttons: u8,
}

impl MapEntities for FbInput {
    fn map_entities<M: EntityMapper>(&mut self, _: &mut M) {}
}

impl From<FbInput> for InputFrame {
    fn from(i: FbInput) -> Self {
        InputFrame {
            mx: i.mx,
            mz: i.mz,
            buttons: i.buttons,
        }
    }
}

impl From<InputFrame> for FbInput {
    fn from(f: InputFrame) -> Self {
        FbInput {
            mx: f.mx,
            mz: f.mz,
            buttons: f.buttons,
        }
    }
}

/// Map events with the tick they happened at (clients apply them on that tick).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum MapEventKind {
    Bonus { i: u32, id: u32, at: f64 },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MapEventMsg {
    pub round: u32,
    pub tick: u32,
    pub ev: MapEventKind,
}

pub struct MapEventsChannel;

#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(input::native::InputPlugin::<FbInput>::default());
        app.register_message::<MapEventMsg>()
            .add_direction(NetworkDirection::ServerToClient);
        app.add_channel::<MapEventsChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);
        app.component::<PlayerId>().replicate();
        app.component::<BeanColor>().replicate();
        app.component::<Round>().replicate();
        app.component::<BodyFull>()
            .replicate()
            .predict()
            .with_rollback_condition(body_differs);
        app.component::<RemotePose>().replicate().add_linear_interpolation();
    }
}
