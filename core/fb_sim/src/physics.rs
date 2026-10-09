//! The bean: a kinematic character of two spheres with a small state machine.
use fb_shared::NEVER;
use fb_shared::PlayerId;
use fb_shared::cause::Hazard;
use serde::{Deserialize, Serialize};

use crate::beans::Note;
use crate::collider::{ColId, Collider, Contact};
use crate::m::{self, MinMax};
use crate::math::V3;
use crate::world::World;

pub const R: f64 = 0.5;
pub const SPHERES: [f64; 2] = [0.5, 1.1];
pub const GRAVITY: f64 = 28.0;
pub const RUN_SPEED: f64 = 8.5;
pub const JUMP_V: f64 = 10.5;
pub const DIVE_SPEED: f64 = 12.5;

/// A bonus's power (`bonus.rs`), for a while.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Power {
    Giant = 1,
    Jump = 2,
    Speed = 3,
}

impl Power {
    pub const ALL: [Power; 3] = [Power::Giant, Power::Jump, Power::Speed];

    /// How long it lasts (s).
    pub fn duration(self) -> f64 {
        match self {
            Power::Giant => 9.0,
            Power::Jump => 10.0,
            Power::Speed => 8.0,
        }
    }
}
pub const GIANT_SIZE: f64 = 1.8;
pub const GIANT_MASS: f64 = 4.0;
const MEGA_JUMP: f64 = 1.5;
const SPEED_UP: f64 = 1.4;

pub const PORTAL_T: f64 = 0.5;
const LIE: f64 = 1.4;
const DIVE_TILT_MIN: f64 = 0.95;
const DIVE_TILT_MAX: f64 = 1.75;
const SLIDE_TILT: f64 = 1.45;
/// How fast the stick turns a dive and a belly slide (rad/s).
const DIVE_TURN: f64 = 1.2;
const SLIDE_TURN: f64 = 2.0;
/// Below this speed (m/s) the stick no longer turns a dive or a slide.
const STEER_FROM: f64 = 4.0;
const SCOOP_V: f64 = 7.0;
const LEDGE_MIN: f64 = 0.5;
const LEDGE_MAX: f64 = 1.75;
const CLIMB_HANG: f64 = 0.08;
const CLIMB_UP: f64 = 5.5;
const CLIMB_OVER: f64 = 4.5;
const CLIMB_T: f64 = 1.2;
const LADDER_SPEED: f64 = 4.2;
const LADDER_GAP: f64 = 0.17;
const LADDER_LEAP: f64 = 4.0;
const LADDER_AGAIN: f64 = 0.35;
const SPINE: f64 = SPHERES[1] - SPHERES[0];
pub const BEAN_GAP: f64 = 1.05;
const TUMBLE_E: f64 = 0.4;
/// A dive or slide into a wall faster than this (m/s) bonks: thrown back, stunned for BONK_T.
const BONK_SPEED: f64 = 8.0;
const BONK_BACK: f64 = 3.0;
const BONK_UP: f64 = 2.5;
const BONK_T: f64 = 0.5;
const AIR_ACCEL: f64 = 24.0;
pub(crate) const SUBSTEP_REACH: f64 = 0.6;
const MAX_SUBSTEPS: f64 = 8.0;
const STEEP_FROM: f64 = 0.77;
const STEEP_SLIP: f64 = 7.0;
const DOWN: V3 = V3::new(0.0, -1.0, 0.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum BodyState {
    #[default]
    Normal,
    Stun,
    Dive,
    Slide,
    Tumble,
    Getup,
    Climb,
    Portal,
    Ladder,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyInput {
    pub mx: f64,
    pub mz: f64,
    pub jump: bool,
    pub dive: bool,
}

impl From<fb_shared::input::InputFrame> for BodyInput {
    fn from(f: fb_shared::input::InputFrame) -> Self {
        Self {
            mx: f64::from(f.mx) / 127.0,
            mz: f64::from(f.mz) / 127.0,
            jump: f.jump(),
            dive: f.dive(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OtherBody {
    pub id: PlayerId,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub tilt: f64,
    pub tilt_dir: f64,
    pub size: f64,
}

impl OtherBody {
    pub fn mass(&self) -> f64 {
        if self.size > 1.0 { GIANT_MASS } else { 1.0 }
    }
}

/// Buffers a step reuses: kept by the caller from step to step, never part of a body's state.
#[derive(Default)]
pub struct StepScratch {
    cols: Vec<ColId>,
    hit_done: Vec<ColId>,
}

/// What happened during the last step (sounds, effects, credit); cleared before each tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepEvents {
    pub jumped: bool,
    pub bounced: bool,
    pub hit_something: bool,
    pub stunned: bool,
    pub knocked: bool,
    pub bumped: f64,
    /// Knocked over by this bean's tackle.
    pub tackled_by: Option<PlayerId>,
    /// Tackles this bean landed.
    pub tackles: u32,
    /// Contacts, tackles and dives in detail (`--trace-hits`).
    pub notes: Vec<Note>,
    pub hazard: Option<Hazard>,
    pub portal_in: bool,
    pub portal_out: bool,
}

/// A collider that reports touches was touched (`on_touch`: with the contact normal) or stood on
/// (`on_ground`), in the middle of a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub col: ColId,
    /// The contact normal; None when standing on it.
    pub normal: Option<V3>,
}

/// Called as the step touches a collider or stands on it: it may change the body (a portal)
/// and the world (a door that breaks) before the step goes on.
pub type OnTouch<'a> = dyn FnMut(&mut World, &mut Body, &mut StepEvents, Touch) + 'a;

/// Where the body stands on its ground before the world moves (see `before_world_update`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Carry {
    col: Option<ColId>,
    local: V3,
}

/// The full state of a bean: everything prediction needs to carry on from a server state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub actor: i32,
    pub pos: V3,
    pub vel: V3,
    pub yaw: f64,
    pub grounded: bool,
    /// The collider underfoot.
    pub ground_col: Option<ColId>,
    pub state: BodyState,
    pub state_t: f64,
    pub coyote: f64,
    pub jump_buf: f64,
    pub slow_until: f64,
    pub slow_k: f64,
    pub land_impact: f64,
    pub tilt: f64,
    pub tilt_dir: f64,
    pub power: Option<Power>,
    pub power_until: f64,
    pub size: f64,
    pub climb_to: V3,
    /// The last bean this dive (and the slide after it) knocked over: it is not knocked over again.
    pub tackled: Option<PlayerId>,
}

/// Centre of collision sphere i of a body with its feet at `pos`, tipped by `tilt` towards `tilt_dir`.
#[inline]
pub(crate) fn sphere_at(pos: V3, tilt: f64, tilt_dir: f64, size: f64, i: usize) -> V3 {
    let c = V3::new(pos.x, pos.y + SPHERES[0] * size, pos.z);
    if i == 0 {
        return c;
    }
    let s = m::sin(tilt) * SPINE * size;
    V3::new(
        c.x + m::sin(tilt_dir) * s,
        c.y + m::cos(tilt) * SPINE * size,
        c.z + m::cos(tilt_dir) * s,
    )
}

/// Moves where a body is drawn out of solid colliders, as its simulation would.
pub fn push_out(world: &World, mut pos: V3, tilt: f64, tilt_dir: f64, size: f64) -> V3 {
    let r = R * size;
    let mut cols = Vec::new();
    world.query(
        pos.x,
        pos.z,
        r + 1.2 * size + if tilt > 0.0 { SPINE * size } else { 0.0 },
        &mut cols,
    );
    let mut hit = Contact::default();
    for _ in 0..2 {
        let mut any = false;
        for &ci in &cols {
            let col = world.col(ci);
            if !col.enabled || col.opts.trigger {
                continue;
            }
            for si in 0..2 {
                if !col.contact(sphere_at(pos, tilt, tilt_dir, size, si), r, &mut hit) {
                    continue;
                }
                pos += hit.normal * hit.depth;
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    pos
}

/// Solid ground a hand can hold on to: no hazards, pads, bumpers, ice or triggers.
fn holdable(c: &Collider) -> bool {
    c.enabled
        && c.opts.is_static
        && !c.opts.trigger
        && c.opts.hit == 0.0
        && c.opts.tag.is_none()
        && !c.opts.sweep
        && c.opts.bounce == 0.0
        && c.opts.pad == 0.0
        && c.opts.slip < 0.5
        && !c.opts.no_grab
}

/// A moving platform the body rides on; not a hazard like a rotor arm or a hammer.
fn rides(c: Option<&Collider>) -> bool {
    c.is_some_and(|c| !c.opts.is_static && c.enabled && c.opts.hit == 0.0 && c.opts.tag.is_none())
}

/// Ground the feet stay on as it falls away under them (a slope, a crest): any solid, harmless ground but
/// a pad or a bumper, which throw the body off it.
fn keeps_feet(c: Option<&Collider>) -> bool {
    let Some(c) = c else { return false };
    c.enabled
        && !c.opts.trigger
        && c.opts.hit == 0.0
        && c.opts.tag.is_none()
        && c.opts.bounce == 0.0
        && c.opts.pad == 0.0
}

impl Body {
    pub fn new(actor: i32) -> Self {
        Self {
            actor,
            pos: V3::ZERO,
            vel: V3::ZERO,
            yaw: 0.0,
            grounded: false,
            ground_col: None,
            state: BodyState::Normal,
            state_t: 0.0,
            coyote: 0.0,
            jump_buf: 0.0,
            slow_until: NEVER,
            slow_k: 1.0,
            land_impact: 0.0,
            tilt: 0.0,
            tilt_dir: 0.0,
            power: None,
            power_until: NEVER,
            size: 1.0,
            climb_to: V3::ZERO,
            tackled: None,
        }
    }

    pub fn reset(&mut self, p: V3, yaw: f64) {
        *self = Self {
            pos: p,
            yaw,
            ..Self::new(self.actor)
        };
    }

    pub fn down(&self) -> bool {
        matches!(self.state, BodyState::Tumble | BodyState::Getup)
    }

    pub fn mass(&self) -> f64 {
        if self.size > 1.0 { GIANT_MASS } else { 1.0 }
    }

    pub fn in_portal(&self) -> bool {
        self.state == BodyState::Portal
    }

    pub fn climbing_over(&self) -> bool {
        self.state == BodyState::Climb && self.state_t <= CLIMB_T - CLIMB_HANG && self.pos.y >= self.climb_to.y - 1e-4
    }

    fn ground<'w>(&self, world: &'w World) -> Option<&'w Collider> {
        self.ground_col.map(|c| world.col(c))
    }

    pub fn give_power(&mut self, kind: Power, t: f64) {
        self.power = Some(kind);
        self.power_until = t + kind.duration();
    }

    pub fn stun(&mut self, t: f64) {
        // (In a portal the trip is already under way: nothing may stop it.)
        if self.down() || self.in_portal() {
            return;
        }
        self.state = BodyState::Stun;
        self.state_t = t;
    }

    /// Knocked over towards (vx, vz); a giant shrugs most of it off unless `force` (a sweeping arm).
    pub fn knock(&mut self, ev: &mut StepEvents, mut vx: f64, mut vz: f64, mut vy: f64, t: f64, force: bool) {
        if self.in_portal() {
            return;
        }
        if self.size > 1.0 {
            let k = if force { 0.5 } else { 0.25 };
            vx *= k;
            vz *= k;
            vy *= if force { 0.7 } else { 0.4 };
            if !force {
                self.vel.x += vx;
                self.vel.z += vz;
                self.vel.y = self.vel.y.at_least(vy);
                self.stun(0.35);
                return;
            }
        }
        if force {
            self.vel.x = vx;
            self.vel.z = vz;
        } else {
            self.vel.x += vx;
            self.vel.z += vz;
        }
        self.vel.y = self.vel.y.at_least(vy);
        self.grounded = false;
        if m::hypot(vx, vz) > 0.1 {
            self.tilt_dir = m::atan2(vx, vz);
        }
        let was = self.state == BodyState::Tumble;
        if !was {
            ev.knocked = true;
        }
        self.state = BodyState::Tumble;
        self.state_t = (if was { self.state_t } else { 0.0 }).at_least(t);
    }

    pub fn sphere(&self, i: usize) -> V3 {
        sphere_at(self.pos, self.tilt, self.tilt_dir, self.size, i)
    }

    /// Into a portal: out of play for PORTAL_T, gliding to the exit `p`.
    pub fn enter_portal(&mut self, ev: &mut StepEvents, p: V3, yaw: f64, min_speed: f64, lift: Option<f64>) {
        if self.state == BodyState::Portal {
            return;
        }
        let sp = min_speed.at_least(m::hypot(self.vel.x, self.vel.z));
        self.climb_to = p;
        self.vel = V3::new(
            m::sin(yaw) * sp,
            lift.unwrap_or(self.vel.y.at_least(3.0)),
            m::cos(yaw) * sp,
        );
        self.yaw = yaw;
        self.state = BodyState::Portal;
        self.state_t = PORTAL_T;
        self.tilt = 0.0;
        self.grounded = false;
        self.ground_col = None;
        ev.portal_in = true;
    }

    fn portal_step(&mut self, ev: &mut StepEvents, dt: f64) {
        if self.state_t <= dt + 1e-9 {
            self.pos = self.climb_to;
            self.state = BodyState::Normal;
            self.state_t = 0.0;
            ev.portal_out = true;
            return;
        }
        self.pos = self.pos.lerp(self.climb_to, dt / self.state_t);
        self.state_t -= dt;
    }

    /// Before moving the world: where we stand on the ground collider.
    pub fn before_world_update(&self, world: &World) -> Carry {
        if let Some(g) = self.ground(world)
            && self.grounded
            && g.enabled
            && !g.opts.sweep
            // (Static ground does not move: the round trip through its matrices would only add drift.)
            && !g.opts.is_static
        {
            return Carry {
                col: Some(g.index),
                local: g.inv.transform_point3(self.pos),
            };
        }
        Carry::default()
    }

    /// After moving the world: ride along with a moving platform.
    pub fn after_world_update(&mut self, carry: &Carry, world: &World) {
        let Some(ci) = carry.col else { return };
        let c = world.col(ci);
        if !c.enabled {
            return;
        }
        self.pos = c.cur.transform_point3(carry.local);
        let dir = c.cur.transform_vector3(V3::new(0.0, 0.0, 1.0)).normalize_or_zero();
        let w = c.prev.transform_vector3(V3::new(0.0, 0.0, 1.0)).normalize_or_zero();
        let mut d_yaw = m::atan2(dir.x, dir.z) - m::atan2(w.x, w.z);
        if d_yaw.abs() > m::PI {
            // Across ±π: the short way round.
            d_yaw = m::atan2(m::sin(d_yaw), m::cos(d_yaw));
        }
        if d_yaw.abs() < 0.5 {
            self.yaw += d_yaw;
            self.tilt_dir += d_yaw;
        }
    }

    fn ground_velocity(&self, world: &World, dt: f64) -> V3 {
        let c = self.ground(world);
        if !rides(c) {
            return V3::ZERO;
        }
        let c = c.unwrap();
        let l = c.inv.transform_point3(self.pos);
        let out = c.surface_velocity(l, dt);
        let len = out.length();
        if len > 6.0 { out * (6.0 / len) } else { out }
    }

    fn accelerate(&mut self, tx: f64, tz: f64, acc: f64) {
        let dx = tx - self.vel.x;
        let dz = tz - self.vel.z;
        let dl = m::hypot(dx, dz);
        if dl <= acc {
            self.vel.x = tx;
            self.vel.z = tz;
        } else {
            self.vel.x += (dx / dl) * acc;
            self.vel.z += (dz / dl) * acc;
        }
    }

    /// Turns the horizontal velocity towards the stick, at most `rate` rad/s, keeping its speed, and the body with
    /// it by as much. (Not to face the velocity: after a bump what is left of it points anywhere.)
    fn steer(&mut self, input: BodyInput, rate: f64, dt: f64) {
        let sp = m::hypot(self.vel.x, self.vel.z);
        if m::hypot(input.mx, input.mz) < 0.3 || sp < STEER_FROM {
            return;
        }
        let cur = m::atan2(self.vel.x, self.vel.z);
        let mut d = m::atan2(input.mx, input.mz) - cur;
        d = m::clamp(m::atan2(m::sin(d), m::cos(d)), -rate * dt, rate * dt);
        self.vel.x = m::sin(cur + d) * sp;
        self.vel.z = m::cos(cur + d) * sp;
        self.yaw += d;
    }

    pub fn step(
        &mut self,
        scratch: &mut StepScratch,
        ev: &mut StepEvents,
        dt: f64,
        input: BodyInput,
        world: &mut World,
        t: f64,
        touch: &mut OnTouch,
    ) {
        if self.state == BodyState::Portal {
            return self.portal_step(ev, dt);
        }
        if self.power.is_some() && t >= self.power_until {
            self.power = None;
        }
        let pw = self.power;
        self.size = if pw == Some(Power::Giant) { GIANT_SIZE } else { 1.0 };
        let size = self.size;
        let r = R * size;
        let slow = (if t < self.slow_until { self.slow_k } else { 1.0 })
            * (if pw == Some(Power::Speed) { SPEED_UP } else { 1.0 });
        let g = self.grounded;
        let slip = if g {
            self.ground(world).map_or(0.0, |c| c.opts.slip)
        } else {
            0.0
        };
        self.coyote = if g { 0.12 } else { (self.coyote - dt).at_least(0.0) };
        self.jump_buf = if input.jump {
            0.12
        } else {
            (self.jump_buf - dt).at_least(0.0)
        };
        self.state_t -= dt;
        let state_before = self.state;
        let plat_v = if g { self.ground_velocity(world, dt) } else { V3::ZERO };

        match self.state {
            BodyState::Tumble => {
                self.accelerate(input.mx * RUN_SPEED * 0.3, input.mz * RUN_SPEED * 0.3, 5.0 * dt);
                if g {
                    let f = m::exp(-(2.4 - slip * 2.0) * dt);
                    self.vel.x *= f;
                    self.vel.z *= f;
                }
                self.tilt += (LIE - self.tilt) * (9.0 * dt).at_most(1.0);
                let settled = g && m::hypot(self.vel.x, self.vel.z) < 3.0;
                if (self.state_t <= 0.0 && settled) || self.state_t < -2.5 {
                    self.state = BodyState::Getup;
                    self.state_t = 0.4;
                }
            }
            BodyState::Getup => {
                let f = m::exp(-8.0 * dt);
                self.vel.x *= f;
                self.vel.z *= f;
                self.tilt *= m::exp(-10.0 * dt);
                if self.state_t <= 0.0 || (g && self.jump_buf > 0.0 && self.state_t < 0.3) {
                    self.state = BodyState::Normal;
                    self.tilt = 0.0;
                }
            }
            BodyState::Stun | BodyState::Slide => {
                if self.state == BodyState::Slide {
                    self.steer(input, SLIDE_TURN, dt);
                }
                let f = if g {
                    m::exp(-(if self.state == BodyState::Slide { 3.5 } else { 5.0 }) * (1.0 - slip * 0.8) * dt)
                } else {
                    1.0
                };
                self.vel.x *= f;
                self.vel.z *= f;
                if self.state_t <= 0.0 && (g || self.state == BodyState::Slide) {
                    self.state = BodyState::Normal;
                }
                if self.state == BodyState::Slide && g && self.jump_buf > 0.0 && self.state_t < 0.33 {
                    self.state = BodyState::Normal;
                }
                if self.state_t <= -3.0 {
                    self.state = BodyState::Normal;
                }
            }
            BodyState::Dive => {
                self.steer(input, DIVE_TURN, dt);
                if g && self.state_t < 0.25 {
                    self.state = BodyState::Slide;
                    self.state_t = 0.45;
                }
            }
            BodyState::Climb => self.climb(dt, input),
            BodyState::Ladder => self.ladder_step(ev, dt, input, world),
            _ => {
                let acc = (if g { 60.0 - 58.5 * slip } else { AIR_ACCEL }) * dt;
                let cap = RUN_SPEED * slow;
                let fly = if g { 0.0 } else { m::hypot(self.vel.x, self.vel.z) };
                let top = if fly > cap { fly } else { cap };
                self.accelerate(input.mx * top, input.mz * top, acc);
                if m::hypot(input.mx, input.mz) > 0.1 {
                    let mut d = m::atan2(input.mx, input.mz) - self.yaw;
                    d = m::atan2(m::sin(d), m::cos(d));
                    self.yaw += d * (14.0 * dt).at_most(1.0);
                }
                if self.jump_buf > 0.0 && self.coyote > 0.0 {
                    self.vel.y = JUMP_V
                        * (if slow < 1.0 { 0.8 } else { 1.0 })
                        * (if pw == Some(Power::Jump) { MEGA_JUMP } else { 1.0 });
                    self.coyote = 0.0;
                    self.jump_buf = 0.0;
                    self.grounded = false;
                    ev.jumped = true;
                    if g {
                        self.vel.x += plat_v.x;
                        self.vel.z += plat_v.z;
                        self.vel.y += plat_v.y.at_least(0.0);
                    }
                }
                if input.dive {
                    self.state = BodyState::Dive;
                    self.state_t = 0.6;
                    self.tackled = None;
                    let l = m::hypot(input.mx, input.mz);
                    let mut fx = m::sin(self.yaw);
                    let mut fz = m::cos(self.yaw);
                    if l > 0.1 {
                        fx = input.mx / l;
                        fz = input.mz / l;
                        self.yaw = m::atan2(fx, fz);
                    }
                    let along = (self.vel.x * fx + self.vel.z * fz).at_least(0.0);
                    let sp = (DIVE_SPEED * slow).at_least(along.at_most(DIVE_SPEED * 1.25));
                    self.vel.x = fx * sp + if g { plat_v.x } else { 0.0 };
                    self.vel.z = fz * sp + if g { plat_v.z } else { 0.0 };
                    // (In the air it adds no height: a lunge, not a second jump.)
                    if g {
                        self.vel.y = 6.0 + plat_v.y.at_least(0.0);
                    }
                    self.grounded = false;
                    ev.notes.push(Note::Dive {
                        air: !g,
                        speed: sp,
                        vy: self.vel.y,
                    });
                }
            }
        }
        if self.state == BodyState::Dive || self.state == BodyState::Slide {
            let along = self.vel.x * m::sin(self.yaw) + self.vel.z * m::cos(self.yaw);
            let want = if self.state == BodyState::Dive {
                m::clamp(m::atan2(along.at_least(2.0), self.vel.y), DIVE_TILT_MIN, DIVE_TILT_MAX)
            } else {
                SLIDE_TILT
            };
            self.tilt += (want - self.tilt) * (12.0 * dt).at_most(1.0);
            self.tilt_dir = self.yaw;
        } else if !self.down() {
            self.tilt = if self.tilt > 0.02 {
                self.tilt * m::exp(-12.0 * dt)
            } else {
                0.0
            };
        }

        let climbing = self.state == BodyState::Climb;
        let on_ladder = self.state == BodyState::Ladder;
        let still = climbing || on_ladder;
        if !still {
            self.vel.y = (self.vel.y - GRAVITY * dt).at_least(-32.0);
        }
        let travel = if still { 0.0 } else { self.vel.length() * dt };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 1…MAX_SUBSTEPS first"
        )]
        let substeps = MAX_SUBSTEPS.at_most(1f64.at_least((travel / (r * SUBSTEP_REACH)).ceil())) as usize;

        let vy_before = self.vel.y;
        self.grounded = false;
        let mut new_ground: Option<ColId> = None;
        let mut ground_n = V3::ZERO;
        let mut wall_n = V3::ZERO;
        let StepScratch { cols, hit_done } = scratch;
        hit_done.clear();
        let mut ladder: Option<ColId> = None;
        let mut hit = Contact::default();
        // A touch that sends the body into a portal ends the step's collisions: it is out of play.
        'substeps: for _ in 0..substeps {
            if !still {
                self.pos += self.vel * (dt / substeps as f64);
            }
            world.query(
                self.pos.x,
                self.pos.z,
                r + 1.2 * size + if self.tilt > 0.0 { SPINE * size } else { 0.0 },
                cols,
            );
            for _ in 0..3 {
                let mut any = false;
                for &ci in cols.iter() {
                    let col = world.col(ci);
                    if !col.enabled {
                        continue;
                    }
                    if climbing && col.opts.hit == 0.0 && !col.opts.sweep && col.opts.bounce == 0.0 && !col.opts.trigger
                    {
                        continue;
                    }
                    for si in 0..SPHERES.len() {
                        // (Read again: a touch may have changed the world; whether the
                        // collider is solid was looked at once, above.)
                        let col = world.col(ci);
                        let c = self.sphere(si);
                        if !col.contact(c, r, &mut hit) {
                            continue;
                        }
                        if col.opts.trigger {
                            if !hit_done.contains(&ci) {
                                hit_done.push(ci);
                                if col.opts.ladder {
                                    ladder = Some(ci);
                                } else if col.opts.on_touch {
                                    let normal = Some(hit.normal);
                                    touch(world, self, ev, Touch { col: ci, normal });
                                    if self.in_portal() {
                                        break 'substeps;
                                    }
                                }
                            }
                            break;
                        }
                        any = true;
                        let n = hit.normal;
                        self.pos += n * hit.depth;
                        let vn = self.vel.dot(n);
                        if col.opts.bounce != 0.0 && !hit_done.contains(&ci) {
                            hit_done.push(ci);
                            let push = col.opts.bounce.at_least(-vn * 0.8);
                            self.vel += n * (push - vn);
                            self.vel.y = self.vel.y.at_least(4.0);
                            if n.y < 0.5 {
                                if -vn > 11.0 && self.state != BodyState::Tumble {
                                    self.knock(ev, n.x * 2.0, n.z * 2.0, 5.0, 0.7, false);
                                } else {
                                    self.stun(0.5);
                                }
                            }
                            ev.hit_something = true;
                        } else if vn < 0.0 {
                            let e = if self.state == BodyState::Tumble && vn < -3.0 {
                                TUMBLE_E
                            } else {
                                0.0
                            };
                            self.vel += n * (-vn * (1.0 + e));
                            // Head first into a wall: a bonk.
                            if matches!(self.state, BodyState::Dive | BodyState::Slide)
                                && -vn > BONK_SPEED
                                && n.y.abs() < 0.35
                            {
                                self.vel += n * BONK_BACK;
                                self.vel.y = self.vel.y.at_least(BONK_UP);
                                self.stun(BONK_T);
                                ev.hit_something = true;
                                ev.notes.push(Note::Bonk { speed: -vn });
                            }
                        }
                        let fresh =
                            self.state != BodyState::Tumble && !(self.state == BodyState::Stun && self.state_t > 0.2);
                        if col.opts.sweep && n.y < 0.55 && !hit_done.contains(&ci) {
                            hit_done.push(ci);
                            let sv = col.surface_velocity(hit.local, dt);
                            let sp = m::hypot(sv.x, sv.z);
                            let behind = sv.x * n.x + sv.z * n.z < -0.3 * sp;
                            if !behind && self.state == BodyState::Tumble {
                                if g && self.vel.y < SCOOP_V * 0.6 {
                                    self.vel.y = SCOOP_V;
                                    ev.hit_something = true;
                                }
                                self.state_t = self.state_t.at_least(0.6);
                            } else if !behind && sp > 1.0 {
                                let k = (11.0 / sp).at_most(1.0) * 1.5 * col.opts.hit;
                                self.knock(
                                    ev,
                                    sv.x * k + n.x * 1.5,
                                    sv.z * k + n.z * 1.5,
                                    5.0 + (sp * 0.15).at_most(2.0),
                                    1.1,
                                    true,
                                );
                                ev.hit_something = true;
                            }
                        } else if col.opts.hit != 0.0 && !col.opts.sweep && !hit_done.contains(&ci) && fresh {
                            hit_done.push(ci);
                            let sv = col.surface_velocity(hit.local, dt);
                            let sp = sv.dot(n).at_least(0.0) * 0.6 + sv.length() * 0.4;
                            if sp > 2.2 {
                                let k = col.opts.hit;
                                let away = (sp * 0.2).at_most(2.5);
                                if sp * k > 4.2 {
                                    self.knock(
                                        ev,
                                        sv.x * k + n.x * away,
                                        sv.z * k + n.z * away,
                                        4.5 + sp * 0.3,
                                        0.8 + (sp * 0.06).at_most(0.8),
                                        false,
                                    );
                                } else {
                                    self.vel.x += sv.x * k + n.x * away * 0.5;
                                    self.vel.z += sv.z * k + n.z * away * 0.5;
                                    self.vel.y = self.vel.y.at_least(3.5);
                                    self.stun(0.6);
                                }
                                ev.hit_something = true;
                            }
                        }
                        if n.y > 0.55 {
                            self.grounded = true;
                            new_ground = Some(ci);
                            if n.y > ground_n.y {
                                ground_n = n;
                            }
                        } else if let Some(tag) = col.opts.tag {
                            ev.hazard = Some(tag);
                        } else if n.y.abs() < 0.35 && holdable(col) {
                            wall_n = n;
                        }
                        if col.opts.on_touch {
                            touch(
                                world,
                                self,
                                ev,
                                Touch {
                                    col: ci,
                                    normal: Some(n),
                                },
                            );
                            if self.in_portal() {
                                break 'substeps;
                            }
                        }
                    }
                }
                if !any {
                    break;
                }
            }
        }
        if self.in_portal() {
            // Nothing else this step: no ground, no platform's speed, no push from the others.
            self.grounded = false;
            self.ground_col = None;
            return;
        }
        // Feet kept on the ground over a crest or down a slope, static or moving: without it a body going
        // down a slope left it every tick (grounded only on the hops' landings), so ice gave no slide and
        // air control steered it.
        if g && !self.grounded
            && !ev.jumped
            && self.state != BodyState::Dive
            && !still
            && self.vel.y < 1.0
            && keeps_feet(self.ground(world))
            && let Some((col, n)) = self.snap_down(world, r, dt, cols)
        {
            self.grounded = true;
            new_ground = Some(col);
            ground_n = n;
        }

        if state_before == BodyState::Normal
            && self.state == BodyState::Normal
            && !g
            && !self.grounded
            && self.vel.y < 4.0
            && wall_n.length_squared() > 0.0
            && self.push_into_wall(input, wall_n) > 0.5
            && let Some(n) = self.grab_ledge(world, wall_n)
        {
            ground_n = n;
        }
        if let Some(l) = ladder
            && state_before == BodyState::Normal
            && self.state == BodyState::Normal
            && self.state_t <= 0.0
            && !ev.jumped
        {
            self.grab_ladder(world.col(l), input);
        }
        if on_ladder && self.state == BodyState::Ladder && self.grounded {
            let back = input.mx * m::sin(self.yaw) + input.mz * m::cos(self.yaw);
            if back < -0.3 {
                self.state = BodyState::Normal;
                self.state_t = LADDER_AGAIN;
            }
        }

        let old_ground = self.ground_col;
        self.ground_col = new_ground;
        let rel_old = g && !ev.jumped && self.state != BodyState::Dive;
        if rel_old && !self.grounded {
            self.vel.x += plat_v.x;
            self.vel.z += plat_v.z;
            self.vel.y += plat_v.y.at_least(0.0);
        } else if self.grounded && self.ground_col != old_ground {
            let gv = self.ground_velocity(world, dt);
            self.vel.x += (if rel_old { plat_v.x } else { 0.0 }) - gv.x;
            self.vel.z += (if rel_old { plat_v.z } else { 0.0 }) - gv.z;
        }
        if self.grounded {
            self.vel.y = self.vel.y.at_least(0.0);
            let ng = new_ground.map(|c| world.col(c));
            if let Some(cv) = ng.and_then(|c| c.opts.conveyor) {
                self.pos += cv * dt;
            }
            if rides(ng) && ground_n.y < STEEP_FROM && ground_n.y > 0.0 {
                let k = ((STEEP_FROM - ground_n.y) / (STEEP_FROM - 0.55)).at_most(1.0);
                let s = m::sqrt(1.0 - ground_n.y * ground_n.y);
                let d = (STEEP_SLIP * k * dt) / s;
                self.pos.x += ground_n.x * ground_n.y * d;
                self.pos.y += (ground_n.y * ground_n.y - 1.0) * d;
                self.pos.z += ground_n.z * ground_n.y * d;
            }
            let slide = (ng.map_or(0.0, |c| c.opts.slip) * 1.3).at_least(if self.state == BodyState::Tumble {
                0.6
            } else {
                0.0
            });
            if slide > 0.0 && ground_n.y > 0.0 {
                self.vel.x += GRAVITY * ground_n.x * ground_n.y * slide * dt;
                self.vel.z += GRAVITY * ground_n.z * ground_n.y * slide * dt;
            }
            if let Some(ci) = new_ground
                && world.col(ci).opts.on_ground
            {
                touch(world, self, ev, Touch { col: ci, normal: None });
                if self.in_portal() {
                    self.grounded = false;
                    self.ground_col = None;
                    return;
                }
            }
            if let Some(c) = new_ground.map(|c| world.col(c))
                && c.opts.pad != 0.0
            {
                self.vel.y = c.opts.pad;
                if let Some(l) = c.opts.launch {
                    self.vel.x = l.x;
                    self.vel.z = l.z;
                }
                self.grounded = false;
                self.ground_col = None;
                ev.bounced = true;
            } else if !g {
                self.land_impact = (-vy_before / 20.0).at_most(1.0);
            }
        }
        if state_before != BodyState::Stun && self.state == BodyState::Stun {
            ev.stunned = true;
        }
    }

    fn push_into_wall(&self, input: BodyInput, n: V3) -> f64 {
        let l = m::hypot(input.mx, input.mz);
        let h = m::hypot(n.x, n.z);
        if l < 0.3 || h < 1e-3 {
            return 0.0;
        }
        -(input.mx * n.x + input.mz * n.z) / (l * h)
    }

    /// Ground within a short drop below the feet: the body is set down on it; returns it and its normal.
    /// (`probe`: a buffer for the colliders around, reused.)
    fn snap_down(&mut self, world: &World, r: f64, dt: f64, probe: &mut Vec<ColId>) -> Option<(ColId, V3)> {
        let reach = (0.05 + m::hypot(self.vel.x, self.vel.z) * dt * 1.5).at_most(0.3) * self.size;
        let y0 = self.pos.y;
        self.pos.y -= reach;
        let c = self.sphere(0);
        let mut best = None;
        let mut lift = 0.0;
        let mut hit = Contact::default();
        world.query(self.pos.x, self.pos.z, r + 0.5, probe);
        for &ci in probe.iter() {
            let col = world.col(ci);
            if !col.enabled || col.opts.trigger || col.opts.bounce != 0.0 || col.opts.pad != 0.0 {
                continue;
            }
            if !col.contact(c, r, &mut hit) || hit.normal.y <= 0.55 {
                continue;
            }
            let up = hit.depth / hit.normal.y;
            if up > lift {
                lift = up;
                best = Some((ci, hit.normal));
            }
        }
        if best.is_none() {
            self.pos.y = y0;
            return None;
        }
        self.pos.y = y0.at_most(self.pos.y + lift);
        self.vel.y = self.vel.y.at_least(0.0);
        best
    }

    /// Room for the upright body with its feet at (x, y, z)?
    fn fits(&self, world: &World, x: f64, y: f64, z: f64) -> bool {
        let k = self.size;
        let mut cols = Vec::new();
        let mut hit = Contact::default();
        world.query(x, z, R * k + 0.2, &mut cols);
        for s in SPHERES {
            let c = V3::new(x, y + s * k, z);
            for &ci in &cols {
                let col = world.col(ci);
                if !col.enabled || col.opts.trigger {
                    continue;
                }
                if col.contact(c, R * k, &mut hit) && hit.depth > 0.03 {
                    return false;
                }
            }
        }
        true
    }

    /// Airborne against a wall: a flat top within reach above it is caught and climbed.
    /// Returns the last ledge normal seen (it becomes the step's ground normal).
    fn grab_ledge(&mut self, world: &World, n: V3) -> Option<V3> {
        let k = self.size;
        let h = m::hypot(n.x, n.z);
        let nx = n.x / h;
        let nz = n.z / h;
        let reach = R * k + 0.3 * k;
        let px = self.pos.x - nx * reach;
        let pz = self.pos.z - nz * reach;
        let y0 = self.pos.y + LEDGE_MAX * k;
        let span = (LEDGE_MAX - LEDGE_MIN) * k;
        let p = V3::new(px, y0, pz);
        let mut best = -1.0;
        let mut best_col: Option<ColId> = None;
        let mut ground_n = V3::ZERO;
        let mut probe = Vec::new();
        let mut hit = Contact::default();
        world.query(px, pz, 0.1, &mut probe);
        for &ci in &probe {
            let col = world.col(ci);
            if !col.enabled || col.opts.trigger {
                continue;
            }
            if col.contact(p, 0.05, &mut hit) {
                return best_col.map(|_| ground_n);
            }
            let Some((t, rn)) = col.raycast(p, DOWN, span) else {
                continue;
            };
            if best >= 0.0 && t >= best {
                continue;
            }
            best = t;
            best_col = Some(ci);
            ground_n = rn;
        }
        let bc = world.col(best_col?);
        if !holdable(bc) || ground_n.y < 0.8 {
            return Some(ground_n);
        }
        let top = y0 - best + 0.02;
        if !self.fits(world, px, top, pz) || !self.fits(world, self.pos.x, top, self.pos.z) {
            return Some(ground_n);
        }
        self.state = BodyState::Climb;
        self.state_t = CLIMB_T;
        self.climb_to = V3::new(px, top, pz);
        self.vel = V3::ZERO;
        self.yaw = m::atan2(-nx, -nz);
        self.tilt = 0.0;
        self.coyote = 0.0;
        self.jump_buf = 0.0;
        Some(ground_n)
    }

    fn climb(&mut self, dt: f64, input: BodyInput) {
        let to = self.climb_to;
        let dx = to.x - self.pos.x;
        let dz = to.z - self.pos.z;
        let d = m::hypot(dx, dz);
        let rising = self.pos.y < to.y - 1e-4;
        let away = d > 1e-3 && m::hypot(input.mx, input.mz) > 0.3 && (input.mx * dx + input.mz * dz) / d < -0.5;
        if (rising && away) || self.state_t <= 0.0 {
            self.state = BodyState::Normal;
            if d > 1e-3 {
                self.vel = V3::new((-dx / d) * 2.0, 0.0, (-dz / d) * 2.0);
            }
            return;
        }
        let (x0, y0, z0) = (self.pos.x, self.pos.y, self.pos.z);
        if self.state_t > CLIMB_T - CLIMB_HANG {
        } else if rising {
            self.pos.y = to.y.at_most(self.pos.y + CLIMB_UP * self.size * dt);
        } else {
            let step = CLIMB_OVER * self.size * dt;
            if d <= step {
                self.pos = to;
                self.state = BodyState::Normal;
                self.vel = if d > 1e-3 {
                    V3::new((dx / d) * 3.0, 0.0, (dz / d) * 3.0)
                } else {
                    V3::new(m::sin(self.yaw) * 3.0, 0.0, m::cos(self.yaw) * 3.0)
                };
                return;
            }
            self.pos.x += (dx / d) * step;
            self.pos.z += (dz / d) * step;
        }
        self.vel = V3::new((self.pos.x - x0) / dt, (self.pos.y - y0) / dt, (self.pos.z - z0) / dt);
        if d > 1e-3 {
            self.yaw = m::atan2(dx, dz);
        }
    }

    fn grab_ladder(&mut self, col: &Collider, input: BodyInput) {
        let dir = col.cur.transform_vector3(V3::new(0.0, 0.0, 1.0)).normalize_or_zero();
        let h = m::hypot(dir.x, dir.z);
        let crate::collider::Shape::Box { hy, .. } = col.shape else {
            return;
        };
        if h < 1e-3 {
            return;
        }
        let nx = dir.x / h;
        let nz = dir.z / h;
        let il = m::hypot(input.mx, input.mz);
        if -(input.mx * nx + input.mz * nz) < 0.5 * il || il < 0.3 {
            return;
        }
        if self.vel.x * nx + self.vel.z * nz > 2.0 || self.size > 1.0 {
            return;
        }
        let top = col.center.y + hy;
        if self.pos.y > top - 0.9 || self.pos.y < col.center.y - hy - 0.4 {
            return;
        }
        self.state = BodyState::Ladder;
        self.state_t = 0.0;
        self.climb_to = V3::new(col.center.x + nx * LADDER_GAP, top, col.center.z + nz * LADDER_GAP);
        self.vel = V3::ZERO;
        self.yaw = m::atan2(-nx, -nz);
        self.tilt = 0.0;
        self.coyote = 0.0;
        self.jump_buf = 0.0;
    }

    fn ladder_step(&mut self, ev: &mut StepEvents, dt: f64, input: BodyInput, world: &World) {
        let fx = m::sin(self.yaw);
        let fz = m::cos(self.yaw);
        let to = self.climb_to;
        if self.jump_buf > 0.0 {
            self.state = BodyState::Normal;
            self.state_t = LADDER_AGAIN;
            self.vel = V3::new(-fx * LADDER_LEAP, JUMP_V * 0.8, -fz * LADDER_LEAP);
            self.jump_buf = 0.0;
            ev.jumped = true;
            return;
        }
        let (x0, y0, z0) = (self.pos.x, self.pos.y, self.pos.z);
        let dx = to.x - self.pos.x;
        let dz = to.z - self.pos.z;
        let d = m::hypot(dx, dz);
        if d > 1.3 {
            self.state = BodyState::Normal;
            return;
        }
        let pull = d.at_most(5.0 * dt);
        if d > 1e-4 {
            self.pos.x += (dx / d) * pull;
            self.pos.z += (dz / d) * pull;
        }
        let up = input.mx * fx + input.mz * fz;
        let climb = if up.abs() > 0.3 {
            up * LADDER_SPEED * self.size
        } else {
            0.0
        };
        self.pos.y = (self.pos.y + climb * dt).at_most(to.y - 0.85);
        if climb > 0.0 && self.pos.y >= to.y - 0.85 - 1e-6 {
            let ox = to.x + fx * 1.05;
            let oz = to.z + fz * 1.05;
            if self.fits(world, ox, to.y + 0.02, oz) {
                self.state = BodyState::Climb;
                self.state_t = CLIMB_T - CLIMB_HANG - 1e-3;
                self.climb_to = V3::new(ox, to.y + 0.02, oz);
            }
        }
        self.vel = V3::new((self.pos.x - x0) / dt, (self.pos.y - y0) / dt, (self.pos.z - z0) / dt);
    }
}
