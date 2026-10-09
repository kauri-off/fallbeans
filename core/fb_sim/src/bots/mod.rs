//! Bot behaviour shared by the maps: personalities, steering, A* routes,
//! getting unstuck, dodging, tackles and grabs, and the waypoint and arena brains. Brains run at
//! 20 Hz on the server; everything random comes from the bot's own `rng`.
use core::borrow::Borrow;
use core::marker::PhantomData;
use std::collections::BTreeMap;
use std::sync::Arc;

use fb_shared::EMOTES;
use fb_shared::NEVER;
use fb_shared::rng::Rng;

use crate::m::{self, MinMax};
use crate::map::{Hook, MapLogic, Steer};
use crate::math::{V3, dist_xz};
use crate::nav::{Nav, NavPoint, PathOpts};
use crate::physics::{Body, BodyState, DIVE_SPEED, GRAVITY, RUN_SPEED};
use crate::world::World;

/// Bot brains run at 20 Hz (BOT_EVERY ticks); timers below use this step.
pub const BOT_DT: f64 = 1.0 / 20.0;
// BOT_DT is written out (not BOT_EVERY · DT, which rounds differently): keep the two in step.
const _: () = assert!(fb_shared::TICK_RATE == 20 * fb_shared::BOT_EVERY);

mod arena;
mod human;
mod moves;
mod route;

pub use arena::*;
pub use human::*;
pub use moves::*;
pub use route::*;

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
            at: NEVER,
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

/// Who a bot is: drawn on its first decision (`init_bot`) and kept for the round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Traits {
    /// 0.45…1: how well it judges gaps and timing.
    pub skill: f64,
    /// How readily it tackles, shoves and grabs.
    pub aggro: f64,
    /// How often it hops for nothing.
    pub jumpy: f64,
    /// Reaction time, s.
    pub react: f64,
    /// Stick deflection when it runs.
    pub spd: f64,
    /// Its side of a wide path, −1…1.
    pub off: f64,
    /// Phase of its wandering.
    pub ph: f64,
}

impl Default for Traits {
    fn default() -> Self {
        Self {
            skill: 0.7,
            aggro: 0.3,
            jumpy: 0.5,
            react: 0.15,
            spd: 1.0,
            off: 0.0,
            ph: 0.0,
        }
    }
}

/// A map's own note in every bot's memory, made by `Builder::note` while the map is built.
#[derive(Debug)]
pub struct Note<T> {
    id: u32,
    of: PhantomData<fn() -> T>,
}

impl<T> Clone for Note<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Note<T> {}

impl<T> Note<T> {
    pub(crate) fn new(id: u32) -> Self {
        Self { id, of: PhantomData }
    }
}

/// What a note holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Num(f64),
    Index(usize),
    Bits(u32),
}

pub trait NoteValue: Copy {
    fn slot(self) -> Slot;
    fn of(s: Slot) -> Option<Self>;
}

impl NoteValue for f64 {
    fn slot(self) -> Slot {
        Slot::Num(self)
    }
    fn of(s: Slot) -> Option<Self> {
        if let Slot::Num(v) = s { Some(v) } else { None }
    }
}

impl NoteValue for usize {
    fn slot(self) -> Slot {
        Slot::Index(self)
    }
    fn of(s: Slot) -> Option<Self> {
        if let Slot::Index(v) = s { Some(v) } else { None }
    }
}

impl NoteValue for u32 {
    fn slot(self) -> Slot {
        Slot::Bits(self)
    }
    fn of(s: Slot) -> Option<Self> {
        if let Slot::Bits(v) = s { Some(v) } else { None }
    }
}

/// What a bot remembers between decisions.
#[derive(Clone, Debug)]
pub struct BotMem {
    /// Drawn on the first decision (`init_bot`).
    traits: Option<Traits>,
    /// Seconds it has been trying to move without getting anywhere, and where it was then.
    pub stuck: f64,
    pub anchor: Option<(f64, f64)>,
    /// Sidesteps out of a jam so far (they alternate sides), the current one's direction and its end.
    pub tries: u32,
    pub side: (f64, f64),
    pub side_until: f64,
    /// Dodging a dive: the stick to hold and until when; a dive seen until when.
    pub dodge: (f64, f64),
    pub dodge_until: f64,
    pub dodge_seen: f64,
    /// The bean it is fighting, until when.
    pub foe: Option<u32>,
    pub foe_until: f64,
    /// Holding the grab until.
    pub grab_until: f64,
    /// This decision's dive is aimed at someone (not smoothed).
    pub aimed: bool,
    /// The stick of the last decision (smoothing).
    pub stick: Option<(f64, f64)>,
    pub way: WaypointMem,
    pub arena: ArenaMem,
    /// The map's own notes.
    notes: BTreeMap<u32, Slot>,
}

/// Waypoint brains: the current waypoint, the one it may go on from, and when it got ready to go;
/// where it was at its last decision (position, time, speed), to tell a respawn or a portal.
#[derive(Clone, Debug, Default)]
pub struct WaypointMem {
    pub wp: Option<usize>,
    pub go: Option<usize>,
    pub ready_at: Option<f64>,
    pub seen: Option<(V3, f64, f64)>,
}

/// Arena brains: preferred distance from the centre, ground height, whom it hunts and until when, the
/// next hunt, the spot it heads for and until when, and whether it is there.
#[derive(Clone, Debug)]
pub struct ArenaMem {
    pub pref: Option<f64>,
    pub gy: Option<f64>,
    pub hunt: Option<u32>,
    pub hunt_until: f64,
    pub next_hunt: Option<f64>,
    pub target: Option<(f64, f64)>,
    pub until: f64,
    pub arrived: bool,
}

impl Default for ArenaMem {
    fn default() -> Self {
        Self {
            pref: None,
            gy: None,
            hunt: None,
            hunt_until: 0.0,
            next_hunt: None,
            target: None,
            until: NEVER,
            arrived: false,
        }
    }
}

impl Default for BotMem {
    fn default() -> Self {
        Self {
            traits: None,
            stuck: 0.0,
            anchor: None,
            tries: 0,
            side: (1.0, 0.0),
            side_until: NEVER,
            dodge: (0.0, 0.0),
            dodge_until: NEVER,
            dodge_seen: NEVER,
            foe: None,
            foe_until: NEVER,
            grab_until: NEVER,
            aimed: false,
            stick: None,
            way: WaypointMem::default(),
            arena: ArenaMem::default(),
            notes: BTreeMap::new(),
        }
    }
}

impl BotMem {
    /// Who the bot is (the defaults before its first decision).
    pub fn traits(&self) -> Traits {
        self.traits.unwrap_or_default()
    }

    pub fn get<T: NoteValue>(&self, n: Note<T>) -> Option<T> {
        self.notes.get(&n.id).copied().and_then(T::of)
    }

    pub fn set<T: NoteValue>(&mut self, n: Note<T>, v: T) {
        self.notes.insert(n.id, v.slot());
    }

    pub fn remove<T>(&mut self, n: Note<T>) {
        self.notes.remove(&n.id);
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
    /// The map's world (moving parts, portals); the map's own state is its logic's.
    pub world: &'a World,
    /// Points of everybody in the round (points games).
    pub scores: &'a BTreeMap<u32, i64>,
}

impl BotView<'_> {
    pub fn score(&self, id: u32) -> i64 {
        self.scores.get(&id).copied().unwrap_or(0)
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
    /// Lateral spread between bots (static x only).
    pub lane: Lane,
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
    /// The map's logic steers here by its state (`MapLogic::steer`), before `drive` and `detour`.
    pub hook: Option<Hook>,
}

/// How bots share the width at a waypoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lane {
    /// Spread up to 1.5 m to either side.
    Default,
    Spread(f64),
    /// An exact line (narrow or timed sections).
    Exact,
}

impl Lane {
    fn width(self) -> f64 {
        match self {
            Lane::Default => 1.5,
            Lane::Spread(w) => w,
            Lane::Exact => 0.0,
        }
    }
}

pub type Detour = Box<dyn Fn(&mut BotView) -> Option<(f64, f64)> + Send + Sync>;
pub type Drive = Box<dyn Fn(&mut BotView, &mut BotInput) -> bool + Send + Sync>;
/// A test several waypoints share.
pub type SharedTest = Arc<dyn Fn(&mut BotView) -> bool + Send + Sync>;

impl Waypoint {
    pub fn at(x: f64, z: f64) -> Self {
        Self::new(WpX::At(x), z)
    }

    /// A fixed target, bots spread up to `w` to either side of it (0: an exact line).
    pub fn spread(x: f64, z: f64, w: f64) -> Self {
        let lane = if w == 0.0 { Lane::Exact } else { Lane::Spread(w) };
        Self { lane, ..Self::at(x, z) }
    }

    /// A fixed target all bots go to exactly.
    pub fn exact(x: f64, z: f64) -> Self {
        Self {
            lane: Lane::Exact,
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
            lane: Lane::Default,
            jump: false,
            jump_when: None,
            wait: None,
            speed: None,
            detour: None,
            drive: None,
            hook: None,
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

    pub fn hook(mut self, h: Hook) -> Self {
        self.hook = Some(h);
        self
    }
}
