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
use std::sync::atomic::Ordering;

use bevy::app::{ScheduleRunnerPlugin, TaskPoolOptions, TaskPoolPlugin};
use bevy::diagnostic::DiagnosticsPlugin;
use bevy::ecs::schedule::SingleThreadedExecutor;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use fb_net::{NetStatsPlugin, ProtocolPlugin, SEND_INTERVAL, SERVER_FRAME, TICK};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

pub use crate::opts::Opts;

/// The server's app; `log`: the process is the server's own, with Bevy's logging and task pools sized for it
/// (a process has one of each: the client's tests bring theirs).
pub fn app(opts: Opts, log: bool) -> App {
    // (The one map every room plays: its bots' grid is built now, off the main thread, not in a room's tick.)
    rooms::room::prebuild_lobby_nav();
    let mut app = App::new();
    // FixedUpdate runs the 120 Hz ticks that are due each frame.
    let plugins = MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(SERVER_FRAME));
    if log {
        // One thread per pool (IO, async compute, compute) instead of one per core: every schedule runs on the
        // main thread (below), and on a many-core host the defaults took the process past systemd's `TasksMax`.
        app.add_plugins(plugins.set(TaskPoolPlugin {
            task_pool_options: TaskPoolOptions::with_num_threads(3),
        }));
    } else {
        app.add_plugins(plugins);
    }
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
    app.init_resource::<net::Shutdown>();
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

/// SIGTERM, SIGINT and SIGHUP (`systemctl stop`, a package upgrade, Ctrl+C) shut `app` down gracefully: the
/// players are told the server is restarting before it goes (`net::shut_down`). A second signal exits at once.
pub fn exit_on_signals(app: &App) {
    let signal = app.world().resource::<net::Shutdown>().signal.clone();
    let handler = move || {
        if signal.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
    };
    if let Err(e) = ctrlc::set_handler(handler) {
        warn!("no signal handler, the server will not shut down gracefully: {e}");
    }
}
