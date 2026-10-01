//! Listening on UDP and WebSocket, and turning connections into pawns.
use core::net::{Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use fb_net::*;
use lightyear::connection::client::Connected;
use lightyear::netcode::NetcodeServer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;
use crate::room::{Pawn, Room};

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_servers);
        app.add_observer(on_link);
        app.add_observer(on_connected);
    }
}

/// Two Lightyear servers, one per transport. Lightyear supports a single started server: with two its
/// `NetworkTopology` is `Invalid` and the systems gated on it stay off. The only one that matters here
/// copies inputs into `ActionState`; `room::frame_for` reads the buffers directly instead.
fn start_servers(mut commands: Commands, opts: Res<Opts>) {
    let netcode = || {
        NetcodeServer::new(NetcodeConfig {
            protocol_id: PROTOCOL_ID,
            private_key: DEV_KEY,
            ..default()
        })
    };
    let cond = || opts.net.config().map(RecvLinkConditioner::new);
    let udp_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), opts.udp_port);
    let udp = commands
        .spawn((
            Name::new("udp"),
            Server::new(cond()),
            netcode(),
            LocalAddr(udp_addr),
            ServerUdpIo::default(),
        ))
        .id();
    let ws_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), opts.ws_port);
    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(ws_addr)
        .with_no_encryption();
    let ws = commands
        .spawn((
            Name::new("ws"),
            Server::new(cond()),
            netcode(),
            LocalAddr(ws_addr),
            WebSocketServerIo { config: ws_config },
        ))
        .id();
    commands.trigger(Start { entity: udp });
    commands.trigger(Start { entity: ws });
    info!(
        "listening: udp {udp_addr}, ws {ws_addr}, tick {} Hz",
        fb_shared::TICK_RATE
    );
}

/// A new link (not yet authenticated): it may receive replication once connected.
fn on_link(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert((ReplicationSender, Name::new("client")));
}

/// A client is in: it gets a pawn (despawned by Lightyear when the client leaves) and the round's events so far.
fn on_connected(
    trigger: On<Add, Connected>,
    links: Query<(&RemoteId, &LinkOf), With<ClientOf>>,
    mut room: ResMut<Room>,
    mut commands: Commands,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
) {
    let link = trigger.entity;
    let Ok((remote, link_of)) = links.get(link) else { return };
    let peer = remote.0;
    let id = room.next_player_id();
    let pawn = room.arena.add_pawn(id.0);
    let full = BodyFull {
        body: pawn.body.clone(),
        teleports: pawn.teleports,
    };
    let color = BeanColor(((id.0 - 1) % fb_shared::BEAN_COLORS.len() as u32) as u8);
    commands.spawn((
        Pawn,
        id,
        color,
        RemotePose::of(&full),
        full,
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::Single(peer)),
        InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(peer)),
        OwnerOnly(link),
        OthersOnly(link),
        ControlledBy {
            owner: link,
            lifetime: default(),
        },
    ));
    if let Ok(server) = servers.get(link_of.server) {
        for msg in &room.events {
            let _ = sender.send::<_, MapEventsChannel>(msg, server, &NetworkTarget::Single(peer));
        }
    }
    info!("player {} joined ({peer:?})", id.0);
}
