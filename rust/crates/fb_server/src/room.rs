//! The room: its arena, the round being played, and the tick that feeds inputs in and states out.
use std::fs::File;
use std::io::{BufWriter, Write};

use bevy::prelude::*;
use fb_arena::{Arena, MapEvent};
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
    let (arena, _) = Arena::new(map, seed, now.0 as i64 - zero_tick as i64, false);
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

/// The input for `tick` straight from the buffer (see `net::start_servers` for why). TS rules for a tick
/// without input: keep the stick for a moment, never repeat a jump or dive.
fn frame_for(tick: Tick, buffer: Option<&Inputs>) -> InputFrame {
    let Some(b) = buffer else { return InputFrame::IDLE };
    if let Some(s) = b.get(tick) {
        return s.0.into();
    }
    match b.get_last_with_tick() {
        Some((last, s)) if last < tick && tick.0 - last.0 <= INPUT_HOLD => {
            let f: InputFrame = s.0.into();
            InputFrame {
                buttons: f.buttons & !(BTN_JUMP | BTN_DIVE),
                ..f
            }
        }
        _ => InputFrame::IDLE,
    }
}

fn tick_room(
    timeline: Res<LocalTimeline>,
    opts: Res<Opts>,
    mut room: ResMut<Room>,
    mut pawns: Query<(&PlayerId, Option<&Inputs>, &mut BodyFull, &mut RemotePose), With<Pawn>>,
    mut rounds: Query<&mut Round>,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
    trace: Option<ResMut<Trace>>,
) {
    let tick = timeline.tick();
    let room = &mut *room;
    let k = room.round.arena_tick(tick);
    if k as f64 * DT > room.arena.map.meta().duration + RESULTS_S {
        let (mut arena, round) = new_round(&opts, tick, room.round.number + 1);
        for p in &room.arena.pawns {
            arena.add_pawn(p.id);
        }
        room.arena = arena;
        room.round = round.clone();
        room.events.clear();
        if let Ok(mut r) = rounds.get_mut(room.round_entity) {
            *r = round;
        }
        return;
    }
    let mut frames: Vec<(u32, InputFrame)> = pawns
        .iter()
        .map(|(id, buffer, _, _)| (id.0, frame_for(tick, buffer).clamped()))
        .collect();
    frames.sort_unstable_by_key(|f| f.0);
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
        let MapEvent::Bonus(b) = e;
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
    for (id, _, mut full, mut pose) in &mut pawns {
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
