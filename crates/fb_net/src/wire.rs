//! How the replicated bean components travel (postcard: integers are varints, floats fixed). The types in
//! memory stay full precision; only these forms are sent.
use core::f32::consts::{PI, TAU};

use bevy::prelude::*;
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyState, GIANT_SIZE, power};
use serde::{Deserialize, Serialize};

use crate::{Anim, BodyFull, RemotePose};

const STATES: [BodyState; 9] = [
    BodyState::Normal,
    BodyState::Stun,
    BodyState::Dive,
    BodyState::Slide,
    BodyState::Tumble,
    BodyState::Getup,
    BodyState::Climb,
    BodyState::Portal,
    BodyState::Ladder,
];

fn state(i: u8) -> BodyState {
    STATES.get(i as usize).copied().unwrap_or_default()
}

/// The own bean: what prediction resumes from. Everything the physics step reads is exact: after a
/// rollback the client must go on from the server's very state. (Timers in f32, as the TS snapshot sent
/// them, cross zero a tick earlier or later than on the server: the state changes at another tick and
/// the next rollback follows a second later.) Only the landing impact, drawn and never read, is f32.
/// The size is not sent: it follows the bonus (`Body::step` sets it from `power` every tick).
#[derive(Serialize, Deserialize)]
pub struct Full {
    actor: i32,
    pos: [f64; 3],
    vel: [f64; 3],
    yaw: f64,
    grounded: bool,
    ground_col: i32,
    state: u8,
    state_t: f64,
    coyote: f64,
    jump_buf: f64,
    slow_until: f64,
    slow_k: f64,
    land_impact: f32,
    tilt: f64,
    tilt_dir: f64,
    power: u8,
    power_until: f64,
    climb_to: [f64; 3],
    teleports: u32,
    checkpoint: Option<u16>,
    spawn: u16,
}

impl From<BodyFull> for Full {
    fn from(f: BodyFull) -> Self {
        let b = f.body;
        Self {
            actor: b.actor,
            pos: b.pos.to_array(),
            vel: b.vel.to_array(),
            yaw: b.yaw,
            grounded: b.grounded,
            ground_col: b.ground_col,
            state: b.state as u8,
            state_t: b.state_t,
            coyote: b.coyote,
            jump_buf: b.jump_buf,
            slow_until: b.slow_until,
            slow_k: b.slow_k,
            land_impact: b.land_impact as f32,
            tilt: b.tilt,
            tilt_dir: b.tilt_dir,
            power: b.power,
            power_until: b.power_until,
            climb_to: b.climb_to.to_array(),
            teleports: f.teleports,
            checkpoint: f.checkpoint,
            spawn: f.spawn,
        }
    }
}

impl From<Full> for BodyFull {
    fn from(w: Full) -> Self {
        Self {
            body: Body {
                actor: w.actor,
                pos: V3::from_array(w.pos),
                vel: V3::from_array(w.vel),
                yaw: w.yaw,
                grounded: w.grounded,
                ground_col: w.ground_col,
                state: state(w.state),
                state_t: w.state_t,
                coyote: w.coyote,
                jump_buf: w.jump_buf,
                slow_until: w.slow_until,
                slow_k: w.slow_k,
                land_impact: w.land_impact.into(),
                tilt: w.tilt,
                tilt_dir: w.tilt_dir,
                power: w.power,
                power_until: w.power_until,
                size: if w.power == power::GIANT { GIANT_SIZE } else { 1.0 },
                climb_to: V3::from_array(w.climb_to),
            },
            teleports: w.teleports,
            checkpoint: w.checkpoint,
            spawn: w.spawn,
        }
    }
}

/// Everybody else's view of a bean.
#[derive(Serialize, Deserialize)]
pub struct Pose {
    /// Centimetres (varints: 2 bytes within 80 m of the origin, 3 within 10 km).
    pos: [i32; 3],
    /// Full turn = 65536.
    yaw: u16,
    /// 0..π in 255 steps.
    tilt: u8,
    /// Full turn = 256.
    tilt_dir: u8,
    anim: u8,
    power: u8,
    /// Hundredths.
    size: u8,
    /// cm/s.
    vel: [i16; 2],
    /// Wraps: only changes matter.
    teleports: u8,
}

fn turn<const N: u32>(a: f32) -> u32 {
    ((a.rem_euclid(TAU) / TAU * N as f32).round() as u32) % N
}

impl From<RemotePose> for Pose {
    fn from(p: RemotePose) -> Self {
        Self {
            pos: p.pos.to_array().map(|v| (v * 100.0).round() as i32),
            yaw: turn::<65536>(p.yaw) as u16,
            tilt: (p.tilt.clamp(0.0, PI) / PI * 255.0).round() as u8,
            tilt_dir: turn::<256>(p.tilt_dir) as u8,
            anim: p.anim as u8,
            power: p.power,
            size: (p.size * 100.0).round().clamp(0.0, 255.0) as u8,
            vel: p
                .vel
                .to_array()
                .map(|v| (v * 100.0).round().clamp(-32767.0, 32767.0) as i16),
            teleports: p.teleports as u8,
        }
    }
}

impl From<Pose> for RemotePose {
    fn from(w: Pose) -> Self {
        Self {
            pos: Vec3::from_array(w.pos.map(|v| v as f32)) / 100.0,
            yaw: w.yaw as f32 / 65536.0 * TAU,
            tilt: w.tilt as f32 / 255.0 * PI,
            tilt_dir: w.tilt_dir as f32 / 256.0 * TAU,
            anim: Anim::ALL.get(w.anim as usize).copied().unwrap_or_default(),
            power: w.power,
            size: w.size as f32 / 100.0,
            vel: Vec2::new(w.vel[0] as f32, w.vel[1] as f32) / 100.0,
            teleports: w.teleports.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_replicon::postcard;

    use super::*;

    fn moving_body() -> BodyFull {
        let mut body = Body::new(3);
        body.pos = V3::new(12.345678901, -3.25, 7.000001);
        body.vel = V3::new(-8.5, 10.5, 0.125);
        body.yaw = 2.9;
        body.state = BodyState::Tumble;
        body.state_t = 0.4;
        body.tilt = 1.4;
        body.tilt_dir = -0.7;
        body.power = power::GIANT;
        body.power_until = 61.25;
        body.size = 1.8;
        body.ground_col = 17;
        BodyFull {
            body,
            teleports: 300,
            checkpoint: Some(2),
            spawn: 5,
        }
    }

    #[test]
    fn full_is_exact() {
        let f = moving_body();
        let mut buf = [0u8; 256];
        let bytes = postcard::to_slice(&f, &mut buf).unwrap();
        let back: BodyFull = postcard::from_bytes(bytes).unwrap();
        assert_eq!(
            (back.body.pos, back.body.vel, back.body.yaw),
            (f.body.pos, f.body.vel, f.body.yaw)
        );
        assert_eq!(
            (back.body.state, back.body.ground_col, back.teleports),
            (f.body.state, 17, 300)
        );
        assert_eq!(back.body.size, GIANT_SIZE);
        let exact = Body {
            land_impact: back.body.land_impact,
            ..f.body.clone()
        };
        assert_eq!(back.body, exact, "all but the landing impact is exact");
        assert!(bytes.len() <= 160, "{} bytes", bytes.len());
    }

    #[test]
    fn pose_is_small_and_close() {
        let p = RemotePose::of(&moving_body(), &Default::default());
        let mut buf = [0u8; 256];
        let bytes = postcard::to_slice(&p, &mut buf).unwrap();
        let back: RemotePose = postcard::from_bytes(bytes).unwrap();
        assert!(bytes.len() <= 21, "{} bytes", bytes.len());
        assert!((back.pos - p.pos).abs().max_element() <= 0.005);
        let angle = |a: f32, b: f32| ((a - b + PI).rem_euclid(TAU) - PI).abs();
        assert!(angle(back.yaw, p.yaw) < 1e-4 && angle(back.tilt_dir, p.tilt_dir) < 0.013);
        assert!((back.tilt - p.tilt).abs() < 0.007 && (back.size - 1.8).abs() < 1e-6);
        assert!((back.vel - p.vel).length() < 0.01);
        assert_eq!((back.anim, back.power), (p.anim, p.power));
        assert_ne!(
            back.teleports,
            RemotePose::of(
                &BodyFull {
                    teleports: 301,
                    ..moving_body()
                },
                &Default::default()
            )
            .teleports as u8 as u32
        );
    }
}
