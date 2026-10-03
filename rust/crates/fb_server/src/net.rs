//! Listening on UDP and WebSocket, and turning connections into pawns.
use core::net::{Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use fb_net::*;
use lightyear::connection::client::Connected;
use lightyear::netcode::NetcodeServer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::opts::Opts;
use crate::room::{InputState, Pawn, Room};

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_server);
        app.add_observer(on_link);
        app.add_observer(on_connected);
    }
}

/// One Lightyear server listening on both transports: the UDP socket binds `LocalAddr`, the WebSocket
/// listener its own config's address, and every client link (`LinkOf`) is of this server whichever way
/// it came in. One netcode server sees all clients (one id cannot be in twice over two transports),
/// and Lightyear's topology is a valid `Server` (with a server per transport it was `Invalid`,
/// lightyear#1693, and the systems that need one server stayed off).
fn start_server(mut commands: Commands, opts: Res<Opts>) {
    let udp_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), opts.udp_port);
    let ws_addr = SocketAddr::new(opts.ws_addr, opts.ws_port);
    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(ws_addr)
        .with_no_encryption();
    let server = commands
        .spawn((
            Name::new("server"),
            Server::new(opts.net.config().map(RecvLinkConditioner::new)),
            NetcodeServer::new(NetcodeConfig {
                protocol_id: PROTOCOL_ID,
                private_key: DEV_KEY,
                // Netcode drops a connect token whose server address is not `LocalAddr`, and 0.0.0.0
                // matches only loopback: no client from another machine would get in (the token holds
                // the public address; behind nginx, nginx's). The WebSocket listener also rewrites
                // `LocalAddr` to its own port. Phase 3 tokens come from /api/session; with a real key,
                // `additional_expected_addresses` (the public UDP and WS addresses) can turn it back on.
                server_addr_check: false,
                ..default()
            }),
            LocalAddr(udp_addr),
            ServerUdpIo::default(),
            WebSocketServerIo { config: ws_config },
        ))
        .id();
    commands.trigger(Start { entity: server });
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
    links: Query<(&RemoteId, &LinkOf, Option<&PeerAddr>), With<ClientOf>>,
    mut room: ResMut<Room>,
    mut commands: Commands,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
    timeline: Res<LocalTimeline>,
) {
    let link = trigger.entity;
    let Ok((remote, link_of, addr)) = links.get(link) else {
        return;
    };
    let peer = remote.0;
    let id = room.next_player_id();
    let pawn = room.arena.add_pawn(id.0, false);
    let full = BodyFull {
        body: pawn.body.clone(),
        teleports: pawn.teleports,
    };
    let color = BeanColor(((id.0 - 1) % fb_shared::BEAN_COLORS.len() as u32) as u8);
    commands.spawn((
        Pawn,
        InputState::new(timeline.tick()),
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
    // The address the link came from: a player's own, a VPN's exit, or nginx's (WebSocket: its access log
    // has the player's).
    let from = addr.map_or_else(|| "?".to_string(), |a| a.0.to_string());
    info!("player {} joined ({peer:?} from {from})", id.0);
}
