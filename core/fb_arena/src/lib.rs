//! The authoritative arena: pawns (players and bots), bot brains,
//! grabs and tackles, finishes, checkpoints, falls and knockouts, bonuses, the map's own logic, the
//! debug journal and trace, and recording for replays. The network side (inputs from packets,
//! snapshots) stays in the server.
use std::collections::{BTreeMap, VecDeque};

use fb_shared::hash::Fnv;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_shared::m::MinMax;
use fb_shared::rng::Rng;
use fb_shared::{BOT_EVERY, DT, m};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::bots::{BotInput, BotMem, BotPlan, BotView, OtherView, smooth_stick};
use fb_sim::builder::Builder;
use fb_sim::map::{Bodies, Cx, MapCtx, MapDef, MapEvent, MapOut, MapSpec, NoBodies, Touches, spec_problems};
use fb_sim::math::V3;
use fb_sim::nav::{Nav, NavGrid};
use fb_sim::physics::{Body, BodyInput, BodyState, OtherBody, StepEvents, Touch};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;
use serde_json::{Value, json};

pub use fb_shared::game::{ArenaKind, FallBehaviour, can_move, fall_behaviour};
pub use fb_shared::rules::RoundStats;
pub use record::{Op, Recording, Replay, replay};

mod record;

/// How far a grab reaches (centre to centre; beans never get closer than BEAN_GAP).
const GRAB_REACH: f64 = 2.1;
/// Holding: rope length, how far it stretches before breaking, how long it lasts (s).
const HOLD_LEN: f64 = 1.35;
const HOLD_BREAK: f64 = 2.8;
const HOLD_MAX: f64 = 3.0;
/// Jumps a held bean needs to break free.
const STRUGGLE: u32 = 3;
const GRAB_COOLDOWN: f64 = 1.2;
/// Centre distance for a dive to tackle.
const DIVE_REACH: f64 = 1.6;
/// Bots' navigation grid is built this long (s) before the start, while nobody may move yet.
const NAV_PREBUILD: f64 = 1.5;
/// A hit (by a player or a hazard) counts for a fall this long after it (s).
const CREDIT_WINDOW: f64 = 3.0;
/// Time (s) a bean may stand somewhere forbidden before it counts as a shortcut.
const FORBIDDEN_GRACE: f64 = 0.6;
/// How far (m) a lobby spawn point must be from every bean to count as free.
const SPAWN_CLEAR: f64 = 2.5;
/// Trace: one entry every TRACE_EVERY ticks, the last TRACE_LEN entries per bean (30 s).
const TRACE_EVERY: i64 = 12;
const TRACE_LEN: usize = 300;
const JOURNAL_LEN: usize = 300;

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PawnStatus {
    Play,
    Finished,
    Out,
}

pub struct BotState {
    pub mem: BotMem,
    pub plan: BotPlan,
    pub rng: Rng,
    pub input: BotInput,
}

/// The last thing that hit a bean: another player (by) and/or a hazard (cause).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub by: Option<u32>,
    pub cause: &'static str,
    pub t: f64,
}

pub struct Pawn {
    pub id: u32,
    pub body: Body,
    pub ev: StepEvents,
    pub status: PawnStatus,
    pub bot: Option<BotState>,
    /// The frame used this tick (idle before the start).
    pub frame: InputFrame,
    pub spawn: V3,
    /// Index of `spawn` in the map's spawns.
    pub spawn_i: usize,
    /// Index into the map's checkpoints.
    pub checkpoint: Option<usize>,
    pub progress: f64,
    pub grabbing: Option<u32>,
    pub hold_since: f64,
    pub grab_ready_at: f64,
    /// Jumps while held (breaking free).
    pub struggle: u32,
    dive_hits: BTreeMap<u32, f64>,
    /// Respawns so far: views snap instead of smoothing when it changes.
    pub teleports: u32,
    pub stats: RoundStats,
    pub last_hit: Option<Hit>,
    /// Sim time of the last input with movement or buttons.
    pub active_at: f64,
    pub forbidden_for: f64,
    /// Dev: holds on as if the grab button were pressed until this sim time.
    pub force_grab_until: f64,
    /// Grab button held with nobody in hand (the arms reach out).
    pub reaching: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KoInfo {
    pub id: u32,
    /// Eliminated from the round (survival), not just respawned.
    pub out: bool,
    pub by: Option<u32>,
    pub cause: &'static str,
    pub shortcut: bool,
    /// Where the bean was when it fell.
    pub pos: V3,
}

/// Something that happened in a tick, for the room to pass on.
#[derive(Clone, Debug, PartialEq)]
pub enum ArenaEvent {
    Bonus(BonusTaken),
    Finish {
        id: u32,
        t: f64,
    },
    Ko(KoInfo),
    Emote {
        id: u32,
        e: u32,
    },
    /// A map event (`keep`: sent again to whoever joins later).
    Event {
        ev: MapEvent,
        keep: bool,
    },
    Score {
        id: u32,
        v: f64,
    },
}

/// Something that happened in the arena (debug journal).
#[derive(Clone, Debug, PartialEq)]
pub struct JournalEntry {
    pub t: f64,
    pub what: &'static str,
    pub id: Option<u32>,
    pub data: Option<Value>,
}

/// One line of a bean's recent history (debug tracer, 10 per second).
#[derive(Clone, Debug, PartialEq)]
pub struct TraceEntry {
    pub t: f64,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub state: BodyState,
    pub grounded: bool,
    /// Input used: move x/z (−127…127) and buttons (1 jump, 2 dive, 4 grab).
    pub input: [i32; 3],
    pub grabbing: Option<u32>,
    pub hazard: Option<&'static str>,
}

/// The beans in play, for map logic.
struct PawnBodies<'a>(&'a mut [Pawn]);

impl Bodies for PawnBodies<'_> {
    fn ids(&self) -> Vec<u32> {
        self.0
            .iter()
            .filter(|p| p.status == PawnStatus::Play)
            .map(|p| p.id)
            .collect()
    }

    fn get(&self, id: u32) -> Option<&Body> {
        self.0
            .iter()
            .find(|p| p.id == id && p.status == PawnStatus::Play)
            .map(|p| &p.body)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut Body> {
        self.0
            .iter_mut()
            .find(|p| p.id == id && p.status == PawnStatus::Play)
            .map(|p| &mut p.body)
    }
}

pub struct Arena {
    pub map: &'static dyn MapDef,
    pub kind: ArenaKind,
    pub seed: u32,
    pub world: World,
    pub spec: MapSpec,
    pub bonuses: Bonuses,
    pub participants: Vec<u32>,
    /// In the order they were added.
    pub pawns: Vec<Pawn>,
    pub fall: FallBehaviour,
    /// Arena tick: sim time is tick × DT, negative during the intro.
    pub tick: i64,
    pub static_hash: String,
    pub scores: BTreeMap<u32, f64>,
    /// Map events and bonuses taken so far that whoever joins later must hear of (debug: the room keeps them).
    pub kept_events: usize,
    /// Debug: recent history of every bean, and of the arena.
    pub trace: BTreeMap<u32, VecDeque<TraceEntry>>,
    pub journal: VecDeque<JournalEntry>,
    /// The round so far, for replays (None unless recording).
    pub recording: Option<Recording>,
    /// What map logic said during the current handler call.
    map_out: Vec<MapOut>,
    /// In finishing order.
    pub finished: Vec<u32>,
    /// In elimination order.
    pub out: Vec<u32>,
    /// Results are decided: no more finishes or outs are recorded.
    pub frozen: bool,
    /// Dev: bot brains run (false: bots stand still).
    pub bots_on: bool,
    spawn_cursor: usize,
    /// Bot navigation grid, built on first use once the round has started (gates are open).
    nav: Option<NavGrid>,
    /// The grid built ahead of the start, and the static world it was built for.
    nav_pre: Option<(NavGrid, String)>,
}

/// One body for `tick_bodies`.
pub struct Stepper<'a> {
    pub id: u32,
    pub body: &'a mut Body,
    pub ev: &'a mut StepEvents,
    pub input: BodyInput,
}

/// What a body's step does when it touches a collider with a handler: (world, id, body, events, touch).
pub type TouchHook<'a> = dyn FnMut(&mut World, u32, &mut Body, &mut StepEvents, Touch) + 'a;

/// For worlds without map logic.
pub fn no_touch(_: &mut World, _: u32, _: &mut Body, _: &mut StepEvents, _: Touch) {}

/// Runs a map's touch handlers for bodies stepped outside an arena (client prediction, tests): no
/// events are applied here (on a client they come from the server).
pub fn touch_hook<'a>(
    touches: &'a mut Touches,
    server: bool,
    t: f64,
    me: Option<u32>,
    scores: &'a mut BTreeMap<u32, f64>,
    out: &'a mut Vec<MapOut>,
) -> impl FnMut(&mut World, u32, &mut Body, &mut StepEvents, Touch) + 'a {
    move |world, _, body, ev, tc| {
        let Some(h) = touches.handler(tc) else { return };
        let mut cx = Cx {
            server,
            t,
            me,
            world,
            bodies: &mut NoBodies,
            scores: &mut *scores,
            out: &mut *out,
            on_event: None,
            in_event: false,
        };
        h(&mut cx, body, ev, tc);
    }
}

/// A map event from the server applied to a client's copy of the map: a portal
/// trip shuts its pair, the rest go to the map's handler. What the map says back (sounds, decorations)
/// lands in `out`.
pub fn client_event(
    world: &mut World,
    spec: &mut MapSpec,
    scores: &mut BTreeMap<u32, f64>,
    me: Option<u32>,
    ev: &MapEvent,
    out: &mut Vec<MapOut>,
) {
    if let &MapEvent::Portal { pair, from, t } = ev {
        if (pair as usize) < world.portals.len() {
            world.portal_used(pair as usize, from, t);
        }
        return;
    }
    let Some(h) = spec.on_event.as_mut() else { return };
    let mut cx = Cx {
        server: false,
        t: world.t,
        me,
        world,
        bodies: &mut NoBodies,
        scores,
        out,
        on_event: None,
        in_event: false,
    };
    h(&mut cx, ev);
}

/// A client's map once built: what the map sets up on a client (decorations).
pub fn client_start(
    world: &mut World,
    spec: &mut MapSpec,
    scores: &mut BTreeMap<u32, f64>,
    me: Option<u32>,
    out: &mut Vec<MapOut>,
) {
    let Some(h) = spec.on_start.as_mut() else { return };
    let mut cx = Cx {
        server: false,
        t: world.t,
        me,
        world,
        bodies: &mut NoBodies,
        scores,
        out,
        on_event: None,
        in_event: false,
    };
    h(&mut cx);
}

/// The map's line of HUD text for the local player.
pub fn client_hud(
    world: &mut World,
    spec: &MapSpec,
    scores: &mut BTreeMap<u32, f64>,
    me: Option<u32>,
) -> Option<String> {
    let h = spec.hud.as_ref()?;
    let mut out = Vec::new();
    let cx = Cx {
        server: false,
        t: world.t,
        me,
        world,
        bodies: &mut NoBodies,
        scores,
        out: &mut out,
        on_event: None,
        in_event: false,
    };
    h(&cx)
}

/// One tick of bodies in a world: the shared core of the server arena and client prediction.
/// `extra` are bodies that are not stepped here (on a client: the others, as drawn).
pub fn tick_bodies(world: &mut World, t: f64, bodies: &mut [Stepper], extra: &[OtherBody], touch: &mut TouchHook) {
    // Where a body stands on a moving platform is read in the world of the previous tick. The server's
    // world is always there; a client replaying a rollback finds it at its latest prediction instead.
    // (Compared with a tolerance: k·DT − DT and (k − 1)·DT may differ in the last bit.)
    if (world.t - (t - DT)).abs() > 1e-9 {
        world.goto(t - DT);
    }
    let mut carries = Vec::with_capacity(bodies.len());
    for s in bodies.iter_mut() {
        *s.ev = StepEvents::default();
        carries.push(s.body.before_world_update(world));
    }
    world.goto(t);
    for (s, c) in bodies.iter_mut().zip(&carries) {
        s.body.after_world_update(c, world);
    }
    let others: Vec<OtherBody> = bodies
        .iter()
        .filter(|s| !s.body.in_portal())
        .map(|s| other_of(s.id, s.body))
        .chain(extra.iter().copied())
        .collect();
    let mut rest = Vec::with_capacity(others.len());
    for s in bodies.iter_mut() {
        rest.clear();
        rest.extend(others.iter().copied().filter(|o| o.id != s.id));
        let id = s.id;
        s.body.step(s.ev, DT, s.input, world, t, &mut rest, &mut |w, b, e, tc| {
            touch(w, id, b, e, tc)
        });
    }
}

/// Which way a bean placed at `p` faces.
pub fn face_yaw(spec: &MapSpec, p: V3) -> f64 {
    if spec.face_center { m::atan2(-p.x, -p.z) } else { 0.0 }
}

/// Moves a standing bean's checkpoint on to the farthest one its progress has passed.
pub fn reach_checkpoint(spec: &MapSpec, body: &Body, checkpoint: &mut Option<usize>) {
    if !body.grounded {
        return;
    }
    let prog = spec.progress.as_ref().map_or(body.pos.z, |f| f(body.pos));
    for (ci, cp) in spec.checkpoints.iter().enumerate() {
        if prog >= cp.z && checkpoint.is_none_or(|c| cp.z > spec.checkpoints[c].z) {
            *checkpoint = Some(ci);
        }
    }
}

/// Below the kill line, or where the map says a bean is off the course.
pub fn fell(spec: &MapSpec, pos: V3) -> bool {
    pos.y < spec.kill_y || spec.is_out.as_ref().is_some_and(|f| f(pos))
}

/// Where a bean that fell (or took a shortcut) comes back: its checkpoint (races) or its spawn; None in the
/// lobby (a free spawn, which depends on where everybody stands).
pub fn respawn_point(
    spec: &MapSpec,
    kind: ArenaKind,
    fall: FallBehaviour,
    checkpoint: Option<usize>,
    spawn_i: usize,
) -> Option<V3> {
    match checkpoint {
        Some(c) if fall == FallBehaviour::Checkpoint => Some(spec.checkpoints[c].p),
        _ if kind == ArenaKind::Lobby => None,
        _ => spec.spawns.get(spawn_i).copied(),
    }
}

/// Puts a bean that fell back at `to`, a little aside by its id (beans respawning together do not stack).
pub fn respawn(spec: &MapSpec, id: u32, body: &mut Body, to: V3) {
    // In u64: client ids use all 32 bits.
    let jitter = ((id as u64 * 7919) % 100) as f64 / 100.0 - 0.5;
    body.reset(V3::new(to.x + jitter * 2.0, to.y + 0.5, to.z), face_yaw(spec, to));
}

pub fn other_of(id: u32, b: &Body) -> OtherBody {
    OtherBody {
        id,
        x: b.pos.x,
        y: b.pos.y,
        z: b.pos.z,
        vx: b.vel.x,
        vz: b.vel.z,
        touching: false,
        size: b.size,
    }
}

/// Builds a map's world the way server and clients both do.
pub fn build_map(map: &'static dyn MapDef, seed: u32, with_scene: bool, participants: &[u32]) -> (Builder, MapSpec) {
    let mut b = Builder::new(seed, with_scene);
    let ctx = MapCtx {
        server: !with_scene,
        seed,
        participants,
    };
    let mut spec = map.build(&mut b, &ctx);
    let bad = spec_problems(&spec);
    assert!(bad.is_empty(), "map {}: {}", map.meta().id, bad.join(", "));
    spec.touches = core::mem::take(&mut b.touches);
    (b, spec)
}

/// Two different pawns, both mutable.
fn pair(v: &mut [Pawn], i: usize, j: usize) -> (&mut Pawn, &mut Pawn) {
    assert_ne!(i, j);
    if i < j {
        let (a, b) = v.split_at_mut(j);
        (&mut a[i], &mut b[0])
    } else {
        let (a, b) = v.split_at_mut(i);
        (&mut b[0], &mut a[j])
    }
}

impl Arena {
    pub fn new(
        map: &'static dyn MapDef,
        kind: ArenaKind,
        seed: u32,
        tick: i64,
        participants: &[u32],
        with_scene: bool,
    ) -> (Self, Option<SceneDesc>) {
        let (mut b, spec) = build_map(map, seed, with_scene, participants);
        let meta = map.meta();
        let bonuses = if kind == ArenaKind::Round {
            Bonuses::new(&b.bonus_spots, seed, spec.finish.is_none(), meta.duration)
        } else {
            Bonuses::default()
        };
        b.world.finalize(tick as f64 * DT);
        let static_hash = b.world.hash(true);
        (
            Self {
                map,
                kind,
                seed,
                world: b.world,
                spec,
                bonuses,
                participants: participants.to_vec(),
                pawns: Vec::new(),
                fall: if kind == ArenaKind::Round {
                    fall_behaviour(meta.genre)
                } else {
                    FallBehaviour::Spawn
                },
                tick,
                static_hash,
                scores: BTreeMap::new(),
                kept_events: 0,
                trace: BTreeMap::new(),
                journal: VecDeque::new(),
                recording: None,
                map_out: Vec::new(),
                finished: Vec::new(),
                out: Vec::new(),
                frozen: false,
                bots_on: true,
                spawn_cursor: 0,
                nav: None,
                nav_pre: None,
            },
            b.scene,
        )
    }

    pub fn time(&self) -> f64 {
        self.tick as f64 * DT
    }

    fn face_yaw(&self, p: V3) -> f64 {
        face_yaw(&self.spec, p)
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.pawns.iter().position(|p| p.id == id)
    }

    pub fn pawn(&self, id: u32) -> Option<&Pawn> {
        self.pawns.iter().find(|p| p.id == id)
    }

    /// Adds a player or a bot at the next spawn (in the lobby: a free one).
    pub fn add_pawn(&mut self, id: u32, bot: bool) -> &mut Pawn {
        self.add_pawn_at(id, bot, None)
    }

    /// Adds a pawn at spawn point `spawn` (None: the next one, in the lobby a free one).
    pub fn add_pawn_at(&mut self, id: u32, bot: bool, spawn: Option<usize>) -> &mut Pawn {
        if let Some(i) = self.index(id) {
            return &mut self.pawns[i];
        }
        let i = match spawn {
            // (Moves the turn on as the other ways do: a replay adds pawns by spawn index, and a pawn
            // joining later must get the spawn it got live.)
            Some(i) => {
                self.spawn_cursor = i + 1;
                i
            }
            None if self.kind == ArenaKind::Lobby => self.free_spawn(),
            None => {
                self.spawn_cursor += 1;
                self.spawn_cursor - 1
            }
        };
        let tick = self.tick;
        if let Some(r) = &mut self.recording {
            r.pawns.push((id, bot, Some(i), tick));
        }
        let spawns = &self.spec.spawns;
        let spawn_i = i % spawns.len();
        let spawn = spawns[spawn_i];
        let mut body = Body::new(id as i32);
        body.reset(spawn, self.face_yaw(spawn));
        // By slot in the round, not id: the same seed plays out the same whoever joined when.
        let slot = self
            .participants
            .iter()
            .position(|&p| p == id)
            .map_or(id as u64, |k| k as u64 + 1);
        let bot = bot.then(|| BotState {
            mem: BotMem::default(),
            plan: BotPlan::default(),
            rng: Rng::new(self.seed ^ (slot.wrapping_mul(2_654_435_761) as u32)),
            input: BotInput::default(),
        });
        self.pawns.push(Pawn {
            id,
            body,
            ev: StepEvents::default(),
            status: PawnStatus::Play,
            bot,
            frame: InputFrame::IDLE,
            spawn,
            spawn_i,
            checkpoint: None,
            progress: spawn.z,
            grabbing: None,
            hold_since: 0.0,
            grab_ready_at: -1e9,
            struggle: 0,
            dive_hits: BTreeMap::new(),
            teleports: 0,
            stats: RoundStats::default(),
            last_hit: None,
            active_at: self.time().at_least(0.0),
            forbidden_for: 0.0,
            force_grab_until: -1e9,
            reaching: false,
        });
        self.pawns.last_mut().unwrap()
    }

    /// A spawn point nobody stands on: the next one in turn that is clear, else the one farthest
    /// from every bean.
    fn free_spawn(&mut self) -> usize {
        let spawns = &self.spec.spawns;
        let n = spawns.len();
        let mut best = self.spawn_cursor % n;
        let mut best_d = -1.0;
        for k in 0..n {
            let i = (self.spawn_cursor + k) % n;
            let s = spawns[i];
            let mut d = f64::INFINITY;
            for p in &self.pawns {
                if p.status == PawnStatus::Play {
                    d = d.at_most(p.body.pos.distance_squared(s));
                }
            }
            if d > SPAWN_CLEAR * SPAWN_CLEAR {
                best = i;
                break;
            }
            if d > best_d {
                best_d = d;
                best = i;
            }
        }
        self.spawn_cursor = best + 1;
        best
    }

    pub fn remove_pawn(&mut self, id: u32) {
        self.op(Op::Remove(id));
        self.pawns.retain(|p| p.id != id);
        // (Ids are never reused: in a lobby that stays up for days the traces of those who left would pile up.)
        self.trace.remove(&id);
    }

    /// Dev: bot brains run (false: bots stand still).
    pub fn set_bots_on(&mut self, on: bool) {
        self.op(Op::Bots(on));
        self.bots_on = on;
    }

    /// Records something that changes the simulation from outside, before the next tick.
    fn op(&mut self, op: Op) {
        let tick = self.tick;
        if let Some(r) = &mut self.recording {
            r.ops.push((tick, op));
        }
    }

    /// Starts recording the round for replays (before pawns are added).
    pub fn record(&mut self) {
        self.recording = Some(Recording {
            v: 2,
            game: self.map.meta().id.to_string(),
            kind: record::kind_name(self.kind).to_string(),
            seed: self.seed,
            tick0: self.tick,
            participants: self.participants.clone(),
            pawns: Vec::new(),
            frames: BTreeMap::new(),
            ops: Vec::new(),
            end_tick: 0,
            hash: String::new(),
        });
    }

    /// The recording so far, closed at the current tick.
    pub fn take_recording(&self) -> Option<Recording> {
        let mut r = self.recording.clone()?;
        r.end_tick = self.tick;
        r.hash = self.state_hash();
        Some(r)
    }

    /// Adds a line to the debug journal.
    pub fn note(&mut self, what: &'static str, id: Option<u32>, data: Option<Value>) {
        self.journal.push_back(JournalEntry {
            t: r3(self.time()),
            what,
            id,
            data,
        });
        if self.journal.len() > JOURNAL_LEN {
            self.journal.pop_front();
        }
    }

    /// What map logic said: into the tick's events (and the kept ones).
    fn flush(&mut self, events: &mut Vec<ArenaEvent>) {
        for o in self.map_out.drain(..) {
            match o {
                MapOut::Event { ev, keep } => {
                    self.kept_events += usize::from(keep);
                    events.push(ArenaEvent::Event { ev, keep });
                }
                MapOut::Score { id, v } => events.push(ArenaEvent::Score { id, v }),
                MapOut::Sfx(_) | MapOut::Decorate { .. } => {}
            }
        }
    }

    /// Runs tick k. Players' frames come from `frame` (the room's input buffers); bots think here.
    pub fn step(&mut self, k: i64, frame: impl Fn(u32) -> InputFrame) -> Vec<ArenaEvent> {
        self.tick = k;
        let t = k as f64 * DT;
        let mut events = Vec::new();
        let moving = can_move(self.kind, t);
        let round = self.kind == ArenaKind::Round;
        let mut active = Vec::with_capacity(self.pawns.len());
        for i in 0..self.pawns.len() {
            if self.pawns[i].status != PawnStatus::Play {
                continue;
            }
            active.push(i);
            let got = if self.pawns[i].bot.is_some() {
                self.bot_frame(i, k, &mut events)
            } else {
                frame(self.pawns[i].id)
            };
            let p = &mut self.pawns[i];
            // Before the start (and on the podium) nobody moves.
            p.frame = if moving { got } else { InputFrame::IDLE };
            if round && t >= 0.0 {
                if p.frame != InputFrame::IDLE {
                    p.active_at = t;
                }
                p.stats.idle = p.stats.idle.at_least(t - p.active_at);
            }
            if let Some(r) = &mut self.recording
                && p.bot.is_none()
            {
                r.record_frame(p.id, k, p.frame);
            }
        }

        {
            let mut steppers: Vec<Stepper> = Vec::with_capacity(active.len());
            let mut it = active.iter().peekable();
            for (i, p) in self.pawns.iter_mut().enumerate() {
                if it.peek() == Some(&&i) {
                    it.next();
                    steppers.push(Stepper {
                        id: p.id,
                        input: p.frame.into(),
                        body: &mut p.body,
                        ev: &mut p.ev,
                    });
                }
            }
            let MapSpec { touches, on_event, .. } = &mut self.spec;
            let (scores, out) = (&mut self.scores, &mut self.map_out);
            let mut touch = |world: &mut World, _: u32, body: &mut Body, ev: &mut StepEvents, tc: Touch| {
                let Some(h) = touches.handler(tc) else { return };
                let mut cx = Cx {
                    server: true,
                    t,
                    me: None,
                    world,
                    bodies: &mut NoBodies,
                    scores,
                    out,
                    on_event: on_event.as_mut(),
                    in_event: false,
                };
                h(&mut cx, body, ev, tc);
            };
            tick_bodies(&mut self.world, t, &mut steppers, &[], &mut touch);
        }
        self.flush(&mut events);
        for &i in &active {
            let p = &mut self.pawns[i];
            if let Some(cause) = p.ev.hazard {
                let by = p.last_hit.filter(|h| t - h.t < CREDIT_WINDOW).and_then(|h| h.by);
                p.last_hit = Some(Hit { by, cause, t });
            }
        }
        for &i in &active {
            self.interact(i, &active, t, &mut events);
        }
        if round && t >= 0.0 && !self.frozen {
            let mut bodies: Vec<(u32, &mut Body)> = Vec::with_capacity(active.len());
            let mut it = active.iter().peekable();
            for (i, p) in self.pawns.iter_mut().enumerate() {
                if it.peek() == Some(&&i) {
                    it.next();
                    bodies.push((p.id, &mut p.body));
                }
            }
            for b in self.bonuses.check(t, &mut bodies) {
                self.note("bonus", None, Some(json!({ "i": b.i, "id": b.id, "at": b.at })));
                self.kept_events += 1;
                events.push(ArenaEvent::Bonus(b));
            }
        }
        for &i in &active {
            self.rules(i, t, &mut events);
        }
        if k % TRACE_EVERY == 0 {
            for &i in &active {
                self.trace_pawn(i, t);
            }
        }
        self.map_tick(t, &mut events);
        self.prebuild_nav();
        events
    }

    /// Runs `f` with the map's context (the server's: every bean in play).
    fn with_cx<R>(&mut self, t: f64, f: impl FnOnce(&mut MapSpec, &mut Cx) -> R) -> R {
        let mut bodies = PawnBodies(&mut self.pawns);
        let spec = &mut self.spec;
        let mut on_event = spec.on_event.take();
        let mut cx = Cx {
            server: true,
            t,
            me: None,
            world: &mut self.world,
            bodies: &mut bodies,
            scores: &mut self.scores,
            out: &mut self.map_out,
            on_event: on_event.as_mut(),
            in_event: false,
        };
        let r = f(spec, &mut cx);
        spec.on_event = on_event;
        r
    }

    fn map_tick(&mut self, t: f64, events: &mut Vec<ArenaEvent>) {
        if self.spec.tick.is_none() {
            return;
        }
        self.with_cx(t, |spec, cx| {
            if let Some(f) = spec.tick.as_mut() {
                f(cx, t);
            }
        });
        self.flush(events);
    }

    fn trace_pawn(&mut self, i: usize, t: f64) {
        let p = &self.pawns[i];
        let b = &p.body;
        let e = TraceEntry {
            t: r3(t),
            pos: [r3(b.pos.x), r3(b.pos.y), r3(b.pos.z)],
            vel: [r3(b.vel.x), r3(b.vel.y), r3(b.vel.z)],
            state: b.state,
            grounded: b.grounded,
            input: [i32::from(p.frame.mx), i32::from(p.frame.mz), i32::from(p.frame.buttons)],
            grabbing: p.grabbing,
            hazard: p.ev.hazard,
        };
        let list = self.trace.entry(p.id).or_default();
        list.push_back(e);
        if list.len() > TRACE_LEN {
            list.pop_front();
        }
    }

    fn bot_frame(&mut self, i: usize, k: i64, events: &mut Vec<ArenaEvent>) -> InputFrame {
        let fresh = k % BOT_EVERY as i64 == 0;
        if fresh {
            self.think(i, events);
        }
        let b = self.pawns[i].bot.as_ref().unwrap().input;
        let axis = |v: f64| (v.clamp(-1.0, 1.0) * 127.0).round();
        let (mx, mz) = (axis(b.mx), axis(b.mz));
        InputFrame {
            mx: mx as i8,
            mz: mz as i8,
            buttons: (if fresh && b.jump { BTN_JUMP } else { 0 })
                | (if fresh && b.dive { BTN_DIVE } else { 0 })
                | (if b.grab { BTN_GRAB } else { 0 }),
        }
    }

    fn build_nav(&self) -> NavGrid {
        NavGrid::build(
            &self.world,
            self.spec.forbidden.as_ref().map(|f| f.as_ref() as &dyn Fn(V3) -> bool),
        )
    }

    /// Builds the bots' navigation grid now, ahead of the round (a server room does it off its tick).
    pub fn prepare_nav(&mut self) {
        if self.nav.is_none() && self.nav_pre.is_none() && self.kind == ArenaKind::Round && self.spec.bot.is_some() {
            self.nav_pre = Some((self.build_nav(), self.world.hash(true)));
        }
    }

    /// The navigation grid takes a while to build: it is built before the start, and used at the
    /// start if the static world has not changed.
    fn prebuild_nav(&mut self) {
        if self.nav.is_some() || self.nav_pre.is_some() || self.kind != ArenaKind::Round || self.spec.bot.is_none() {
            return;
        }
        let t = self.time();
        if !(-NAV_PREBUILD..0.0).contains(&t) {
            return;
        }
        if !self.pawns.iter().any(|p| p.bot.is_some()) {
            return;
        }
        self.nav_pre = Some((self.build_nav(), self.world.hash(true)));
    }

    fn think(&mut self, i: usize, events: &mut Vec<ArenaEvent>) {
        let t = self.time();
        self.pawns[i].bot.as_mut().unwrap().input = BotInput::default();
        if self.spec.bot.is_none() || !self.bots_on {
            return;
        }
        if self.nav.is_none() && (t >= 0.0 || self.kind != ArenaKind::Round) {
            // Built ahead during the intro: the same grid if the static world is the same.
            let pre = self.nav_pre.take();
            self.nav = Some(match pre {
                Some((nav, hash)) if hash == self.world.hash(true) => nav,
                _ => self.build_nav(),
            });
        }
        let others: Vec<OtherView> = self
            .pawns
            .iter()
            .enumerate()
            .filter(|(j, o)| *j != i && o.status == PawnStatus::Play)
            .map(|(_, o)| {
                let ob = &o.body;
                let dive =
                    ob.state == BodyState::Dive || (ob.state == BodyState::Slide && m::hypot(ob.vel.x, ob.vel.z) > 6.0);
                OtherView {
                    id: o.id,
                    pos: ob.pos,
                    vel: ob.vel,
                    down: ob.down(),
                    dive,
                    reach: o.reaching,
                }
            })
            .collect();
        let bonuses: Vec<V3> = self.bonuses.available(t).map(|b| b.pos).collect();
        let Arena {
            pawns,
            world,
            nav,
            spec,
            scores,
            ..
        } = self;
        let p = &mut pawns[i];
        let st = p.bot.as_mut().unwrap();
        let mut out = BotInput::default();
        let mut view = BotView {
            id: p.id,
            body: &p.body,
            t,
            rng: &mut st.rng,
            mem: &mut st.mem,
            plan: &mut st.plan,
            others: &others,
            nav: nav.as_ref().map(|g| Nav::new(g, world)),
            bonuses: &bonuses,
            world,
            scores,
        };
        (spec.bot.as_ref().unwrap())(&mut view, &mut out);
        smooth_stick(&mut st.mem, &mut out);
        st.input = out;
        if out.emote != 0 {
            events.push(ArenaEvent::Emote { id: p.id, e: out.emote });
        }
    }

    /// Grabbing holds on (pulling the other bean along) until released, broken free or timed out;
    /// dives knock over.
    fn interact(&mut self, i: usize, active: &[usize], t: f64, events: &mut Vec<ArenaEvent>) {
        let round = self.kind == ArenaKind::Round;
        let f = self.pawns[i].frame;
        let frame_of = |pawns: &[Pawn], j: usize| active.contains(&j).then(|| pawns[j].frame);
        let p = &mut self.pawns[i];
        let wants = ((f.buttons & BTN_GRAB) != 0 || t < p.force_grab_until) && p.body.state == BodyState::Normal;
        p.reaching = wants && p.grabbing.is_none();
        if let Some(gid) = p.grabbing {
            let oi = self.index(gid);
            let p = &self.pawns[i];
            let (dist, escaped, playing) = match oi {
                Some(oi) => {
                    let o = &self.pawns[oi];
                    let ob = &o.body;
                    (
                        m::hypot(ob.pos.x - p.body.pos.x, ob.pos.z - p.body.pos.z),
                        o.struggle >= STRUGGLE || ob.state == BodyState::Dive || ob.down() || ob.in_portal(),
                        o.status == PawnStatus::Play,
                    )
                }
                None => (99.0, false, false),
            };
            let timed_out = t - p.hold_since > HOLD_MAX;
            match oi {
                Some(oi) if wants && playing && !escaped && !timed_out && dist <= HOLD_BREAK => {
                    let of = frame_of(&self.pawns, oi);
                    self.hold(i, oi, dist, t, of);
                }
                _ => {
                    let p = &mut self.pawns[i];
                    p.grabbing = None;
                    p.grab_ready_at = t + if escaped || timed_out { GRAB_COOLDOWN } else { 0.3 };
                    if let Some(oi) = oi {
                        self.pawns[oi].struggle = 0;
                    }
                }
            }
        } else if wants && t >= p.grab_ready_at {
            let b = &self.pawns[i].body;
            let fx = m::sin(b.yaw);
            let fz = m::cos(b.yaw);
            let mut best = f64::INFINITY;
            let mut target = None;
            for &oj in active {
                let ob = &self.pawns[oj].body;
                if oj == i || ob.down() || ob.in_portal() {
                    continue;
                }
                let dx = ob.pos.x - b.pos.x;
                let dz = ob.pos.z - b.pos.z;
                let d = m::hypot(dx, dz);
                // Reach grows with the size of either bean (giants have long arms, and are big targets).
                let size = b.size.at_least(ob.size);
                if d > GRAB_REACH * size || (ob.pos.y - b.pos.y).abs() > 1.6 * size {
                    continue;
                }
                // Anything in front, or right beside (turning to it): forgiving, as grabbing should feel.
                let facing = if d > 0.3 { (dx * fx + dz * fz) / d } else { 1.0 };
                if facing < (if d < 1.5 { -0.35 } else { 0.1 }) {
                    continue;
                }
                // Prefer what is ahead over what is merely close.
                let score = d - facing * 0.6;
                if score > best {
                    continue;
                }
                best = score;
                target = Some(oj);
            }
            if let Some(oj) = target {
                let (pid, tid) = (self.pawns[i].id, self.pawns[oj].id);
                self.note("grab", Some(pid), Some(json!({ "target": tid })));
                let p = &mut self.pawns[i];
                p.grabbing = Some(tid);
                p.hold_since = t;
                self.pawns[oj].struggle = 0;
                if round {
                    self.pawns[i].stats.grabs += 1;
                }
                if self.spec.on_grab.is_some() {
                    self.with_cx(t, |spec, cx| {
                        if let Some(f) = spec.on_grab.as_mut() {
                            f(cx, pid, tid);
                        }
                    });
                    self.flush(events);
                }
                self.pawns[i].reaching = false;
                let (b, ob) = (&self.pawns[i].body, &self.pawns[oj].body);
                let dist = m::hypot(ob.pos.x - b.pos.x, ob.pos.z - b.pos.z);
                let of = frame_of(&self.pawns, oj);
                self.hold(i, oj, dist, t, of);
            }
        }

        let b = &self.pawns[i].body;
        if b.state == BodyState::Dive || (b.state == BodyState::Slide && m::hypot(b.vel.x, b.vel.z) > 6.0) {
            let fx = m::sin(b.yaw);
            let fz = m::cos(b.yaw);
            let pid = self.pawns[i].id;
            for &oj in active {
                if oj == i {
                    continue;
                }
                let (p, o) = pair(&mut self.pawns, i, oj);
                // Anyone can be hit, a bean already down or getting up too (each diver once per dive).
                if p.dive_hits.get(&o.id).copied().unwrap_or(-1.0) > t {
                    continue;
                }
                let b = &mut p.body;
                let dx = o.body.pos.x - b.pos.x;
                let dz = o.body.pos.z - b.pos.z;
                let d = m::hypot(dx, dz);
                if d > DIVE_REACH || (o.body.pos.y - b.pos.y).abs() > 1.3 {
                    continue;
                }
                // Only what is ahead of the diver (or right on top of it).
                if d > 0.4 && (dx * fx + dz * fz) / d < 0.25 {
                    continue;
                }
                let nx = if d > 1e-3 { dx / d } else { fx };
                let nz = if d > 1e-3 { dz / d } else { fz };
                let sp = m::hypot(b.vel.x, b.vel.z);
                let k = 5.0 + (sp * 0.4).at_most(5.0);
                o.body
                    .knock(&mut o.ev, nx * k + fx * 2.0, nz * k + fz * 2.0, 4.5, 0.9, false);
                o.last_hit = Some(Hit {
                    by: Some(pid),
                    cause: "tackle",
                    t,
                });
                // The tackler spends its momentum on the hit.
                b.vel.x *= 0.45;
                b.vel.z *= 0.45;
                p.dive_hits.insert(o.id, t + 0.6);
                if round {
                    p.stats.tackles += 1;
                }
            }
        }
    }

    /// One tick of holding: both slow down, the held bean is pulled back within reach; jumping
    /// struggles free.
    fn hold(&mut self, i: usize, oi: usize, dist: f64, t: f64, of: Option<InputFrame>) {
        let (p, o) = pair(&mut self.pawns, i, oi);
        let b = &mut p.body;
        let ob = &mut o.body;
        // A giant held by a normal bean is barely slowed (and not dragged much).
        let heavy = ob.mass() / b.mass();
        ob.slow_until = t + 0.15;
        ob.slow_k = if heavy > 1.0 { 0.85 } else { 0.5 };
        b.slow_until = t + 0.15;
        b.slow_k = 0.7;
        o.last_hit = Some(Hit {
            by: Some(p.id),
            cause: "grab",
            t,
        });
        if of.is_some_and(|f| f.buttons & BTN_JUMP != 0) {
            o.struggle += 1;
        }
        if dist > HOLD_LEN {
            let dx = (ob.pos.x - b.pos.x) / dist;
            let dz = (ob.pos.z - b.pos.z) / dist;
            // Spring back towards the grabber; the held bean cannot outrun the hand.
            let away = ob.vel.x * dx + ob.vel.z * dz;
            if away > 0.0 {
                ob.vel.x -= (dx * away * 0.6) / heavy;
                ob.vel.z -= (dz * away * 0.6) / heavy;
            }
            let k = ((dist - HOLD_LEN) * 0.5).at_most(1.0) / heavy;
            ob.pos.x -= dx * k * 0.1;
            ob.pos.z -= dz * k * 0.1;
        }
        // Face what we hold.
        b.yaw = m::atan2(ob.pos.x - b.pos.x, ob.pos.z - b.pos.z);
    }

    fn rules(&mut self, i: usize, t: f64, events: &mut Vec<ArenaEvent>) {
        let round = self.kind == ArenaKind::Round;
        let spec = &self.spec;
        let p = &mut self.pawns[i];
        let pos = p.body.pos;
        let prog = spec.progress.as_ref().map_or(pos.z, |f| f(pos));
        if t >= 0.0 {
            p.progress = p.progress.at_least(prog);
        }
        if let Some(fin) = spec.finish
            && round
            && t >= 0.0
            && pos.z >= fin.z
            && pos.y > fin.y - 2.0
            && fin.half_width.is_none_or(|hw| pos.x.abs() <= hw)
        {
            if !self.frozen {
                p.status = PawnStatus::Finished;
                p.stats.finish_at = Some(t);
                let id = p.id;
                self.note("finish", Some(id), None);
                self.finished.push(id);
                events.push(ArenaEvent::Finish { id, t });
            }
            return;
        }
        reach_checkpoint(spec, &p.body, &mut p.checkpoint);
        // Standing where the course does not go (on frames, behind walls): back to the checkpoint,
        // fined. Only while standing there: being knocked off over a rail is a fall, not a shortcut.
        if p.body.grounded {
            p.forbidden_for = if round && t >= 0.0 && spec.forbidden.as_ref().is_some_and(|f| f(pos)) {
                p.forbidden_for + DT
            } else {
                0.0
            };
        }
        let shortcut = p.forbidden_for > FORBIDDEN_GRACE;
        let fell = shortcut || fell(spec, pos);
        if !fell {
            return;
        }
        p.forbidden_for = 0.0;
        let hit = p.last_hit.filter(|h| t - h.t < CREDIT_WINDOW);
        p.last_hit = None;
        let by = if shortcut { None } else { hit.and_then(|h| h.by) };
        let cause = if shortcut {
            "shortcut"
        } else {
            hit.map_or(spec.fall_cause.unwrap_or("fall"), |h| h.cause)
        };
        let id = p.id;
        let counts = round && t >= 0.0 && !self.frozen;
        if counts {
            if let Some(by) = by
                && by != id
                && let Some(a) = self.index(by)
            {
                self.pawns[a].stats.kos += 1;
            }
            if shortcut {
                self.pawns[i].stats.shortcuts += 1;
            }
        }
        let ko = |out| KoInfo {
            id,
            out,
            by,
            cause,
            shortcut,
            pos,
        };
        let what = json!({ "by": by, "cause": cause, "pos": [r3(pos.x), r3(pos.y), r3(pos.z)] });
        if self.fall == FallBehaviour::Out && !shortcut && round {
            if self.frozen {
                return;
            }
            let p = &mut self.pawns[i];
            p.status = PawnStatus::Out;
            p.stats.out_at = Some(t.at_least(0.0));
            self.note("out", Some(id), Some(what));
            self.out.push(id);
            events.push(ArenaEvent::Ko(ko(true)));
            return;
        }
        if counts && !shortcut {
            self.pawns[i].stats.falls += 1;
        }
        self.note(if shortcut { "shortcut" } else { "fall" }, Some(id), Some(what));
        if counts || self.kind == ArenaKind::Lobby {
            events.push(ArenaEvent::Ko(ko(false)));
        }
        if counts && !shortcut && self.spec.on_fall.is_some() {
            self.with_cx(t, |spec, cx| {
                if let Some(f) = spec.on_fall.as_mut() {
                    f(cx, id, by);
                }
            });
            self.flush(events);
        }
        let p = &self.pawns[i];
        let to = match respawn_point(&self.spec, self.kind, self.fall, p.checkpoint, p.spawn_i) {
            Some(to) => to,
            None => {
                let s = self.free_spawn();
                self.spec.spawns[s]
            }
        };
        let p = &mut self.pawns[i];
        respawn(&self.spec, p.id, &mut p.body, to);
        p.teleports += 1;
    }

    /// Hash of every bean's state (replays and determinism checks compare it).
    pub fn state_hash(&self) -> String {
        let mut h = Fnv::default();
        let mut mix = |v: f64| h.mix(v, 1e5);
        let mut sorted: Vec<&Pawn> = self.pawns.iter().collect();
        sorted.sort_by_key(|p| p.id);
        for p in sorted {
            mix(p.id as f64);
            let b = &p.body;
            for v in [b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.yaw, b.tilt] {
                mix(v);
            }
        }
        format!("{:016x}", h.finish())
    }
}

// ------------------------------------------------------------------ dev tools (server --dev only)

impl Arena {
    /// Adds a pawn to a running arena (a bot joining mid-round), at `at` or the next spawn.
    pub fn add_late_pawn(&mut self, id: u32, bot: bool, at: Option<V3>) {
        if self.index(id).is_some() {
            return;
        }
        self.op(Op::Late {
            id,
            bot,
            at: at.map(|p| [p.x, p.y, p.z]),
        });
        // The op adds it again on replay: keep it out of the list of pawns added at the start.
        let n = self.recording.as_ref().map_or(0, |r| r.pawns.len());
        if !self.participants.contains(&id) {
            self.participants.push(id);
        }
        self.add_pawn(id, bot);
        if let Some(r) = &mut self.recording
            && r.pawns.len() > n
        {
            r.pawns.pop();
        }
        if let Some(p) = at {
            self.place(id, p, None);
        }
    }

    fn place(&mut self, id: u32, pos: V3, yaw: Option<f64>) -> bool {
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        let yaw = yaw.unwrap_or(p.body.yaw);
        p.body.reset(pos, yaw);
        p.teleports += 1;
        p.forbidden_for = 0.0;
        true
    }

    pub fn dev_teleport(&mut self, id: u32, pos: V3, yaw: Option<f64>) -> bool {
        self.op(Op::Teleport {
            id,
            pos: [pos.x, pos.y, pos.z],
            yaw,
        });
        self.place(id, pos, yaw)
    }

    /// Where `goto` sends a bean: its spawn, a checkpoint, or 3 m before the finish line.
    pub fn dev_place(&self, id: u32, to: DevPlace) -> Option<V3> {
        let p = self.pawn(id)?;
        match to {
            DevPlace::Spawn => Some(p.spawn),
            DevPlace::Finish => self.spec.finish.map(|f| V3::new(0.0, f.y + 1.5, f.z - 3.0)),
            DevPlace::Checkpoint(i) => self.spec.checkpoints.get(i).map(|c| V3::new(c.p.x, c.p.y + 0.5, c.p.z)),
        }
    }

    pub fn dev_knock(&mut self, id: u32, v: [f64; 3]) -> bool {
        self.op(Op::Knock { id, v });
        let t = self.time();
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        p.body.knock(&mut p.ev, v[0], v[2], v[1], 1.0, false);
        p.last_hit = Some(Hit {
            by: None,
            cause: "dev",
            t,
        });
        true
    }

    /// Drops a bean below the kill height: it falls by the map's rules on the next tick.
    pub fn dev_kill(&mut self, id: u32) -> bool {
        self.op(Op::Kill(id));
        let kill_y = self.spec.kill_y;
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        p.body.pos.y = kill_y - 1.0;
        true
    }

    pub fn dev_grab(&mut self, actor: u32, target: u32, seconds: f64) -> Result<(), &'static str> {
        self.op(Op::Grab { actor, target, seconds });
        let (Some(a), Some(o)) = (self.index(actor), self.index(target)) else {
            return Err("no such beans in play");
        };
        if a == o || self.pawns[a].status != PawnStatus::Play || self.pawns[o].status != PawnStatus::Play {
            return Err("no such beans in play");
        }
        let t = self.time();
        let p = &mut self.pawns[a];
        p.force_grab_until = t + seconds;
        p.grabbing = Some(target);
        p.hold_since = t;
        self.pawns[o].struggle = 0;
        Ok(())
    }

    /// Replays: applies a recorded operation.
    pub fn apply_op(&mut self, op: &Op) {
        let v3 = |a: [f64; 3]| V3::new(a[0], a[1], a[2]);
        match *op {
            Op::Remove(id) => self.remove_pawn(id),
            Op::Late { id, bot, at } => self.add_late_pawn(id, bot, at.map(v3)),
            Op::Teleport { id, pos, yaw } => {
                self.dev_teleport(id, v3(pos), yaw);
            }
            Op::Knock { id, v } => {
                self.dev_knock(id, v);
            }
            Op::Kill(id) => {
                self.dev_kill(id);
            }
            Op::Grab { actor, target, seconds } => {
                let _ = self.dev_grab(actor, target, seconds);
            }
            Op::Bots(on) => self.set_bots_on(on),
        }
    }
}

/// Where a dev `goto` sends a bean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevPlace {
    Spawn,
    Finish,
    Checkpoint(usize),
}
