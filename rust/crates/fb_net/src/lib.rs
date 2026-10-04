//! The Lightyear protocol shared by server and client: replicated components, inputs, channels, plus
//! the network tooling both sides use (link conditioner flags, traffic counters).
use core::time::Duration;

use bevy::prelude::*;
use fb_shared::{PROTOCOL_VERSION, TICK_RATE};
use lightyear::prelude::*;

mod components;
mod conditioner;
mod events;
mod input;
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

pub const PROTOCOL_ID: u64 = 0xFB00_0000 + PROTOCOL_VERSION as u64;
/// Dev key for netcode's manual authentication (phase 0: no session endpoint yet).
pub const DEV_KEY: [u8; 32] = [0; 32];
pub const UDP_PORT: u16 = 5888;
/// The HTTP API (session, health, debug); production: behind nginx at https://…/fallbeans/.
pub const HTTP_PORT: u16 = 5887;
pub const WS_PORT: u16 = 5889;
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE as u64);
/// Snapshots at 30 Hz, as the TS server.
pub const SEND_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);
/// Inputs go out at 60 Hz, as from the TS client (two ticks a message). Lightyear's default, every frame,
/// cost 56 KB/s up from a client running at 1000 frames and as many acks back from the server.
pub const INPUT_SEND_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
/// A jump or dive that reaches the server up to this many ticks after its tick still happens, on the next
/// tick (`fb_server::room::frame_for`); older inputs are of no use to the server.
pub const LATE_TICKS: u32 = 30;
/// Each input message repeats the inputs of this many before it (two ticks a message): a loss burst of up
/// to LATE_TICKS (250 ms, a Wi-Fi hiccup) loses no press. Lightyear's default, 5, covers 83 ms.
pub const INPUT_REDUNDANCY: u16 = (LATE_TICKS / 2) as u16;
/// The server's loop rate: twice the tick rate, so ticks run on time without spinning.
pub const SERVER_FRAME: Duration = Duration::from_nanos(1_000_000_000 / (2 * TICK_RATE as u64));

#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lightyear::prelude::input::native::InputPlugin::<FbInput> {
            config: lightyear::prelude::input::InputConfig {
                send_interval: INPUT_SEND_INTERVAL,
                packet_redundancy: INPUT_REDUNDANCY,
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
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);
        app.add_channel::<ControlChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
        app.component::<PlayerId>().replicate();
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
