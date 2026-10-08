//! What a map is: its build, the spec it returns (spawns, finish, checkpoints) and its `MapLogic`: the rules
//! it runs during a round (ticks, events, grabs, falls, touches, bots) and the state they keep.
use std::collections::BTreeMap;

pub use fb_shared::game::{ArenaKind, FallBehaviour, GameMeta, Genre, can_move, fall_behaviour};
use serde::{Deserialize, Serialize};

use crate::bots::{BotBrain, BotInput, BotView};
use crate::builder::Builder;
use crate::looks::LookId;
use crate::math::V3;
use crate::physics::{Body, StepEvents, Touch};
use crate::scene::LookOut;
use crate::world::{MoveCtx, World};

pub struct MapCtx<'a> {
    pub server: bool,
    pub seed: u32,
    pub participants: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    /// Reached once a bean stands past this z.
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

/// An authoritative map event: the server decides it, applies it at once and sends it; clients apply it at
/// its tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MapEvent {
    /// A portal pair was used from its end `from` at `t`: it shuts for a while.
    Portal { pair: u32, from: u32, t: f64 },
    /// An event of the course's section `seg`.
    Seg { seg: u32, ev: SegEvent },
    /// A floor tile starts to fall at `at`.
    Tile { i: u32, at: f64 },
    /// Bean `id` took star `k`.
    Star { k: u32, id: u32 },
    /// Bean `from` fell with `n` stars: they go to `to` (who knocked it down), or burn.
    Drop { from: u32, to: Option<u32>, n: i64 },
    /// Bean `to` snatched a star from `from`.
    Snatch { from: u32, to: u32 },
    /// Who has a tail now; `by` has just got one.
    Tails { ids: Vec<u32>, by: u32 },
}

/// An event of one section of a course.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SegEvent {
    /// A door breaks.
    Door(u32),
    /// A real pane of a hidden bridge has been stood on: it shows.
    Safe(u32),
    /// A fake pane falls from `at`.
    Fall { i: u32, at: f64 },
}

/// A sound a map plays on a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSfx {
    /// A panel or a door breaking.
    Break,
    Pickup,
    Steal,
}

/// Something map logic tells the room (server) or the view (client).
#[derive(Clone, Debug, PartialEq)]
pub enum MapOut {
    /// An authoritative map event; `keep`: sent again to whoever joins later (a portal trip is not).
    Event {
        ev: MapEvent,
        keep: bool,
    },
    Score {
        id: u32,
        v: i64,
    },
    /// Client: a sound to play.
    Sfx(MapSfx),
    /// Client: a bean's decoration (a tail, a badge by the name tag).
    Decorate {
        id: u32,
        change: DecoChange,
    },
}

/// A change a map makes to a bean's decorations (client).
#[derive(Clone, Debug, PartialEq)]
pub enum DecoChange {
    Tail(bool),
    /// Shown by the name tag.
    Badge(Option<String>),
}

/// Decorations a map has put on a bean (client).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeanDeco {
    pub tail: bool,
    pub badge: Option<String>,
}

impl BeanDeco {
    pub fn apply(&mut self, change: DecoChange) {
        match change {
            DecoChange::Tail(on) => self.tail = on,
            DecoChange::Badge(b) => self.badge = b,
        }
    }
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

/// One of the map's own looks or bot hooks (`Builder::hook`): what the logic draws or steers by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hook(pub u32);

/// What a hooked waypoint tells a bot (`MapLogic::steer`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Steer {
    /// Carry on along the path.
    Follow,
    /// Go there instead for now.
    Detour(f64, f64),
    /// The hook has set the bot's input itself.
    Drove,
}

/// A map's rules and the state they keep, run by the server's arena and by clients (all optional).
pub trait MapLogic: Send + Sync {
    /// Both sides: an authoritative event (the server applies it as it emits it, clients at its tick).
    fn event(&mut self, _cx: &mut Cx, _ev: &MapEvent) {}
    /// Server: per-tick game logic.
    fn tick(&mut self, _cx: &mut Cx, _t: f64) {}
    /// Both sides, after `tick`: what the map does to a bean in play each tick from its state alone (a tail
    /// slows its holder). A client predicts it for its own bean.
    fn bean(&self, _id: u32, _body: &mut Body, _t: f64) {}
    /// Server: `actor` grabbed `target` (grab button, target in reach).
    fn grab(&mut self, _cx: &mut Cx, _actor: u32, _target: u32) {}
    /// Server: `id` fell off during the round (and respawns); `by` is the player credited with it.
    fn fall(&mut self, _cx: &mut Cx, _id: u32, _by: Option<u32>) {}
    /// Client: one line of HUD text (e.g. "Ваши очки: 12").
    fn hud(&self, _cx: &Cx) -> Option<String> {
        None
    }
    /// Client: once the map is built (decorations).
    fn start(&mut self, _cx: &mut Cx) {}
    /// A watched collider (`Builder::watch_touch`, `watch_ground`) was touched or stood on, mid-step.
    fn touch(&mut self, _cx: &mut Cx, _body: &mut Body, _ev: &mut StepEvents, _tc: Touch) {}
    /// Moving parts that follow the map's state, after the world's own movers.
    fn pose(&self, _t: f64, _ctx: &mut MoveCtx) {}
    /// Client: places the pieces of one of the map's specials (`Builder::special_hook`) at sim time t.
    fn look(&self, _hook: Hook, _world: &World, _t: f64, _out: &mut LookOut) {}
    /// A hooked waypoint (`Waypoint::hook`) on a bot's path.
    fn steer(&self, _hook: Hook, _bot: &mut BotView, _out: &mut BotInput) -> Steer {
        Steer::Follow
    }
    /// Bots can play the map (`bot`).
    fn bots(&self) -> bool {
        false
    }
    fn bot(&self, _bot: &mut BotView, _out: &mut BotInput) {}
    /// Out-of-course places (on top of frames, behind walls): standing there counts as a shortcut.
    fn forbidden(&self, _p: V3) -> bool {
        false
    }

    /// Server: emits an authoritative event (`Cx::record`) and applies it right away.
    fn emit(&mut self, cx: &mut Cx, ev: MapEvent)
    where
        Self: Sized,
    {
        if cx.record(&ev) {
            cx.in_event = true;
            self.event(cx, &ev);
            cx.in_event = false;
        }
    }
}

/// A map without rules of its own.
pub struct NoLogic;

impl MapLogic for NoLogic {}

/// A map whose only rules are its bots' (a brain that keeps no state of the map's).
pub struct Brain(pub BotBrain);

impl MapLogic for Brain {
    fn bots(&self) -> bool {
        true
    }

    fn bot(&self, bot: &mut BotView, out: &mut BotInput) {
        (self.0)(bot, out);
    }
}

/// What map code can do besides building. On the server `MapLogic::emit` records an authoritative event,
/// applies it right away and has it sent; on clients it does nothing, the events come from the server.
pub struct Cx<'a> {
    pub server: bool,
    /// Server: an event recorded is applied at once (the arena; not a bare step of an audit).
    apply: bool,
    /// Current sim time in seconds (negative during the intro).
    pub t: f64,
    /// Client: id of the local player.
    pub me: Option<u32>,
    pub world: &'a mut World,
    pub bodies: &'a mut dyn Bodies,
    pub scores: &'a mut BTreeMap<u32, i64>,
    pub out: &'a mut Vec<MapOut>,
    /// An event handler is running (it does not emit).
    pub(crate) in_event: bool,
}

impl<'a> Cx<'a> {
    pub fn new(
        server: bool,
        apply: bool,
        t: f64,
        me: Option<u32>,
        world: &'a mut World,
        bodies: &'a mut dyn Bodies,
        scores: &'a mut BTreeMap<u32, i64>,
        out: &'a mut Vec<MapOut>,
    ) -> Self {
        Self {
            server,
            apply,
            t,
            me,
            world,
            bodies,
            scores,
            out,
            in_event: false,
        }
    }

    /// Server: records an authoritative event to be sent; true when the caller is to apply it now.
    pub fn record(&mut self, ev: &MapEvent) -> bool {
        if !self.server {
            return false;
        }
        // (Clients would apply a nested event with the handler, the server would not.)
        debug_assert!(!self.in_event, "an event handler emits {ev:?}");
        self.out.push(MapOut::Event {
            ev: ev.clone(),
            keep: true,
        });
        self.apply
    }

    pub fn score(&self, id: u32) -> i64 {
        self.scores.get(&id).copied().unwrap_or(0)
    }

    pub fn set_score(&mut self, id: u32, v: i64) {
        self.scores.insert(id, v);
        self.out.push(MapOut::Score { id, v });
    }

    pub fn sfx(&mut self, s: MapSfx) {
        if !self.server {
            self.out.push(MapOut::Sfx(s));
        }
    }

    pub fn decorate(&mut self, id: u32, change: DecoChange) {
        if !self.server {
            self.out.push(MapOut::Decorate { id, change });
        }
    }
}

pub struct MapSpec {
    pub spawns: Vec<V3>,
    pub kill_y: f64,
    pub checkpoints: Vec<Checkpoint>,
    pub finish: Option<Finish>,
    /// Point the camera looks at in arenas.
    pub view: Option<V3>,
    pub face_center: bool,
    pub logic: Box<dyn MapLogic>,
}

impl Default for MapSpec {
    fn default() -> Self {
        Self {
            spawns: Vec::new(),
            kill_y: 0.0,
            checkpoints: Vec::new(),
            finish: None,
            view: None,
            face_center: false,
            logic: Box::new(NoLogic),
        }
    }
}

pub trait MapDef: Sync {
    fn meta(&self) -> &'static GameMeta;
    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec;
    /// Looks of the map (`looks.rs`), its signature one first; none: the classic look.
    fn looks(&self) -> &'static [LookId] {
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
        out.push("kill_y is not a number");
    } else if spec.spawns.iter().any(|p| p.y <= spec.kill_y) {
        out.push("a spawn is below kill_y");
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
