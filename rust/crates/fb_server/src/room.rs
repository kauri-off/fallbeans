//! The room: its arena, the round being played, and the tick that feeds inputs in and states out.
use std::fs::File;
use std::io::{BufWriter, Write};

use bevy::prelude::*;
use fb_arena::{Arena, ArenaEvent, ArenaKind, FallBehaviour};
use fb_net::*;
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_shared::{DT, INPUT_HOLD, RESULTS_S};
use lightyear::input::native::prelude::ActionState;
use lightyear::prelude::input::InputBuffer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;

/// A bean in the room (its owner's link is in its `OwnerOnly`).
#[derive(Component)]
pub struct Pawn;

/// The room tick, for systems that measure or follow it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoomTick;

#[derive(Resource)]
pub struct Room {
    pub arena: Arena,
    pub round: Round,
    round_entity: Entity,
    /// Events of this round so far (sent to whoever joins late).
    pub events: Vec<MapEventMsg>,
    next_id: u32,
}

impl Room {
    /// Player ids are the room's own: small and never reused (the TS server does the same).
    pub fn next_player_id(&mut self) -> PlayerId {
        self.next_id += 1;
        PlayerId(self.next_id)
    }
}

/// `--trace`: one line per pawn per tick, `S tick id mx mz buttons x y z`.
#[derive(Resource)]
struct Trace(BufWriter<File>);

pub struct RoomPlugin;

impl Plugin for RoomPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_room);
        // Lightyear's copy of the input buffer into `ActionState` would also drop every tick before
        // the current one, and with them any input that arrives late: a late press would be lost.
        // The room reads the buffer itself (`frame_for`); the buffer is a ring of 64 ticks.
        app.configure_sets(
            FixedPreUpdate,
            lightyear::input::server::InputSystems::UpdateActionState.run_if(|| false),
        );
        app.add_observer(on_pawn_removed);
        app.add_systems(FixedUpdate, tick_room.in_set(RoomTick));
    }
}

fn new_round(opts: &Opts, now: Tick, number: u32) -> (Arena, Round) {
    let map = fb_maps::by_id(&opts.map).expect("map checked at startup");
    let seed = opts.seed.unwrap_or_else(|| {
        // Not simulation: any source of variety will do for the seed.
        let n = std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_nanos() as u64;
        (n ^ (n >> 29) ^ (number as u64 * 0x9e37_79b9)) as u32
    });
    let intro_ticks = (opts.intro / DT).round() as u32;
    let zero_tick = now.0 + intro_ticks + 1;
    let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, now.0 as i64 - zero_tick as i64, &[], false);
    // The client has no spectator view yet (Phase 4): a fall is a respawn, not the end of the round.
    arena.fall = FallBehaviour::Spawn;
    info!("round {number}: {} seed {seed}, starts at tick {zero_tick}", opts.map);
    let round = Round {
        map: opts.map.clone(),
        seed,
        zero_tick,
        number,
        static_hash: arena.static_hash.clone(),
    };
    (arena, round)
}

fn start_room(mut commands: Commands, opts: Res<Opts>, timeline: Res<LocalTimeline>) {
    let (arena, round) = new_round(&opts, timeline.tick(), 1);
    let round_entity = commands
        .spawn((round.clone(), Replicate::to_clients(NetworkTarget::All)))
        .id();
    commands.insert_resource(Room {
        arena,
        round,
        round_entity,
        events: Vec::new(),
        next_id: 0,
    });
    if let Some(path) = &opts.trace {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace {}: {e}", path.display()));
        commands.insert_resource(Trace(BufWriter::new(file)));
    }
}

fn on_pawn_removed(trigger: On<Remove, Pawn>, pawns: Query<&PlayerId>, room: Option<ResMut<Room>>) {
    if let (Ok(id), Some(mut room)) = (pawns.get(trigger.entity), room) {
        room.arena.remove_pawn(id.0);
        info!("player {} left", id.0);
    }
}

type Inputs = InputBuffer<ActionState<FbInput>, FbInput>;
type PawnData = (
    &'static PlayerId,
    Option<&'static Inputs>,
    &'static mut InputState,
    &'static mut BodyFull,
    &'static mut RemotePose,
);

/// Jump and dive: one tick per press (the client sends them so).
const PRESSES: u8 = BTN_JUMP | BTN_DIVE;

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
    /// Ticks run without the player's input since the last `metrics:` line (after their first input).
    pub missed: u32,
}

impl InputState {
    pub fn new(now: Tick) -> Self {
        Self {
            last: InputFrame::IDLE,
            ack: now.0,
            late: 0,
            late_seen: now.0,
            missed: 0,
        }
    }
}

/// The input for `tick`, read from the buffer directly (see `RoomPlugin` for why). Without one
/// in time the pawn keeps its stick (and grab) for INPUT_HOLD ticks, never repeating a jump or dive; a
/// press that arrives late is not lost but happens now.
fn frame_for(tick: Tick, buffer: Option<&Inputs>, st: &mut InputState) -> InputFrame {
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
    st.missed += u32::from(input.is_none() && buffer.is_some());
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

fn tick_room(
    timeline: Res<LocalTimeline>,
    opts: Res<Opts>,
    mut room: ResMut<Room>,
    mut pawns: Query<PawnData, With<Pawn>>,
    mut rounds: Query<&mut Round>,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
    trace: Option<ResMut<Trace>>,
) {
    let tick = timeline.tick();
    let room = &mut *room;
    let k = room.round.arena_tick(tick);
    let mut frames: Vec<(u32, InputFrame)> = pawns
        .iter_mut()
        .map(|(id, buffer, mut st, _, _)| (id.0, frame_for(tick, buffer, &mut st).clamped()))
        .collect();
    frames.sort_unstable_by_key(|f| f.0);
    if k as f64 * DT > room.arena.map.meta().duration + RESULTS_S {
        let (mut arena, round) = new_round(&opts, tick, room.round.number + 1);
        for p in &room.arena.pawns {
            // To the new spawn as a teleport: views snap instead of gliding across the map.
            arena.add_pawn(p.id, false).teleports = p.teleports + 1;
        }
        room.arena = arena;
        room.round = round.clone();
        room.events.clear();
        if let Ok(mut r) = rounds.get_mut(room.round_entity) {
            *r = round;
        }
        publish(room, &mut pawns);
        return;
    }
    let events = room.arena.step(k, |id| {
        frames
            .binary_search_by_key(&id, |f| f.0)
            .map_or(InputFrame::IDLE, |i| frames[i].1)
    });
    if let Some(mut trace) = trace {
        for (id, f) in &frames {
            if let Some(p) = room.arena.pawn(*id) {
                let b = &p.body;
                let _ = writeln!(
                    trace.0,
                    "S {} {id} {} {} {} {:.6} {:.6} {:.6}",
                    tick.0, f.mx, f.mz, f.buttons, b.pos.x, b.pos.y, b.pos.z
                );
            }
        }
    }
    for e in events {
        let ArenaEvent::Bonus(b) = e else { continue };
        let msg = MapEventMsg {
            round: room.round.number,
            tick: tick.0,
            ev: MapEventKind::Bonus {
                i: b.i,
                id: b.id,
                at: b.at,
            },
        };
        room.events.push(msg);
        for server in &servers {
            let _ = sender.send::<_, MapEventsChannel>(&msg, server, &NetworkTarget::All);
        }
        info!("tick {}: player {} took bonus {}", tick.0, b.id, b.i);
    }
    publish(room, &mut pawns);
}

/// The arena's beans into their replicated components.
fn publish(room: &Room, pawns: &mut Query<PawnData, With<Pawn>>) {
    for (id, _, _, mut full, mut pose) in pawns {
        let Some(p) = room.arena.pawn(id.0) else { continue };
        let next = BodyFull {
            body: p.body.clone(),
            teleports: p.teleports,
        };
        if *full != next {
            *pose = RemotePose::of(&next);
            *full = next;
        }
    }
}
