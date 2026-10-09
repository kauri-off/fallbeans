//! The rooms on the network: the hub as a resource, fed with connections, messages and inputs from Lightyear,
//! and its rooms' arenas published as replicated entities (a `Round` per room, a pawn per bean in play), each
//! visible only to the links in that room.
use std::collections::BTreeMap;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::time::common_conditions::on_real_timer;
use fb_arena::PawnStatus;
use fb_net::*;
use fb_proto::PlayerId;
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_shared::{INPUT_HOLD, TICK_RATE};
use lightyear::connection::client::Disconnecting;
use lightyear::input::input_message::InputMessage;
use lightyear::input::native::prelude::{ActionState, NativeStateSequence};
use lightyear::input::server::{InputValidationAppExt, authorize_controlled_targets};
use lightyear::prelude::input::InputBuffer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;
use crate::rooms::hub::Hub;
use crate::rooms::room::RoomOptions;
use crate::rooms::{Backoff, ConnId, Inputs, Out, ticks};

#[cfg(feature = "traces")]
mod trace;

/// A bean in play in a room (`RoomTag`, `BeanId`).
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
    pawns: BTreeMap<(u32, PlayerId), PawnEntity>,
    rounds: BTreeMap<u32, Entity>,
    /// The room each link is in, as last told to Replicon.
    in_room: BTreeMap<ConnId, u32>,
    real: RealTick,
}

/// The server tick as a count that does not wrap: Lightyear's is a u32, which at 120 Hz wraps after ~414 days,
/// and the rooms' timers would stop at the wrap.
#[derive(Debug, Default)]
struct RealTick {
    last: Option<u32>,
    wraps: u64,
}

impl RealTick {
    fn of(&mut self, tick: u32) -> u64 {
        if let Some(last) = self.last
            && tick < last
            && last - tick > u32::MAX / 2
        {
            self.wraps += 1;
        }
        self.last = Some(tick);
        (self.wraps << 32) | u64::from(tick)
    }
}

/// How far from the server's tick Lightyear takes an input message (`end_tick − tick`, its
/// `MAX_INPUT_PAST_TICKS` and `MAX_INPUT_LOOKAHEAD_TICKS`): it drops the others without a word.
const INPUT_WINDOW: core::ops::RangeInclusive<i32> = -(fb_net::INPUT_RING as i32)..=fb_net::INPUT_RING as i32;

/// Input messages Lightyear is about to drop for being too far from the server's tick: a client that leads
/// by more than ~0.5 s (a slow VPN) and whose bean then stands still. Counted for a warning that backs off.
#[derive(Resource, Default)]
struct FarInputs {
    count: u64,
    /// The farthest one since the last warning (ticks, signed).
    worst: i32,
    log: Backoff,
}

type InputReceiver = MessageReceiver<InputMessage<NativeStateSequence<FbInput>>>;

fn count_far_inputs(
    timeline: Res<LocalTimeline>,
    mut far: ResMut<FarInputs>,
    mut receivers: Query<(&RemoteId, &mut InputReceiver), With<Connected>>,
) {
    let tick = timeline.tick();
    let far = &mut *far;
    for (remote, mut r) in &mut receivers {
        let mut n = 0;
        r.retain_messages(|m| {
            let d = m.end_tick - tick;
            if !INPUT_WINDOW.contains(&d) {
                n += 1;
                if d.abs() > far.worst.abs() {
                    far.worst = d;
                }
            }
            true
        });
        if n == 0 {
            continue;
        }
        far.count += n;
        if let Some(hushed) = far.log.hit(crate::rooms::secs(u64::from(tick.0))) {
            warn!(
                client = ?remote.0,
                dropped = far.count,
                worst = far.worst,
                hushed,
                "input messages too far from the server tick: Lightyear drops them"
            );
            far.count = 0;
            far.worst = 0;
        }
    }
}

pub struct PlayPlugin;

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start);
        #[cfg(feature = "traces")]
        app.add_systems(Startup, trace::open);
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
        app.init_resource::<FarInputs>();
        app.add_input_validator(count_far_inputs);
        app.add_systems(FixedUpdate, (tick_rooms.in_set(RoomTick), watch_inputs.after(RoomTick)));
        // Every frame: Lightyear drops the messages nobody read in the frame they came in, and half the
        // frames run no tick.
        app.add_systems(PreUpdate, receive.after(MessageSystems::Receive));
        app.add_systems(
            Update,
            (close_links, measure_rtt.run_if(on_real_timer(Duration::from_secs(1)))),
        );
    }
}

fn start(mut commands: Commands, opts: Res<Opts>, timeline: Res<LocalTimeline>) {
    let base = RoomOptions {
        min_players: if opts.solo { 1 } else { 2 },
        seed: opts.seed,
        intro_ticks: u32::try_from(ticks(opts.intro)).unwrap_or(u32::MAX),
        dev: opts.dev,
        eliminate: !opts.respawn,
        ..default()
    };
    let mut real = RealTick::default();
    let mut hub = Hub::new(base, real.of(timeline.tick().0));
    hub.max_rooms = opts.max_rooms.clamp(1, fb_shared::MAX_ROOMS);
    for id in &opts.open_rooms {
        hub.open_permanent(id, id);
    }
    commands.insert_resource(Rooms {
        hub,
        pawns: BTreeMap::new(),
        rounds: BTreeMap::new(),
        in_room: BTreeMap::new(),
        real,
    });
}

pub fn conn_of(link: Entity) -> ConnId {
    ConnId(link)
}

fn link_of(conn: ConnId) -> Entity {
    conn.0
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
    /// What the last tick used (`--trace input`).
    #[cfg(feature = "traces")]
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
            #[cfg(feature = "traces")]
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
        // (As a signed difference: across the u32 tick's wrap the last input is a few ticks old, not none.)
        None if (k.wrapping_sub(st.ack) as i32) > INPUT_HOLD as i32 => InputFrame::IDLE,
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
        // After STILL_S, then 2, 4, 8 … times that: a player gone for minutes is a few lines, not one every 5 s.
        let still = STILL_S * rate;
        if st.gap > 0 && st.gap.is_multiple_of(still) && (st.gap / still).is_power_of_two() {
            warn!(
                room,
                %id,
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
                warn!(room, %id, ms = ms(ticks), behind, "input gap");
                st.quiet_until = now + QUIET_S * rate;
            }
        }
        if st.hushed.0 > 0 && now >= st.quiet_until {
            let (n, ticks) = core::mem::take(&mut st.hushed);
            warn!(room, %id, n, ms = ms(ticks), "more input gaps in {QUIET_S} s");
            st.quiet_until = now + QUIET_S * rate;
        }
    }
}

/// Inputs from the pawn entities' buffers (Lightyear writes a client's input into the entity it controls).
struct LinkInputs<'a, 'w, 's> {
    owners: &'a BTreeMap<ConnId, Entity>,
    pawns: &'a mut Query<'w, 's, (Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
    /// How many ticks behind its bean each link sees the others.
    views: &'a BTreeMap<ConnId, u32>,
}

impl Inputs for LinkInputs<'_, '_, '_> {
    fn frame(&mut self, _: PlayerId, conn: ConnId, tick: u32) -> InputFrame {
        let Some(&e) = self.owners.get(&conn) else {
            return InputFrame::IDLE;
        };
        let Ok((buffer, mut st)) = self.pawns.get_mut(e) else {
            return InputFrame::IDLE;
        };
        let f = frame_for(Tick(tick), buffer, &mut st);
        #[cfg(feature = "traces")]
        {
            st.used = Some((tick, f));
        }
        f
    }

    fn view(&mut self, _: PlayerId, conn: ConnId) -> u32 {
        self.views.get(&conn).copied().unwrap_or(0)
    }
}

type Bodies = (
    &'static Pawn,
    &'static mut BodyFull,
    &'static mut RemotePose,
    &'static mut Hold,
);
/// The clients' links: who they are, how far behind they see, and their senders.
#[derive(SystemParam)]
struct Links<'w, 's> {
    remotes: Query<'w, 's, &'static RemoteId, With<ClientOf>>,
    delays: Query<'w, 's, (Entity, &'static InterpolationDelay), With<ClientOf>>,
    control: Query<'w, 's, &'static mut MessageSender<ServerMsg>>,
    events: Query<'w, 's, &'static mut MessageSender<MapEventMsg>>,
}

/// The pawns' inputs and bodies, and the round entities.
#[derive(SystemParam)]
struct PawnsMut<'w, 's> {
    inputs: Query<'w, 's, (Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
    bodies: Query<'w, 's, Bodies>,
    rounds: Query<'w, 's, &'static mut Round>,
}

/// The dev server's traces, when asked for.
#[cfg(feature = "traces")]
#[derive(SystemParam)]
struct Traces<'w> {
    input: Option<ResMut<'w, trace::Input>>,
    hits: Option<ResMut<'w, trace::Hits>>,
}

fn tick_rooms(
    timeline: Res<LocalTimeline>,
    time: Res<Time<Real>>,
    mut rooms: ResMut<Rooms>,
    mut commands: Commands,
    mut links: Links,
    pawns: PawnsMut,
    #[cfg(feature = "traces")] traces: Traces,
) {
    let PawnsMut {
        mut inputs,
        mut bodies,
        mut rounds,
    } = pawns;
    let tick = timeline.tick();
    let rooms = &mut *rooms;
    let owners: BTreeMap<ConnId, Entity> = rooms
        .pawns
        .values()
        .filter_map(|p| Some((p.owner?, p.entity)))
        .collect();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "milliseconds of delay"
    )]
    let views: BTreeMap<ConnId, u32> = links
        .delays
        .iter()
        .map(|(link, d)| (conn_of(link), d.delay.value.to_num::<f64>().round() as u32))
        .collect();
    let real = rooms.real.of(tick.0);
    rooms.hub.update(
        real,
        &mut LinkInputs {
            owners: &owners,
            pawns: &mut inputs,
            views: &views,
        },
    );
    #[cfg(feature = "traces")]
    trace::write(traces.input, traces.hits, rooms, &mut inputs, tick);
    for out in rooms.hub.take_out() {
        match out {
            Out::Msg(c, m) => {
                if let Ok(mut s) = links.control.get_mut(link_of(c)) {
                    s.send::<ControlChannel>(m);
                }
            }
            Out::Event(c, e) => {
                if let Ok(mut s) = links.events.get_mut(link_of(c)) {
                    s.send::<MapEventsChannel>(e);
                }
            }
            Out::Close(c) => {
                if let Ok(mut e) = commands.get_entity(link_of(c)) {
                    e.insert(CloseAt(time.elapsed_secs_f64() + 0.5));
                }
            }
        }
    }
    publish(rooms, &mut commands, &mut bodies, &mut rounds, &links.remotes, tick);
}

fn receive(mut rooms: ResMut<Rooms>, mut receivers: Query<(Entity, &mut MessageReceiver<ClientMsg>), With<ClientOf>>) {
    for (link, mut r) in &mut receivers {
        for msg in r.receive() {
            rooms.hub.message(conn_of(link), msg);
        }
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
        let map = room.arena.map.meta().id;
        let zero_tick = room.zero_tick();
        // Compared field by field: a new `Round` (two Strings) only when something changed, not every tick.
        let same = |r: &Round| {
            r.arena == room.arena_id
                && r.kind == room.arena.kind
                && r.map == map
                && r.seed == room.arena.seed
                && r.zero_tick == zero_tick
                && r.fall == room.arena.fall
                && r.static_hash == room.arena.static_hash.to_string()
        };
        let round = || Round {
            arena: room.arena_id,
            kind: room.arena.kind,
            map,
            seed: room.arena.seed,
            zero_tick,
            fall: room.arena.fall,
            static_hash: room.arena.static_hash.to_string(),
        };
        let existing = rounds.get(&key).copied();
        let fresh = match existing.map(|e| round_q.get_mut(e)) {
            Some(Ok(mut r)) => {
                if !same(&r) {
                    *r = round();
                }
                false
            }
            // Its entity is gone (or lost its `Round`): clients would keep an old arena; replicate it again.
            Some(Err(_)) => {
                warn!(room = %room.id, "the room's round entity is gone: a new one");
                if let Some(old) = existing
                    && let Ok(mut e) = commands.get_entity(old)
                {
                    e.despawn();
                }
                true
            }
            None => true,
        };
        if fresh {
            let e = commands
                .spawn((
                    Name::new(format!("room {}", room.id)),
                    round(),
                    RoomTag(key),
                    Replicate::to_clients(NetworkTarget::All),
                ))
                .id();
            rounds.insert(key, e);
        }
        for p in room.arena.pawns.iter().filter(|p| p.status == PawnStatus::Play) {
            // (A link already gone counts as nobody: the room hears of it in a moment.)
            let owner = room
                .player(p.id)
                .and_then(|pl| pl.conn())
                .filter(|c| remotes.get(link_of(*c)).is_ok());
            let color = room.player(p.id).map_or(0, |pl| pl.color);
            live.push((key, p.id));
            let full = BodyFull {
                body: p.body.clone(),
                teleports: p.teleports,
                checkpoint: p.checkpoint.and_then(|c| u16::try_from(c).ok()),
                spawn: u16::try_from(p.spawn_i).unwrap_or(u16::MAX),
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
                BeanId(p.id),
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
    let want: BTreeMap<ConnId, u32> = hub.seated().collect();
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

/// A link the hub let go of, to close once what was sent to it has gone out (server time, s).
#[derive(Component, Clone, Copy, Debug)]
struct CloseAt(f64);

/// Links the hub let go of: Lightyear forgets them (the client was told why and disconnects itself).
fn close_links(time: Res<Time<Real>>, links: Query<(Entity, &CloseAt)>, mut commands: Commands) {
    let now = time.elapsed_secs_f64();
    for (link, at) in &links {
        if at.0 <= now {
            commands.entity(link).remove::<CloseAt>().insert(Disconnecting);
        }
    }
}

/// Players' round trips into their rooms (shown in the lobby), once a second.
fn measure_rtt(rooms: Option<ResMut<Rooms>>, links: Query<(Entity, &Link), With<ClientOf>>) {
    let Some(mut rooms) = rooms else { return };
    for (link, l) in &links {
        let Some((key, id)) = rooms.hub.seat(conn_of(link)) else {
            continue;
        };
        let rtt = u32::try_from(l.stats.rtt.as_millis()).unwrap_or(u32::MAX);
        if let Some(r) = rooms.hub.rooms.get_mut(&key) {
            r.set_rtt(id, rtt);
        }
    }
}
