use std::sync::Arc;

use fb_shared::cause::Hazard;

use crate::builder::{Builder, PrimOpts};
use crate::collider::{ColliderOpts, Shape};
use crate::m::{self, MinMax};
use crate::math::{V3, v3};
use crate::nodes::ROOT;
use crate::scene::Model;
use crate::scene::{Palette, pal};

/// A rotor angle that starts at `start`, eases up to `w` rad/s over about `ease` s and keeps speeding up by `acc`.
#[derive(Clone, Copy, Debug)]
pub struct SpinUp {
    pub start: f64,
    pub w: f64,
    pub acc: f64,
    pub ease: f64,
}

impl SpinUp {
    pub fn new(start: f64, w: f64, acc: f64) -> Self {
        Self {
            start,
            w,
            acc,
            ease: 1.5,
        }
    }

    pub fn angle(&self, t: f64) -> f64 {
        if t <= 0.0 {
            self.start
        } else {
            self.start + (self.w * t * t) / (t + self.ease) + self.acc * t * t
        }
    }

    pub fn omega(&self, t: f64) -> f64 {
        if t <= 0.0 {
            0.2
        } else {
            let e = t + self.ease;
            (0.2f64).at_least((self.w * t * (t + 2.0 * self.ease)) / (e * e) + 2.0 * self.acc * t)
        }
    }
}

/// Seconds until a rotor arm sweeps over (x, z).
pub fn sweep_eta(x: f64, z: f64, angle: f64, omega: f64, arms: u32, cx: f64, cz: f64) -> f64 {
    let phi = m::atan2(-(z - cz), x - cx);
    let period = m::TAU / f64::from(arms);
    let mut d = (phi - angle) % period;
    if d < 0.0 {
        d += period;
    }
    if omega > 0.0 { d / omega } else { (period - d) / -omega }
}

/// Seconds until a rotor arm (half thickness `half`) first touches a bean at `pos`; negative while touching.
pub fn arm_contact_eta(pos: V3, angle: f64, omega: f64, arms: u32, cx: f64, cz: f64, half: f64) -> f64 {
    let r = m::hypot(pos.x - cx, pos.z - cz).at_least(0.5);
    let margin = (half + 0.55) / r / omega.abs();
    let eta = sweep_eta(pos.x, pos.z, angle, omega, arms, cx, cz);
    let period = (m::PI * 2.0) / f64::from(arms) / omega.abs();
    // Just passed: still touching until the arm clears the other side.
    if eta > period - margin {
        eta - period - margin
    } else {
        eta - margin
    }
}

pub fn y_on_ramp(z: f64, z0: f64, y0: f64, z1: f64, y1: f64) -> f64 {
    y0 + ((z - z0) / (z1 - z0)) * (y1 - y0)
}

pub type SpeedFn = Arc<dyn Fn(f64) -> f64 + Send + Sync>;

pub struct BallLaneOpts {
    pub lanes: Vec<f64>,
    pub z_top: f64,
    pub y_top: f64,
    pub z_bottom: f64,
    pub y_bottom: f64,
    pub radius: f64,
    pub speed: SpeedFn,
    pub period: f64,
    pub per_lane: u32,
    pub pal: Option<Palette>,
}

/// Balls rolling down a ramp in lanes (their position is a pure function of time); bots ask `danger`.
pub struct BallLanes {
    balls: Vec<(f64, f64)>,
    z_top: f64,
    len: f64,
    period: f64,
    radius: f64,
    speed: SpeedFn,
}

impl BallLanes {
    fn ball_z(&self, phase: f64, t: f64) -> Option<f64> {
        let tt = t.at_least(0.0) + phase;
        let s = tt - (tt / self.period).floor() * self.period;
        let dist = s * (self.speed)(t);
        (t > 0.0 && dist < self.len).then_some(self.z_top - dist)
    }

    /// Will a ball pass through the box x ∈ [x0, x1], z ∈ [z0, z1] during the next `horizon` seconds?
    pub fn danger(&self, x0: f64, x1: f64, z0: f64, z1: f64, t: f64, horizon: f64) -> bool {
        let reach = self.radius + 0.7;
        for &(x, phase) in &self.balls {
            if x + reach < x0 || x - reach > x1 {
                continue;
            }
            let mut dt = 0.0;
            while dt <= horizon {
                if let Some(z) = self.ball_z(phase, t + dt)
                    && z + reach > z0
                    && z - reach < z1
                {
                    return true;
                }
                dt += 0.1;
            }
        }
        false
    }
}

/// Balls rolling down a ramp in lanes; position is a pure function of time.
pub fn rolling_balls(b: &mut Builder, o: BallLaneOpts) -> Arc<BallLanes> {
    let len = o.z_top - o.z_bottom;
    let slope = (o.y_top - o.y_bottom) / len;
    let cos_a = m::cos(m::atan(slope));
    let palettes = [pal::PINK, pal::ORANGE, pal::PURPLE, pal::RED];
    let mut balls = Vec::new();
    for (li, &x) in o.lanes.iter().enumerate() {
        for k in 0..o.per_lane {
            let phase = b.rng.unit() * o.period + (f64::from(k) * o.period) / f64::from(o.per_lane);
            balls.push((x, phase));
            let ball = b.sphere(
                v3(x, o.y_top + o.radius, o.z_top),
                o.radius,
                o.pal.unwrap_or(palettes[(li + k as usize) % palettes.len()]),
                PrimOpts {
                    dynamic: true,
                    col: ColliderOpts {
                        hit: 1.1,
                        tag: Some(Hazard::Ball),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            let (node, col) = (ball.node, ball.col());
            let (period, r, speed) = (o.period, o.radius, o.speed.clone());
            let (z_top, z_bottom, y_bottom) = (o.z_top, o.z_bottom, o.y_bottom);
            b.mover(move |t, ctx| {
                let tt = t.at_least(0.0) + phase;
                let cycle = (tt / period).floor();
                let s = tt - cycle * period;
                let dist = s * speed(t);
                let alive = t > 0.0 && dist < len;
                // Not solid in the first moments of a cycle: the jump back to the top is not a hit.
                ctx.set_enabled(col, alive && s > 0.1);
                let n = ctx.node(node);
                n.visible = alive;
                if !alive {
                    return;
                }
                let z = z_top - dist;
                let grow = (s / 0.3).at_most(1.0) * ((len - dist) / 1.5).at_most(1.0);
                n.scale = V3::splat(grow.at_least(0.01));
                n.pos = V3::new(x, y_bottom + (z - z_bottom) * slope + r / cos_a, z);
                n.rot.x = -dist / r;
            });
        }
    }
    Arc::new(BallLanes {
        balls,
        z_top: o.z_top,
        len,
        period: o.period,
        radius: o.radius,
        speed: o.speed,
    })
}

#[derive(Clone, Copy)]
pub struct GloveOpts {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// −1: comes from the left (punches towards +x), 1: from the right.
    pub side: f64,
    /// Punch rhythm (rad/s) and phase.
    pub w: f64,
    pub ph: f64,
    /// How far the punch reaches out of its post (m).
    pub reach: f64,
    pub scale: f64,
    /// A post under the glove down to this height (none: hangs from nothing, e.g. out of a wall).
    pub post_to: Option<f64>,
}

/// Where a glove is: its x over time (for bots).
#[derive(Clone, Copy, Debug)]
pub struct Glove {
    x: f64,
    side: f64,
    reach: f64,
    w: f64,
    ph: f64,
}

impl Glove {
    pub fn x_at(&self, t: f64) -> f64 {
        let out = self.reach * m::pow(m::sin(t.at_least(0.0) * self.w + self.ph).at_least(0.0), 3.0);
        self.x - self.side * out
    }
}

/// A boxing glove on a rod that rests in its post and punches out along x now and then: a quick jab
/// and a slower pull back.
pub fn glove_puncher(b: &mut Builder, o: GloveOpts) -> Glove {
    let s = o.scale;
    if let Some(post_to) = o.post_to {
        let h = o.y - post_to + 0.6;
        let deco = PrimOpts {
            no_collide: true,
            ..Default::default()
        };
        b.box_(
            v3(o.x + o.side * 1.4 * s, post_to + h / 2.0, o.z),
            v3(1.1, h, 1.3),
            pal::ORANGE,
            deco,
        );
    }
    let glove = b.anchor(v3(o.x, o.y, o.z), ROOT);
    let at = b.anchor(v3(-o.side * 0.56 * s, 0.0, 0.0), glove);
    b.collider(
        at,
        Shape::Box {
            hx: 0.54 * s,
            hy: 0.48 * s,
            hz: 0.48 * s,
        },
        ColliderOpts {
            hit: 1.1,
            tag: Some(Hazard::Glove),
            sinks: true,
            ..Default::default()
        },
    );
    let model = b.model(Model::Glove, glove);
    let n = b.world.nodes.get_mut(model);
    // The model punches along its +z: turned to punch across.
    n.rot.y = -o.side * (m::PI / 2.0);
    n.scale = V3::splat(s);
    let g = Glove {
        x: o.x,
        side: o.side,
        reach: o.reach,
        w: o.w,
        ph: o.ph,
    };
    b.mover(move |t, ctx| ctx.node(glove).pos.x = g.x_at(t));
    g
}
