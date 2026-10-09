//! The Lightyear protocol shared by server and client: replicated components, inputs, channels, plus
//! the network tooling both sides use (link conditioner flags, traffic counters).
use core::time::Duration;

use bevy::prelude::*;
use fb_shared::{SIM_FINGERPRINT, TICK_RATE, fnv1a_lines};
use lightyear::prelude::*;

mod components;
mod conditioner;
pub mod errors;
mod events;
mod input;
pub mod logbook;
#[cfg(test)]
mod schema;
mod stats;
mod visibility;
mod wire;

pub use components::*;
pub use conditioner::NetSim;
pub use events::*;
pub use fb_proto::{ClientMsg, MapEventKind, MapEventMsg, ServerMsg};
pub use input::FbInput;
pub use stats::{NetStats, NetStatsPlugin};
pub use visibility::{InRoom, OthersOnly, OwnerOnly, RoomTag, add_server_filters};

/// The game's version, with the commit when the build had `FB_COMMIT` (releases).
pub fn build() -> String {
    let v = env!("CARGO_PKG_VERSION");
    match option_env!("FB_COMMIT") {
        Some(c) if !c.is_empty() => format!("{v} ({c})"),
        _ => v.into(),
    }
}

/// What client and server compare: the fingerprints of the simulation and of the wire schema (`protocol.txt`).
pub const PROTOCOL_VERSION: u32 = fnv1a_lines(SIM_FINGERPRINT, include_bytes!("../protocol.txt"));

/// Netcode's id in every connect token (`PROTOCOL_VERSION`), so a token of another protocol is refused too.
pub const PROTOCOL_ID: u64 = 0xFB00_0000 + PROTOCOL_VERSION as u64;
pub const UDP_PORT: u16 = 5888;
/// The HTTP API (session, health, debug); behind a reverse proxy at https://…/fallbeans/.
pub const HTTP_PORT: u16 = 5887;
pub const WS_PORT: u16 = 5889;
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE as u64);
/// Snapshots at 60 Hz: other beans move smoother and closer to the present than at 30.
pub const SEND_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
/// Inputs go out at 60 Hz (two ticks a message). Lightyear's default, every frame,
/// cost 56 KB/s up from a client running at 1000 frames and as many acks back from the server.
pub const INPUT_SEND_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
/// A jump or dive that reaches the server up to this many ticks after its tick still happens, on the next
/// tick (`fb_server::play::frame_for`); older inputs are of no use to the server.
pub const LATE_TICKS: u32 = 30;
/// Lightyear's server takes input messages at most this many ticks ahead of or behind its tick, and its input
/// buffer is a ring of as many (`MAX_INPUT_LOOKAHEAD_TICKS`, `MAX_INPUT_PAST_TICKS`: private there).
pub const INPUT_RING: u32 = 64;
/// Each input message repeats the inputs of this many before it (two ticks a message): a loss burst of up
/// to LATE_TICKS (250 ms, a Wi-Fi hiccup) loses no press. Lightyear's default, 5, covers 83 ms.
pub const INPUT_REDUNDANCY: u16 = (LATE_TICKS / 2) as u16;
/// The server's loop rate: twice the tick rate, so ticks run on time without spinning.
pub const SERVER_FRAME: Duration = Duration::from_nanos(1_000_000_000 / (2 * TICK_RATE as u64));
/// An unacked reliable message goes again after 1.5 × RTT, never sooner than this. Lightyear's minimum, 0,
/// resent every unacked message every 1.5 × RTT through a WebSocket stall (TCP loses nothing): bursts of
/// duplicates once it clears. On UDP a lost message waits at least this long (channels are not per
/// transport). Lightyear's own channels (replication, inputs) keep their settings.
pub const RESEND_MIN: Duration = Duration::from_millis(150);

/// The game's own reliable channels.
fn reliable() -> ReliableSettings {
    ReliableSettings {
        rtt_resend_min_delay: RESEND_MIN,
        ..default()
    }
}

#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lightyear::prelude::input::native::InputPlugin::<FbInput> {
            config: lightyear::prelude::input::InputConfig {
                send_interval: INPUT_SEND_INTERVAL,
                packet_redundancy: INPUT_REDUNDANCY,
                // The client's interpolation delay rides along: tackles are judged by what the player saw.
                lag_compensation: true,
                ..default()
            },
        });
        app.register_message::<MapEventMsg>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ServerMsg>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ClientMsg>()
            .add_direction(NetworkDirection::ClientToServer);
        app.add_channel::<MapEventsChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(reliable()),
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);
        app.add_channel::<ControlChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(reliable()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
        app.component::<BeanId>().replicate();
        app.component::<BeanColor>().replicate();
        app.component::<Round>().replicate();
        app.component::<BodyFull>()
            .replicate()
            .predict()
            .with_rollback_condition(body_differs);
        app.component::<RemotePose>().replicate().add_linear_interpolation();
        app.component::<Hold>().replicate();
    }
}
