//! What a map is (port of `sim/map.ts`): its build, the spec it returns (spawns, rules, bot brain) and
//! the logic it runs during a round (`tick`, events, grabs, falls, touches), through `Cx`.
use std::collections::BTreeMap;

pub use fb_shared::game::{ArenaKind, FallBehaviour, GameMeta, Genre, can_move, fall_behaviour};
pub use serde_json::{Value, json};

use crate::bots::BotBrain;
use crate::builder::Builder;
use crate::collider::ColId;
use crate::math::V3;
use crate::physics::{Body, StepEvents, Touch};
use crate::world::World;

pub struct MapCtx<'a> {
    pub server: bool,
    pub seed: u32,
    pub participants: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    /// Progress threshold (z unless the map defines `progress`).
    pub z: f64,
    pub p: V3,
}

/// Crossing z (above y − 2, within |x| ≤ half_width when given) finishes the race.
#[derive(Clone, Copy, Debug)]
pub struct Finish {
    pub z: f64,
    pub y: f64,
    pub half_width: Option<f64>,
}

pub type PosTest = Box<dyn Fn(V3) -> bool + Send + Sync>;

/// Something map logic tells the room (server) or the view (client).
#[derive(Clone, Debug, PartialEq)]
pub enum MapOut {
    /// An authoritative map event; `keep`: sent again to whoever joins later (a portal trip is not).
    Event {
        name: String,
        data: Value,
        keep: bool,
    },
    Score {
        id: u32,
        v: f64,
    },
    /// Client: a sound to play.
    Sfx(&'static str),
    /// Client: a bean's decoration (a tail, a badge by the name tag).
    Decorate {
        id: u32,
        deco: BeanDeco,
    },
}

/// Decorations a map puts on beans (client).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeanDeco {
    pub tail: Option<bool>,
    /// Shown by the name tag ("" for none).
    pub badge: Option<String>,
}

/// The beans in play (server: all of them, in the order they joined; client: the local one).
pub trait Bodies {
    fn ids(&self) -> Vec<u32>;
    fn get(&self, id: u32) -> Option<&Body>;
    fn get_mut(&mut self, id: u32) -> Option<&mut Body>;
}

/// No beans (map logic running inside a bean's own step).
pub struct NoBodies;

impl Bodies for NoBodies {
    fn ids(&self) -> Vec<u32> {
        Vec::new()
    }
    fn get(&self, _: u32) -> Option<&Body> {
        None
    }
    fn get_mut(&mut self, _: u32) -> Option<&mut Body> {
        None
    }
}

pub type OnEvent = Box<dyn FnMut(&mut Cx, &str, &Value) + Send + Sync>;
pub type OnTick = Box<dyn FnMut(&mut Cx, f64) + Send + Sync>;
pub type OnGrab = Box<dyn FnMut(&mut Cx, u32, u32) + Send + Sync>;
pub type OnFall = Box<dyn FnMut(&mut Cx, u32, Option<u32>) + Send + Sync>;
pub type OnHud = Box<dyn Fn(&Cx) -> Option<String> + Send + Sync>;
pub type OnStart = Box<dyn FnMut(&mut Cx) + Send + Sync>;
/// A collider's touch (or stand-on) handler, run inside the step of the body that touched it.
pub type TouchFn = Box<dyn FnMut(&mut Cx, &mut Body, &mut StepEvents, Touch) + Send + Sync>;

/// What map code can do besides building (TS `MapCtx` at run time). On the server `emit` records an
/// authoritative event, applies it right away (the spec's `on_event`) and has it sent; on clients it
/// does nothing, the events come from the server.
pub struct Cx<'a> {
    pub server: bool,
    /// Current sim time in seconds (negative during the intro).
    pub t: f64,
    /// Client: id of the local player.
    pub me: Option<u32>,
    pub world: &'a mut World,
    pub bodies: &'a mut dyn Bodies,
    pub scores: &'a mut BTreeMap<u32, f64>,
    pub out: &'a mut Vec<MapOut>,
    /// The spec's handler, for `emit` (taken while it runs: an event handler does not emit).
    pub on_event: Option<&'a mut OnEvent>,
    /// The spec's handler is running.
    pub in_event: bool,
}

impl Cx<'_> {
    pub fn emit(&mut self, name: &str, data: Value) {
        if !self.server {
            return;
        }
        // (Clients would apply a nested event with the handler, the server would not.)
        debug_assert!(!self.in_event, "an event handler emits {name}");
        self.out.push(MapOut::Event {
            name: name.to_string(),
            data: data.clone(),
            keep: true,
        });
        if let Some(h) = self.on_event.take() {
            self.in_event = true;
            h(self, name, &data);
            self.in_event = false;
            self.on_event = Some(h);
        }
    }

    pub fn score(&self, id: u32) -> f64 {
        self.scores.get(&id).copied().unwrap_or(0.0)
    }

    pub fn set_score(&mut self, id: u32, v: f64) {
        self.scores.insert(id, v);
        self.out.push(MapOut::Score { id, v });
    }

    pub fn sfx(&mut self, s: &'static str) {
        if !self.server {
            self.out.push(MapOut::Sfx(s));
        }
    }

    pub fn decorate(&mut self, id: u32, deco: BeanDeco) {
        if !self.server {
            self.out.push(MapOut::Decorate { id, deco });
        }
    }

    /// Client: the local player (−1 on the server, as TS `me()`).
    pub fn me(&self) -> i64 {
        self.me.map_or(-1, i64::from)
    }
}

/// Touch handlers of colliders, by collider (TS `onTouch` and `onGround`).
#[derive(Default)]
pub struct Touches {
    pub touch: BTreeMap<ColId, TouchFn>,
    pub ground: BTreeMap<ColId, TouchFn>,
}

impl Touches {
    pub fn handler(&mut self, t: Touch) -> Option<&mut TouchFn> {
        if t.normal.is_some() {
            self.touch.get_mut(&t.col)
        } else {
            self.ground.get_mut(&t.col)
        }
    }
}

#[derive(Default)]
pub struct MapSpec {
    pub spawns: Vec<V3>,
    pub kill_y: f64,
    pub is_out: Option<PosTest>,
    pub checkpoints: Vec<Checkpoint>,
    /// Progress along the course (default: z); checkpoint thresholds and race ranking use it.
    pub progress: Option<Box<dyn Fn(V3) -> f64 + Send + Sync>>,
    pub finish: Option<Finish>,
    /// Out-of-course places (on top of frames, behind walls): standing there counts as a shortcut.
    pub forbidden: Option<PosTest>,
    /// What a fall is blamed on when no hazard or player was involved (default "fall").
    pub fall_cause: Option<&'static str>,
    /// Point the camera looks at in arenas.
    pub view: Option<V3>,
    pub face_center: bool,
    /// Authoritative event (both sides apply it; the server first).
    pub on_event: Option<OnEvent>,
    /// Server: per-tick game logic.
    pub tick: Option<OnTick>,
    /// Server: `actor` grabbed `target` (grab button, target in reach).
    pub on_grab: Option<OnGrab>,
    /// Server: `id` fell off during the round (and respawns); `by` is the player credited with it.
    pub on_fall: Option<OnFall>,
    /// Client: one line of HUD text (e.g. "Ваши очки: 12").
    pub hud: Option<OnHud>,
    /// Client: once the map is built (what TS map code does at build time with the client's `ctx`:
    /// decorations).
    pub on_start: Option<OnStart>,
    pub bot: Option<BotBrain>,
    /// Filled from the builder by `build_map`.
    pub touches: Touches,
}

pub trait MapDef: Sync {
    fn meta(&self) -> &'static GameMeta;
    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec;
    /// Looks of the map (`looks.rs`), its signature one first; none: the classic look.
    fn looks(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Problems with a built map that would break a round.
pub fn spec_problems(spec: &MapSpec) -> Vec<&'static str> {
    let mut out = Vec::new();
    if spec.spawns.is_empty() {
        out.push("no spawns");
    }
    if spec.spawns.iter().any(|p| !p.is_finite()) {
        out.push("a spawn is not a finite position");
    }
    if !spec.kill_y.is_finite() {
        out.push("killY is not a number");
    } else if spec.spawns.iter().any(|p| p.y <= spec.kill_y) {
        out.push("a spawn is below killY");
    }
    if spec.checkpoints.iter().any(|c| !c.p.is_finite() || !c.z.is_finite()) {
        out.push("a checkpoint is not finite");
    }
    if spec
        .finish
        .is_some_and(|f| !(f.z + f.y).is_finite() || f.half_width.is_some_and(|w| !w.is_finite()))
    {
        out.push("the finish is not finite");
    }
    out
}
