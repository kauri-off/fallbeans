//! The authoritative arena: the round's rules and state, without any network code (that stays in the server).
#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value
)]
use std::collections::{BTreeMap, VecDeque};

use fb_shared::NEVER;
use fb_shared::cause::{Cause, Hazard};
use fb_shared::hash::Fnv;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_shared::m::MinMax;
use fb_shared::rng::Rng;
use fb_shared::{BOT_EVERY, DT, PlayerId, m};
use fb_sim::beans::{self, Other, Side};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::bots::{BotInput, BotMem, BotPlan, BotView, OtherView, smooth_stick};
use fb_sim::builder::{Builder, enter_gate};
use fb_sim::map::{
    Bodies, Cx, MapCtx, MapDef, MapEvent, MapLogic, MapOut, MapSpec, NoBodies, NoLogic, Role, spec_problems,
};
use fb_sim::math::{V3, dist_xz};
use fb_sim::nav::{Nav, NavGrid};
use fb_sim::physics::{Body, BodyInput, BodyState, Carry, OtherBody, StepEvents, StepScratch, Touch};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;
use serde_json::{Value, json};

pub use fb_shared::game::{ArenaKind, FallBehaviour, MapId, can_move, fall_behaviour};
pub use fb_shared::hash::{Fingerprint, StateHash};
pub use fb_shared::rules::RoundStats;

mod dev;
mod hash;
mod pawns;
mod record;
mod tick;

pub use dev::DevPlace;
pub use record::{FrameRec, Op, PawnRec, Recording, Replay, replay};
pub use tick::*;

/// How far a grab reaches (centre to centre; beans never get closer than BEAN_GAP).
const GRAB_REACH: f64 = 2.1;
/// Holding: rope length, how far it stretches before breaking, how long it lasts (s).
const HOLD_LEN: f64 = 1.35;
const HOLD_BREAK: f64 = 2.8;
const HOLD_MAX: f64 = 3.0;
/// Jumps a held bean needs to break free.
const STRUGGLE: u32 = 3;
const GRAB_COOLDOWN: f64 = 1.2;
/// The farthest back (ticks) a tackle is judged by where its player saw the others.
pub const MAX_VIEW: u32 = 30;
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
    pub by: Option<PlayerId>,
    pub cause: Cause,
    pub t: f64,
}

pub struct Pawn {
    pub id: PlayerId,
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
    pub grabbing: Option<PlayerId>,
    pub hold_since: f64,
    pub grab_ready_at: f64,
    /// Jumps while held (breaking free).
    pub struggle: u32,
    /// Respawns so far: views snap instead of smoothing when it changes.
    pub teleports: u32,
    pub stats: RoundStats,
    pub last_hit: Option<Hit>,
    pub forbidden_for: f64,
    /// Dev: holds on as if the grab button were pressed until this sim time.
    pub force_grab_until: f64,
    /// Grab button held with nobody in hand (the arms reach out).
    pub reaching: bool,
    /// How many ticks behind its own bean the player sees the others (lag compensation of its tackles).
    pub view: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KoInfo {
    pub id: PlayerId,
    /// Eliminated from the round (survival), not just respawned.
    pub out: bool,
    pub by: Option<PlayerId>,
    pub cause: Cause,
    pub shortcut: bool,
    /// Where the bean was when it fell.
    pub pos: V3,
}

/// Something that happened in a tick, for the room to pass on.
#[derive(Clone, Debug, PartialEq)]
pub enum ArenaEvent {
    Bonus(BonusTaken),
    Finish {
        id: PlayerId,
        t: f64,
    },
    Ko(KoInfo),
    Emote {
        id: PlayerId,
        e: u32,
    },
    /// A map event (`keep`: sent again to whoever joins later).
    Event {
        ev: MapEvent,
        keep: bool,
    },
    Score {
        id: PlayerId,
        v: i64,
    },
}

/// Something that happened in the arena (debug journal).
#[derive(Clone, Debug, PartialEq)]
pub struct JournalEntry {
    pub t: f64,
    pub what: &'static str,
    pub id: Option<PlayerId>,
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
    pub grabbing: Option<PlayerId>,
    pub hazard: Option<Hazard>,
}

/// The beans in play, for map logic.
struct PawnBodies<'a>(&'a mut [Pawn]);

impl Bodies for PawnBodies<'_> {
    fn ids(&self) -> Vec<PlayerId> {
        self.0
            .iter()
            .filter(|p| p.status == PawnStatus::Play)
            .map(|p| p.id)
            .collect()
    }

    fn get(&self, id: PlayerId) -> Option<&Body> {
        self.0
            .iter()
            .find(|p| p.id == id && p.status == PawnStatus::Play)
            .map(|p| &p.body)
    }

    fn get_mut(&mut self, id: PlayerId) -> Option<&mut Body> {
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
    pub participants: Vec<PlayerId>,
    /// In the order they were added.
    pub pawns: Vec<Pawn>,
    pub fall: FallBehaviour,
    /// Arena tick: sim time is tick × DT, negative during the intro.
    pub tick: i64,
    pub static_hash: Fingerprint,
    pub scores: BTreeMap<PlayerId, i64>,
    /// Map events and bonuses taken so far that whoever joins later must hear of (debug: the room keeps them).
    pub kept_events: usize,
    /// Debug: recent history of every bean, and of the arena.
    pub trace: BTreeMap<PlayerId, VecDeque<TraceEntry>>,
    pub journal: VecDeque<JournalEntry>,
    /// The round so far, for replays (None unless recording).
    pub recording: Option<Recording>,
    /// What map logic said during the current handler call.
    map_out: Vec<MapOut>,
    /// In finishing order.
    pub finished: Vec<PlayerId>,
    /// In elimination order.
    pub out: Vec<PlayerId>,
    /// Results are decided: no more finishes or outs are recorded.
    pub frozen: bool,
    /// Dev: bot brains run (false: bots stand still).
    pub bots_on: bool,
    spawn_cursor: usize,
    /// Bot navigation grid, built on first use once the round has started (gates are open).
    nav: Option<NavGrid>,
    /// The grid built ahead of the start, and the static world it was built for.
    nav_pre: Option<PreparedNav>,
    /// Buffers ticks reuse (not state).
    scratch: TickScratch,
    views: Vec<OtherView>,
    spots: Vec<V3>,
    /// The beans in play after each of the last MAX_VIEW ticks: (tick, [(id, teleports, down, pose)]).
    seen: SeenTicks,
    /// `--trace hits`: lines about contacts, tackles and dives, for the caller to take.
    #[cfg(feature = "traces")]
    pub hit_log: Option<Vec<String>>,
    #[cfg(feature = "traces")]
    hit_quiet: beans::NoteQuiet,
}

type SeenTicks = VecDeque<(i64, Vec<(PlayerId, u32, bool, OtherBody)>)>;

/// Where a player seeing `view` ticks behind saw bean `b` at tick k. None: now, or the bean is not where it was
/// any more (respawned since, or knocked over since by someone else: no tackle on what already flies away).
fn saw_in(seen: &SeenTicks, k: i64, view: u32, b: PlayerId, teleports: u32, down: bool) -> Option<OtherBody> {
    if view == 0 {
        return None;
    }
    let (_, list) = seen.iter().find(|h| h.0 == k - i64::from(view))?;
    let &(_, t, was_down, o) = list.iter().find(|e| e.0 == b)?;
    (t == teleports && (was_down || !down)).then_some(o)
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
        participants: &[PlayerId],
        with_scene: bool,
    ) -> (Self, Option<SceneDesc>) {
        let (mut b, spec) = build_map(map, seed, with_scene, participants);
        let meta = map.meta();
        let bonuses = if kind == ArenaKind::Round {
            Bonuses::new(&b.bonus_spots, seed, spec.finish.is_none(), meta.duration)
        } else {
            Bonuses::default()
        };
        b.world.finalize(tick as f64 * DT, &*spec.logic);
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
                scratch: TickScratch::default(),
                views: Vec::new(),
                spots: Vec::new(),
                seen: VecDeque::new(),
                #[cfg(feature = "traces")]
                hit_log: None,
                #[cfg(feature = "traces")]
                hit_quiet: beans::NoteQuiet::default(),
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

    fn index(&self, id: PlayerId) -> Option<usize> {
        self.pawns.iter().position(|p| p.id == id)
    }

    pub fn pawn(&self, id: PlayerId) -> Option<&Pawn> {
        self.pawns.iter().find(|p| p.id == id)
    }

    /// Adds a player or a bot at the next spawn (in the lobby: a free one).
    pub fn add_pawn(&mut self, id: PlayerId, bot: bool) -> &mut Pawn {
        self.add_pawn_at(id, bot, None)
    }

    /// Adds a pawn at spawn point `spawn` (None: the next one, in the lobby a free one).
    pub fn add_pawn_at(&mut self, id: PlayerId, bot: bool, spawn: Option<usize>) -> &mut Pawn {
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
            r.pawns.push(PawnRec {
                id,
                bot,
                spawn: Some(i),
                at: tick,
            });
        }
        let spawns = &self.spec.spawns;
        let spawn_i = i % spawns.len();
        let spawn = spawns[spawn_i];
        let mut body = Body::new(id.0 as i32);
        body.reset(spawn, self.face_yaw(spawn));
        // By slot in the round, not id: the same seed plays out the same whoever joined when; others after them.
        let n = self.participants.len() as u64;
        let slot = self
            .participants
            .iter()
            .position(|&p| p == id)
            .map_or(n + 1 + u64::from(id), |k| k as u64 + 1);
        #[expect(clippy::cast_possible_truncation, reason = "the low bits of a mixed slot")]
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
            grab_ready_at: NEVER,
            struggle: 0,
            teleports: 0,
            stats: RoundStats::default(),
            last_hit: None,
            forbidden_for: 0.0,
            force_grab_until: NEVER,
            reaching: false,
            view: 0,
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

    pub fn remove_pawn(&mut self, id: PlayerId) {
        self.op(Op::Remove(id));
        self.pawns.retain(|p| p.id != id);
        // (Ids are never reused: in a lobby that stays up for days the traces of those who left would pile up.)
        self.trace.remove(&id);
    }

    /// Results are decided: no more finishes, outs or credits (recorded: a replay freezes at the same tick).
    pub fn freeze(&mut self) {
        self.op(Op::Freeze);
        self.frozen = true;
    }

    /// Jumps to tick `k` without simulating the ones between (a server that fell too far behind).
    pub fn skip_to(&mut self, k: i64) {
        self.op(Op::Skip(k));
        self.tick = k;
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
            // 5: ticks the server skipped (`Op::Skip`).
            v: 5,
            game: self.map.meta().id,
            kind: self.kind,
            seed: self.seed,
            tick0: self.tick,
            participants: self.participants.clone(),
            pawns: Vec::new(),
            frames: BTreeMap::new(),
            ops: Vec::new(),
            end_tick: 0,
            hash: StateHash(0),
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
    pub fn note(&mut self, what: &'static str, id: Option<PlayerId>, data: Option<Value>) {
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
    pub fn step(&mut self, k: i64, frame: impl Fn(PlayerId) -> InputFrame) -> Vec<ArenaEvent> {
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
            let got = match self.bot_frame(i, k, &mut events) {
                Some(f) => f,
                None => frame(self.pawns[i].id),
            };
            let p = &mut self.pawns[i];
            // Before the start (and on the podium) nobody moves.
            p.frame = if moving { got } else { InputFrame::IDLE };
            if let Some(r) = &mut self.recording
                && p.bot.is_none()
            {
                r.record_frame(p.id, k, p.frame);
            }
        }

        {
            let looks: Vec<(PlayerId, u32, u32, bool)> = active
                .iter()
                .map(|&i| {
                    let p = &self.pawns[i];
                    (p.id, p.view, p.teleports, p.body.down())
                })
                .collect();
            let history = &self.seen;
            let seen = |a: PlayerId, b: PlayerId| -> Option<OtherBody> {
                let look = |id: PlayerId| looks.iter().find(|l| l.0 == id);
                let o = look(b)?;
                saw_in(history, k, look(a)?.1, b, o.2, o.3)
            };
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
            let mut map = MapRun {
                logic: &mut *self.spec.logic,
                server: true,
                apply: true,
                me: None,
                scores: &mut self.scores,
                out: &mut self.map_out,
            };
            tick_bodies_with(
                &mut self.scratch,
                &mut self.world,
                t,
                &mut steppers,
                &[],
                &seen,
                &mut map,
            );
        }
        self.flush(&mut events);
        for &i in &active {
            let p = &mut self.pawns[i];
            if let Some(by) = p.ev.tackled_by {
                p.last_hit = Some(Hit {
                    by: Some(by),
                    cause: Cause::Tackle,
                    t,
                });
            }
            if round {
                p.stats.tackles += p.ev.tackles;
            }
        }
        #[cfg(feature = "traces")]
        self.log_hits(k, &active);
        for &i in &active {
            let p = &mut self.pawns[i];
            if let Some(cause) = p.ev.hazard {
                let by = p.last_hit.filter(|h| t - h.t < CREDIT_WINDOW).and_then(|h| h.by);
                p.last_hit = Some(Hit {
                    by,
                    cause: Cause::Hazard(cause),
                    t,
                });
            }
        }
        for &i in &active {
            self.interact(i, &active, t, &mut events);
        }
        if round && t >= 0.0 && !self.frozen {
            let mut bodies: Vec<(PlayerId, &mut Body)> = Vec::with_capacity(active.len());
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
        self.remember_seen(k);
        events
    }

    /// Keeps where every bean in play is after tick k, for tackles judged by what a lagging player saw.
    fn remember_seen(&mut self, k: i64) {
        let mut list = if self.seen.len() > MAX_VIEW as usize {
            self.seen.pop_front().map(|e| e.1).unwrap_or_default()
        } else {
            Vec::new()
        };
        list.clear();
        list.extend(
            self.pawns
                .iter()
                .filter(|p| p.status == PawnStatus::Play && !p.body.in_portal())
                .map(|p| (p.id, p.teleports, p.body.down(), other_of(p.id, &p.body))),
        );
        self.seen.push_back((k, list));
    }

    /// How many ticks behind its bean player `id` sees the others (from its client; recorded).
    pub fn set_view(&mut self, id: PlayerId, ticks: u32) {
        let ticks = ticks.min(MAX_VIEW);
        let Some(i) = self.index(id) else { return };
        if self.pawns[i].view != ticks {
            self.op(Op::View { id, ticks });
            self.pawns[i].view = ticks;
            #[cfg(feature = "traces")]
            if let Some(log) = &mut self.hit_log {
                log.push(format!("{} {id} view {ticks}", self.tick));
            }
        }
    }

    /// `--trace hits` lines of tick k: every bean's notes; a tackling bean's pose and, within reach, the others'
    /// now and where its player saw them.
    #[cfg(feature = "traces")]
    fn log_hits(&mut self, k: i64, active: &[usize]) {
        let Some(mut log) = self.hit_log.take() else { return };
        let v3 = |v: V3| format!("{:.2},{:.2},{:.2}", v.x, v.y, v.z);
        for &i in active {
            let p = &self.pawns[i];
            let b = &p.body;
            for n in &p.ev.notes {
                if self.hit_quiet.fresh(k, p.id, n) {
                    log.push(format!("{k} {} {n}", p.id));
                }
            }
            if p.ev.tackled_by.is_some() || p.ev.knocked {
                log.push(format!("{k} {} state {:?} v={}", p.id, b.state, v3(b.vel)));
            }
            if !beans::tackling(b) {
                continue;
            }
            log.push(format!(
                "{k} {} {:?} pos={} v={} view={}",
                p.id,
                b.state,
                v3(b.pos),
                v3(b.vel),
                p.view
            ));
            let cb = beans::Capsule::of_body(b);
            for &j in active {
                let o = &self.pawns[j];
                if j == i || dist_xz(o.body.pos, b.pos) > 4.0 {
                    continue;
                }
                let now = beans::Capsule::of_body(&o.body);
                let saw = saw_in(&self.seen, k, p.view, o.id, o.teleports, o.body.down());
                let seen = saw.map_or("-".to_string(), |s| {
                    format!(
                        "{} gap={:.2}",
                        v3(V3::new(s.x, s.y, s.z)),
                        beans::gap(&cb, &beans::Capsule::of_other(&s))
                    )
                });
                log.push(format!(
                    "{k} {}   near {} {:?} now={} gap={:.2} seen={seen}",
                    p.id,
                    o.id,
                    o.body.state,
                    v3(o.body.pos),
                    beans::gap(&cb, &now)
                ));
            }
        }
        self.hit_log = Some(log);
    }

    /// Runs `f` with the map's logic and context (the server's: every bean in play).
    fn with_cx<R>(&mut self, t: f64, f: impl FnOnce(&mut dyn MapLogic, &mut Cx) -> R) -> R {
        let mut bodies = PawnBodies(&mut self.pawns);
        let mut cx = Cx::new(
            Role::SERVER,
            t,
            &mut self.world,
            &mut bodies,
            &mut self.scores,
            &mut self.map_out,
        );
        f(&mut *self.spec.logic, &mut cx)
    }

    fn map_tick(&mut self, t: f64, events: &mut Vec<ArenaEvent>) {
        self.with_cx(t, |logic, cx| logic.tick(cx, t));
        self.flush(events);
        let logic = &*self.spec.logic;
        for p in self.pawns.iter_mut().filter(|p| p.status == PawnStatus::Play) {
            logic.bean(p.id, &mut p.body, t);
        }
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

    /// A bot's input frame (None for a player).
    #[expect(clippy::cast_possible_truncation, reason = "rounded and clamped to ±127 first")]
    fn bot_frame(&mut self, i: usize, k: i64, events: &mut Vec<ArenaEvent>) -> Option<InputFrame> {
        self.pawns[i].bot.as_ref()?;
        let fresh = k % i64::from(BOT_EVERY) == 0;
        if fresh {
            self.think(i, events);
        }
        let b = self.pawns[i].bot.as_ref()?.input;
        let axis = |v: f64| (v.clamp(-1.0, 1.0) * 127.0).round();
        let (mx, mz) = (axis(b.mx), axis(b.mz));
        // Clamped to unit length as the server clamps every player's frame.
        InputFrame {
            mx: mx as i8,
            mz: mz as i8,
            buttons: (if fresh && b.jump { BTN_JUMP } else { 0 })
                | (if fresh && b.dive { BTN_DIVE } else { 0 })
                | (if b.grab { BTN_GRAB } else { 0 }),
        }
        .clamped()
        .into()
    }

    fn build_nav(&self) -> NavGrid {
        let logic = &*self.spec.logic;
        NavGrid::build(&self.world, Some(&|p| logic.forbidden(p)))
    }

    /// Builds the bots' navigation grid now, ahead of the round or the lobby (a server room does it off its tick).
    pub fn prepare_nav(&mut self) {
        if self.nav.is_none() && self.nav_pre.is_none() && self.kind != ArenaKind::Podium && self.spec.logic.bots() {
            self.nav_pre = Some((self.build_nav(), NavGrid::key(&self.world)));
        }
    }

    /// The grid `prepare_nav` built and the static world it was built for: for another arena of the same map
    /// and seed (`give_nav`).
    pub fn prepared_nav(&self) -> Option<&PreparedNav> {
        self.nav_pre.as_ref()
    }

    /// A grid built ahead elsewhere (another thread, another arena of this map): used at the first need if the
    /// static world is still the one it was built for, else built again as without it.
    pub fn give_nav(&mut self, pre: PreparedNav) {
        if self.nav.is_none() {
            self.nav_pre = Some(pre);
        }
    }

    /// The navigation grid takes a while to build: it is built before the start, and used at the
    /// start if the static world has not changed.
    fn prebuild_nav(&mut self) {
        if self.nav.is_some() || self.nav_pre.is_some() || self.kind != ArenaKind::Round || !self.spec.logic.bots() {
            return;
        }
        let t = self.time();
        if !(-NAV_PREBUILD..0.0).contains(&t) {
            return;
        }
        if !self.pawns.iter().any(|p| p.bot.is_some()) {
            return;
        }
        self.nav_pre = Some((self.build_nav(), NavGrid::key(&self.world)));
    }
}
