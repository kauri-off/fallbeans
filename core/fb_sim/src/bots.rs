//! Bot behaviour shared by the maps (port of `sim/bots.ts`): personalities, steering, A* routes,
//! getting unstuck, dodging, tackles and grabs, and the waypoint and arena brains. Brains run at
//! 20 Hz on the server; everything random comes from the bot's own `rng`.
use core::borrow::Borrow;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::m::{self, MinMaxJs};
use crate::math::V3;
use crate::nav::{Nav, NavPoint, PathOpts};
use crate::physics::{Body, BodyState, DIVE_SPEED, GRAVITY, RUN_SPEED};
use crate::world::World;
use fb_shared::EMOTES;
use fb_shared::rng::Rng;

/// Bot brains run at 20 Hz (BOT_EVERY ticks); timers below use this step.
pub const BOT_DT: f64 = 1.0 / 20.0;
// BOT_DT is written out (not BOT_EVERY · DT, which rounds differently): keep the two in step.
const _: () = assert!(fb_shared::TICK_RATE == 20 * fb_shared::BOT_EVERY);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BotInput {
    pub mx: f64,
    pub mz: f64,
    pub jump: bool,
    pub dive: bool,
    pub grab: bool,
    /// Play an emote (1–5), 0 for none.
    pub emote: u32,
}

/// A bot's current route.
#[derive(Clone, Debug)]
pub struct BotPlan {
    pub path: Option<Vec<NavPoint>>,
    pub i: usize,
    pub tx: f64,
    pub tz: f64,
    pub at: f64,
}

impl Default for BotPlan {
    fn default() -> Self {
        Self {
            path: None,
            i: 0,
            tx: 0.0,
            tz: 0.0,
            at: -1e9,
        }
    }
}

/// Another bean as a bot sees it.
#[derive(Clone, Copy, Debug)]
pub struct OtherView {
    pub id: u32,
    pub pos: V3,
    pub vel: V3,
    pub down: bool,
    /// Diving (or sliding fast): a tackle on its way.
    pub dive: bool,
    /// Reaching out with the grab button, nobody in hand.
    pub reach: bool,
}

/// What a bot remembers between decisions (the TS `BotMem` keys; `None` is `undefined`).
#[derive(Clone, Debug, Default)]
pub struct BotMem {
    pub init: bool,
    pub skill: Option<f64>,
    pub aggro: Option<f64>,
    pub jumpy: Option<f64>,
    pub react: Option<f64>,
    pub spd: Option<f64>,
    pub off: Option<f64>,
    pub ph: Option<f64>,
    pub stuck: Option<f64>,
    pub ax: Option<f64>,
    pub az: Option<f64>,
    pub tries: Option<f64>,
    pub sdx: Option<f64>,
    pub sdz: Option<f64>,
    pub side_until: Option<f64>,
    pub dodge_until: Option<f64>,
    pub dodge_x: Option<f64>,
    pub dodge_z: Option<f64>,
    pub dodge_seen: Option<f64>,
    pub foe: Option<u32>,
    pub foe_until: Option<f64>,
    pub dodges: Option<f64>,
    pub grab_until: Option<f64>,
    pub aimed: bool,
    pub counters: Option<f64>,
    pub shoves: Option<f64>,
    pub smx: Option<f64>,
    pub smz: Option<f64>,
    pub wp: Option<usize>,
    pub go: Option<usize>,
    pub ready_at: Option<f64>,
    pub route: Option<usize>,
    pub pref: Option<f64>,
    pub gy: Option<f64>,
    pub hunt: Option<u32>,
    pub hunt_until: Option<f64>,
    pub next_hunt: Option<f64>,
    pub tx: Option<f64>,
    pub tz: Option<f64>,
    pub until: Option<f64>,
    pub arrived: bool,
    /// Keys of a map's own brain.
    pub ext: BTreeMap<String, f64>,
}

impl BotMem {
    pub fn get(&self, key: &str) -> Option<f64> {
        self.ext.get(key).copied()
    }

    pub fn set(&mut self, key: &str, v: f64) {
        match self.ext.get_mut(key) {
            Some(x) => *x = v,
            None => {
                self.ext.insert(key.to_string(), v);
            }
        }
    }

    /// The key back to `undefined`.
    pub fn remove(&mut self, key: &str) {
        self.ext.remove(key);
    }

    fn skill(&self) -> f64 {
        self.skill.unwrap_or(0.7)
    }
}

/// What a brain sees.
pub struct BotView<'a> {
    pub id: u32,
    pub body: &'a Body,
    /// Sim time.
    pub t: f64,
    pub rng: &'a mut Rng,
    pub mem: &'a mut BotMem,
    pub plan: &'a mut BotPlan,
    pub others: &'a [OtherView],
    /// Walkable ground of the static course (None before the start).
    pub nav: Option<Nav<'a>>,
    /// Bonuses lying on the course right now.
    pub bonuses: &'a [V3],
    /// The map (its state: tiles that fell, doors that broke).
    pub world: &'a World,
    /// Points of everybody in the round (points games).
    pub scores: &'a BTreeMap<u32, f64>,
}

impl BotView<'_> {
    pub fn score(&self, id: u32) -> f64 {
        self.scores.get(&id).copied().unwrap_or(0.0)
    }
}

pub type BotBrain = Box<dyn Fn(&mut BotView, &mut BotInput) + Send + Sync>;
/// A test on the bot (it may keep notes in its memory).
pub type BotTest = Box<dyn Fn(&mut BotView) -> bool + Send + Sync>;

/// Target x of a waypoint: fixed, or a function of time for moving targets.
pub enum WpX {
    At(f64),
    Moving(Box<dyn Fn(f64) -> f64 + Send + Sync>),
}

pub struct Waypoint {
    pub x: WpX,
    pub z: f64,
    /// Lateral spread between bots (static x only); 0 = an exact line (narrow or timed sections).
    pub w: Option<f64>,
    /// Jump when within 2.6 m.
    pub jump: bool,
    /// Jump whenever this says so (e.g. a rotor arm is about to sweep by).
    pub jump_when: Option<BotTest>,
    /// Stand still until this is true (e.g. the path ahead is clear).
    pub wait: Option<BotTest>,
    /// Stick deflection towards it (default: full). Careful sections go slower.
    pub speed: Option<f64>,
    /// Somewhere to go instead for now; None: carry on along the path.
    pub detour: Option<Detour>,
    /// Full control for a special stretch (returns false to follow the waypoint as usual).
    pub drive: Option<Drive>,
}

pub type Detour = Box<dyn Fn(&mut BotView) -> Option<(f64, f64)> + Send + Sync>;
pub type Drive = Box<dyn Fn(&mut BotView, &mut BotInput) -> bool + Send + Sync>;
/// A test several waypoints share.
pub type SharedTest = Arc<dyn Fn(&mut BotView) -> bool + Send + Sync>;

impl Waypoint {
    pub fn at(x: f64, z: f64) -> Self {
        Self::new(WpX::At(x), z)
    }

    /// `{ x, z, w }` in TS.
    pub fn w(x: f64, z: f64, w: f64) -> Self {
        Self {
            w: Some(w),
            ..Self::at(x, z)
        }
    }

    /// A moving target: x is a function of time.
    pub fn moving(x: impl Fn(f64) -> f64 + Send + Sync + 'static, z: f64) -> Self {
        Self::new(WpX::Moving(Box::new(x)), z)
    }

    fn new(x: WpX, z: f64) -> Self {
        Self {
            x,
            z,
            w: None,
            jump: false,
            jump_when: None,
            wait: None,
            speed: None,
            detour: None,
            drive: None,
        }
    }

    pub fn jump_when(mut self, f: impl Fn(&mut BotView) -> bool + Send + Sync + 'static) -> Self {
        self.jump_when = Some(Box::new(f));
        self
    }

    pub fn jump_shared(self, f: &SharedTest) -> Self {
        let f = f.clone();
        self.jump_when(move |bot| f(bot))
    }

    pub fn wait(mut self, f: impl Fn(&mut BotView) -> bool + Send + Sync + 'static) -> Self {
        self.wait = Some(Box::new(f));
        self
    }

    pub fn speed(mut self, s: f64) -> Self {
        self.speed = Some(s);
        self
    }

    pub fn detour(mut self, f: impl Fn(&mut BotView) -> Option<(f64, f64)> + Send + Sync + 'static) -> Self {
        self.detour = Some(Box::new(f));
        self
    }

    pub fn drive(mut self, f: impl Fn(&mut BotView, &mut BotInput) -> bool + Send + Sync + 'static) -> Self {
        self.drive = Some(Box::new(f));
        self
    }

    pub fn drive_boxed(mut self, f: Drive) -> Self {
        self.drive = Some(f);
        self
    }
}

/// A bot's personality, fixed for the round: how good, how pushy, how bouncy, how quick to react.
pub fn init_bot(bot: &mut BotView) {
    if bot.mem.init {
        return;
    }
    let r = &mut *bot.rng;
    let mem = &mut *bot.mem;
    mem.init = true;
    mem.skill = Some(0.45 + r.next() * 0.55);
    // Most players get stuck in: few bots are entirely peaceful.
    mem.aggro = Some(0.15 + 0.85 * m::pow(r.next(), 1.1));
    mem.jumpy = Some(r.next());
    mem.react = Some(0.06 + (1.0 - mem.skill()) * 0.25 + r.next() * 0.05);
    // People hold the stick all the way; only a few ease off.
    mem.spd = Some(if r.next() < 0.8 { 1.0 } else { 0.9 + r.next() * 0.1 });
    mem.off = Some(r.next() * 2.0 - 1.0);
    mem.ph = Some(r.next() * 100.0);
    mem.stuck = Some(0.0);
}

pub fn steer(bot: &BotView, tx: f64, tz: f64, out: &mut BotInput, speed: f64) -> f64 {
    let dx = tx - bot.body.pos.x;
    let dz = tz - bot.body.pos.z;
    let d = m::hypot(dx, dz);
    if d < 0.3 {
        out.mx = 0.0;
        out.mz = 0.0;
        return d;
    }
    let k = (d / 1.5).min_js(1.0) * speed;
    out.mx = (dx / d) * k;
    out.mz = (dz / d) * k;
    d
}

/// Precise line following for narrow safe corridors, and holding a spot.
pub fn follow(bot: &BotView, tx: f64, tz: f64, out: &mut BotInput) -> f64 {
    let b = bot.body;
    let ex = tx - b.pos.x;
    let dz = tz - b.pos.z;
    let mx = (ex * 0.55).clamp(-1.0, 1.0);
    let mz = m::sign(dz) * (1.0 - mx.abs() * 0.5).min_js(dz.abs() * 0.6);
    let l = m::hypot(mx, mz);
    out.mx = if l > 1.0 { mx / l } else { mx };
    out.mz = if l > 1.0 { mz / l } else { mz };
    m::hypot(ex, dz)
}

/// Runs to (tx, tz) along an A* route over the course. Without a route (moving parts, no grid yet)
/// it heads straight there. Returns the remaining straight-line distance.
pub fn nav_to(bot: &mut BotView, tx: f64, tz: f64, out: &mut BotInput, speed: f64, radius: f64) -> f64 {
    let b = bot.body;
    let direct = m::hypot(tx - b.pos.x, tz - b.pos.z);
    let Some(nav) = bot.nav else {
        steer(bot, tx, tz, out, speed);
        return direct;
    };
    if direct < 1.2 {
        steer(bot, tx, tz, out, speed);
        return direct;
    }
    // In the air (or after a long time) an old route is worthless: head straight for it.
    if !b.grounded && (m::hypot(bot.plan.tx - tx, bot.plan.tz - tz) > 1.0 || bot.t - bot.plan.at > 2.0) {
        steer(bot, tx, tz, out, speed);
        return direct;
    }
    let off = match bot.plan.path.as_ref().and_then(|p| p.get(bot.plan.i)) {
        Some(cur) => m::hypot(cur.x - b.pos.x, cur.z - b.pos.z) > 6.0 || cur.y - b.pos.y > 2.5,
        None => true,
    };
    let stale = bot.t - bot.plan.at > 0.6 + (bot.id % 5) as f64 * 0.07;
    if b.grounded && (off || stale || m::hypot(bot.plan.tx - tx, bot.plan.tz - tz) > 1.0) {
        // Extra route cost near other beans on the ground: bots run around a crowd instead of into it.
        let y = b.pos.y;
        let near: Vec<V3> = bot
            .others
            .iter()
            .filter(|o| (o.pos.y - y).abs() < 1.2 && m::hypot(o.pos.x - b.pos.x, o.pos.z - b.pos.z) < 12.0)
            .map(|o| o.pos)
            .collect();
        let crowd = |x: f64, z: f64| {
            let mut c = 0.0;
            for o in &near {
                let d = m::hypot(o.x - x, o.z - z);
                if d < 1.6 {
                    c += (1.6 - d) * 2.5;
                }
            }
            c
        };
        let opts = PathOpts {
            cost: if near.is_empty() { None } else { Some(&crowd) },
            radius: Some(radius),
            max_nodes: Some(2500),
        };
        bot.plan.path = nav.path(b.pos, tx, tz, None, &opts);
        bot.plan.i = 0;
        bot.plan.tx = tx;
        bot.plan.tz = tz;
        bot.plan.at = bot.t;
    }
    let path: &[NavPoint] = match bot.plan.path.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => {
            steer(bot, tx, tz, out, speed);
            return direct;
        }
    };
    // Move on past points we have reached or passed, and past points that would be a step back or
    // sideways (a fresh route starts at the nearest cell, which may lie behind).
    let mut i = bot.plan.i;
    while i < path.len() - 1 {
        let p = path[i];
        let q = path[i + 1];
        let d = m::hypot(p.x - b.pos.x, p.z - b.pos.z);
        let seg = m::hypot(q.x - p.x, q.z - p.z);
        let ahead = (q.x - p.x) * (b.pos.x - p.x) + (q.z - p.z) * (b.pos.z - p.z) > 0.0;
        let closer = m::hypot(q.x - b.pos.x, q.z - b.pos.z) <= seg + 0.2;
        let level = (p.y - b.pos.y).abs() < 0.6 && (q.y - b.pos.y).abs() < 0.6;
        if d < 0.7 || (!q.jump && !p.jump && d < 2.0 && (ahead || (closer && level))) {
            i += 1;
        } else {
            break;
        }
    }
    bot.plan.i = i;
    let p = path[i];
    let last = i == path.len() - 1;
    // Aim a little further along when the way there is plain ground: rounded turns, not zig-zags.
    let mut ax = p.x;
    let mut az = p.z;
    if let Some(&q) = path.get(i + 1)
        && !last
        && !p.jump
        && !q.jump
        && (q.y - b.pos.y).abs() < 0.6
        && clear_to(nav, b.pos, q.x, q.z)
    {
        ax = (p.x + q.x) / 2.0;
        az = (p.z + q.z) / 2.0;
    }
    let dx = ax - b.pos.x;
    let dz = az - b.pos.z;
    let d = m::hypot(dx, dz);
    // Last leg: the exact target.
    if last {
        steer(bot, tx, tz, out, speed);
    } else if d > 1e-3 {
        out.mx = (dx / d) * speed;
        out.mz = (dz / d) * speed;
    }
    if p.jump && b.grounded {
        let gap = if i > 0 {
            let prev = path[i - 1];
            m::hypot(p.x - prev.x, p.z - prev.z)
        } else {
            d
        };
        let climb = p.y - b.pos.y > 0.5;
        let along = if d > 1e-3 {
            (b.vel.x * dx + b.vel.z * dz) / d
        } else {
            0.0
        };
        let when = if climb { 1.3 } else { (gap * 0.8 + 0.5).min_js(2.8) };
        if d < when && (along > 2.5 || climb) {
            out.jump = true;
        }
    }
    direct
}

/// Walkable all the way along a straight line from `from` to (x, z) (samples every half metre)?
fn clear_to(nav: Nav, from: V3, x: f64, z: f64) -> bool {
    let d = m::hypot(x - from.x, z - from.z);
    let n = (d / 0.5).ceil() as i64;
    for i in 1..=n {
        let f = i as f64 / n as f64;
        if !nav.safe(from.x + (x - from.x) * f, from.z + (z - from.z) * f, from.y) {
            return false;
        }
    }
    true
}

/// Stuck against something while trying to move: hop, then side-step.
pub fn unstick(bot: &mut BotView, out: &mut BotInput) {
    let b = bot.body;
    let p = b.pos;
    let moving = m::hypot(out.mx, out.mz) > 0.3;
    let mem = &mut *bot.mem;
    match mem.ax {
        Some(ax) if moving && m::hypot(p.x - ax, p.z - mem.az.unwrap_or(0.0)) <= 0.6 => {
            if b.grounded && bot.t > 0.0 {
                mem.stuck = Some(mem.stuck.unwrap_or(0.0) + BOT_DT);
            }
        }
        _ => {
            mem.ax = Some(p.x);
            mem.az = Some(p.z);
            mem.stuck = Some(0.0);
        }
    }
    let stuck = mem.stuck.unwrap_or(0.0);
    if stuck > 0.45 && b.grounded {
        out.jump = true;
        bot.plan.at = -1e9;
    }
    if stuck > 1.4 {
        // Still stuck: side-step for a moment, to the own right of the line to a bean in the way, or
        // to one side of the heading; every other try the other way, and never off an edge.
        let (hx, hz) = heading(bot, out);
        let odd = if bot.id % 2 == 1 { -1.0 } else { 1.0 };
        let mut sx = hz * odd;
        let mut sz = -hx * odd;
        let mut near = 1.6;
        for o in bot.others {
            let dx = o.pos.x - p.x;
            let dz = o.pos.z - p.z;
            let d = m::hypot(dx, dz);
            if d < near && d > 1e-3 && (o.pos.y - p.y).abs() < 1.2 {
                near = d;
                sx = dz / d;
                sz = -dx / d;
            }
        }
        let mem = &mut *bot.mem;
        let tries = mem.tries.unwrap_or(0.0) + 1.0;
        mem.tries = Some(tries);
        let flip = if tries % 2.0 == 0.0 { -1.0 } else { 1.0 };
        sx *= flip;
        sz *= flip;
        if bot.nav.is_some_and(|n| !n.safe(p.x + sx * 1.5, p.z + sz * 1.5, p.y)) {
            sx = -sx;
            sz = -sz;
        }
        mem.sdx = Some(sx);
        mem.sdz = Some(sz);
        mem.side_until = Some(bot.t + 0.6);
        mem.stuck = Some(0.0);
    }
    if bot.mem.side_until.unwrap_or(-1.0) > bot.t {
        let (hx, hz) = heading(bot, out);
        let x = bot.mem.sdx.unwrap_or(1.0) + hx * 0.25;
        let z = bot.mem.sdz.unwrap_or(0.0) + hz * 0.25;
        let l = m::hypot(x, z);
        out.mx = x / l;
        out.mz = z / l;
    }
}

/// Nearest other bean ahead of the bot within `range`, on about the same level.
fn bean_ahead(bot: &BotView, range: f64, cone: f64) -> Option<(OtherView, f64)> {
    let b = bot.body;
    let fx = m::sin(b.yaw);
    let fz = m::cos(b.yaw);
    let mut best = None;
    let mut bd = range;
    for o in bot.others {
        if o.down || (o.pos.y - b.pos.y).abs() > 1.0 {
            continue;
        }
        let dx = o.pos.x - b.pos.x;
        let dz = o.pos.z - b.pos.z;
        let d = m::hypot(dx, dz);
        if d >= bd || d < 1e-3 || (dx * fx + dz * fz) / d < cone {
            continue;
        }
        bd = d;
        best = Some(*o);
    }
    best.map(|o| (o, bd))
}

/// Where a flying body comes down (and whether that is safe), and when.
#[derive(Clone, Copy, Debug)]
struct Landing {
    x: f64,
    z: f64,
    y: f64,
    t: f64,
    safe: bool,
}

/// Where a body flying with (vx, vz, vy) comes down on the navigation grid (checked every 50 ms).
fn landing(bot: &BotView, vx: f64, vz: f64, vy: f64) -> Option<Landing> {
    let nav = bot.nav?;
    let p = bot.body.pos;
    let mut py = p.y;
    let mut t = 0.05;
    while t <= 2.0 {
        let y = p.y + vy * t - (GRAVITY / 2.0) * t * t;
        let x = p.x + vx * t;
        let z = p.z + vz * t;
        if let Some((fy, safe)) = nav.floor_below(x, z, py)
            && y <= fy
        {
            return Some(Landing { x, z, y: fy, t, safe });
        }
        py = y;
        t += 0.05;
    }
    None
}

/// Ground the map knows better than the navigation grid (tiles that drop, floors that move): what
/// level the bean is going for, and whether (x, z) has ground there at time t.
pub struct LandCheck<'a> {
    pub y: f64,
    pub ok: &'a dyn Fn(f64, f64, f64) -> bool,
}

/// Where a body flying with (vx, vy, vz) gets back down to the level of `land`.
fn landing_on(bot: &BotView, vx: f64, vz: f64, vy: f64, land: &LandCheck) -> Option<Landing> {
    let p = bot.body.pos;
    if p.y < land.y {
        return None;
    }
    let mut t = 0.05;
    while t <= 2.0 {
        if p.y + vy * t - (GRAVITY / 2.0) * t * t > land.y {
            t += 0.05;
            continue;
        }
        let x = p.x + vx * t;
        let z = p.z + vz * t;
        return Some(Landing {
            x,
            z,
            y: land.y,
            t,
            safe: (land.ok)(x, z, bot.t + t),
        });
    }
    None
}

/// Heading of the stick (or the body, without one).
fn heading(bot: &BotView, out: &BotInput) -> (f64, f64) {
    let l = m::hypot(out.mx, out.mz);
    if l > 0.1 {
        return (out.mx / l, out.mz / l);
    }
    (m::sin(bot.body.yaw), m::cos(bot.body.yaw))
}

/// Dives that are worth it: in the air, when the jump falls short and a dive still reaches ground;
/// held by someone, to break free when the way ahead is safe.
pub fn smart_dive(bot: &mut BotView, out: &mut BotInput, land: Option<&LandCheck>) {
    let b = bot.body;
    let nav = bot.nav;
    if (nav.is_none() && land.is_none()) || b.state != BodyState::Normal || out.dive {
        return;
    }
    let skill = bot.mem.skill();
    let (fx, fz) = heading(bot, out);
    if !b.grounded && b.vel.y < 1.5 {
        let fly = |bot: &BotView, vx, vz, vy| match land {
            Some(l) => landing_on(bot, vx, vz, vy, l),
            None => landing(bot, vx, vz, vy),
        };
        let short = fly(bot, b.vel.x, b.vel.z, b.vel.y);
        if short.is_some_and(|s| s.safe) || (land.is_some() && short.is_none()) {
            return;
        }
        let along = (b.vel.x * fx + b.vel.z * fz).max_js(0.0);
        let sp = DIVE_SPEED.max_js(along.min_js(DIVE_SPEED * 1.25));
        let Some(far) = fly(bot, fx * sp, fz * sp, b.vel.y.max_js(3.0)) else {
            return;
        };
        if !far.safe {
            return;
        }
        // Landing on safe ground, with room to slide on it.
        let room = match land {
            Some(l) => (l.ok)(far.x + fx * 1.2, far.z + fz * 1.2, bot.t + far.t + 0.1),
            None => nav.is_some_and(|n| n.safe(far.x + fx * 1.2, far.z + fz * 1.2, far.y)),
        };
        if room && bot.rng.next() < 0.35 + skill * 0.6 {
            out.dive = true;
        }
        return;
    }
    let Some(nav) = nav else { return };
    let held = b.grounded && b.slow_until > bot.t && b.slow_k <= 0.5;
    if held && bot.rng.next() < (0.25 + skill * 0.5) * BOT_DT * 6.0 {
        let y = b.pos.y;
        if [2.0, 4.0, 6.0]
            .iter()
            .all(|&d| nav.safe(b.pos.x + fx * d, b.pos.z + fz * d, y))
        {
            out.dive = true;
        } else {
            out.jump = true;
        }
    }
}

/// Whether a shove from the bot sends a bean at `o` off the course: the ground beyond it is unsafe,
/// while the bot's own run-up and landing are safe. Returns the direction and distance.
pub fn shove_off(bot: &BotView, o: V3, ok: &dyn Fn(f64, f64) -> bool) -> Option<(f64, f64, f64)> {
    let b = bot.body;
    let dx = o.x - b.pos.x;
    let dz = o.z - b.pos.z;
    let d = m::hypot(dx, dz);
    if d < 0.3 {
        return None;
    }
    let ux = dx / d;
    let uz = dz / d;
    let beyond = [2.0, 3.0, 4.0].iter().any(|&k| !ok(o.x + ux * k, o.z + uz * k));
    let own = [0.8, 1.6]
        .iter()
        .all(|&k: &f64| ok(b.pos.x + ux * k.min_js(d), b.pos.z + uz * k.min_js(d)));
    (beyond && own).then_some((ux, uz, d))
}

/// Direction from `o` towards the nearest unsafe ground (within 3 m), or None when it is well inside.
fn edge_dir(o: V3, ok: &dyn Fn(f64, f64) -> bool) -> Option<(f64, f64)> {
    let mut sx = 0.0;
    let mut sz = 0.0;
    for k in 0..12 {
        let a = (k as f64 / 12.0) * m::PI * 2.0;
        let ax = m::sin(a);
        let az = m::cos(a);
        for r in [1.5, 2.5, 3.5] {
            if !ok(o.x + ax * r, o.z + az * r) {
                sx += ax / r;
                sz += az / r;
                break;
            }
        }
    }
    let l = m::hypot(sx, sz);
    (l > 1e-3).then(|| (sx / l, sz / l))
}

pub struct HumanOpts<'a> {
    /// Narrow or timed section: tiny wobble, no playing around.
    pub precise: bool,
    /// Hops for fun on open ground.
    pub fun: bool,
    /// Grab / tackle beans that get in the way.
    pub rough: bool,
    /// Step round beans in the way.
    pub avoid: bool,
    /// Real ground for rescue dives, where the navigation grid does not know it.
    pub land: Option<&'a LandCheck<'a>>,
    /// How keen on tackling others with a dive (0: never). Default 1.
    pub attack: f64,
    /// A race: crowders are shoved only ahead and to the sides.
    pub forward: bool,
    /// Where the bot may step aside to dodge (default: the navigation grid).
    pub safe: Option<&'a dyn Fn(f64, f64) -> bool>,
}

impl Default for HumanOpts<'_> {
    fn default() -> Self {
        Self {
            precise: false,
            fun: true,
            rough: true,
            avoid: true,
            land: None,
            attack: 1.0,
            forward: false,
            safe: None,
        }
    }
}

/// How close another bean may come before the bot shoves it off with a dive (m).
const SPACE: f64 = 2.6;
/// Tackle reach (centre to centre) and the height a tackle still connects within.
const TACKLE_REACH: f64 = 1.6;
const TACKLE_HEIGHT: f64 = 1.3;

struct Threat {
    t: f64,
    ax: f64,
    az: f64,
    dive: bool,
    id: u32,
}

/// The most urgent attack coming at the bot: a dive whose line passes within tackle reach soon, or
/// somebody reaching out to grab it from close by.
fn incoming(bot: &BotView) -> Option<Threat> {
    let b = bot.body;
    let mut best: Option<Threat> = None;
    for o in bot.others {
        if o.down || (o.pos.y - b.pos.y).abs() > TACKLE_HEIGHT + 0.4 {
            continue;
        }
        let rx = b.pos.x - o.pos.x;
        let rz = b.pos.z - o.pos.z;
        let d = m::hypot(rx, rz);
        if d > 7.0 || d < 1e-3 {
            continue;
        }
        if o.dive {
            let vx = o.vel.x - b.vel.x;
            let vz = o.vel.z - b.vel.z;
            let v2 = vx * vx + vz * vz;
            if v2 < 16.0 {
                continue;
            }
            // Closest approach of the attacker's line.
            let tca = (rx * vx + rz * vz) / v2;
            if !(0.0..=0.6).contains(&tca) {
                continue;
            }
            let mx = rx - vx * tca;
            let mz = rz - vz * tca;
            let miss = m::hypot(mx, mz);
            if miss > TACKLE_REACH + 0.3 {
                continue;
            }
            // Out across its line, on the side the bot is already on.
            let v = m::sqrt(v2);
            let mut sx = -vz / v;
            let mut sz = vx / v;
            if sx * mx + sz * mz < 0.0 || (miss < 0.2 && bot.mem.off.unwrap_or(0.0) < 0.0) {
                sx = -sx;
                sz = -sz;
            }
            if best.as_ref().is_none_or(|b| tca < b.t) {
                best = Some(Threat {
                    t: tca,
                    ax: sx,
                    az: sz,
                    dive: true,
                    id: o.id,
                });
            }
        } else if o.reach && d < 2.6 {
            // Somebody about to grab: back off out of reach.
            let closing = (o.vel.x * rx + o.vel.z * rz) / d;
            let t = (d - 2.1).max_js(0.0) / closing.max_js(1.0);
            if best.as_ref().is_none_or(|b| t < b.t) {
                best = Some(Threat {
                    t,
                    ax: rx / d,
                    az: rz / d,
                    dive: false,
                    id: o.id,
                });
            }
        }
    }
    best
}

/// Getting out of the way of a tackle or a grab: a step aside where there is ground, else a jump.
fn dodge(bot: &mut BotView, out: &mut BotInput, safe: Option<&dyn Fn(f64, f64) -> bool>) -> bool {
    let b = bot.body;
    let t = bot.t;
    if bot.mem.dodge_until.unwrap_or(-1.0) > t {
        out.mx = bot.mem.dodge_x.unwrap_or(0.0);
        out.mz = bot.mem.dodge_z.unwrap_or(0.0);
        return true;
    }
    if b.state != BodyState::Normal || bot.mem.dodge_seen.unwrap_or(-1.0) > t {
        return false;
    }
    let Some(threat) = incoming(bot) else {
        return false;
    };
    let mem = &mut *bot.mem;
    // Whoever went for the bot is remembered for a counterattack, dodged or not.
    mem.foe = Some(threat.id);
    mem.foe_until = Some(t + 3.0);
    let skill = mem.skill();
    let react = mem.react.unwrap_or(0.15);
    // One look per attack: whether this bot notices it in time is decided once.
    mem.dodge_seen = Some(t + 0.3f64.max_js(threat.t + 0.1));
    if threat.t < react * 0.6 || bot.rng.next() > 0.25 + skill * 0.6 {
        return false;
    }
    let step = [0.8, 1.6]
        .iter()
        .all(|&k| safe.is_none_or(|s| s(b.pos.x + threat.ax * k, b.pos.z + threat.az * k)));
    let mem = &mut *bot.mem;
    mem.dodges = Some(mem.dodges.unwrap_or(0.0) + 1.0);
    if safe.is_some() && step {
        mem.dodge_until = Some(t + 0.45f64.min_js(threat.t + 0.2));
        mem.dodge_x = Some(threat.ax);
        mem.dodge_z = Some(threat.az);
        out.mx = threat.ax;
        out.mz = threat.az;
        return true;
    }
    // No room to step aside: hop over a dive (timed: just before it arrives).
    if threat.dive && b.grounded && threat.t < 0.35 {
        out.jump = true;
        return true;
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum Payback {
    Counter,
    Space,
}

/// A tackle the bot has a reason for, and where to aim it.
fn retaliate(
    bot: &BotView,
    safe: Option<&dyn Fn(f64, f64) -> bool>,
    forward: Option<(f64, f64)>,
) -> Option<(f64, f64, Payback)> {
    let b = bot.body;
    let mem = &*bot.mem;
    let t = bot.t;
    let lands_safe =
        |ux: f64, uz: f64| safe.is_none_or(|s| [2.0, 3.5].iter().all(|&k| s(b.pos.x + ux * k, b.pos.z + uz * k)));
    // Counterattack.
    let foe = match mem.foe {
        Some(id) if mem.foe_until.unwrap_or(-1.0) > t => bot.others.iter().find(|o| o.id == id),
        _ => None,
    };
    if let Some(foe) = foe
        && !foe.dive
        && (foe.pos.y - b.pos.y).abs() < 1.0
    {
        let dx = foe.pos.x + foe.vel.x * 0.2 - b.pos.x;
        let dz = foe.pos.z + foe.vel.z * 0.2 - b.pos.z;
        let d = m::hypot(dx, dz);
        if d > 0.5 && d < 3.4 && lands_safe(dx / d, dz / d) {
            return Some((dx / d, dz / d, Payback::Counter));
        }
    }
    // Personal space: the nearest bean closing in (or already right on top of the bot).
    let mut near: Option<(f64, f64, f64)> = None;
    for o in bot.others {
        if o.down || o.dive || (o.pos.y - b.pos.y).abs() > 0.8 {
            continue;
        }
        let dx = o.pos.x - b.pos.x;
        let dz = o.pos.z - b.pos.z;
        let d = m::hypot(dx, dz);
        if d > SPACE || d < 1e-3 || near.is_some_and(|n| d > n.2) {
            continue;
        }
        if let Some((fx, fz)) = forward
            && (dx * fx + dz * fz) / d < -0.3
        {
            continue;
        }
        let closing = -((o.vel.x - b.vel.x) * dx + (o.vel.z - b.vel.z) * dz) / d;
        if closing > 0.2 || d < 1.6 {
            near = Some((dx / d, dz / d, d));
        }
    }
    match near {
        Some((ux, uz, _)) if lands_safe(ux, uz) => Some((ux, uz, Payback::Space)),
        _ => None,
    }
}

/// Somebody worth tackling with a dive: within range where they will be in a moment, near the way
/// the bot is going, on the same level, with safe ground to land on. Returns the direction.
fn tackle_aim(bot: &BotView, out: &BotInput, safe: Option<&dyn Fn(f64, f64) -> bool>) -> Option<(f64, f64)> {
    let b = bot.body;
    let (hx, hz) = heading(bot, out);
    let mut best: Option<(f64, f64, f64)> = None;
    for o in bot.others {
        if o.dive || (o.pos.y - b.pos.y).abs() > 0.8 {
            continue;
        }
        // Where they will be when the dive gets there (it covers ~3 m in a quarter of a second).
        let px = o.pos.x + o.vel.x * 0.22;
        let pz = o.pos.z + o.vel.z * 0.22;
        let dx = px - b.pos.x;
        let dz = pz - b.pos.z;
        let d = m::hypot(dx, dz);
        if !(1.2..=3.8).contains(&d) {
            continue;
        }
        let ux = dx / d;
        let uz = dz / d;
        // Roughly the way the bot is going: no turning round for it.
        if ux * hx + uz * hz < 0.6 {
            continue;
        }
        if let Some(s) = safe
            && ![2.0, 3.5, 5.0].iter().all(|&k| s(b.pos.x + ux * k, b.pos.z + uz * k))
        {
            continue;
        }
        if best.is_none_or(|b| d < b.2) {
            best = Some((ux, uz, d));
        }
    }
    best.map(|b| (b.0, b.1))
}

/// The human touch on top of a brain's decision: unsteady hands, hops for no reason, sidesteps,
/// shoves and grabs, dives at others.
pub fn humanize(bot: &mut BotView, out: &mut BotInput, o: &HumanOpts) {
    let b = bot.body;
    let skill = bot.mem.skill();
    let ph = bot.mem.ph.unwrap_or(0.0);
    let t = bot.t;
    smart_dive(bot, out, o.land);
    let nav = bot.nav;
    let gy = b.pos.y;
    let nav_safe = move |x: f64, z: f64| nav.is_some_and(|n| n.safe(x, z, gy));
    let safe: Option<&dyn Fn(f64, f64) -> bool> = match (o.safe, nav) {
        (Some(s), _) => Some(s),
        (None, Some(_)) => Some(&nav_safe),
        (None, None) => None,
    };
    // Out of the way of a tackle or a grab (on narrow ground: only by jumping).
    if !out.dive && dodge(bot, out, if o.precise { None } else { safe }) {
        return;
    }
    // Held by somebody: that is who gets it once the bot is free.
    if b.grounded && b.slow_until > t && b.slow_k <= 0.5 {
        let mut d = 2.6;
        for x in bot.others {
            let dx = m::hypot(x.pos.x - b.pos.x, x.pos.z - b.pos.z);
            if dx < d {
                d = dx;
                bot.mem.foe = Some(x.id);
                bot.mem.foe_until = Some(t + 3.0);
            }
        }
    }
    // Wobble: a slow, uneven drift of the aim.
    let amount = if o.precise {
        0.03 + (1.0 - skill) * 0.03
    } else {
        0.12 + (1.0 - skill) * 0.14
    };
    let a = (m::sin(t * 0.9 + ph) * 0.6 + m::sin(t * 2.3 + ph * 1.7) * 0.4) * amount;
    let c = m::cos(a);
    let s = m::sin(a);
    let mx = out.mx * c - out.mz * s;
    out.mz = out.mx * s + out.mz * c;
    out.mx = mx;
    if o.precise {
        return;
    }
    let moving = m::hypot(out.mx, out.mz) > 0.5;
    // Someone right in front: step around them (or shove them, if that is the kind of player we are).
    let ahead = bean_ahead(bot, 1.8, 0.55);
    if let Some((ao, ad)) = ahead
        && moving
        && o.avoid
    {
        let dx = ao.pos.x - b.pos.x;
        let dz = ao.pos.z - b.pos.z;
        let side = if bot.mem.off.unwrap_or(0.0) >= 0.0 { 1.0 } else { -1.0 };
        let k = (1.8 - ad) * 0.6;
        out.mx += (-dz / ad) * side * k;
        out.mz += (dx / ad) * side * k;
    }
    if o.rough {
        let aggro = bot.mem.aggro.unwrap_or(0.3);
        let shove = match (ahead, nav) {
            (Some((ao, ad)), Some(n)) if b.grounded && t > 5.0 && ad > 1.3 && ad < 2.8 => {
                let oy = ao.pos.y;
                shove_off(bot, ao.pos, &|x, z| n.safe(x, z, oy))
            }
            _ => None,
        };
        // Somebody at the edge right ahead: tackle them over it (the pushy ones, mostly).
        if shove.is_some() && bot.rng.next() < (0.1 + aggro * 0.5) * BOT_DT * 3.0 {
            out.dive = true;
        } else if bot.mem.grab_until.unwrap_or(-1.0) > t {
            out.grab = true;
        } else if ahead.is_some_and(|(_, d)| d < 1.35) && b.grounded && bot.rng.next() < aggro * 1.2 * BOT_DT {
            bot.mem.grab_until = Some(t + 0.4 + bot.rng.next() * 0.9);
            out.grab = true;
        } else if b.grounded && b.state == BodyState::Normal && t > 2.0 {
            let scale = o.attack;
            // Paying back an attack, or shoving off whoever crowds the bot (now and then grabbing instead).
            let back = retaliate(bot, safe, if o.forward { Some(heading(bot, out)) } else { None });
            let rate = match back {
                Some((_, _, Payback::Counter)) => (1.0 + aggro * 2.5) * scale,
                Some((_, _, Payback::Space)) => (1.0 + aggro * 3.0) * scale,
                None => 0.0,
            };
            if let Some((ux, uz, kind)) = back
                && bot.rng.next() < rate * BOT_DT
            {
                if kind == Payback::Space && bot.rng.next() < 0.3 {
                    bot.mem.grab_until = Some(t + 0.4 + bot.rng.next() * 0.6);
                    out.grab = true;
                    return;
                }
                out.mx = ux;
                out.mz = uz;
                out.dive = true;
                bot.mem.aimed = true;
                // (Counted for audits and tuning.)
                if kind == Payback::Counter {
                    bot.mem.foe = None;
                    bot.mem.counters = Some(bot.mem.counters.unwrap_or(0.0) + 1.0);
                } else {
                    bot.mem.shoves = Some(bot.mem.shoves.unwrap_or(0.0) + 1.0);
                }
                return;
            }
            // A tackle: dive at where somebody will be (the pushy ones, much more often).
            let aim = tackle_aim(bot, out, safe);
            let keen = (0.35 + aggro * 2.0) * scale;
            if let Some((ux, uz)) = aim
                && bot.rng.next() < keen * BOT_DT
            {
                out.mx = ux;
                out.mz = uz;
                out.dive = true;
                bot.mem.aimed = true;
                return;
            }
        }
    }
    // Bunny hops on open ground.
    if o.fun
        && b.grounded
        && moving
        && m::hypot(b.vel.x, b.vel.z) > 5.0
        && bot.rng.next() < bot.mem.jumpy.unwrap_or(0.5) * 0.3 * BOT_DT
    {
        out.jump = true;
    }
    let l = m::hypot(out.mx, out.mz);
    if l > 1.0 {
        out.mx /= l;
        out.mz /= l;
    }
}

/// The stick as a hand moves it: eased towards what the brain wants. Letting go is quicker.
pub fn smooth_stick(mem: &mut BotMem, out: &mut BotInput) {
    // A tackle goes where it is aimed, not where the eased stick happens to point.
    let aimed = out.dive && mem.aimed;
    mem.aimed = false;
    if aimed {
        mem.smx = Some(out.mx);
        mem.smz = Some(out.mz);
        return;
    }
    let px = mem.smx.unwrap_or(out.mx);
    let pz = mem.smz.unwrap_or(out.mz);
    let k = if m::hypot(out.mx, out.mz) < 0.15 { 0.7 } else { 0.5 };
    out.mx = px + (out.mx - px) * k;
    out.mz = pz + (out.mz - pz) * k;
    mem.smx = Some(out.mx);
    mem.smz = Some(out.mz);
}

/// In the air: steers so that the body comes down on (tx, tz) when it falls to height ty. Returns
/// false when it cannot get down there any more (already below it): it heads straight there.
pub fn aim_landing(bot: &BotView, tx: f64, ty: f64, tz: f64, out: &mut BotInput) -> bool {
    let b = bot.body;
    let dx = tx - b.pos.x;
    let dz = tz - b.pos.z;
    let disc = b.vel.y * b.vel.y + 2.0 * GRAVITY * (b.pos.y - ty);
    if disc < 0.0 {
        steer(bot, tx, tz, out, 1.0);
        return false;
    }
    let t = 0.08f64.max_js((b.vel.y + m::sqrt(disc)) / GRAVITY);
    // (Air control: full stick asks for a run, or for the speed the body already flies at.)
    let top = RUN_SPEED.max_js(m::hypot(b.vel.x, b.vel.z));
    let mut mx = dx / t / top;
    let mut mz = dz / t / top;
    let l = m::hypot(mx, mz);
    if l > 1.0 {
        mx /= l;
        mz /= l;
    }
    out.mx = mx;
    out.mz = mz;
    true
}

/// A pad in a chain of bounces (x may move with time).
pub struct Hop {
    pub x: WpX,
    pub y: f64,
    pub z: f64,
    /// Radius of the pad, and how hard it throws up (m/s).
    pub r: f64,
    pub power: f64,
}

fn hop_x(h: &Hop, t: f64) -> f64 {
    match &h.x {
        WpX::At(x) => *x,
        WpX::Moving(f) => f(t),
    }
}

/// Bouncing across pads to a landing spot: a waypoint `drive`. Returns false once on the ground past `edge`.
pub fn hop_chain(key: String, hops: Vec<Hop>, land: V3, edge: f64, ready: Option<BotTest>) -> Drive {
    Box::new(move |bot, out| {
        let b = bot.body;
        let p = b.pos;
        if b.grounded {
            if p.z > edge + 0.5 {
                // Missed, and down in a basin with the pads: back onto the nearest one.
                if p.y > land.y - 1.0 {
                    return false;
                }
                let mut near = 0;
                for (j, h) in hops.iter().enumerate() {
                    let n = &hops[near];
                    if m::hypot(hop_x(h, bot.t) - p.x, h.z - p.z) < m::hypot(hop_x(n, bot.t) - p.x, n.z - p.z) {
                        near = j;
                    }
                }
                let n = &hops[near];
                if (n.y - p.y).abs() > 1.0 {
                    return false;
                }
                bot.mem.set(&key, near as f64);
                steer(bot, hop_x(n, bot.t), n.z, out, 1.0);
                return true;
            }
            bot.mem.set(&key, 0.0);
            let h = &hops[0];
            if let Some(ready) = &ready
                && !ready(bot)
            {
                // Wait where the first pad passes.
                let mut lo = f64::INFINITY;
                let mut hi = f64::NEG_INFINITY;
                for k in 0..40 {
                    let x = hop_x(h, bot.t + k as f64 * 0.25);
                    lo = lo.min_js(x);
                    hi = hi.max_js(x);
                }
                let wx = if hi - lo > 1.0 {
                    (hi - 0.5).min_js((lo + 0.5).max_js(p.x))
                } else {
                    lo
                };
                follow(bot, wx, edge - 1.3, out);
                return true;
            }
            let x = hop_x(h, bot.t + 0.5);
            steer(bot, x, h.z, out, 1.0);
            if p.z > edge - 1.4 {
                out.jump = true;
            }
            return true;
        }
        if b.state != BodyState::Normal {
            return false;
        }
        let mut i = bot.mem.get(&key).unwrap_or(0.0) as usize;
        // Just thrown up by a pad: the next one is the target.
        for (j, h) in hops.iter().enumerate() {
            if b.vel.y > h.power * 0.7
                && m::hypot(p.x - hop_x(h, bot.t), p.z - h.z) < h.r + 1.2
                && (p.y - h.y).abs() < 2.5
            {
                i = i.max(j + 1);
            }
        }
        bot.mem.set(&key, i as f64);
        let Some(h) = hops.get(i) else {
            aim_landing(bot, land.x, land.y, land.z, out);
            return true;
        };
        // Where a moving pad will be by the time the body comes down to it.
        let disc = b.vel.y * b.vel.y + 2.0 * GRAVITY * (p.y - h.y);
        let t = if disc > 0.0 {
            (b.vel.y + m::sqrt(disc)) / GRAVITY
        } else {
            0.0
        };
        aim_landing(bot, hop_x(h, bot.t + t), h.y, h.z, out);
        true
    })
}

/// A bonus lying close by (ahead of the bot, on its level), if any: worth a small detour.
pub fn bonus_near(bot: &BotView, range: f64, ahead: bool) -> Option<(f64, f64)> {
    let b = bot.body;
    let mut best = None;
    let mut bd = range;
    for x in bot.bonuses {
        if (x.y - b.pos.y).abs() > 1.2 {
            continue;
        }
        let d = m::hypot(x.x - b.pos.x, x.z - b.pos.z);
        if d < bd && (!ahead || x.z > b.pos.z - 1.5) {
            bd = d;
            best = Some((x.x, x.z));
        }
    }
    best
}

fn target_x(bot: &BotView, w: &Waypoint) -> f64 {
    match &w.x {
        WpX::Moving(f) => f(bot.t),
        WpX::At(x) => x + bot.mem.off.unwrap_or(0.0) * w.w.unwrap_or(1.5),
    }
}

/// Follows waypoints along a course (by z), with waits, timed jumps, detours and special stretches.
pub fn path_brain(points: Vec<Waypoint>, dive_chance: f64) -> BotBrain {
    Box::new(move |bot, out| path_step(&points, dive_chance, bot, out))
}

pub fn path_step<W: Borrow<Waypoint>>(points: &[W], dive_chance: f64, bot: &mut BotView, out: &mut BotInput) {
    init_bot(bot);
    let b = bot.body;
    let n = points.len();
    let reach = |w: &Waypoint| if w.w == Some(0.0) { 0.5 } else { 1.2 };
    let mut i = bot.mem.wp.map_or(-1, |w| w as i64);
    let behind = i >= 1 && b.pos.z < points[i as usize - 1].borrow().z - 4.0;
    if i < 0 || behind {
        i = points
            .iter()
            .position(|p| p.borrow().z > b.pos.z - 0.5)
            .map_or(n as i64 - 1, |k| k as i64);
    }
    let mut i = i as usize;
    while i < n
        && (b.pos.z > points[i].borrow().z + 0.3
            || m::hypot(
                target_x(bot, points[i].borrow()) - b.pos.x,
                points[i].borrow().z - b.pos.z,
            ) < reach(points[i].borrow()))
        && i < n - 1
    {
        i += 1;
    }
    bot.mem.wp = Some(i);
    let Some(wp) = points.get(i).map(Borrow::borrow) else {
        return;
    };
    if let Some(drive) = &wp.drive
        && drive(bot, out)
    {
        // Special stretches jam too (bots backing off for another run into the ones behind them).
        unstick(bot, out);
        return;
    }
    if let Some(detour) = &wp.detour
        && let Some((ax, az)) = detour(bot)
    {
        nav_to(bot, ax, az, out, 1.0, 0.4);
        humanize(
            bot,
            out,
            &HumanOpts {
                precise: true,
                ..Default::default()
            },
        );
        unstick(bot, out);
        return;
    }
    let moving_x = matches!(wp.x, WpX::Moving(_));
    let precise = wp.w == Some(0.0) || wp.wait.is_some() || moving_x || wp.jump_when.is_some();
    let skill = bot.mem.skill();
    // Once a wait is over the bot commits to that waypoint. The less patient ones sometimes just go for it.
    if let Some(wait) = &wp.wait
        && bot.mem.go != Some(i)
    {
        let ready = wait(bot);
        let reckless = !ready && bot.rng.next() < (1.0 - skill) * 0.12 * BOT_DT;
        if (ready || reckless) && bot.mem.ready_at.is_none() {
            bot.mem.ready_at = Some(
                bot.t
                    + if reckless {
                        0.0
                    } else {
                        bot.mem.react.unwrap_or(0.15) * 0.5
                    },
            );
        }
        if let Some(at) = bot.mem.ready_at
            && bot.t >= at
        {
            bot.mem.go = Some(i);
            bot.mem.ready_at = None;
        }
    }
    if wp.wait.is_some() && bot.mem.go != Some(i) {
        // Hold at the previous waypoint (keeps a corridor position exact).
        if i > 0 {
            let hold = points[i - 1].borrow();
            let hx = target_x(bot, hold);
            follow(bot, hx, hold.z, out);
        } else {
            out.mx = 0.0;
            out.mz = 0.0;
        }
        let when = wp.jump_when.as_ref().is_some_and(|f| f(bot));
        if when && b.grounded {
            out.jump = true;
        } else if b.grounded
            && bot.rng.next() < bot.mem.jumpy.unwrap_or(0.5) * 0.25 * BOT_DT
            && bot.nav.is_some_and(|nav| nav.safe(b.pos.x, b.pos.z, b.pos.y))
        {
            // Impatient hops while waiting, where it is safe to.
            out.jump = true;
        }
        humanize(
            bot,
            out,
            &HumanOpts {
                precise: true,
                ..Default::default()
            },
        );
        return;
    }
    let mut tx = target_x(bot, wp);
    let mut tz = wp.z;
    // A bonus a few steps off the line (in open sections): go and take it.
    if !precise
        && let Some((bx, bz)) = bonus_near(bot, 6.0, true)
        && bz < wp.z + 2.0
    {
        tx = bx;
        tz = bz;
    }
    let speed = wp.speed.unwrap_or(1.0) * if moving_x { 1.0 } else { bot.mem.spd.unwrap_or(1.0) };
    let d = if precise {
        if wp.w == Some(0.0) {
            follow(bot, tx, tz, out)
        } else {
            steer(bot, tx, tz, out, speed)
        }
    } else {
        nav_to(bot, tx, tz, out, speed, reach(wp))
    };
    if precise
        && wp.w == Some(0.0)
        && let Some(s) = wp.speed
        && s != 0.0
    {
        out.mx *= s;
        out.mz *= s;
    }
    // Jumps work a moment after leaving the ground too (coyote time), as for players.
    let can_jump = b.grounded || b.coyote > 0.02;
    if wp.jump && d < 2.6 && can_jump {
        out.jump = true;
    }
    if wp.jump_when.as_ref().is_some_and(|f| f(bot)) && can_jump {
        out.jump = true;
    }
    if dive_chance != 0.0 && b.grounded && bot.rng.next() < dive_chance * BOT_DT {
        out.dive = true;
    }
    // The last stretch: dive over the line like everybody does.
    if i == n - 1 && !precise && d < 6.0 && d > 3.0 && b.grounded && bot.rng.next() < 0.4 + skill * 0.4 {
        out.dive = true;
    }
    humanize(
        bot,
        out,
        &HumanOpts {
            precise,
            fun: !precise,
            attack: 0.9,
            forward: true,
            ..Default::default()
        },
    );
    unstick(bot, out);
}

/// Several routes (e.g. one per safe lane); each bot takes the one starting nearest to it.
pub fn routes_brain(routes: Vec<(f64, Vec<Waypoint>)>) -> BotBrain {
    Box::new(move |bot, out| {
        let route = match bot.mem.route {
            Some(r) => r,
            None => {
                let x = bot.body.pos.x;
                let mut best = 0;
                for (i, r) in routes.iter().enumerate() {
                    if (r.0 - x).abs() < (routes[best].0 - x).abs() {
                        best = i;
                    }
                }
                bot.mem.route = Some(best);
                best
            }
        };
        path_step(&routes[route].1, 0.0, bot, out);
    })
}

pub struct ArenaOpts {
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub safe: Option<Box<dyn Fn(f64, f64, f64) -> bool + Send + Sync>>,
    pub jump_when: Option<BotTest>,
    pub retarget: Option<f64>,
    /// Places worth going to (steps, pads, bumpers…): the playground.
    pub pois: Vec<(f64, f64)>,
    /// Emote now and then (lobby).
    pub social: bool,
    /// The floor moves or drops (not in the navigation grid): only `safe` decides where to stand.
    pub ignore_nav: bool,
    /// Is there ground to stand on at (x, z) at time t? Bots steer round holes (with `ignore_nav`).
    pub floor: Option<Box<dyn Fn(f64, f64, f64) -> bool + Send + Sync>>,
}

impl ArenaOpts {
    pub fn new(radius: f64) -> Self {
        Self {
            x: 0.0,
            z: 0.0,
            radius,
            safe: None,
            jump_when: None,
            retarget: None,
            pois: Vec::new(),
            social: false,
            ignore_nav: false,
            floor: None,
        }
    }
}

/// Turns the stick away from holes just ahead (small turns first); stops at the edge if all else fails.
fn avoid_holes(bot: &BotView, out: &mut BotInput, floor: &dyn Fn(f64, f64, f64) -> bool) {
    let b = bot.body;
    let base = m::atan2(out.mx, out.mz);
    let len = m::hypot(out.mx, out.mz);
    let clear = |a: f64| {
        [0.9, 1.8, 2.7]
            .iter()
            .all(|&d| floor(b.pos.x + m::sin(a) * d, b.pos.z + m::cos(a) * d, bot.t + d / 8.0))
    };
    for turn in [0.0, 0.4, -0.4, 0.8, -0.8, 1.3, -1.3, 1.9, -1.9, m::PI] {
        if !clear(base + turn) {
            continue;
        }
        out.mx = m::sin(base + turn) * len;
        out.mz = m::cos(base + turn) * len;
        return;
    }
    out.mx = 0.0;
    out.mz = 0.0;
}

/// Moves around an arena like a player: picks spots that are safe and not crowded (or, for the pushy
/// ones, goes after somebody), runs there by A*, and times its jumps with a reaction delay.
pub fn arena_brain(opts: ArenaOpts) -> BotBrain {
    Box::new(move |bot, out| arena_step(&opts, bot, out))
}

fn arena_step(opts: &ArenaOpts, bot: &mut BotView, out: &mut BotInput) {
    let cx = opts.x;
    let cz = opts.z;
    init_bot(bot);
    let b = bot.body;
    let t = bot.t;
    let aggro = bot.mem.aggro.unwrap_or(0.3);
    if bot.mem.pref.is_none() {
        bot.mem.pref = Some(opts.radius * (0.25 + bot.rng.next() * 0.45));
    }
    // Judged from the ground the bot stands (or last stood) on: in the air everything looks unsafe.
    if b.grounded {
        bot.mem.gy = Some(b.pos.y);
    }
    let gy = bot.mem.gy.unwrap_or(b.pos.y);
    let nav = bot.nav;
    let ok_at = |x: f64, z: f64| {
        opts.safe.as_ref().is_none_or(|s| s(x, z, t)) && (opts.ignore_nav || nav.is_none_or(|n| n.safe(x, z, gy)))
    };
    // Hunting someone: best a bean near the edge, to shove it off.
    let mut hunt = bot
        .mem
        .hunt
        .and_then(|id| bot.others.iter().find(|o| o.id == id).copied());
    if let Some(h) = hunt
        && (t > bot.mem.hunt_until.unwrap_or(0.0) || h.down)
    {
        hunt = None;
        bot.mem.hunt = None;
    }
    // (Everyone starts near the edge: nobody goes after anybody in the first few seconds.)
    if bot.mem.next_hunt.is_none() {
        bot.mem.next_hunt = Some(5.0 + bot.rng.next() * 6.0);
    }
    if hunt.is_none() && b.grounded && t > bot.mem.next_hunt.unwrap_or(0.0) {
        bot.mem.next_hunt = Some(t + 3.0 + bot.rng.next() * 6.0);
        if bot.rng.next() < aggro * 0.7 {
            let mut best = f64::NEG_INFINITY;
            for o in bot.others {
                let d = m::hypot(o.pos.x - b.pos.x, o.pos.z - b.pos.z);
                if o.down || d > 12.0 || (o.pos.y - b.pos.y).abs() > 1.0 {
                    continue;
                }
                let score =
                    (if edge_dir(o.pos, &ok_at).is_some() { 5.0 } else { 0.0 }) - d * 0.35 + bot.rng.next() * 2.0;
                if score > best {
                    best = score;
                    hunt = Some(*o);
                }
            }
            if let Some(h) = hunt {
                bot.mem.hunt = Some(h.id);
                bot.mem.hunt_until = Some(t + 3.0 + bot.rng.next() * 3.0);
            }
        }
    }
    let expired = t > bot.mem.until.unwrap_or(-1e9);
    let unsafe_spot = bot.mem.tx.is_some_and(|tx| !ok_at(tx, bot.mem.tz.unwrap_or(0.0)));
    if hunt.is_none() && (bot.mem.tx.is_none() || (b.grounded && (expired || unsafe_spot))) {
        bot.mem.arrived = false;
        let mut best = f64::NEG_INFINITY;
        let pois = &opts.pois;
        for k in 0..8 {
            let (x, z);
            if !pois.is_empty() && k < 2 && bot.rng.next() < 0.5 {
                let p = pois[(bot.rng.next() * pois.len() as f64).floor() as usize];
                x = p.0;
                z = p.1;
            } else {
                let a = bot.rng.next() * m::PI * 2.0;
                let r = m::sqrt(bot.rng.next()) * opts.radius;
                x = cx + m::cos(a) * r;
                z = cz + m::sin(a) * r;
            }
            let mut score = if ok_at(x, z) { 0.0 } else { -100.0 };
            score -= (m::hypot(x - cx, z - cz) - bot.mem.pref.unwrap_or(5.0)).abs() * 0.25;
            let mut crowd: f64 = 9.0;
            for o in bot.others {
                crowd = crowd.min_js(m::hypot(o.pos.x - x, o.pos.z - z));
            }
            score += crowd.min_js(4.0) * (0.6 - aggro * 0.5);
            score -= m::hypot(x - b.pos.x, z - b.pos.z) * 0.08;
            score += bot.rng.next() * 1.5;
            if score > best {
                best = score;
                bot.mem.tx = Some(x);
                bot.mem.tz = Some(z);
            }
        }
        bot.mem.until = Some(t + opts.retarget.unwrap_or(2.0) + bot.rng.next() * 2.5);
    }
    let bonus = if hunt.is_some() {
        None
    } else {
        bonus_near(bot, 8.0, false)
    };
    let mut tx = match bonus {
        Some((bx, bz)) if ok_at(bx, bz) => bx,
        _ => bot.mem.tx.unwrap_or(cx),
    };
    let mut tz = match bonus {
        Some((bx, bz)) if ok_at(bx, bz) => bz,
        _ => bot.mem.tz.unwrap_or(cz),
    };
    // Shoving: from the inside, towards the edge. Not lined up yet: get round to the inside first.
    let shove = hunt.and_then(|h| shove_off(bot, h.pos, &ok_at));
    if let Some(h) = hunt {
        tx = h.pos.x;
        tz = h.pos.z;
        let e = if shove.is_some() { None } else { edge_dir(h.pos, &ok_at) };
        if let Some((ex, ez)) = e
            && ok_at(h.pos.x - ex * 1.9, h.pos.z - ez * 1.9)
        {
            tx = h.pos.x - ex * 1.9;
            tz = h.pos.z - ez * 1.9;
        }
    }
    let speed = if hunt.is_some() {
        1.0
    } else {
        0.85 + bot.mem.spd.unwrap_or(1.0) * 0.15
    };
    let d = nav_to(bot, tx, tz, out, speed, if hunt.is_some() { 1.1 } else { 0.9 });
    if let (Some(_), Some((ux, uz, sd))) = (hunt, shove) {
        // Lined up: charge straight at them, and tackle from close by (or just run them over).
        out.mx = ux;
        out.mz = uz;
        if sd > 1.3 && sd < 2.7 && b.grounded && bot.rng.next() < (0.2 + aggro * 0.6) * BOT_DT * 6.0 {
            out.dive = true;
        }
    } else if let Some(h) = hunt {
        let hd = m::hypot(h.pos.x - b.pos.x, h.pos.z - b.pos.z);
        // Where the prey will be when a dive gets there.
        let px = h.pos.x + h.vel.x * 0.22 - b.pos.x;
        let pz = h.pos.z + h.vel.z * 0.22 - b.pos.z;
        let pd = m::hypot(px, pz);
        if hd < 1.4 {
            out.grab = true;
            bot.mem.grab_until = Some(t + 0.3);
        } else if pd > 1.6
            && pd < 3.2
            && b.grounded
            && b.state == BodyState::Normal
            && [2.0, 3.5]
                .iter()
                .all(|&k| ok_at(b.pos.x + (px / pd) * k, b.pos.z + (pz / pd) * k))
            && bot.rng.next() < (0.3 + aggro * 1.5) * BOT_DT
        {
            out.mx = px / pd;
            out.mz = pz / pd;
            out.dive = true;
            bot.mem.aimed = true;
        }
    } else if d < (if bot.mem.arrived { 1.8 } else { 1.0 }) {
        // There: potter about round the spot instead of freezing.
        bot.mem.arrived = true;
        let w = t * 0.5 + bot.mem.ph.unwrap_or(0.0);
        let px = tx + m::sin(w) * 0.6;
        let pz = tz + m::cos(w * 1.3) * 0.6;
        if ok_at(px, pz) {
            steer(bot, px, pz, out, 0.3);
        } else {
            steer(bot, tx, tz, out, 0.3);
        }
    }
    if let Some(floor) = &opts.floor
        && m::hypot(out.mx, out.mz) > 0.1
    {
        avoid_holes(bot, out, floor.as_ref());
    }
    if opts.social && d < 1.5 && b.grounded && bot.rng.next() < 0.12 * BOT_DT {
        out.emote = 1 + (bot.rng.next() * EMOTES as f64).floor() as u32;
    }
    // jumpWhen models reaction time itself. Just before landing counts too: the jump is kept for a
    // moment and goes off on touchdown.
    let landing = !b.grounded && b.vel.y < 0.0 && b.pos.y - gy < 0.35;
    if opts.jump_when.as_ref().is_some_and(|f| f(bot)) && (b.grounded || landing) {
        out.jump = true;
    }
    humanize(
        bot,
        out,
        &HumanOpts {
            fun: opts.jump_when.is_none(),
            rough: hunt.is_none(),
            avoid: hunt.is_none(),
            safe: Some(&ok_at),
            ..Default::default()
        },
    );
    unstick(bot, out);
}
