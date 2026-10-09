//! `--trace input`: `S tick room id mx mz buttons x y z` per pawn per tick. `--trace hits`: `room arena map tick
//! id …`, the arenas' `hit_log`.
use bevy::prelude::*;
use fb_net::trace::{TraceFile, TraceKind};
use lightyear::prelude::*;

use super::{InputBuf, InputState, Pawn, Rooms};
use crate::opts::Opts;

#[derive(Resource)]
pub struct Input(TraceFile);

#[derive(Resource)]
pub struct Hits(TraceFile);

pub fn open(mut commands: Commands, opts: Res<Opts>) {
    if let Some(f) = opts.traces.open(TraceKind::Input, "server") {
        commands.insert_resource(Input(f));
    }
    if let Some(f) = opts.traces.open(TraceKind::Hits, "server") {
        commands.insert_resource(Hits(f));
    }
    if opts.traces.on(TraceKind::Clicks) {
        warn!("--trace clicks: a client's trace, the server has none");
    }
}

pub fn write(
    input: Option<ResMut<Input>>,
    hits: Option<ResMut<Hits>>,
    rooms: &mut Rooms,
    inputs: &mut Query<(Option<&'static InputBuf>, &'static mut InputState), With<Pawn>>,
    tick: Tick,
) {
    if let Some(mut out) = input {
        write_input(&mut out.0, rooms, inputs, tick);
    }
    if let Some(mut out) = hits {
        for room in rooms.hub.rooms.values_mut() {
            for l in room.hits.get_or_insert_default().drain(..) {
                out.0.line(format_args!("{} {l}", room.id));
            }
        }
        // (The dev server is killed, not stopped: nothing may wait in the buffer.)
        out.0.flush();
    }
}

fn write_input(
    out: &mut TraceFile,
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
        out.line(format_args!(
            "S {} {} {id} {} {} {} {:.6} {:.6} {:.6}",
            tick.0, room.id, f.mx, f.mz, f.buttons, b.pos.x, b.pos.y, b.pos.z
        ));
    }
}
