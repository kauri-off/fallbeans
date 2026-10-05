//! The rooms on the network: the hub as a resource, fed with connections, messages and inputs from Lightyear,
//! and its rooms' arenas published as replicated entities (a `Round` per room, a pawn per bean in play), each
//! visible only to the links in that room.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Duration;

use bevy::prelude::*;
use fb_arena::PawnStatus;
use fb_net::*;
use fb_proto::Pid;
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_shared::{INPUT_HOLD, TICK_RATE};
use lightyear::connection::client::Disconnecting;
use lightyear::input::native::prelude::{ActionState, NativeStateSequence};
use lightyear::input::server::{InputValidationAppExt, authorize_controlled_targets};
use lightyear::prelude::input::InputBuffer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::http::HttpShared;
use crate::opts::Opts;
use crate::rooms::hub::Hub;
use crate::rooms::room::RoomOptions;
use crate::rooms::{ConnId, Inputs, Out, ticks};

/// A bean in play in a room (`RoomTag`, `PlayerId`).
#[derive(Component, Clone, Copy, Debug)]
pub struct Pawn;

/// The room tick, for systems that measure or follow it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoomTick;

struct PawnEntity {
    entity: Entity,
    owner: Option<ConnId>,
}

#[derive(Resource)]
pub struct Rooms {
    pub hub: Hub,
    pawns: BTreeMap<(u32, Pid), PawnEntity>,
    rounds: BTreeMap<u32, Entity>,
    /// The room each link is in, as last told to Replicon.
    in_room: BTreeMap<ConnId, u32>,
    /// Links to let go of once what was sent to them has gone out (server time, s).
    closing: Vec<(Entity, f64)>,
}

/// `--trace`: one line per pawn per tick, `S tick room id mx mz buttons x y z`.
#[derive(Resource)]
struct Trace(BufWriter<File>);

pub struct PlayPlugin;

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start);
        // Lightyear's copy of the input buffer into `ActionState` would also drop every tick before
        // the current one, and with them any input that arrives late: a late press would be lost.
        // The room reads the buffer itself (`frame_for`); the buffer is a ring of 64 ticks.
        app.configure_sets(
            FixedPreUpdate,
            lightyear::input::server::InputSystems::UpdateActionState.run_if(|| false),
        );
        // A client's input goes only into the pawn it controls (`ControlledBy`): Lightyear writes an input
        // message into whatever entity it names, so a modified client could drive another player's bean
        // (or hang input buffers on any entity). The check is opt-in in Lightyear 0.30.
        app.add_input_validator(authorize_controlled_targets::<NativeStateSequence<FbInput>>);
        app.add_systems(FixedUpdate, (tick_rooms.in_set(RoomTick), watch_inputs.after(RoomTick)));
        // Every frame: Lightyear drops the messages nobody read in the frame they came in, and half the
        // frames run no tick.
        app.add_systems(PreUpdate, receive.after(MessageSystems::Receive));
        app.add_systems(Update, (close_links, measure_rtt, watch_update_flag));
    }
}

fn start(mut commands: Commands, opts: Res<Opts>, timeline: Res<LocalTimeline>) {
    let base = RoomOptions {
        min_players: if opts.solo { 1 } else { 2 },
        seed: opts.seed,
        intro_ticks: ticks(opts.intro) as u32,
        dev: opts.dev,
        eliminate: !opts.respawn,
        ..default()
    };
    let mut hub = Hub::new(base, u64::from(timeline.tick().0));
    for id in &opts.open_rooms {
        hub.open_permanent(id, id);
    }
    commands.insert_resource(Rooms {
        hub,
        pawns: BTreeMap::new(),
        rounds: BTreeMap::new(),
        in_room: BTreeMap::new(),
        closing: Vec::new(),
    });
    if let Some(path) = &opts.trace {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace {}: {e}", path.display()));
        commands.insert_resource(Trace(BufWriter::new(file)));
    }
}

pub fn conn_of(link: Entity) -> ConnId {
    link.to_bits()
}

fn link_of(conn: ConnId) -> Entity {
    Entity::from_bits(conn)
}

type InputBuf = InputBuffer<ActionState<FbInput>, FbInput>;

/// Jump and dive: one tick per press (the client sends them so).
const PRESSES: u8 = BTN_JUMP | BTN_DIVE;
/// Input gaps shorter than this are not logged, ticks (0.2 s).
const GAP_MIN: u32 = TICK_RATE / 5;
/// After an `input gap` line, a player's further gaps are only counted for this long, s.
const QUIET_S: u32 = 10;
/// A gap still going is logged every this many seconds.
const STILL_S: u32 = 5;

/// What the room keeps of a player's input between ticks.
#[derive(Component, Debug)]
pub struct InputState {
    /// The frame used last tick: carried on while input is missing.
    last: InputFrame,
    /// Last tick that had the player's input in time.
    ack: u32,
    /// Presses that arrived after their tick: they happen on the next one.
    late: u8,
    /// Newest tick looked at for late presses.
    late_seen: u32,
    /// What the last tick used (`--trace`).
    used: Option<(u32, InputFrame)>,
    /// Ticks run without the player's input since the last `metrics:` line (after their first input).
    pub missed: u32,
    /// The current run of ticks without input, and how far the newest input was behind the tick at worst.
    gap: u32,
    behind: u32,
    /// A run just over: (ticks, worst behind), for `watch_inputs`.
    ended: Option<(u32, u32)>,
    /// No `input gap` line before this tick; runs in between are only counted (number, ticks).
    quiet_until: u32,
    hushed: (u32, u32),
}

impl InputState {
    pub fn new(now: Tick) -> Self {
        Self {
            last: InputFrame::IDLE,
            ack: now.0,
            late: 0,
            late_seen: now.0,
            used: None,
            missed: 0,
            gap: 0,
            behind: 0,
            ended: None,
            quiet_until: 0,
            hushed: (0, 0),
        }
    }
}

/// The input for `tick`, read from the buffer directly (see `PlayPlugin` for why). Without one in time the
/// pawn keeps its stick (and grab) for INPUT_HOLD ticks, never repeating a jump or dive; a press that
/// arrives late is not lost but happens now.
fn frame_for(tick: Tick, buffer: Option<&InputBuf>, st: &mut InputState) -> InputFrame {
    let k = tick.0;
    if let Some(b) = buffer {
        let buttons = |j: u32| b.get(Tick(j)).map(|s| InputFrame::from(s.0).buttons);
        for j in st.ack.max(st.late_seen).max(k.saturating_sub(LATE_TICKS)) + 1..k {
            let Some(now) = buttons(j) else { continue };
            // Only a new press: Lightyear fills a gap with the tick before it, a held bit is that copy.
            st.late |= now & !buttons(j - 1).unwrap_or(0) & PRESSES;
            st.late_seen = j;
        }
    }
    let input = buffer.and_then(|b| b.get(tick));
    if let Some(b) = buffer {
        if input.is_none() {
            st.missed = st.missed.saturating_add(1);
            st.gap += 1;
            let newest = b.end_tick().map_or(0, |t| t.0);
            st.behind = st.behind.max(k.saturating_sub(newest));
        } else if st.gap > 0 {
            st.ended = Some((core::mem::take(&mut st.gap), core::mem::take(&mut st.behind)));
        }
    }
    let frame = match input {
        Some(s) => {
            st.ack = k;
            s.0.into()
        }
        None if k.saturating_sub(st.ack) > INPUT_HOLD => InputFrame::IDLE,
        None => InputFrame {
            buttons: st.last.buttons & !PRESSES,
            ..st.last
        },
    };
    st.last = frame;
    InputFrame {
        buttons: frame.buttons | core::mem::take(&mut st.late),
        ..frame
    }
}

/// The server's side of a bean that snaps back: runs of ticks it had to play without the player's input.
/// `behind`: how many ticks the newest input that had come was behind the tick at worst (0: later ticks' input
/// came, these were lost; a few: input comes, but late; as long as the gap: nothing came at all).
fn watch_inputs(timeline: Res<LocalTimeline>, rooms: Res<Rooms>, mut inputs: Query<&mut InputState, With<Pawn>>) {
    let now = timeline.tick().0;
    let rate = TICK_RATE;
    let ms = |ticks: u32| ticks * 1000 / rate;
    for (&(key, id), pe) in &rooms.pawns {
        let Ok(mut st) = inputs.get_mut(pe.entity) else {
            continue;
        };
        let st = &mut *st;
        let room = rooms.hub.rooms.get(&key).map_or("?", |r| r.id.as_str());
        if st.gap > 0 && st.gap % (STILL_S * rate) == 0 {
            warn!(
                room,
                id,
                secs = st.gap / rate,
                behind = st.behind,
                "still no input from the player"
            );
        }
        if let Some((ticks, behind)) = st.ended.take().filter(|&(t, _)| t >= GAP_MIN) {
            if now < st.quiet_until {
                st.hushed.0 += 1;
                st.hushed.1 += ticks;
            } else {
                warn!(room, id, ms = ms(ticks), behind, "input gap");
                st.quiet_until = now + QUIET_S * rate;
            }
        }
        if st.hushed.0 > 0 && now >= st.quiet_until {
            let (n, ticks) = core::mem::take(&mut st.hushed);
            warn!(room, id, n, ms = ms(ticks), "more input gaps in {QUIET_S} s");
            st.quiet_until = now + QUIET_S * rate;
        }
    }
}

/// Inputs from the pawn entities' buffers (Lightyear writes a client's input into the entity it controls).
struct LinkInputs<'a, 'w, 's> {
    owners: &'a BTreeMap<ConnId, Entity>,
    pawns: &'a mut Query<'w, 's, (Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
}

impl Inputs for LinkInputs<'_, '_, '_> {
    fn frame(&mut self, _: Pid, conn: ConnId, tick: u32) -> InputFrame {
        let Some(&e) = self.owners.get(&conn) else {
            return InputFrame::IDLE;
        };
        let Ok((buffer, mut st)) = self.pawns.get_mut(e) else {
            return InputFrame::IDLE;
        };
        let f = frame_for(Tick(tick), buffer, &mut st);
        st.used = Some((tick, f));
        f
    }
}

type Bodies = (
    &'static Pawn,
    &'static mut BodyFull,
    &'static mut RemotePose,
    &'static mut Hold,
);

fn tick_rooms(
    timeline: Res<LocalTimeline>,
    mut rooms: ResMut<Rooms>,
    mut commands: Commands,
    mut control: Query<&mut MessageSender<ServerMsg>>,
    mut events: Query<&mut MessageSender<MapEventMsg>>,
    mut inputs: Query<(Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
    mut bodies: Query<Bodies>,
    mut round_q: Query<&mut Round>,
    remotes: Query<&RemoteId, With<ClientOf>>,
    time: Res<Time<Real>>,
    trace: Option<ResMut<Trace>>,
) {
    let tick = timeline.tick();
    let rooms = &mut *rooms;
    let owners: BTreeMap<ConnId, Entity> = rooms
        .pawns
        .values()
        .filter_map(|p| Some((p.owner?, p.entity)))
        .collect();
    rooms.hub.update(
        u64::from(tick.0),
        &mut LinkInputs {
            owners: &owners,
            pawns: &mut inputs,
        },
    );
    if let Some(mut trace) = trace {
        write_trace(&mut trace, rooms, &mut inputs, tick);
    }
    for out in rooms.hub.take_out() {
        match out {
            Out::Msg(c, m) => {
                if let Ok(mut s) = control.get_mut(link_of(c)) {
                    s.send::<ControlChannel>(m);
                }
            }
            Out::Event(c, e) => {
                if let Ok(mut s) = events.get_mut(link_of(c)) {
                    s.send::<MapEventsChannel>(e);
                }
            }
            Out::Close(c) => {
                let at = time.elapsed_secs_f64() + 0.5;
                rooms.closing.push((link_of(c), at));
            }
        }
    }
    publish(rooms, &mut commands, &mut bodies, &mut round_q, &remotes, tick);
}

fn receive(mut rooms: ResMut<Rooms>, mut receivers: Query<(Entity, &mut MessageReceiver<ClientMsg>), With<ClientOf>>) {
    for (link, mut r) in &mut receivers {
        for msg in r.receive() {
            rooms.hub.message(conn_of(link), msg);
        }
    }
}

fn write_trace(
    trace: &mut Trace,
    rooms: &Rooms,
    inputs: &mut Query<(Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
    tick: Tick,
) {
    for (&(key, id), pe) in &rooms.pawns {
        let Ok((_, mut st)) = inputs.get_mut(pe.entity) else {
            continue;
        };
        let Some((t, f)) = st.used.take() else { continue };
        let Some(room) = rooms.hub.rooms.get(&key) else {
            continue;
        };
        let Some(p) = room.arena.pawn(id) else { continue };
        if t != tick.0 {
            continue;
        }
        let b = &p.body;
        let _ = writeln!(
            trace.0,
            "S {} {} {id} {} {} {} {:.6} {:.6} {:.6}",
            tick.0, room.id, f.mx, f.mz, f.buttons, b.pos.x, b.pos.y, b.pos.z
        );
    }
}

/// The rooms' arenas into replicated entities, and each link into its room.
fn publish(
    rooms: &mut Rooms,
    commands: &mut Commands,
    bodies: &mut Query<Bodies>,
    round_q: &mut Query<&mut Round>,
    remotes: &Query<&RemoteId, With<ClientOf>>,
    tick: Tick,
) {
    let Rooms {
        hub,
        pawns,
        rounds,
        in_room,
        ..
    } = rooms;
    // Rooms that closed.
    rounds.retain(|key, e| {
        let keep = hub.rooms.contains_key(key);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let mut live = Vec::new();
    for (&key, room) in &hub.rooms {
        let round = Round {
            arena: room.arena_id,
            kind: room.arena.kind,
            map: room.arena.map.meta().id.into(),
            seed: room.arena.seed,
            zero_tick: room.zero_tick(),
            fall: room.arena.fall,
            static_hash: room.arena.static_hash.clone(),
        };
        match rounds.get(&key).map(|e| round_q.get_mut(*e)) {
            Some(Ok(mut r)) => {
                if *r != round {
                    *r = round;
                }
            }
            Some(Err(_)) => {}
            None => {
                let e = commands
                    .spawn((
                        Name::new(format!("room {}", room.id)),
                        round,
                        RoomTag(key),
                        Replicate::to_clients(NetworkTarget::All),
                    ))
                    .id();
                rounds.insert(key, e);
            }
        }
        for p in room.arena.pawns.iter().filter(|p| p.status == PawnStatus::Play) {
            // (A link already gone counts as nobody: the room hears of it in a moment.)
            let owner = room
                .player(p.id)
                .and_then(|pl| pl.conn)
                .filter(|c| remotes.get(link_of(*c)).is_ok());
            let color = room.player(p.id).map_or(0, |pl| pl.color);
            live.push((key, p.id));
            let full = BodyFull {
                body: p.body.clone(),
                teleports: p.teleports,
                checkpoint: p.checkpoint.map(|c| c as u16),
                spawn: p.spawn_i as u16,
            };
            let hold = Hold {
                target: p.grabbing,
                reaching: p.reaching,
            };
            let entry = pawns.get(&(key, p.id));
            if let Some(pe) = entry.filter(|pe| pe.owner == owner) {
                if let Ok((_, mut f, mut pose, mut h)) = bodies.get_mut(pe.entity) {
                    if *f != full || *h != hold {
                        *pose = RemotePose::of(&full, &hold);
                    }
                    if *f != full {
                        *f = full;
                    }
                    h.set_if_neq(hold);
                }
                continue;
            }
            // New, or someone else controls it now (a reconnect): a new entity, so prediction starts afresh.
            if let Some(old) = entry {
                commands.entity(old.entity).despawn();
            }
            let mut e = commands.spawn((
                Pawn,
                RoomTag(key),
                PlayerId(p.id),
                BeanColor(color),
                InputState::new(tick),
                RemotePose::of(&full, &hold),
                full,
                hold,
                Replicate::to_clients(NetworkTarget::All),
            ));
            match owner.and_then(|c| Some((link_of(c), remotes.get(link_of(c)).ok()?.0))) {
                Some((link, peer)) => {
                    e.insert((
                        PredictionTarget::to_clients(NetworkTarget::Single(peer)),
                        InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(peer)),
                        OwnerOnly(link),
                        OthersOnly(link),
                        ControlledBy {
                            owner: link,
                            lifetime: Lifetime::Persistent,
                        },
                    ));
                }
                None => {
                    e.insert((
                        InterpolationTarget::to_clients(NetworkTarget::All),
                        OwnerOnly(Entity::PLACEHOLDER),
                        OthersOnly(Entity::PLACEHOLDER),
                    ));
                }
            }
            let entity = e.id();
            pawns.insert((key, p.id), PawnEntity { entity, owner });
        }
    }
    live.sort_unstable();
    pawns.retain(|k, pe| {
        let keep = live.binary_search(k).is_ok();
        if !keep {
            commands.entity(pe.entity).despawn();
        }
        keep
    });
    // Each link into the room it is in (Replicon shows it that room's entities only).
    let want: BTreeMap<ConnId, u32> = hub
        .sessions
        .iter()
        .filter_map(|(c, s)| Some((*c, s.member.as_ref().filter(|m| !m.gone)?.room?)))
        .collect();
    for (c, key) in &want {
        if in_room.get(c) != Some(key)
            && let Ok(mut e) = commands.get_entity(link_of(*c))
        {
            e.insert(InRoom(*key));
        }
    }
    for c in in_room.keys() {
        if !want.contains_key(c)
            && let Ok(mut e) = commands.get_entity(link_of(*c))
        {
            e.remove::<InRoom>();
        }
    }
    *in_room = want;
}

/// Links the hub let go of: Lightyear forgets them (the client was told why and disconnects itself).
fn close_links(time: Res<Time<Real>>, rooms: Option<ResMut<Rooms>>, mut commands: Commands) {
    let Some(mut rooms) = rooms else { return };
    let now = time.elapsed_secs_f64();
    rooms.closing.retain(|&(link, at)| {
        if at > now {
            return true;
        }
        if let Ok(mut e) = commands.get_entity(link) {
            e.insert(Disconnecting);
        }
        false
    });
}

/// Players' round trips into their rooms (shown in the lobby), once a second.
fn measure_rtt(
    time: Res<Time<Real>>,
    mut last: Local<f64>,
    rooms: Option<ResMut<Rooms>>,
    links: Query<(Entity, &Link), With<ClientOf>>,
) {
    let Some(mut rooms) = rooms else { return };
    let now = time.elapsed_secs_f64();
    if now - *last < 1.0 {
        return;
    }
    *last = now;
    for (link, l) in &links {
        let Some(m) = rooms.hub.member(conn_of(link)) else {
            continue;
        };
        let (Some(key), id) = (m.room, m.id) else { continue };
        let rtt = l.stats.rtt.as_millis() as u32;
        if let Some(r) = rooms.hub.rooms.get_mut(&key) {
            r.set_rtt(id, rtt);
        }
    }
}

/// `--maintenance-file`: while it exists the game is being updated (the deploy makes it).
fn watch_update_flag(
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    mut last: Local<f64>,
    rooms: Option<ResMut<Rooms>>,
    shared: Res<HttpShared>,
) {
    let (Some(flag), Some(mut rooms)) = (&opts.maintenance_file, rooms) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if now - *last < Duration::from_millis(500).as_secs_f64() {
        return;
    }
    *last = now;
    let on = flag.exists();
    if on != rooms.hub.updating() {
        rooms.hub.set_updating(on);
        shared.0.updating.store(on, core::sync::atomic::Ordering::Relaxed);
    }
}
