//! Listening on UDP and WebSocket, and turning connections into pawns.
use core::net::{Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use fb_net::*;
use lightyear::connection::client::Connected;
use lightyear::netcode::{NetcodeServer, TokenUserData};
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use crate::auth::uid_from_user_data;
use crate::http::Keys;
use crate::opts::Opts;
use crate::play::{Rooms, conn_of};

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_server);
        app.add_observer(on_link);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

/// One Lightyear server listening on both transports: the UDP socket binds `LocalAddr`, the WebSocket
/// listener its own config's address, and every client link (`LinkOf`) is of this server whichever way
/// it came in. One netcode server sees all clients (one id cannot be in twice over two transports),
/// and Lightyear's topology is a valid `Server` (with a server per transport it was `Invalid`,
/// lightyear#1693, and the systems that need one server stayed off).
fn start_server(mut commands: Commands, opts: Res<Opts>, keys: Res<Keys>) {
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
                private_key: keys.0.netcode_key(),
                // Tokens name the public UDP address, never `LocalAddr`; without `--public-host` there is none to check.
                server_addr_check: opts.public_host.is_some(),
                additional_expected_addresses: opts
                    .public_host
                    .map(|h| SocketAddr::new(h, opts.udp_port))
                    .into_iter()
                    .collect(),
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

/// A client is in, as the player of its connect token: the hub waits for its hello.
fn on_connected(
    trigger: On<Add, Connected>,
    links: Query<(&RemoteId, Option<&PeerAddr>, Option<&TokenUserData>), With<ClientOf>>,
    rooms: Option<ResMut<Rooms>>,
) {
    let link = trigger.entity;
    let (Ok((remote, addr, data)), Some(mut rooms)) = (links.get(link), rooms) else {
        return;
    };
    // The address the link came from: a player's own, a VPN's exit, or nginx's (WebSocket: its access log
    // has the player's).
    let from = addr.map_or_else(|| "?".to_string(), |a| a.0.ip().to_string());
    info!("connected: {:?} from {from}", remote.0);
    let uid = data.map(|d| uid_from_user_data(&d.0)).unwrap_or_default();
    rooms.hub.open(conn_of(link), from, uid);
}

/// A client is gone (it left, timed out, or was let go of).
fn on_disconnected(trigger: On<Remove, Connected>, rooms: Option<ResMut<Rooms>>) {
    if let Some(mut rooms) = rooms {
        rooms.hub.close(conn_of(trigger.entity));
    }
}
