//! `--trace input`: one line per predicted tick while connected, `C tick room id mx mz buttons x y z` (rollback
//! replays repeat ticks), and `R tick` before the first one of each connection (`cargo xtask stress` reads it).
use bevy::prelude::*;
use fb_net::trace::TraceFile;
use fb_proto::PlayerId;
use fb_shared::input::InputFrame;
use fb_sim::physics::Body;
use lightyear::prelude::*;

use crate::session::Session;

#[derive(Resource)]
pub struct Input {
    out: TraceFile,
    fresh: bool,
}

pub fn add(app: &mut App, out: TraceFile) {
    app.insert_resource(Input { out, fresh: true });
    app.add_observer(|_: On<Add, Connected>, mut t: ResMut<Input>| t.fresh = true);
}

impl Input {
    /// The own bean at `tick`.
    pub fn write(&mut self, tick: Tick, session: &Session, id: PlayerId, frame: InputFrame, b: &Body) {
        if core::mem::take(&mut self.fresh) {
            self.out.line(format_args!("R {}", tick.0));
        }
        self.out.line(format_args!(
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
        ));
    }
}
