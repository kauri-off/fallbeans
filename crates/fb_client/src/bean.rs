//! The bean's procedural animation: damped springs on the limbs and the
//! body, poses by what the bean is doing, emotes, podium poses, arms reaching for whom it holds, the
//! tumble's spin, squash and stretch, and the eyes (blinking, opening by expression). The mouth and brows
//! (`face.rs`) follow the expression and eye opening set here.
use core::f32::consts::{PI, TAU};

use bevy::prelude::*;
use fb_net::Anim;

/// Height of the tip-over pivot (the lower collision sphere).
pub const PIVOT_Y: f32 = 0.5;
/// Ground covered by one full run cycle (two steps), m.
const STRIDE: f32 = 2.2;
/// Shoulders (model space) and the length from shoulder to hand.
const SHOULDER_X: f32 = 0.5;
const SHOULDER_Y: f32 = 1.08;
const ARM_LEN: f32 = 0.5;

fn smooth(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A damped spring: lags and overshoots, which reads as soft and floppy.
#[derive(Clone, Copy, Default)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    fn at(x: f32) -> Self {
        Self { x, v: 0.0 }
    }

    pub fn step(&mut self, target: f32, k: f32, c: f32, dt: f32) -> f32 {
        // Semi-implicit in small substeps: stays stable with stiff springs and long frames.
        let n = (dt * 240.0).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as u32 {
            self.v += (k * (target - self.x) - c * self.v) * h;
            self.x += self.v * h;
        }
        self.x
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Expr {
    #[default]
    Smile,
    Grin,
    Laugh,
    Surprised,
    Scared,
    Sad,
    Cry,
    Dizzy,
    Strain,
    Determined,
}

impl Expr {
    /// Eye opening and pupil size.
    fn eyes(self) -> (f32, f32) {
        match self {
            Expr::Smile => (1.0, 1.0),
            Expr::Grin => (0.85, 1.0),
            Expr::Laugh => (0.22, 1.0),
            Expr::Surprised => (1.06, 0.8),
            Expr::Scared => (1.06, 0.72),
            Expr::Sad => (0.72, 1.05),
            Expr::Cry => (0.35, 1.0),
            Expr::Dizzy => (0.9, 0.85),
            Expr::Strain => (0.5, 1.0),
            Expr::Determined => (0.7, 1.0),
        }
    }
}

impl Expr {
    /// Brows: lift (m) and tilt (rad; above 0 raises the inner ends: worried, below lowers them: cross).
    fn brows(self) -> (f32, f32) {
        match self {
            Expr::Smile => (0.0, 0.0),
            Expr::Grin => (0.006, 0.05),
            Expr::Laugh => (0.01, 0.1),
            Expr::Surprised => (0.012, 0.12),
            Expr::Scared => (0.008, 0.36),
            Expr::Sad => (0.0, 0.34),
            Expr::Cry => (0.0, 0.42),
            Expr::Dizzy => (0.004, 0.16),
            Expr::Strain => (-0.004, -0.32),
            Expr::Determined => (-0.003, -0.26),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Podium {
    Cheer,
    Clap,
    Sad,
}

/// What the animation is told each frame.
pub struct Frame {
    /// World velocity (m/s).
    pub vel: Vec3,
    pub anim: Anim,
    /// Seconds, for the cycles.
    pub t: f32,
    pub land_impact: f32,
    pub tilt: f32,
    /// World yaw the body tips towards.
    pub tilt_dir: f32,
    /// The yaw the bean faces.
    pub yaw: f32,
    /// The held bean's feet in the model's space, and its size.
    pub grab_at: Option<(Vec3, f32)>,
    pub size: f32,
    pub power: u8,
    pub pose: Option<Podium>,
}

#[derive(Clone, Copy)]
struct Targets {
    arm_lx: f32,
    arm_lz: f32,
    arm_rx: f32,
    arm_rz: f32,
    leg_lx: f32,
    leg_lz: f32,
    leg_rx: f32,
    leg_rz: f32,
    lean: f32,
    roll: f32,
    twist: f32,
    lift: f32,
    /// Limb spring stiffness and damping.
    k: f32,
    c: f32,
}

impl Default for Targets {
    fn default() -> Self {
        Self {
            arm_lx: 0.1,
            arm_lz: 0.15,
            arm_rx: 0.1,
            arm_rz: 0.15,
            leg_lx: 0.0,
            leg_lz: 0.03,
            leg_rx: 0.0,
            leg_rz: 0.03,
            lean: 0.0,
            roll: 0.0,
            twist: 0.0,
            lift: 0.0,
            k: 240.0,
            c: 20.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Limb {
    sx: Spring,
    sz: Spring,
}

/// What the animation sets this frame (applied to the model's nodes by the view).
#[derive(Clone, Copy, Default)]
pub struct Out {
    /// Rotation x and z of ArmL, ArmR, LegL, LegR on top of their bases.
    pub limbs: [(f32, f32); 4],
    /// Length of each arm (the hands keep theirs).
    pub stretch: [f32; 2],
    pub pivot: Quat,
    pub lean: f32,
    pub twist: f32,
    pub roll: f32,
    pub lift: f32,
    pub squash: Vec3,
    pub grow: f32,
    pub eye_open: f32,
    pub pupil: f32,
    /// The face: expression, eye opening without blinks (the brows follow it), brow lift and tilt.
    pub expr: Expr,
    pub wide: f32,
    pub brow_lift: f32,
    pub brow_tilt: f32,
    /// Pupils rolling (dizzy), offsets of the left and right one.
    pub pupil_roll: [Vec2; 2],
    pub crying: bool,
    pub aura: Option<f32>,
    /// Each tail segment's turn (y, x).
    pub tail: [(f32, f32); 5],
    pub speed: f32,
}

/// One bean's animation state.
#[derive(Component)]
pub struct BeanAnim {
    limbs: [Limb; 4],
    stretch: [Spring; 2],
    react: Option<(Expr, f32)>,
    fall_t: f32,
    grow: Spring,
    squash: Spring,
    lean: Spring,
    roll: Spring,
    twist: Spring,
    lift: Spring,
    tilt_now: f32,
    spin: f32,
    spin_rate: f32,
    emote: u8,
    emote_t: f32,
    blink_at: f32,
    seed: f32,
    rng: u32,
    last_vel: Vec3,
    last_yaw: f32,
    yaw_rate: f32,
    last_anim: Option<Anim>,
    fidget: u8,
    fidget_at: f32,
    step_side: bool,
    phase: f32,
    expr: Expr,
    open: f32,
    pupil: f32,
    /// Bases of the arms (Euler x, z), from the model.
    pub arm_base: [(f32, f32); 2],
    /// Smoothed velocity of a bean drawn from snapshots, and where it was drawn last frame.
    pub drawn_vel: Vec3,
    pub drawn_at: Option<Vec3>,
    /// The own bean: grounded last frame (landing squash).
    pub was_grounded: bool,
    pub rotor: f32,
    pub out: Out,
}

impl BeanAnim {
    pub fn new(id: u32) -> Self {
        let mut a = Self {
            limbs: [Limb::default(); 4],
            stretch: [Spring::at(1.0); 2],
            react: None,
            fall_t: 0.0,
            grow: Spring::at(1.0),
            squash: Spring::default(),
            lean: Spring::default(),
            roll: Spring::default(),
            twist: Spring::default(),
            lift: Spring::default(),
            tilt_now: 0.0,
            spin: 0.0,
            spin_rate: 0.0,
            emote: 0,
            emote_t: 0.0,
            blink_at: 0.0,
            seed: 0.0,
            rng: id.wrapping_mul(2_654_435_761) | 1,
            last_vel: Vec3::ZERO,
            last_yaw: 0.0,
            yaw_rate: 0.0,
            last_anim: None,
            fidget: 0,
            fidget_at: 0.0,
            step_side: false,
            phase: 0.0,
            expr: Expr::Smile,
            open: 1.0,
            pupil: 1.0,
            arm_base: [(0.0, -0.2), (0.0, 0.2)],
            drawn_vel: Vec3::ZERO,
            drawn_at: None,
            was_grounded: true,
            rotor: 0.0,
            out: Out::default(),
        };
        a.seed = a.random() * 100.0;
        a.blink_at = 1.0 + a.random() * 3.0;
        a.fidget_at = 4.0 + a.random() * 6.0;
        a
    }

    /// Looks only: no need for the simulation's generator.
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn play_emote(&mut self, e: u8) {
        self.emote = e;
        self.emote_t = 2.6;
    }

    /// Shows a face for a while (joy on finishing, tears when knocked out).
    pub fn react(&mut self, e: Expr, seconds: f32) {
        self.react = Some((e, seconds));
    }

    fn pick_expr(&self, a: Anim, f: &Frame, speed: f32) -> Expr {
        if let Some((e, t)) = self.react
            && t > 0.0
        {
            return e;
        }
        match f.pose {
            Some(Podium::Cheer) => return Expr::Laugh,
            Some(Podium::Clap) => return Expr::Grin,
            Some(Podium::Sad) => return Expr::Cry,
            None => {}
        }
        if self.emote_t > 0.0 && speed < 1.2 && matches!(a, Anim::Idle | Anim::Air) {
            return match self.emote {
                1 | 2 => Expr::Grin,
                3 => Expr::Laugh,
                4 => Expr::Cry,
                5 => Expr::Scared,
                _ => Expr::Smile,
            };
        }
        match a {
            Anim::Tumble => Expr::Scared,
            Anim::Stun => Expr::Dizzy,
            Anim::Getup | Anim::Grab | Anim::Reach => Expr::Strain,
            Anim::Dive | Anim::Slide => Expr::Determined,
            // A long fall: fright.
            Anim::Air if self.fall_t > 0.45 => Expr::Scared,
            Anim::Air if f.vel.y > 8.0 => Expr::Surprised,
            _ if speed > 6.0 => Expr::Grin,
            _ => Expr::Smile,
        }
    }

    pub fn animate(&mut self, dt: f32, f: &Frame) {
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(1.0 / 20.0);
        let (a, t) = (f.anim, f.t);
        let yaw = f.yaw;
        // Velocity in the bean's own frame: forward (+z) and right.
        let (sy, cy) = yaw.sin_cos();
        let fwd = f.vel.x * sy + f.vel.z * cy;
        let side = f.vel.x * cy - f.vel.z * sy;
        let speed = f.vel.x.hypot(f.vel.z);
        let accel = (f.vel - self.last_vel) / dt;
        let a_fwd = accel.x * sy + accel.z * cy;
        let a_side = accel.x * cy - accel.z * sy;
        self.last_vel = f.vel;
        let d_yaw = yaw - self.last_yaw;
        let d_yaw = d_yaw.sin().atan2(d_yaw.cos());
        self.last_yaw = yaw;
        self.yaw_rate += (d_yaw / dt - self.yaw_rate) * (dt * 10.0).min(1.0);
        let entered = Some(a) != self.last_anim;
        let prev = self.last_anim;
        self.last_anim = Some(a);
        self.emote_t -= dt;
        if let Some((_, t)) = &mut self.react {
            *t -= dt;
        }
        self.fall_t = if a == Anim::Air && f.vel.y < -9.0 {
            self.fall_t + dt
        } else {
            0.0
        };

        // Squash and stretch impulses on events.
        if f.land_impact > 0.1 {
            self.squash.v -= f.land_impact * 11.0;
        }
        if entered && a == Anim::Air && f.vel.y > 3.0 {
            self.squash.v += 4.5;
        }
        if entered && a == Anim::Dive {
            self.squash.v += 3.0;
        }

        let mut tg = match f.pose {
            Some(p) if a == Anim::Idle && speed < 1.2 => podium(p, t),
            _ => self.pick_pose(a, t, dt, speed, fwd, side, f.vel.y, a_fwd),
        };

        // Secondary motion: limbs are thrown against the body's acceleration (strongest when loose).
        let loose = 1.0 - (tg.k / 260.0).clamp(0.0, 1.0);
        let kick = (0.0015 + loose * 0.006) * dt * 60.0;
        for l in &mut self.limbs[..2] {
            l.sx.v += a_fwd * kick;
            l.sz.v += a_side * kick;
        }
        for l in &mut self.limbs[2..] {
            l.sx.v += a_fwd * kick * 0.6;
        }

        // Grabbing or reaching out: the arms point at the target and stretch to it.
        let reach = self.aim_arms(&mut tg, f, a, t);
        for (i, r) in reach.iter().enumerate() {
            self.out.stretch[i] = self.stretch[i].step(*r, 260.0, 22.0, dt).max(0.6);
        }
        let targets = [
            (tg.arm_lx, -tg.arm_lz),
            (tg.arm_rx, tg.arm_rz),
            (tg.leg_lx, -tg.leg_lz),
            (tg.leg_rx, tg.leg_rz),
        ];
        for (i, (x, z)) in targets.into_iter().enumerate() {
            let l = &mut self.limbs[i];
            self.out.limbs[i] = (l.sx.step(x, tg.k, tg.c, dt), l.sz.step(z, tg.k, tg.c, dt));
        }

        // Body: tip-over (tumble) around the lower body, spinning through the air, then lean/roll on top.
        self.tilt_now += (f.tilt - self.tilt_now) * (dt * 14.0).min(1.0);
        let tumbling = a == Anim::Tumble;
        if tumbling && entered {
            self.spin_rate = (speed * 1.3).clamp(5.0, 13.0);
        }
        let airborne = matches!(a, Anim::Air | Anim::Tumble | Anim::Dive);
        if tumbling && f.vel.y.abs() > 1.5 && airborne {
            self.spin += self.spin_rate * dt;
        } else {
            // On the ground (or getting up): settle to the nearest whole turn.
            let target = (self.spin / TAU).round() * TAU;
            self.spin += (target - self.spin) * (dt * if tumbling { 6.0 } else { 10.0 }).min(1.0);
            self.spin_rate *= (-dt * 3.0).exp();
        }
        if !tumbling && prev == Some(Anim::Tumble) && a != Anim::Getup {
            self.spin = 0.0;
        }
        let local = f.tilt_dir - yaw;
        let axis = Vec3::new(local.cos(), 0.0, -local.sin());
        let mut q = Quat::from_axis_angle(axis, self.tilt_now + self.spin);
        // Rolling from side to side while sliding along on the back.
        if tumbling {
            let along = Vec3::new(local.sin(), 0.0, local.cos());
            q *= Quat::from_axis_angle(along, (t * 7.0 + self.seed).sin() * (speed * 0.08).min(0.5));
        }
        self.out.pivot = q;
        self.out.lean = self.lean.step(tg.lean, 110.0, 14.0, dt);
        self.out.roll = self.roll.step(tg.roll, 110.0, 13.0, dt);
        self.out.twist = self.twist.step(tg.twist, 90.0, 12.0, dt);
        self.out.lift = self.lift.step(tg.lift, 220.0, 20.0, dt);

        // Squash and stretch: a spring around 0, plus stretch with vertical speed in the air.
        let air_stretch = if a == Anim::Air {
            (f.vel.y.abs() / 40.0).clamp(0.0, 0.12)
        } else {
            0.0
        };
        let sq = (self.squash.step(0.0, 260.0, 12.0, dt) + air_stretch).clamp(-0.32, 0.28);
        let breathe = if a == Anim::Idle && speed < 1.0 {
            (t * 2.6 + self.seed).sin() * 0.014
        } else {
            0.0
        };
        let w = 1.0 - sq * 0.5 - breathe * 0.5;
        self.out.squash = Vec3::new(w, 1.0 + sq + breathe, w);

        // Eyes: blink every few seconds; squeezed shut when tumbling.
        self.blink_at -= dt;
        let blink = if self.blink_at < 0.12 && self.blink_at > 0.0 {
            0.12
        } else {
            1.0
        };
        if self.blink_at < 0.0 {
            self.blink_at = 2.0 + self.random() * 3.5;
        }
        self.expr = self.pick_expr(a, f, speed);
        let squeeze = if tumbling { 0.3 } else { 1.0 };
        let (open, pupil) = self.expr.eyes();
        let k = (dt * 18.0).min(1.0);
        self.open += (open * blink * squeeze - self.open) * k;
        self.pupil += (pupil - self.pupil) * k;
        self.out.eye_open = self.open;
        self.out.pupil = self.pupil;
        self.out.expr = self.expr;
        self.out.wide += (open * squeeze - self.out.wide) * k;
        let (lift, tilt) = self.expr.brows();
        self.out.brow_lift += (lift - self.out.brow_lift) * k;
        self.out.brow_tilt += (tilt - self.out.brow_tilt) * k;
        self.out.pupil_roll = if self.expr == Expr::Dizzy {
            // Eyes rolling in circles (in opposite directions).
            [1.0f32, -1.0].map(|s| {
                let a = t * 9.0 * s;
                Vec2::new(a.cos() * 0.018, a.sin() * 0.022)
            })
        } else {
            [Vec2::ZERO; 2]
        };
        self.out.crying = self.expr == Expr::Cry;
        // Giants grow (and shrink back) with a wobble.
        self.out.grow = self.grow.step(f.size, 90.0, 9.0, dt).max(0.5);
        self.out.aura = (f.power != 0).then(|| 1.0 + (t * 6.0).sin() * 0.12);
        for (i, seg) in self.out.tail.iter_mut().enumerate() {
            // Each segment follows the one before: a wagging, trailing tail.
            *seg = (
                (t * 8.0 - i as f32 * 0.7).sin() * (0.12 + (speed * 0.03).min(0.25)),
                (-fwd * 0.02).clamp(-0.3, 0.2) + if a == Anim::Air { 0.15 } else { 0.0 },
            );
        }
        self.out.speed = speed;
    }

    /// Arm targets for grabbing (hands on the held bean) and reaching out (grasping ahead); returns the
    /// stretch of each arm (1 = normal length).
    fn aim_arms(&self, tg: &mut Targets, f: &Frame, a: Anim, t: f32) -> [f32; 2] {
        let holding = f.grab_at.filter(|_| matches!(a, Anim::Grab | Anim::Reach));
        if holding.is_none() && a != Anim::Reach {
            return [1.0, 1.0];
        }
        let aim = match holding {
            // Both hands on the near side of the held bean, about its middle (already in model space).
            Some((at, _)) => at,
            // Grasping at the air ahead, the hands opening and closing.
            None => Vec3::new(0.0, 0.98, 0.62 + (t * 9.0).sin().abs() * 0.38),
        };
        let mut out = [1.0, 1.0];
        for (i, o) in out.iter_mut().enumerate() {
            let side = if i == 1 { 1.0 } else { -1.0 };
            let v = Vec3::new(aim.x + side * 0.2 - side * SHOULDER_X, aim.y - SHOULDER_Y, aim.z);
            let len = v.length().max(0.2);
            let v = v / len;
            // Euler XYZ that turns the arm (hanging along −y) to point along v.
            let rz = v.x.clamp(-1.0, 1.0).asin();
            let rx = (-v.z).atan2(-v.y);
            let (bx, bz) = self.arm_base[i];
            if i == 1 {
                tg.arm_rx = rx - bx;
                tg.arm_rz = rz - bz;
            } else {
                tg.arm_lx = rx - bx;
                tg.arm_lz = bz - rz;
            }
            *o = (len / ARM_LEN).clamp(0.8, 7.0);
        }
        tg.k = tg.k.max(260.0);
        out
    }

    fn pick_pose(&mut self, a: Anim, t: f32, dt: f32, speed: f32, fwd: f32, side: f32, vy: f32, a_fwd: f32) -> Targets {
        let mut tg = Targets::default();
        let w = t + self.seed;
        match a {
            Anim::Tumble => {
                // Ragdoll: loose limbs flung around by the tumble.
                tg.k = 30.0;
                tg.c = 2.8;
                tg.arm_lx = -2.2 + (w * 9.0).sin() * 1.2;
                tg.arm_rx = -2.0 + (w * 8.0 + 1.3).sin() * 1.2;
                tg.arm_lz = 1.2 + (w * 11.0).cos() * 0.6;
                tg.arm_rz = 1.1 + (w * 10.0 + 2.0).cos() * 0.6;
                tg.leg_lx = (w * 7.0 + 1.0).sin();
                tg.leg_rx = (w * 7.5 + 2.4).sin();
                tg.leg_lz = 0.5 + (w * 6.0).sin() * 0.3;
                tg.leg_rz = 0.5 + (w * 6.4).cos() * 0.3;
                return tg;
            }
            Anim::Getup => {
                // Arms push against the ground, legs gather, a little hop at the end.
                tg.k = 150.0;
                tg.c = 14.0;
                (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (0.9, 0.9, 0.7, 0.7);
                (tg.leg_lx, tg.leg_rx, tg.leg_lz, tg.leg_rz) = (-0.6, -0.6, 0.25, 0.25);
                tg.lift = 0.08;
                return tg;
            }
            Anim::Dive | Anim::Slide => {
                // Superman: arms stretched ahead, legs straight back (the body lies along the flight).
                (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-2.95, -2.95, 0.28, 0.28);
                (tg.leg_lx, tg.leg_rx, tg.leg_lz, tg.leg_rz) = (0.45, 0.45, 0.15, 0.15);
                if a == Anim::Slide {
                    let k = (t * 16.0).sin() * 0.25 * (speed / 4.0).min(1.0);
                    tg.leg_lx += k;
                    tg.leg_rx -= k;
                }
                tg.roll = (-side * 0.04).clamp(-0.3, 0.3);
                tg.k = 200.0;
                return tg;
            }
            Anim::Climb | Anim::ClimbOver => {
                // Hanging on and pulling up, then a knee over the edge and a push down.
                tg.k = 200.0;
                tg.c = 16.0;
                if a == Anim::Climb {
                    let scramble = (t * 17.0).sin();
                    (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-2.75, -2.75, 0.32, 0.32);
                    tg.leg_lx = -0.5 + scramble * 0.45;
                    tg.leg_rx = -0.5 - scramble * 0.45;
                    tg.lean = 0.12;
                } else {
                    (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-0.75, -0.75, 0.5, 0.5);
                    (tg.leg_lx, tg.leg_rx) = (-1.3, 0.35);
                    tg.lean = 0.5;
                }
                return tg;
            }
            Anim::Ladder => {
                // Hand over hand up the rungs (the rhythm follows the climb), feet stepping after them.
                // (Kept within a turn: an f32 phase growing for hours would step the cycle.)
                self.phase = (self.phase + vy.abs() * dt * PI / 0.76).rem_euclid(TAU);
                let s = self.phase.sin();
                tg.k = 220.0;
                tg.c = 18.0;
                tg.arm_lx = -2.6 + s * 0.45;
                tg.arm_rx = -2.6 - s * 0.45;
                (tg.arm_lz, tg.arm_rz) = (0.25, 0.25);
                tg.leg_lx = -0.55 - (-s).max(0.0) * 0.6;
                tg.leg_rx = -0.55 - s.max(0.0) * 0.6;
                (tg.leg_lz, tg.leg_rz) = (0.12, 0.12);
                tg.lean = 0.08;
                return tg;
            }
            Anim::Stun => {
                // Dazed: the body circles, arms dangle loosely.
                tg.k = 70.0;
                tg.c = 6.0;
                tg.lean = (t * 6.0).sin() * 0.22 + 0.1;
                tg.roll = (t * 6.0).cos() * 0.22;
                tg.arm_lx = (t * 6.0 + 1.0).sin() * 0.6;
                tg.arm_rx = (t * 6.0 + 2.5).sin() * 0.6;
                tg.arm_lz = 0.7 + (t * 5.0).cos() * 0.3;
                tg.arm_rz = 0.7 + (t * 5.0).sin() * 0.3;
                (tg.leg_lz, tg.leg_rz) = (0.2, 0.2);
                return tg;
            }
            Anim::Air => {
                if vy > 1.5 {
                    // Take-off: arms thrown up, one knee up.
                    (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-2.5, -2.3, 0.45, 0.5);
                    (tg.leg_lx, tg.leg_rx) = if self.step_side { (-0.9, 0.2) } else { (0.2, -0.9) };
                    tg.lean = -0.05 + (fwd * 0.012).clamp(0.0, 0.12);
                } else if vy < -6.0 {
                    // Falling: flailing arms, pedalling legs.
                    let k = ((-vy - 6.0) / 10.0).min(1.0);
                    tg.k = 160.0;
                    tg.c = 10.0;
                    tg.arm_lx = -2.7 + (t * 17.0).sin() * (0.3 + k * 0.4);
                    tg.arm_rx = -2.7 + (t * 17.0 + 2.0).sin() * (0.3 + k * 0.4);
                    tg.arm_lz = 0.8 + (t * 13.0).cos() * 0.25;
                    tg.arm_rz = 0.8 + (t * 13.0 + 1.0).cos() * 0.25;
                    tg.leg_lx = (t * 12.0).sin() * 0.7;
                    tg.leg_rx = -(t * 12.0).sin() * 0.7;
                    (tg.leg_lz, tg.leg_rz) = (0.2, 0.2);
                    tg.lean = 0.1 + k * 0.15;
                } else {
                    // Apex: arms out for balance, legs gathered.
                    (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-1.6, -1.5, 1.0, 1.0);
                    (tg.leg_lx, tg.leg_rx, tg.leg_lz, tg.leg_rz) = (-0.5, -0.3, 0.15, 0.15);
                    tg.lean = 0.08;
                }
                // Leaning into the flight: the faster forward and down, the more.
                if vy < 0.0 {
                    tg.lean += (fwd.max(0.0).atan2(-vy + 4.0) * 0.7).clamp(0.0, 0.45);
                }
                tg.roll = (-side * 0.03).clamp(-0.25, 0.25);
                return tg;
            }
            _ => {}
        }
        // Ground: run cycle scaled by speed, grab or idle on top.
        let run = smooth(speed, 0.4, 7.0);
        self.phase = (self.phase + speed * dt / STRIDE * TAU).rem_euclid(TAU);
        let (s, c) = self.phase.sin_cos();
        self.step_side = s > 0.0;
        let back = if fwd < -0.5 { -1.0 } else { 1.0 };
        // Long strides: each leg swings far, and lifts its knee on the way forward.
        let swing = 0.3 + 0.95 * run;
        let knee = 0.55 * run;
        tg.leg_lx = s * swing * back - (-c).max(0.0) * knee;
        tg.leg_rx = -s * swing * back - c.max(0.0) * knee;
        tg.leg_lz = 0.03 + 0.06 * run;
        tg.leg_rz = tg.leg_lz;
        tg.arm_lx = -s * (0.3 + 1.05 * run) * back + 0.1 * run;
        tg.arm_rx = s * (0.3 + 1.05 * run) * back + 0.1 * run;
        tg.arm_lz = 0.22 + 0.28 * run;
        tg.arm_rz = tg.arm_lz;
        tg.lean = 0.26 * run * back + (a_fwd * 0.012).clamp(-0.22, 0.3);
        // Two bounces per cycle; hips twist and roll.
        tg.lift = (1.0 - c.abs()) * 0.13 * run;
        tg.twist = s * 0.2 * run;
        tg.roll = c * 0.08 * run + (-self.yaw_rate * speed * 0.012).clamp(-0.32, 0.32);
        if side.abs() > 1.0 && fwd.abs() < 2.0 {
            // Strafing: side steps.
            tg.leg_lz = 0.05 + s.max(0.0) * 0.35;
            tg.leg_rz = 0.05 + (-s).max(0.0) * 0.35;
            tg.roll -= (side * 0.02).clamp(-0.15, 0.15);
        }
        if a == Anim::Reach {
            tg.arm_lx = -1.5 + (t * 9.0).sin() * 0.15;
            tg.arm_rx = -1.5 - (t * 9.0).sin() * 0.15;
            (tg.arm_lz, tg.arm_rz) = (0.18, 0.18);
            tg.lean = 0.15 + tg.lean * 0.5;
            return tg;
        }
        if a == Anim::Grab {
            (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-1.55, -1.55, 0.08, 0.08);
            tg.lean = -0.18;
            return tg;
        }
        if speed > 1.2 {
            self.fidget = 0;
            return tg;
        }
        if self.emote_t > 0.0 {
            return self.emote_pose(tg, t);
        }
        // Idle: breathing, weight shifting, every now and then a look around or a stretch.
        let idle = 1.0 - smooth(speed, 0.2, 1.2);
        self.fidget_at -= dt;
        if self.fidget_at < 0.0 {
            self.fidget = 1 + (self.random() * 3.0) as u8;
            self.fidget_at = 5.0 + self.random() * 7.0;
        }
        let since = 5.0 + 7.0 - self.fidget_at;
        tg.arm_lx = 0.12 + (t * 1.9 + self.seed).sin() * 0.05;
        tg.arm_rx = 0.12 + (t * 1.9 + self.seed + 0.5).sin() * 0.05;
        tg.roll += (t * 0.9 + self.seed).sin() * 0.04 * idle;
        tg.lift += (t * 2.6 + self.seed).sin() * 0.006 * idle;
        if self.fidget == 1 && since < 2.0 {
            tg.twist = (since * PI).sin() * 0.45 * if self.seed.sin() > 0.0 { 1.0 } else { -1.0 };
        } else if self.fidget == 2 && since < 1.4 {
            let k = (since / 1.4 * PI).sin();
            (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-2.9 * k, -2.9 * k, 0.3, 0.3);
            tg.lean = -0.15 * k;
        } else if self.fidget == 3 && since < 1.0 {
            tg.leg_rx = -0.4 * (since * PI).sin();
            tg.roll -= 0.1 * (since * PI).sin();
        }
        tg
    }

    fn emote_pose(&self, mut tg: Targets, t: f32) -> Targets {
        match self.emote {
            1 => {
                // Wave with one arm, the other on the hip, bouncing.
                tg.arm_rx = -2.7;
                tg.arm_rz = 0.6 + (t * 14.0).sin() * 0.45;
                (tg.arm_lx, tg.arm_lz) = (0.3, 0.9);
                tg.lift = (t * 7.0).sin().abs() * 0.12;
                tg.roll = (t * 7.0).sin() * 0.06;
            }
            2 => {
                // Dance: hips swing, arms up in turn, feet tapping.
                let b = (t * 9.0).sin();
                tg.arm_lx = -2.4 + b * 0.5;
                tg.arm_rx = -2.4 - b * 0.5;
                (tg.arm_lz, tg.arm_rz) = (0.5, 0.5);
                tg.roll = b * 0.25;
                tg.twist = (t * 4.5).sin() * 0.35;
                tg.leg_lx = b.max(0.0) * -0.6;
                tg.leg_rx = (-b).max(0.0) * -0.6;
                tg.lift = b.abs() * 0.1;
            }
            4 => {
                // Crying: slumped, hands rubbing the eyes, shoulders shaking.
                let sob = (t * 14.0).sin() * 0.06;
                tg.lean = 0.3 + sob;
                tg.arm_lx = -2.3 + sob;
                tg.arm_rx = -2.3 - sob;
                (tg.arm_lz, tg.arm_rz) = (-0.35, -0.35);
                tg.lift = sob.abs() * 0.3;
            }
            5 => {
                // Fright: arms thrown up, knees knocking, trembling.
                let tr = (t * 30.0).sin() * 0.05;
                tg.arm_lx = -2.8 + tr;
                tg.arm_rx = -2.8 - tr;
                (tg.arm_lz, tg.arm_rz) = (0.9, 0.9);
                tg.lean = -0.2;
                tg.leg_lz = -0.15 + tr;
                tg.leg_rz = -0.15 - tr;
                tg.roll = tr;
            }
            _ => {
                // Laugh: rocking back, hands on the belly.
                tg.lean = -0.35 + (t * 18.0).sin() * 0.06;
                (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (-0.8, -0.8, -0.15, -0.15);
                tg.lift = (t * 9.0).sin().abs() * 0.06;
                tg.roll = (t * 3.0).sin() * 0.05;
            }
        }
        // Ease out of the emote over the last moment.
        if self.emote_t < 0.3 {
            tg.lift *= self.emote_t / 0.3;
        }
        tg
    }
}

fn podium(p: Podium, t: f32) -> Targets {
    let mut tg = Targets::default();
    match p {
        Podium::Cheer => {
            let hop = (t * 5.0).sin().max(0.0);
            tg.arm_lx = -2.9 + (t * 12.0).sin() * 0.25;
            tg.arm_rx = -2.9 + (t * 12.0 + 1.0).sin() * 0.25;
            tg.arm_lz = 0.55 + (t * 6.0).sin() * 0.2;
            tg.arm_rz = 0.55 + (t * 6.0 + 1.0).sin() * 0.2;
            tg.lift = hop * 0.55;
            tg.leg_lx = -hop * 0.5;
            tg.leg_rx = -hop * 0.3;
            tg.roll = (t * 5.0).sin() * 0.1;
        }
        Podium::Clap => {
            let c = (t * 9.0).sin().abs();
            (tg.arm_lx, tg.arm_rx) = (-1.45, -1.45);
            tg.arm_lz = -0.3 + c * 0.55;
            tg.arm_rz = tg.arm_lz;
            tg.lift = (t * 4.5).sin().abs() * 0.06;
            tg.twist = (t * 1.3).sin() * 0.15;
        }
        Podium::Sad => {
            tg.lean = 0.42 + (t * 1.5).sin() * 0.05;
            (tg.arm_lx, tg.arm_rx, tg.arm_lz, tg.arm_rz) = (0.3, 0.3, 0.02, 0.02);
            tg.roll = (t * 0.9).sin() * 0.08;
            tg.k = 120.0;
            tg.c = 14.0;
        }
    }
    tg
}

/// The pose on the podium by the place in the game.
pub fn podium_pose(place: Option<usize>, n: usize) -> Podium {
    let place = place.unwrap_or(98);
    if place == 0 {
        Podium::Cheer
    } else if place < 3 && place + 1 < n {
        Podium::Clap
    } else if place + 2 >= n && n > 3 {
        Podium::Sad
    } else {
        Podium::Clap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(anim: Anim, t: f32) -> Frame {
        Frame {
            vel: Vec3::new(3.0, if anim == Anim::Air { -12.0 } else { 0.0 }, 5.0),
            anim,
            t,
            land_impact: if t < 0.1 { 0.8 } else { 0.0 },
            tilt: if anim == Anim::Tumble { 1.4 } else { 0.0 },
            tilt_dir: 0.7,
            yaw: t * 0.3,
            grab_at: (anim == Anim::Grab).then_some((Vec3::new(0.3, 0.9, 0.8), 1.0)),
            size: if anim == Anim::Dive { 1.6 } else { 1.0 },
            power: 1,
            pose: None,
        }
    }

    #[test]
    fn every_state_animates_to_finite_poses() {
        for anim in Anim::ALL {
            let mut a = BeanAnim::new(5);
            a.play_emote(2);
            for i in 0..400 {
                // Frames from 1 ms to 80 ms (a hitch): the springs stay stable.
                let dt = [0.001, 1.0 / 60.0, 1.0 / 144.0, 0.08][i % 4];
                a.animate(dt, &frame(anim, i as f32 * 0.01));
            }
            let o = a.out;
            let all = o
                .limbs
                .iter()
                .flat_map(|(x, z)| [*x, *z])
                .chain(o.stretch)
                .chain([o.lean, o.twist, o.roll, o.lift, o.grow, o.eye_open, o.pupil]);
            for v in all {
                assert!(v.is_finite() && v.abs() < 20.0, "{anim:?}: {v}");
            }
            assert!(o.pivot.is_finite() && o.squash.is_finite(), "{anim:?}");
        }
    }

    #[test]
    fn podium_poses_by_place() {
        assert_eq!(podium_pose(Some(0), 8), Podium::Cheer);
        assert_eq!(podium_pose(Some(1), 8), Podium::Clap);
        assert_eq!(podium_pose(Some(7), 8), Podium::Sad);
        assert_eq!(podium_pose(Some(1), 2), Podium::Clap);
        assert_eq!(podium_pose(None, 3), Podium::Clap);
    }
}
