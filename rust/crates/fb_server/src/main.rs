//! Authoritative server (phase 0): one room playing rounds of one map over UDP and WebSocket.
mod metrics;
mod net;
mod opts;
mod room;

use bevy::app::ScheduleRunnerPlugin;
use bevy::diagnostic::{DiagnosticsPlugin, SystemInformationDiagnosticsPlugin};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use clap::Parser;
use fb_net::{NetStatsPlugin, ProtocolPlugin, SEND_INTERVAL, SERVER_FRAME, TICK};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;

fn main() -> AppExit {
    let opts = Opts::parse();
    if fb_maps::by_id(&opts.map).is_none() {
        let ids: Vec<_> = fb_maps::MAPS.iter().map(|m| m.meta().id).collect();
        eprintln!("no map {:?}; maps: {}", opts.map, ids.join(", "));
        return AppExit::from_code(2);
    }
    let mut app = App::new();
    app.add_plugins((
        // FixedUpdate runs the 120 Hz ticks that are due each frame.
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(SERVER_FRAME)),
        LogPlugin {
            filter: "bevy_ecs=warn,lightyear=warn,aeronet=warn".into(),
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
    app.add_plugins((net::NetPlugin, room::RoomPlugin, metrics::MetricsPlugin));
    app.run()
}
