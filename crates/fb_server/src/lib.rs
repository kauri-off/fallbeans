//! Authoritative server: rooms of players and bots over UDP and WebSocket. A library so that the client's
//! tests can run a real server in their process.
mod auth;
mod http;
mod metrics;
mod net;
pub mod opts;
mod play;
mod rooms;

use std::sync::Arc;

use bevy::app::ScheduleRunnerPlugin;
use bevy::diagnostic::DiagnosticsPlugin;
use bevy::ecs::schedule::SingleThreadedExecutor;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use fb_net::{NetStatsPlugin, ProtocolPlugin, SEND_INTERVAL, SERVER_FRAME, TICK};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

pub use crate::opts::Opts;

/// The server's app; `log`: Bevy's logging (a process has one, the client's tests bring their own).
pub fn app(opts: Opts, log: bool) -> App {
    let mut app = App::new();
    // FixedUpdate runs the 120 Hz ticks that are due each frame.
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(SERVER_FRAME)));
    if log {
        app.add_plugins(LogPlugin {
            filter: "bevy_ecs=warn,lightyear=warn,aeronet=warn".into(),
            custom_layer: fb_net::logbook::layer,
            ..default()
        });
    }
    app.add_plugins((bevy::state::app::StatesPlugin, DiagnosticsPlugin));
    app.add_plugins(ServerPlugins { tick_duration: TICK });
    app.add_plugins((ProtocolPlugin, NetStatsPlugin, fb_net::errors::ErrorPolicyPlugin));
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
    app
}
