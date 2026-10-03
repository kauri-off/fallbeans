//! Authoritative server: rooms of players and bots over UDP and WebSocket.
mod auth;
mod http;
mod logbook;
mod metrics;
mod net;
mod opts;
mod play;
mod rooms;

use std::sync::Arc;

use bevy::app::ScheduleRunnerPlugin;
use bevy::diagnostic::{DiagnosticsPlugin, SystemInformationDiagnosticsPlugin};
use bevy::ecs::schedule::SingleThreadedExecutor;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use clap::Parser;
use fb_net::{NetStatsPlugin, ProtocolPlugin, SEND_INTERVAL, SERVER_FRAME, TICK};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;

fn main() -> AppExit {
    let opts = Opts::parse();
    if let Some(bad) = opts.open_rooms.iter().find(|r| !fb_proto::valid_room_id(r)) {
        eprintln!("--open-rooms: {bad:?} is not a room id (2–8 of a-z, 0-9)");
        return AppExit::from_code(2);
    }
    let mut app = App::new();
    app.add_plugins((
        // FixedUpdate runs the 120 Hz ticks that are due each frame.
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(SERVER_FRAME)),
        LogPlugin {
            filter: "bevy_ecs=warn,lightyear=warn,aeronet=warn".into(),
            custom_layer: logbook::layer,
            ..default()
        },
        bevy::state::app::StatesPlugin,
        DiagnosticsPlugin,
        SystemInformationDiagnosticsPlugin,
    ));
    app.add_plugins(ServerPlugins { tick_duration: TICK });
    app.add_plugins((ProtocolPlugin, NetStatsPlugin));
    fb_net::add_server_filters(&mut app);
    app.insert_resource(ReplicationMetadata::new(SEND_INTERVAL));
    app.insert_resource(opts);
    app.insert_resource(http::Keys(Arc::new(auth::Auth::new(&auth::secret()))));
    app.insert_resource(http::HttpShared(Arc::default()));
    app.add_plugins((
        net::NetPlugin,
        play::PlayPlugin,
        metrics::MetricsPlugin,
        http::HttpPlugin,
    ));
    // Every schedule on the main thread. The multi-threaded executor hands systems to the compute pool and
    // waits for them each frame: on the 1-vCPU host that cost 15% of the core and 9 000 context switches a
    // second with nobody playing, for a frame of a few dozen microseconds of work.
    for (_, schedule) in app.world_mut().resource_mut::<Schedules>().iter_mut() {
        schedule.set_executor(SingleThreadedExecutor::new());
    }
    app.run()
}
