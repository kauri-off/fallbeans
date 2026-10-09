//! The client's copy of the round: the same map built from the seed, own-bean prediction, map events,
//! and the input it sends.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use fb_arena::{
    ArenaKind, FallBehaviour, MAX_VIEW, MapRun, Stepper, TickScratch, build_map, can_move, client_event, client_start,
    fell, reach_checkpoint, respawn, respawn_point, tick_bodies_with,
};
use fb_net::*;
use fb_proto::{ArenaInfo, Pid};
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_shared::{DT, m};
use fb_sim::beans;
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::map::{BeanDeco, MapOut, MapSfx, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::Nodes;
use fb_sim::physics::{Body, BodyInput, BodyState, OtherBody, StepEvents};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;
use lightyear::core::confirmed_history::ConfirmedHistory;
use lightyear::input::native::prelude::{ActionState, InputMarker};
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::*;

use crate::keys::Bind;
use crate::opts::Opts;
use crate::settings::Bindings;

/// The game takes the player's keys, mouse and pad (no menu, chat line or text field has them).
#[derive(Resource)]
pub struct Gate {
    pub play: bool,
}

impl Default for Gate {
    fn default() -> Self {
        Self { play: true }
    }
}

/// Input a tool holds for a while (`fb/input` over BRP).
#[derive(Resource)]
pub struct ProbeInput {
    pub frame: InputFrame,
    pub until: f64,
}
use crate::session::{Feed, FeedLog, Session};

/// Map events by their tick.
#[derive(Default)]
pub struct Seen(BTreeMap<u32, Vec<MapEventKind>>);

impl Seen {
    fn contains(&self, msg: &MapEventMsg) -> bool {
        self.0.get(&msg.tick).is_some_and(|evs| evs.contains(&msg.ev))
    }

    fn insert(&mut self, msg: &MapEventMsg) {
        self.0.entry(msg.tick).or_default().push(msg.ev.clone());
    }
}

/// The map of the current round as this client built it.
#[derive(Resource)]
pub struct Map {
    pub round: Round,
    pub world: World,
    pub spec: MapSpec,
    pub scene: SceneDesc,
    pub bonuses: Bonuses,
    /// Nodes posed for drawing (at the frame's time, not the tick's).
    pub render: Nodes,
    pub static_hash: String,
    /// Map events waiting for their tick on the interpolated timeline.
    pub pending: Vec<MapEventMsg>,
    /// Map events applied to this arena: the history the room sends again after a reconnect is not applied a
    /// second time.
    pub seen: Seen,
    pub generation: u32,
    /// Who plays this arena, and who of them finished (in order) or is out.
    pub info: ArenaInfo,
    /// Decorations maps put on beans (tails, badges).
    pub deco: BTreeMap<Pid, BeanDeco>,
    /// Sounds the map asked for, for the audio to take.
    pub sfx: Vec<MapSfx>,
    /// How this round looks (`fb_sim::looks`): palettes, sky, light, fog, scenery.
    pub look: fb_sim::looks::ResolvedLook,
    /// The room it was built in: every room's first lobby is arena 1 with seed 1, another room's is not this.
    room: Option<String>,
    /// Built by the warm-up behind the loading screen (`render/warmup.rs`), in no room: nobody plays it.
    pub warmup: bool,
    /// The entity of its `Round`: the server changes it there for the room's next arena.
    source: Option<Entity>,
    /// The others as drawn at each predicted tick, and how many ticks behind the own bean: a rollback replays a
    /// tick against the same bodies, not against where they are drawn now.
    others: BTreeMap<i64, (u32, Vec<OtherBody>)>,
}

/// The maps built so far, the warm-up's too: each its own `generation`, by which views tell maps apart.
#[derive(Resource, Default)]
pub struct Generations(u32);

impl Generations {
    pub fn next(&mut self) -> u32 {
        let g = self.0;
        self.0 = g.wrapping_add(1);
        g
    }
}

/// Ticks of the others kept (`PredictionManager::max_rollback_ticks` and a little).
const OTHERS_KEPT: usize = 128;

impl Map {
    pub fn time(&self, tick: f64) -> f64 {
        (tick - self.round.zero_tick as f64) * DT
    }

    /// The room is in another arena already, its map not built yet.
    fn stale(&self, rounds: &Query<&Round>, session: &Session) -> bool {
        let moved = self
            .source
            .is_some_and(|e| rounds.get(e).is_ok_and(|r| !r.same_arena(&self.round)));
        !self.warmup && (moved || session.arena.as_ref().is_some_and(|a| a.id != self.round.arena))
    }

    /// Finished or out: the bean has left the arena.
    pub fn gone(&self, id: Pid) -> bool {
        self.info.finished.contains(&id) || self.info.out.contains(&id)
    }

    /// `def` as a round of it would be built here, for the warm-up to draw (`render/warmup.rs`): seed 1, eight
    /// players nobody plays, the look given; time 0 at `zero_tick`.
    pub fn warmup(
        def: &'static dyn fb_sim::map::MapDef,
        kind: ArenaKind,
        look: fb_sim::looks::ResolvedLook,
        generation: u32,
        zero_tick: i64,
    ) -> Map {
        const SEED: u32 = 1;
        let participants: Vec<Pid> = (1..=8).collect();
        let meta = def.meta();
        let (mut b, mut spec) = build_map(def, SEED, true, &participants);
        let bonuses = if kind == ArenaKind::Round {
            Bonuses::new(&b.bonus_spots, SEED, spec.finish.is_none(), meta.duration)
        } else {
            Bonuses::default()
        };
        b.world.finalize(-1e3, &*spec.logic);
        let static_hash = b.world.hash(true).to_string();
        let (mut scores, mut out) = (BTreeMap::new(), Vec::new());
        client_start(&mut b.world, &mut spec, &mut scores, None, &mut out);
        let render = b.world.nodes.clone();
        let mut map = Map {
            round: Round {
                arena: 0,
                kind,
                map: meta.id.to_string(),
                seed: SEED,
                zero_tick,
                fall: fb_arena::fall_behaviour(meta.genre),
                static_hash: static_hash.clone(),
            },
            world: b.world,
            spec,
            scene: b.scene.unwrap_or_default(),
            bonuses,
            render,
            static_hash,
            pending: Vec::new(),
            seen: Seen::default(),
            generation,
            info: ArenaInfo {
                id: 0,
                kind,
                game: meta.id.to_string(),
                participants,
                index: 0,
                total: 0,
                practice: false,
                late: false,
                finished: Vec::new(),
                out: Vec::new(),
                scores: Vec::new(),
            },
            deco: BTreeMap::new(),
            sfx: Vec::new(),
            look,
            room: None,
            others: BTreeMap::new(),
            warmup: true,
            source: None,
        };
        // (Its decorations, not its sounds: nobody is there to hear them.)
        map.take_out(out);
        map.sfx.clear();
        map
    }

    fn take_out(&mut self, out: Vec<MapOut>) {
        for o in out {
            match o {
                MapOut::Sfx(s) => self.sfx.push(s),
                MapOut::Decorate { id, change } => self.deco.entry(id).or_default().apply(change),
                MapOut::Event { .. } | MapOut::Score { .. } => {}
            }
        }
    }
}

/// Something that happened to a bean, for its face and the sounds. The own bean's moves come from
/// predicted ticks (rollback replays say nothing again).
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    Jumped,
    Dived,
    Bounced,
    /// Stunned by a hit, knocked over, or pushed (how hard).
    Hit,
    Knocked,
    Bumped(f32),
    Landed(f32),
    Finish(Pid),
    Ko {
        id: Pid,
        out: bool,
    },
    Bonus(Pid),
    Emote {
        id: Pid,
        e: u8,
    },
    /// A round's results are in.
    Results,
    /// Somebody rang the lobby's bell (climbed its tower).
    Bell(Pid),
    /// A sound the map asked for (`MapSfx`).
    Sfx(crate::audio::Sfx),
}

/// Camera yaw (radians; forward is (sin, cos) on x/z) and pitch.
#[derive(Resource)]
pub struct CameraAngles {
    pub yaw: f32,
    pub pitch: f32,
}

/// The own bean's position before the last tick (frames are drawn between ticks).
#[derive(Component, Default)]
pub struct PrevPos(pub V3);

#[derive(Component, Default)]
pub struct OwnEvents(pub StepEvents);

/// Seconds a rollback's correction of the own bean takes to fade out of the drawing.
pub const SMOOTH_T: f32 = 0.1;
/// A correction farther than this (m) is not smoothed: the bean is put where it is at once.
const SMOOTH_MAX: f64 = 1.5;

/// A rollback's correction of the own bean, drawn away: the bean is drawn and animated off its state by `pos`,
/// `vel` and `yaw`, which fade over SMOOTH_T (`beans::place_beans`), instead of jumping (and the body's jelly
/// with its speed).
#[derive(Component, Default)]
pub struct Smoothing {
    /// The own bean as last predicted at each recent tick (pos, vel, yaw).
    pred: BTreeMap<i64, (V3, V3, f64)>,
    /// The correction of the last tick a rollback replayed, for the drawing to take.
    pending: Option<(V3, V3, f64)>,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
}

impl Smoothing {
    /// The frame's drawing: takes a new correction in, fades the rest by `dt` seconds.
    pub fn frame(&mut self, dt: f32) {
        if let Some((p, v, y)) = self.pending.take() {
            self.pos += p.as_vec3();
            self.vel += v.as_vec3();
            self.yaw += y as f32;
        }
        let k = (-dt / SMOOTH_T).exp();
        self.pos *= k;
        self.vel *= k;
        self.yaw *= k;
    }

    /// The own bean at tick k (`replay`: by a rollback); `snapped`: put somewhere else (a respawn, a portal).
    fn predicted(&mut self, k: i64, b: &Body, replay: bool, snapped: bool) {
        let old = self.pred.insert(k, (b.pos, b.vel, b.yaw));
        if snapped {
            *self = Self::default();
            return;
        }
        if let Some((p, v, y)) = old.filter(|_| replay) {
            let dy = y - b.yaw;
            self.pending =
                (p.distance(b.pos) < SMOOTH_MAX).then(|| (p - b.pos, v - b.vel, m::atan2(m::sin(dy), m::cos(dy))));
        }
        while self.pred.len() > 256 {
            self.pred.pop_first();
        }
    }
}

#[derive(Resource, Default)]
pub struct Stats {
    /// Prediction steps, replays of rollbacks included.
    pub ticks: u64,
    pub map_events: u32,
    pub hash_mismatch: bool,
}

/// `--trace`: one line per predicted tick while connected, `C tick room id mx mz buttons x y z` (rollback
/// replays repeat ticks), and `R tick` before the first one of each connection.
#[derive(Resource)]
struct Trace {
    out: BufWriter<File>,
    fresh: bool,
}

/// `--trace-hits`: the own bean's notes per predicted tick, `C tick id …` (`c`: a rollback's replay), and while
/// it tackles the others as drawn; the interpolation delay (`view`) when it changes.
#[derive(Resource)]
pub struct HitTrace {
    out: BufWriter<File>,
    view: Option<u32>,
    quiet: beans::NoteQuiet,
    /// The own bean as last predicted at each recent tick (pos, vel, yaw): a rollback's replay is compared with it.
    pred: BTreeMap<i64, (V3, V3, f64)>,
}

impl HitTrace {
    /// One line as it is (the drawing's `F` lines, `beans::animate_beans`).
    pub fn line(&mut self, l: core::fmt::Arguments) {
        let _ = self.out.write_fmt(l);
        let _ = self.out.write_all(b"\n");
    }
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraAngles { yaw: 0.0, pitch: 0.32 });
        app.init_resource::<Stats>();
        app.init_resource::<Inbox>();
        app.init_resource::<Presses>();
        app.init_resource::<Gate>();
        app.init_resource::<Generations>();
        app.init_resource::<LastRollback>();
        app.add_message::<Cue>();
        app.add_systems(Startup, open_trace);
        app.add_systems(
            PreUpdate,
            (
                (drop_map, build_round, reconcile, pace_pushes)
                    .chain()
                    .after(ReplicationSystems::Receive)
                    .before(RollbackSystems::Check),
                latch_presses.after(bevy::input::InputSystems),
            ),
        );
        app.add_systems(PreUpdate, note_rollback.after(RollbackSystems::Check));
        app.add_systems(FixedPreUpdate, write_input.in_set(InputSystems::WriteClientInputs));
        app.add_systems(FixedUpdate, predict);
        app.add_systems(
            Update,
            ((receive_map_events, apply_map_events, map_sounds).chain(), send_emotes),
        );
        app.add_observer(on_controlled);
        app.add_observer(|_: On<Add, Connected>, trace: Option<ResMut<Trace>>| {
            if let Some(mut t) = trace {
                t.fresh = true;
            }
        });
    }
}

fn open_trace(mut commands: Commands, opts: Res<Opts>) {
    if let Some(path) = &opts.trace_hits {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace-hits {}: {e}", path.display()));
        commands.insert_resource(HitTrace {
            out: BufWriter::new(file),
            view: None,
            quiet: Default::default(),
            pred: BTreeMap::new(),
        });
    }
    if let Some(path) = &opts.trace {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace {}: {e}", path.display()));
        commands.insert_resource(Trace {
            out: BufWriter::new(file),
            fresh: true,
        });
    }
}

/// Out of the room (at the room list, on another server): its map goes, and with it what the next room would
/// take for its own (players, bonuses, decorations). The scene stays drawn until the next map's replaces it. The
/// warm-up's maps are in no room: they go when it is done with them.
fn drop_map(map: Option<Res<Map>>, session: Res<Session>, mut commands: Commands) {
    if map.is_some_and(|m| !m.warmup) && session.room.is_none() {
        commands.remove_resource::<Map>();
    }
}

/// Builds the arena's map once both the replicated `Round` and its `ArenaInfo` (who plays: maps deal
/// tails and stars from it) are in.
fn build_round(
    mut commands: Commands,
    rounds: Query<(Entity, &Round)>,
    map: Option<ResMut<Map>>,
    mut session: ResMut<Session>,
    mut stats: ResMut<Stats>,
    mut unknown: Local<Option<u32>>,
    mut generations: ResMut<Generations>,
) {
    // (Out of the room, its arena may linger a moment: `drop_map`.)
    if session.room.is_none() {
        return;
    }
    // The arena the player is in: the last one's `Round` may linger a moment beside it.
    let in_arena = |(_, r): &(Entity, &Round)| session.arena.as_ref().is_some_and(|a| a.id == r.arena);
    let Some((source, round)) = rounds.iter().find(in_arena) else {
        return;
    };
    let mut map = map;
    if let Some(m) = map
        .as_mut()
        .filter(|m| m.round.same_arena(round) && m.room == session.room)
    {
        // The same arena with its clock moved (a dev warp or pause): no new map.
        if m.round != *round {
            m.round = round.clone();
        }
        if m.source != Some(source) {
            m.source = Some(source);
        }
        return;
    }
    let Some(info) = session.arena.clone().filter(|a| a.id == round.arena) else {
        return;
    };
    let Some(def) = fb_maps::by_id(&round.map) else {
        // (Once per arena: this runs every frame.)
        if unknown.replace(round.arena) != Some(round.arena) {
            error!("unknown map {}", round.map);
        }
        return;
    };
    let (mut b, mut spec) = build_map(def, round.seed, true, &info.participants);
    let meta = def.meta();
    let bonuses = if round.kind == ArenaKind::Round {
        Bonuses::new(&b.bonus_spots, round.seed, spec.finish.is_none(), meta.duration)
    } else {
        Bonuses::default()
    };
    b.world.finalize(-1e3, &*spec.logic);
    let static_hash = b.world.hash(true).to_string();
    if static_hash != round.static_hash {
        // The client would predict against a different map: a bug in determinism, never expected.
        error!("map hash {static_hash} differs from the server's {}", round.static_hash);
        stats.hash_mismatch = true;
    }
    info!(
        "arena {}: {} seed {} (static {static_hash})",
        round.arena, round.map, round.seed
    );
    let mut out = Vec::new();
    let me = session.me;
    client_start(&mut b.world, &mut spec, &mut session.scores, me, &mut out);
    let render = b.world.nodes.clone();
    // (Counted here, not from the map before: there may be none, and views tell maps apart by it.)
    let generation = generations.next();
    let mut next = Map {
        round: round.clone(),
        world: b.world,
        spec,
        scene: b.scene.unwrap_or_default(),
        bonuses,
        render,
        static_hash,
        pending: Vec::new(),
        seen: Seen::default(),
        generation,
        info,
        deco: BTreeMap::new(),
        sfx: Vec::new(),
        look: fb_sim::looks::look_for(def.looks(), round.seed),
        room: session.room.clone(),
        others: BTreeMap::new(),
        warmup: false,
        source: Some(source),
    };
    next.take_out(out);
    commands.insert_resource(next);
}

/// Lightyear checks the prediction at completed server ticks only, and only where the prediction history reaches
/// back that far. Just after the clock is first set (or for a bean given since) it has nothing so old and the
/// check passes silently, and in an intro where nothing moves no tick completes at all: the own bean kept what it
/// had (the lobby's place) while the server had it at the round's spawn, until the start. So the newest server
/// state of the own bean is rolled back to as it comes when the prediction has nothing at its tick, or another
/// respawn count (a respawn, a new arena); and once more when a new arena's map is built (a rollback in the frame
/// its `Round` came replayed its first ticks on the old map). Anything else is left to Lightyear.
fn reconcile(
    timeline: SyncedLocalTimeline,
    map: Option<Res<Map>>,
    manager: Res<PredictionManager>,
    own: Query<(&PredictionHistory<BodyFull>, &ConfirmedHistory<BodyFull>), With<Predicted>>,
    mut meta: ResMut<StateRollbackMetadata>,
    mut seen: Local<(Option<Tick>, Option<u32>)>,
) {
    let tick = timeline.tick();
    let Ok((predicted, confirmed)) = own.single() else {
        return;
    };
    let Some((at, server)) = confirmed.newest_present() else {
        return;
    };
    if at > tick || tick - at > i32::from(manager.rollback_policy.max_rollback_ticks) {
        return;
    }
    let (last, built) = &mut *seen;
    let generation = map.map(|m| m.generation);
    let off = predicted.get(at).is_none_or(|p| p.teleports != server.teleports);
    if (*last != Some(at) && off) || *built != generation {
        meta.request_forced_rollback(at);
        *built = generation;
    }
    *last = Some(at);
}

/// Ticks between rollbacks for a push (below).
const PUSH_EVERY: i32 = 8;
/// Closer than this (m, between feet) another bean may be leaning on the own one.
const PUSH_NEAR: f32 = 2.5;
/// A difference this small (m) in position can be a push.
const PUSH_MAX: f64 = 0.3;

/// The last tick a rollback was started at.
#[derive(Resource, Default)]
struct LastRollback(Option<Tick>);

fn note_rollback(timeline: Res<LocalTimeline>, manager: Res<PredictionManager>, mut last: ResMut<LastRollback>) {
    if manager.is_rollback() {
        last.0 = Some(timeline.tick());
    }
}

/// Beans leaning on each other: the client pushes against the others where it draws them, a little in the
/// past, so while they touch nearly every snapshot finds the own bean a few millimetres off and rolls back
/// (portal-panic's crowds at a checkpoint: 600–1200 rollbacks in 100 s). A difference only in where the bean
/// is and how fast it goes, with another bean near, waits until PUSH_EVERY ticks after the last rollback
/// (state checks are off for the frame); anything else rolls back at once. Waiting loses nothing: the
/// difference stays in the history, and the next check finds it.
fn pace_pushes(
    timeline: SyncedLocalTimeline,
    checkpoints: Res<ReplicationCheckpointMap>,
    mutate: Option<Res<ServerMutateTicks>>,
    mut manager: ResMut<PredictionManager>,
    meta: Res<StateRollbackMetadata>,
    last: Res<LastRollback>,
    own: Query<(&PredictionHistory<BodyFull>, &ConfirmedHistory<BodyFull>), With<Predicted>>,
    others: Query<&RemotePose, (With<Interpolated>, Without<Predicted>)>,
) {
    let tick = timeline.tick();
    let recent = last.0.is_some_and(|l| tick - l < PUSH_EVERY);
    let wait = recent
        && meta.forced_rollback_tick().is_none()
        && mutate
            .and_then(|m| checkpoints.latest_completed_at_or_before(&m, tick))
            .zip(own.single().ok())
            .and_then(|(c, (predicted, confirmed))| Some((predicted.get(c.tick)?, confirmed.get_present(c.tick)?)))
            .is_some_and(|(p, s)| {
                let mut moved = p.clone();
                (moved.body.pos, moved.body.vel) = (s.body.pos, s.body.vel);
                let at = s.body.pos.as_vec3();
                !body_differs(&moved, s)
                    && (p.body.pos - s.body.pos).length() < PUSH_MAX
                    && others.iter().any(|o| o.pos.distance(at) < PUSH_NEAR)
            });
    if matches!(manager.rollback_policy.state, RollbackMode::Disabled) != wait {
        manager.rollback_policy.state = if wait {
            RollbackMode::Disabled
        } else {
            RollbackMode::Check
        };
    }
}

fn on_controlled(trigger: On<Add, Controlled>, mut commands: Commands, pawns: Query<(), With<PlayerId>>) {
    if pawns.get(trigger.entity).is_ok() {
        commands.entity(trigger.entity).insert((
            InputMarker::<FbInput>::default(),
            PrevPos::default(),
            OwnEvents::default(),
            Smoothing::default(),
        ));
    }
}

/// Where the autopilot steers.
enum Steer {
    /// Relative to the camera: (forward, right, buttons).
    Camera(f64, f64, u8),
    /// In world space.
    World(InputFrame),
}

/// A slow circle with a hop now and then, and a run for any bonus lying about: exercises prediction and
/// the map event path without a player.
fn autopilot(k: u32, map: Option<&Map>, own: Option<(&PlayerId, &BodyFull)>) -> Steer {
    // Each player on its own rhythm, so a room of autopilots does not move in lockstep.
    let id = own.map_or(0, |(id, _)| id.0);
    let p = k + id * 53;
    let buttons = if p % (89 + id % 13) < 2 { BTN_JUMP } else { 0 };
    if let (Some(map), Some((_, own))) = (map, own) {
        let p = own.body.pos;
        if let Some(b) = map.bonuses.available(map.time(k as f64)).next() {
            let (dx, dz) = (b.pos.x - p.x, b.pos.z - p.z);
            let l = (dx * dx + dz * dz).sqrt().max(1e-6);
            return Steer::World(InputFrame::from_stick(dx / l, dz / l, buttons));
        }
    }
    let turn = if (p / (200 + 17 * (id % 5))).is_multiple_of(2) {
        0.6
    } else {
        -0.6
    };
    Steer::Camera(1.0, turn, buttons)
}

/// Jump and dive fire once per press (a held key does not repeat them, nor does the server: see
/// `room::frame_for`). Presses made between two ticks wait here for the next one that is not a rollback's.
#[derive(Resource, Default)]
struct Presses(u8);

fn latch_presses(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    pads: Query<&Gamepad>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut presses: ResMut<Presses>,
    gate: Res<Gate>,
    binds: Res<Bindings>,
) {
    if !gate.play {
        return;
    }
    for pad in &pads {
        if pad.just_pressed(GamepadButton::South) {
            presses.0 |= BTN_JUMP;
        }
        if pad.any_just_pressed([GamepadButton::West, GamepadButton::East]) {
            presses.0 |= BTN_DIVE;
        }
    }
    let (Some(keys), Some(mouse)) = (keys, mouse) else {
        return;
    };
    if keys.any_just_pressed(binds.keys(Bind::Jump).iter().copied()) {
        presses.0 |= BTN_JUMP;
    }
    // The click that captures the mouse is not a dive.
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if keys.any_just_pressed(binds.keys(Bind::Dive).iter().copied())
        || (captured && mouse.just_pressed(MouseButton::Left))
    {
        presses.0 |= BTN_DIVE;
    }
}

/// Stick values within this of the centre count as none (worn sticks drift).
const PAD_DEADZONE: f32 = 0.15;

/// The gamepads' left stick (forward, right) once out of the dead zone, and their grab buttons (RB, RT).
fn gamepad(pads: &Query<&Gamepad>) -> ((f64, f64), bool) {
    let dz = |v: f32| if v.abs() < PAD_DEADZONE { 0.0 } else { f64::from(v) };
    let mut stick = (0.0, 0.0);
    let mut grab = false;
    for pad in pads {
        let s = pad.left_stick();
        if stick == (0.0, 0.0) {
            stick = (dz(s.y), dz(s.x));
        }
        grab |= pad.any_pressed([GamepadButton::RightTrigger, GamepadButton::RightTrigger2]);
    }
    (stick, grab)
}

/// The stick (forward, right) and the held buttons (grab).
fn keyboard(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>, binds: &Bindings) -> (f64, f64, u8) {
    let held = |b: Bind| keys.any_pressed(binds.keys(b).iter().copied());
    let axis = |pos: Bind, neg: Bind| f64::from(i8::from(held(pos)) - i8::from(held(neg)));
    let f = axis(Bind::Forward, Bind::Back);
    let r = axis(Bind::Right, Bind::Left);
    let grab = held(Bind::Grab) || mouse.pressed(MouseButton::Right);
    (f, r, if grab { BTN_GRAB } else { 0 })
}

/// The stick relative to the camera, quantized as the server takes it.
fn write_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    pads: Query<&Gamepad>,
    mut presses: ResMut<Presses>,
    cam: Res<CameraAngles>,
    opts: Res<Opts>,
    timeline: Res<LocalTimeline>,
    map: Option<Res<Map>>,
    mut probe: Option<ResMut<ProbeInput>>,
    time: Res<Time<Real>>,
    mut q: Query<(&mut ActionState<FbInput>, Option<(&PlayerId, &BodyFull)>), With<InputMarker<FbInput>>>,
    gate: Res<Gate>,
    binds: Res<Bindings>,
    rollback: Option<Res<Rollback>>,
    sync: Res<LocalTimelineSync>,
) {
    // A rollback replays the buffered input over what is written here: a press taken now would be lost.
    if rollback.is_some() {
        return;
    }
    // Taken by the first tick after the press (or dropped while there is no bean to press it).
    let pressed = core::mem::take(&mut presses.0);
    let Ok((mut state, own)) = q.single_mut() else { return };
    // Before the clock is set the ticks are not the server's: what is written now is moved onto ticks the
    // server played without it, and the rollback that follows the setting would replay it there.
    if !sync.is_synced() {
        state.0 = InputFrame::IDLE.into();
        return;
    }
    if let Some(p) = probe.as_mut().filter(|p| time.elapsed_secs_f64() < p.until) {
        state.0 = p.frame.into();
        p.frame.buttons &= BTN_GRAB;
        return;
    }
    let (f, r, buttons) = if opts.autopilot() {
        match autopilot(timeline.tick().0, map.as_deref(), own) {
            Steer::Camera(f, r, buttons) => (f, r, buttons),
            Steer::World(frame) => {
                state.0 = frame.into();
                return;
            }
        }
    } else if !gate.play {
        (0.0, 0.0, 0)
    } else {
        let (mut f, mut r, mut held) = match (keys, mouse) {
            (Some(keys), Some(mouse)) => keyboard(&keys, &mouse, &binds),
            _ => (0.0, 0.0, 0),
        };
        // The pad's stick wins over the keys while it is pushed.
        let (stick, grab) = gamepad(&pads);
        if stick != (0.0, 0.0) {
            (f, r) = stick;
        }
        if grab {
            held |= BTN_GRAB;
        }
        (f, r, held | pressed)
    };
    let (fx, fz) = (cam.yaw.sin() as f64, cam.yaw.cos() as f64);
    let (mut mx, mut mz) = (f * fx - r * fz, f * fz + r * fx);
    let l = (mx * mx + mz * mz).sqrt();
    if l > 1.0 {
        mx /= l;
        mz /= l;
    }
    state.0 = InputFrame::from_stick(mx, mz, buttons).into();
}

/// Own bean: the same tick as the server's, against the others as drawn.
fn predict(
    timeline: Res<LocalTimeline>,
    map: Option<ResMut<Map>>,
    mut stats: ResMut<Stats>,
    mut own: Query<
        (
            &PlayerId,
            &mut BodyFull,
            &ActionState<FbInput>,
            &mut OwnEvents,
            &mut PrevPos,
            &mut Smoothing,
        ),
        With<Predicted>,
    >,
    others: Query<(&PlayerId, &RemotePose), (With<Interpolated>, Without<Predicted>)>,
    trace: Option<ResMut<Trace>>,
    (hits, interp): (Option<ResMut<HitTrace>>, Option<Res<InterpolationTimeline>>),
    session: Res<Session>,
    link: Query<(), (With<Client>, With<Connected>)>,
    rollback: Option<Res<Rollback>>,
    mut cues: MessageWriter<Cue>,
    rounds: Query<&Round>,
) {
    let Some(mut map) = map else { return };
    let Ok((id, mut full, state, mut ev, mut prev, mut smooth)) = own.single_mut() else {
        return;
    };
    let frame = InputFrame::from(state.0);
    let mut trace = trace.filter(|_| !link.is_empty());
    // Not on the last arena's ground and clock: the bean waits as the server last had it until the map is built,
    // and `reconcile` replays these ticks on it.
    if map.stale(&rounds, &session) {
        if let Some(trace) = &mut trace {
            write_trace(trace, timeline.tick(), &session, id.0, frame, &full.body);
        }
        return;
    }
    let teleports = full.teleports;
    let k = map.round.arena_tick(timeline.tick());
    let t = k as f64 * DT;
    let input: BodyInput = if can_move(map.round.kind, t) {
        frame.into()
    } else {
        BodyInput::default()
    };
    // The others as drawn, the first time this tick is predicted; a rollback's replay meets them where they
    // were then. As on the server, a bean that finished or is out (or is in a portal) is in nobody's way.
    let map = &mut *map;
    let replayed = rollback.is_some().then(|| map.others.get(&k).cloned()).flatten();
    let (lag, extra) = match replayed {
        Some(extra) => extra,
        None => {
            let lag = interp.as_ref().map_or(0, |i| {
                (timeline.tick().0.wrapping_sub(i.tick().0) as i32).clamp(0, MAX_VIEW as i32) as u32
            });
            let mut extra = if map.others.len() >= OTHERS_KEPT {
                map.others.pop_first().map(|(_, v)| v.1).unwrap_or_default()
            } else {
                Vec::new()
            };
            extra.clear();
            extra.extend(
                others
                    .iter()
                    .filter(|(o, p)| p.anim != Anim::Portal && !map.gone(o.0))
                    .map(|(o, p)| OtherBody {
                        id: o.0,
                        x: p.pos.x as f64,
                        y: p.pos.y as f64,
                        z: p.pos.z as f64,
                        vx: p.vel.x as f64,
                        vy: 0.0,
                        vz: p.vel.y as f64,
                        tilt: p.tilt as f64,
                        tilt_dir: p.tilt_dir as f64,
                        size: p.size as f64,
                    }),
            );
            // (By id: the query's order may change between a tick and its replay.)
            extra.sort_by_key(|o| o.id);
            map.others.insert(k, (lag, extra.clone()));
            (lag, extra)
        }
    };
    // They are drawn `lag` ticks in the past: the own bean bumps into where they will be by now, going on as they
    // went; its tackles connect with them as drawn, as the server judges them.
    let ahead: Vec<OtherBody> = extra
        .iter()
        .map(|o| {
            let s = f64::from(lag) * DT;
            OtherBody {
                x: o.x + o.vx * s,
                z: o.z + o.vz * s,
                ..*o
            }
        })
        .collect();
    let drawn = |_: u32, b: u32| extra.iter().find(|o| o.id == b).copied();
    let full = &mut *full;
    // A body from the previous arena may stand on a collider this world does not have.
    if full
        .body
        .ground_col
        .is_some_and(|c| c as usize >= map.world.colliders.len())
    {
        full.body.ground_col = None;
    }
    prev.0 = full.body.pos;
    let (was_grounded, was_dive) = (full.body.grounded, full.body.state == BodyState::Dive);
    let mut steppers = [Stepper {
        id: id.0,
        body: &mut full.body,
        ev: &mut ev.0,
        input,
    }];
    // Map logic predicted here (a portal, a pane that breaks), and the sounds it asks for.
    let (mut scores, mut out) = (Default::default(), Vec::new());
    let mut run = MapRun {
        logic: &mut *map.spec.logic,
        server: false,
        apply: false,
        me: Some(id.0),
        scores: &mut scores,
        out: &mut out,
    };
    tick_bodies_with(
        &mut TickScratch::default(),
        &mut map.world,
        t,
        &mut steppers,
        &ahead,
        &drawn,
        &mut run,
    );
    if let Some(mut hits) = hits {
        write_hits(
            &mut hits,
            k,
            id.0,
            rollback.is_some(),
            Some(lag),
            &full.body,
            &ev.0,
            &extra,
        );
    }
    if rollback.is_none() {
        cues.write_batch(out.iter().filter_map(|o| match o {
            MapOut::Sfx(s) => Some(Cue::Sfx((*s).into())),
            _ => None,
        }));
    }
    predict_respawn(map, id.0, full);
    if !map.gone(id.0) {
        map.spec.logic.bean(id.0, &mut full.body, t);
    }
    let snapped = full.teleports != teleports || full.body.in_portal();
    smooth.predicted(k, &full.body, rollback.is_some(), snapped);
    stats.ticks += 1;
    if rollback.is_none() {
        let (b, e) = (&full.body, &ev.0);
        let said = [
            (e.jumped, Cue::Jumped),
            (!was_dive && b.state == BodyState::Dive, Cue::Dived),
            // (Into a portal: the same spring.)
            (e.bounced || e.portal_in, Cue::Bounced),
            (e.knocked, Cue::Knocked),
            ((e.stunned && !e.knocked) || e.tackles > 0, Cue::Hit),
            (e.bumped > 5.0, Cue::Bumped(e.bumped as f32)),
            (
                !was_grounded && b.grounded && b.land_impact > 0.3,
                Cue::Landed(b.land_impact as f32),
            ),
        ];
        cues.write_batch(said.into_iter().filter(|(on, _)| *on).map(|(_, c)| c));
    }
    if let Some(trace) = &mut trace {
        write_trace(trace, timeline.tick(), &session, id.0, frame, &full.body);
    }
}

/// The `--trace` line of the own bean at `tick`.
fn write_trace(trace: &mut Trace, tick: Tick, session: &Session, id: u32, frame: InputFrame, b: &Body) {
    if core::mem::take(&mut trace.fresh) {
        let _ = writeln!(trace.out, "R {}", tick.0);
    }
    let _ = writeln!(
        trace.out,
        "C {} {} {} {} {} {} {:.6} {:.6} {:.6}",
        tick.0,
        session.room.as_deref().unwrap_or("?"),
        id,
        frame.mx,
        frame.mz,
        frame.buttons,
        b.pos.x,
        b.pos.y,
        b.pos.z
    );
}

/// `--trace-hits` lines of predicted tick k.
#[allow(clippy::too_many_arguments)]
fn write_hits(
    hits: &mut HitTrace,
    k: i64,
    id: u32,
    replay: bool,
    view: Option<u32>,
    b: &Body,
    ev: &StepEvents,
    extra: &[OtherBody],
) {
    let c = if replay { 'c' } else { 'C' };
    let v3 = |x: f64, y: f64, z: f64| format!("{x:.2},{y:.2},{z:.2}");
    // A replay that ends elsewhere than the prediction it replaces: the correction the drawing jumps by.
    let now = (b.pos, b.vel, b.yaw);
    if let Some((p, v, y)) = hits.pred.insert(k, now).filter(|_| replay) {
        let (dp, dv) = (b.pos - p, b.vel - v);
        let dy = (b.yaw - y).sin().atan2((b.yaw - y).cos());
        if dp.length() > 1e-3 || dv.length() > 1e-2 || dy.abs() > 1e-3 {
            let _ = writeln!(
                hits.out,
                "c {k} {id} corr dpos={} dvel={} dyaw={dy:.3}",
                v3(dp.x, dp.y, dp.z),
                v3(dv.x, dv.y, dv.z)
            );
        }
    }
    while hits.pred.len() > 256 {
        hits.pred.pop_first();
    }
    // (It swings a tick either way as frames and ticks beat: only a real change.)
    if let Some(v) = view
        && !replay
        && hits.view.is_none_or(|w| v.abs_diff(w) >= 2)
    {
        hits.view = view;
        let _ = writeln!(hits.out, "{c} {k} {id} view {v}");
    }
    for n in &ev.notes {
        if replay || hits.quiet.fresh(k, id, n) {
            let _ = writeln!(hits.out, "{c} {k} {id} {n}");
        }
    }
    if ev.knocked {
        let _ = writeln!(
            hits.out,
            "{c} {k} {id} state {:?} v={}",
            b.state,
            v3(b.vel.x, b.vel.y, b.vel.z)
        );
    }
    if beans::tackling(b) {
        let _ = writeln!(
            hits.out,
            "{c} {k} {id} {:?} pos={} v={}",
            b.state,
            v3(b.pos.x, b.pos.y, b.pos.z),
            v3(b.vel.x, b.vel.y, b.vel.z)
        );
        let cb = beans::Capsule::of_body(b);
        for o in extra.iter().filter(|o| m::hypot(o.x - b.pos.x, o.z - b.pos.z) <= 4.0) {
            let _ = writeln!(
                hits.out,
                "{c} {k} {id}   near {} drawn={} gap={:.2}",
                o.id,
                v3(o.x, o.y, o.z),
                beans::gap(&cb, &beans::Capsule::of_other(o))
            );
        }
    }
    let _ = hits.out.flush();
}

/// The arena's rules for the own bean that need nobody else: the checkpoint it reached, and where a fall
/// puts it back (the server would say so only a round trip later, after a rollback or three). A fall that
/// puts it out of a survival round, a shortcut and the lobby's free spawn stay the server's.
fn predict_respawn(map: &Map, id: u32, full: &mut BodyFull) {
    let (kind, fall) = (map.round.kind, map.round.fall);
    let mut checkpoint = full
        .checkpoint
        .map(usize::from)
        .filter(|&c| c < map.spec.checkpoints.len());
    reach_checkpoint(&map.spec, &full.body, &mut checkpoint);
    full.checkpoint = checkpoint.map(|c| c as u16);
    if map.gone(id) || !fell(&map.spec, full.body.pos) || (kind == ArenaKind::Round && fall == FallBehaviour::Out) {
        return;
    }
    if let Some(to) = respawn_point(&map.spec, kind, fall, checkpoint, full.spawn.into()) {
        respawn(&map.spec, id, &mut full.body, to);
        full.teleports += 1;
    }
}

/// Map events as they arrive. They may come before the arena they belong to is replicated (the history
/// sent on join, or a round that just started), so they wait here (a few seconds at most) until that
/// arena's map is built.
#[derive(Resource, Default)]
struct Inbox(Vec<(MapEventMsg, f64)>);

/// Seconds an event may wait for its arena; for the arena the player is in or one replicated, whose map is
/// still to be built (a slow machine, a big map), longer.
const INBOX_WAIT: f64 = 5.0;
const INBOX_WAIT_BUILDING: f64 = 60.0;

fn receive_map_events(
    mut receivers: Query<&mut MessageReceiver<MapEventMsg>>,
    mut inbox: ResMut<Inbox>,
    mut stats: ResMut<Stats>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    for mut r in &mut receivers {
        for msg in r.receive() {
            inbox.0.push((msg, now));
            stats.map_events += 1;
        }
    }
}

/// Bonuses about the own bean apply at once (it is drawn ahead), the others' when they are drawn at their
/// tick. The rest apply as they come: a map event changes the world the own bean is predicted in
/// (a tile falls, a portal shuts), and a bean that finished or is out leaves the arena.
fn apply_map_events(
    map: Option<ResMut<Map>>,
    mut inbox: ResMut<Inbox>,
    interp: Option<Res<InterpolationTimeline>>,
    own: Query<&PlayerId, With<Predicted>>,
    rounds: Query<&Round>,
    mut session: ResMut<Session>,
    mut feed: ResMut<FeedLog>,
    time: Res<Time<Real>>,
    mut cues: MessageWriter<Cue>,
) {
    let now = time.elapsed_secs_f64();
    let Some(mut map) = map else {
        if !inbox.0.is_empty() {
            inbox.0.retain(|(_, at)| now - at < INBOX_WAIT_BUILDING);
        }
        return;
    };
    // (Nothing to do: neither `Map` nor `Session` is touched, so neither reads as changed.)
    if inbox.0.is_empty() && map.pending.is_empty() {
        return;
    }
    let map = &mut *map;
    let arena = map.round.arena;
    let real = time.elapsed_secs();
    let building = |a: u32| session.arena.as_ref().is_some_and(|s| s.id == a) || rounds.iter().any(|r| r.arena == a);
    // This arena's events go on; others wait a while for their map.
    inbox.0.retain(|(msg, at)| {
        if msg.arena == arena {
            map.pending.push(msg.clone());
            return false;
        }
        let age = now - at;
        age < INBOX_WAIT || (age < INBOX_WAIT_BUILDING && building(msg.arena))
    });
    if map.pending.is_empty() {
        return;
    }
    let me = own.single().ok().map(|p| p.0);
    let now = interp.map(|t| t.tick().0);
    let mut out = Vec::new();
    let pending = core::mem::take(&mut map.pending);
    for msg in pending {
        if map.seen.contains(&msg) {
            continue;
        }
        // (History: an event from before the player came in. It changes the world, but makes no sound.)
        let loud = !msg.history;
        match &msg.ev {
            MapEventKind::Bonus { i, id, at } => {
                if Some(*id) == me || now.is_none_or(|n| n >= msg.tick) {
                    // (Taken already: the history sent again after a reconnect says nothing new.)
                    let taken = map.bonuses.on_event(BonusTaken {
                        i: *i,
                        id: *id,
                        at: *at,
                    });
                    if let Some(b) = taken.filter(|_| loud) {
                        cues.write(Cue::Bonus(*id));
                        let who = (Some(*id) != session.me).then(|| session.name_of(*id));
                        feed.note(real, crate::ui::text::bonus_note(b.kind, who.as_deref()));
                    }
                } else {
                    map.pending.push(msg);
                    continue;
                }
            }
            MapEventKind::Finish { id, place, time } => {
                info!("player {id} finished, place {place} in {time:.2} s");
                if !map.info.finished.contains(id) {
                    map.info.finished.push(*id);
                    if loud {
                        cues.write(Cue::Finish(*id));
                        let line = crate::ui::text::finish_note(&session.name_of(*id), *place as usize, *time);
                        feed.note(real, line);
                    }
                }
            }
            MapEventKind::Ko {
                id,
                out,
                by,
                cause,
                shortcut,
            } => {
                if *out {
                    info!("player {id} is out ({cause})");
                    if map.info.out.contains(id) {
                        map.seen.insert(&msg);
                        continue;
                    }
                    map.info.out.push(*id);
                }
                if loud {
                    cues.write(Cue::Ko { id: *id, out: *out });
                    if map.round.kind == ArenaKind::Round {
                        let what = Feed::Ko {
                            victim: *id,
                            by: *by,
                            cause: *cause,
                            out: *out,
                            shortcut: *shortcut,
                        };
                        feed.push(real, what);
                    }
                }
            }
            MapEventKind::Map(ev) => {
                let mut said = Vec::new();
                // (`Session` reads as changed only when the scores do: the HUD redraws on it.)
                let mut scores = session.scores.clone();
                client_event(&mut map.world, &mut map.spec, &mut scores, session.me, ev, &mut said);
                if scores != session.scores {
                    session.scores = scores;
                }
                if !loud {
                    said.retain(|o| !matches!(o, MapOut::Sfx(_)));
                }
                out.extend(said);
            }
        }
        map.seen.insert(&msg);
    }
    map.take_out(out);
}

/// The sounds map events asked for.
fn map_sounds(map: Option<ResMut<Map>>, mut cues: MessageWriter<Cue>) {
    if let Some(mut map) = map
        && !map.sfx.is_empty()
    {
        cues.write_batch(core::mem::take(&mut map.sfx).into_iter().map(|s| Cue::Sfx(s.into())));
    }
}

const EMOTE_KEYS: [KeyCode; 5] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
];
/// The pad's cross: emotes 1–4.
const EMOTE_PAD: [GamepadButton; 4] = [
    GamepadButton::DPadUp,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
    GamepadButton::DPadDown,
];

/// Keys 1–5 and the pad's cross: an emote, while the player has a bean.
fn send_emotes(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    pads: Query<&Gamepad>,
    own: Query<(), (With<Predicted>, With<PlayerId>)>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
    gate: Res<Gate>,
) {
    if own.is_empty() || !gate.play {
        return;
    }
    let key = keys.and_then(|k| EMOTE_KEYS.iter().position(|c| k.just_pressed(*c)));
    let pad = pads
        .iter()
        .find_map(|p| EMOTE_PAD.iter().position(|b| p.just_pressed(*b)));
    if let Some(i) = key.or(pad) {
        crate::session::send(&mut senders, ClientMsg::Emote(i as u8 + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(z: f64) -> Body {
        let mut b = Body::new(1);
        b.pos.z = z;
        b
    }

    #[test]
    fn a_correction_is_drawn_away_and_fades() {
        let mut s = Smoothing::default();
        s.predicted(10, &at(1.0), false, false);
        s.predicted(10, &at(0.8), true, false);
        s.frame(0.0);
        assert!((s.pos.z - 0.2).abs() < 1e-6, "{}", s.pos.z);
        for _ in 0..30 {
            s.frame(1.0 / 60.0);
        }
        assert!(s.pos.z.abs() < 0.01, "{}", s.pos.z);
    }

    #[test]
    fn a_respawn_or_a_far_correction_is_not_smoothed() {
        let mut s = Smoothing::default();
        s.predicted(10, &at(1.0), false, false);
        s.predicted(10, &at(9.0), true, false);
        s.frame(0.0);
        assert_eq!(s.pos, Vec3::ZERO);
        s.predicted(11, &at(1.0), false, false);
        s.predicted(11, &at(0.9), true, true);
        s.frame(0.0);
        assert_eq!(s.pos, Vec3::ZERO);
    }
}
