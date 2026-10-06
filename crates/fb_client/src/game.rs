//! The client's copy of the round: the same map built from the seed, own-bean prediction, map events,
//! and the input it sends.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufWriter, Write};

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use fb_arena::{
    ArenaKind, FallBehaviour, Stepper, build_map, can_move, client_event, client_start, fell, reach_checkpoint,
    respawn, respawn_point, tick_bodies, touch_hook,
};
use fb_net::*;
use fb_proto::{ArenaInfo, Pid};
use fb_shared::DT;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::map::{BeanDeco, MapOut, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::Nodes;
use fb_sim::physics::{BodyInput, BodyState, OtherBody, StepEvents};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;
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
use crate::session::{Feed, Session};

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
    /// Map events applied to this arena (tick, digest): the history the room sends again after a reconnect
    /// is not applied a second time.
    pub seen: BTreeSet<(u32, u64)>,
    pub generation: u32,
    /// Who plays this arena, and who of them finished (in order) or is out.
    pub info: ArenaInfo,
    /// Decorations maps put on beans (tails, badges).
    pub deco: BTreeMap<Pid, BeanDeco>,
    /// Sounds the map asked for, for the audio to take.
    pub sfx: Vec<&'static str>,
    /// How this round looks (`fb_sim::looks`): palettes, sky, light, fog, scenery.
    pub look: fb_sim::looks::ResolvedLook,
    /// The room it was built in: every room's first lobby is arena 1 with seed 1, another room's is not this.
    room: Option<String>,
    /// The others as the own bean met them at each predicted tick: a rollback replays a tick against the same
    /// bodies, not against where they are drawn now.
    others: BTreeMap<i64, Vec<OtherBody>>,
}

/// Ticks of the others kept (`PredictionManager::max_rollback_ticks` and a little).
const OTHERS_KEPT: usize = 128;

impl Map {
    pub fn time(&self, tick: f64) -> f64 {
        (tick - self.round.zero_tick as f64) * DT
    }

    /// Finished or out: the bean has left the arena.
    pub fn gone(&self, id: Pid) -> bool {
        self.info.finished.contains(&id) || self.info.out.contains(&id)
    }

    fn take_out(&mut self, out: Vec<MapOut>) {
        for o in out {
            match o {
                MapOut::Sfx(s) => self.sfx.push(s),
                MapOut::Decorate { id, deco } => {
                    let d = self.deco.entry(id).or_default();
                    if deco.tail.is_some() {
                        d.tail = deco.tail;
                    }
                    if deco.badge.is_some() {
                        d.badge = deco.badge;
                    }
                }
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
    Sfx(&'static str),
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

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraAngles { yaw: 0.0, pitch: 0.32 });
        app.init_resource::<Stats>();
        app.init_resource::<Inbox>();
        app.init_resource::<Presses>();
        app.init_resource::<Gate>();
        app.add_message::<Cue>();
        app.add_systems(Startup, open_trace);
        app.add_systems(
            PreUpdate,
            (
                (drop_map, build_round).chain(),
                latch_presses.after(bevy::input::InputSystems),
            ),
        );
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
    if let Some(path) = &opts.trace {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace {}: {e}", path.display()));
        commands.insert_resource(Trace {
            out: BufWriter::new(file),
            fresh: true,
        });
    }
}

/// Out of the room (at the room list, on another server): its map goes, and with it what the next room would
/// take for its own (players, bonuses, decorations). The scene stays drawn until the next map's replaces it.
fn drop_map(map: Option<Res<Map>>, session: Res<Session>, mut commands: Commands) {
    if map.is_some() && session.room.is_none() {
        commands.remove_resource::<Map>();
    }
}

/// Builds the arena's map once both the replicated `Round` and its `ArenaInfo` (who plays: maps deal
/// tails and stars from it) are in.
fn build_round(
    mut commands: Commands,
    rounds: Query<&Round>,
    map: Option<ResMut<Map>>,
    mut session: ResMut<Session>,
    mut stats: ResMut<Stats>,
    mut unknown: Local<Option<u32>>,
    mut generations: Local<u32>,
) {
    // (Out of the room, its arena may linger a moment: `drop_map`.)
    if session.room.is_none() {
        return;
    }
    // The arena the player is in: the last one's `Round` may linger a moment beside it.
    let in_arena = |r: &&Round| session.arena.as_ref().is_some_and(|a| a.id == r.arena);
    let Some(round) = rounds.iter().find(in_arena) else {
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
    b.world.finalize(-1e3);
    let static_hash = b.world.hash(true);
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
    let generation = *generations;
    *generations = generation.wrapping_add(1);
    let mut next = Map {
        round: round.clone(),
        world: b.world,
        spec,
        scene: b.scene.unwrap_or_default(),
        bonuses,
        render,
        static_hash,
        pending: Vec::new(),
        seen: BTreeSet::new(),
        generation,
        info,
        deco: BTreeMap::new(),
        sfx: Vec::new(),
        look: fb_sim::looks::look_for(def.looks(), round.seed),
        room: session.room.clone(),
        others: BTreeMap::new(),
    };
    next.take_out(out);
    commands.insert_resource(next);
}

fn on_controlled(trigger: On<Add, Controlled>, mut commands: Commands, pawns: Query<(), With<PlayerId>>) {
    if pawns.get(trigger.entity).is_ok() {
        commands.entity(trigger.entity).insert((
            InputMarker::<FbInput>::default(),
            PrevPos::default(),
            OwnEvents::default(),
        ));
    }
}

/// A slow circle with a hop now and then, and a run for any bonus lying about: exercises prediction and
/// the map event path without a player. Returns (forward, right, buttons) or a world-space stick.
fn autopilot(k: u32, map: Option<&Map>, own: Option<(&PlayerId, &BodyFull)>) -> Result<(f64, f64, u8), InputFrame> {
    // Each player on its own rhythm, so a room of autopilots does not move in lockstep.
    let id = own.map_or(0, |(id, _)| id.0);
    let p = k + id * 53;
    let buttons = if p % (89 + id % 13) < 2 { BTN_JUMP } else { 0 };
    if let (Some(map), Some((_, own))) = (map, own) {
        let p = own.body.pos;
        if let Some(b) = map.bonuses.available(map.time(k as f64)).next() {
            let (dx, dz) = (b.pos.x - p.x, b.pos.z - p.z);
            let l = (dx * dx + dz * dz).sqrt().max(1e-6);
            return Err(InputFrame::from_stick(dx / l, dz / l, buttons));
        }
    }
    let turn = if (p / (200 + 17 * (id % 5))).is_multiple_of(2) {
        0.6
    } else {
        -0.6
    };
    Ok((1.0, turn, buttons))
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
    if keys.any_just_pressed(binds.keys(Bind::Jump)) {
        presses.0 |= BTN_JUMP;
    }
    // The click that captures the mouse is not a dive.
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if keys.any_just_pressed(binds.keys(Bind::Dive)) || (captured && mouse.just_pressed(MouseButton::Left)) {
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
    let held = |b: Bind| binds.keys(b).iter().any(|k| keys.pressed(*k));
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
) {
    // A rollback replays the buffered input over what is written here: a press taken now would be lost.
    if rollback.is_some() {
        return;
    }
    // Taken by the first tick after the press (or dropped while there is no bean to press it).
    let pressed = core::mem::take(&mut presses.0);
    let Ok((mut state, own)) = q.single_mut() else { return };
    if let Some(p) = probe.as_mut().filter(|p| time.elapsed_secs_f64() < p.until) {
        state.0 = p.frame.into();
        p.frame.buttons &= BTN_GRAB;
        return;
    }
    let (f, r, buttons) = if opts.autopilot() {
        match autopilot(timeline.tick().0, map.as_deref(), own) {
            Ok(v) => v,
            Err(frame) => {
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
        ),
        With<Predicted>,
    >,
    others: Query<(&PlayerId, &RemotePose), (With<Interpolated>, Without<Predicted>)>,
    trace: Option<ResMut<Trace>>,
    session: Res<Session>,
    link: Query<(), (With<Client>, With<Connected>)>,
    rollback: Option<Res<Rollback>>,
    mut cues: MessageWriter<Cue>,
) {
    let Some(mut map) = map else { return };
    let Ok((id, mut full, state, mut ev, mut prev)) = own.single_mut() else {
        return;
    };
    let k = map.round.arena_tick(timeline.tick());
    let t = k as f64 * DT;
    let frame = InputFrame::from(state.0);
    let input: BodyInput = if can_move(map.round.kind, t) {
        frame.into()
    } else {
        BodyInput::default()
    };
    // The others as drawn, the first time this tick is predicted; a rollback's replay meets them where they
    // were then. As on the server, a bean that finished or is out (or is in a portal) is in nobody's way.
    let map = &mut *map;
    let replayed = rollback.is_some().then(|| map.others.get(&k).cloned()).flatten();
    let extra = match replayed {
        Some(extra) => extra,
        None => {
            let mut extra = if map.others.len() >= OTHERS_KEPT {
                map.others.pop_first().map(|(_, v)| v).unwrap_or_default()
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
                        vz: p.vel.y as f64,
                        touching: false,
                        size: p.size as f64,
                    }),
            );
            // (By id: the query's order may change between a tick and its replay.)
            extra.sort_by_key(|o| o.id);
            map.others.insert(k, extra.clone());
            extra
        }
    };
    let full = &mut *full;
    // A body from the previous arena may stand on a collider this world does not have.
    if full.body.ground_col >= map.world.colliders.len() as i32 {
        full.body.ground_col = -1;
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
    let mut touch = touch_hook(&mut map.spec.touches, false, t, Some(id.0), &mut scores, &mut out);
    tick_bodies(&mut map.world, t, &mut steppers, &extra, &mut touch);
    drop(touch);
    if rollback.is_none() {
        cues.write_batch(out.iter().filter_map(|o| match o {
            MapOut::Sfx(s) => Some(Cue::Sfx(s)),
            _ => None,
        }));
    }
    predict_respawn(map, id.0, full);
    stats.ticks += 1;
    if rollback.is_none() {
        let (b, e) = (&full.body, &ev.0);
        let said = [
            (e.jumped, Cue::Jumped),
            (!was_dive && b.state == BodyState::Dive, Cue::Dived),
            // (Into a portal: the same spring.)
            (e.bounced || e.portal_in, Cue::Bounced),
            (e.knocked, Cue::Knocked),
            (e.stunned && !e.knocked, Cue::Hit),
            (e.bumped > 5.0, Cue::Bumped(e.bumped as f32)),
            (
                !was_grounded && b.grounded && b.land_impact > 0.3,
                Cue::Landed(b.land_impact as f32),
            ),
        ];
        cues.write_batch(said.into_iter().filter(|(on, _)| *on).map(|(_, c)| c));
    }
    if let Some(mut trace) = trace
        && !link.is_empty()
    {
        let b = &full.body;
        if core::mem::take(&mut trace.fresh) {
            let _ = writeln!(trace.out, "R {}", timeline.tick().0);
        }
        let _ = writeln!(
            trace.out,
            "C {} {} {} {} {} {} {:.6} {:.6} {:.6}",
            timeline.tick().0,
            session.room.as_deref().unwrap_or("?"),
            id.0,
            frame.mx,
            frame.mz,
            frame.buttons,
            b.pos.x,
            b.pos.y,
            b.pos.z
        );
    }
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
    let session = &mut *session;
    let mut out = Vec::new();
    let pending = core::mem::take(&mut map.pending);
    for msg in pending {
        let key = (msg.tick, crate::ui::key_of(&msg.ev));
        if map.seen.contains(&key) {
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
                        let (icon, title) = crate::ui::text::bonus(b.kind);
                        let who = if Some(*id) == session.me {
                            "Вы".to_string()
                        } else {
                            session.name_of(*id)
                        };
                        session.note(real, format!("{icon} {who}: {title}!"));
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
                        session.note(real, line);
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
                        map.seen.insert(key);
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
                            cause: cause.clone(),
                            out: *out,
                            shortcut: *shortcut,
                        };
                        session.push_feed(real, what);
                    }
                }
            }
            MapEventKind::Map(ev) => {
                let mut said = Vec::new();
                client_event(
                    &mut map.world,
                    &mut map.spec,
                    &mut session.scores,
                    session.me,
                    ev,
                    &mut said,
                );
                if !loud {
                    said.retain(|o| !matches!(o, MapOut::Sfx(_)));
                }
                out.extend(said);
            }
        }
        map.seen.insert(key);
    }
    map.take_out(out);
}

/// The sounds map events asked for.
fn map_sounds(map: Option<ResMut<Map>>, mut cues: MessageWriter<Cue>) {
    if let Some(mut map) = map
        && !map.sfx.is_empty()
    {
        cues.write_batch(core::mem::take(&mut map.sfx).into_iter().map(Cue::Sfx));
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
