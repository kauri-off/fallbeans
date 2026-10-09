//! Listening on UDP and WebSocket, and turning connections into pawns.
use core::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use fb_net::*;
use lightyear::connection::client::{Connected, Disconnected, Disconnecting};
use lightyear::netcode::{NetcodeServer, TokenUserData};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear::websocket::prelude::tungstenite::protocol::WebSocketConfig;

use crate::auth::from_user_data;
use crate::http::Keys;
use crate::opts::Opts;
use crate::play::{Rooms, conn_of};

/// Largest WebSocket frame or message a client may send (bytes).
const WS_MESSAGE_MAX: usize = 64 * 1024;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_server);
        app.add_systems(Update, (reap_links, drop_broken_links, watch_server, shut_down));
        app.add_observer(on_link);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

/// A link that has not connected this long after its first packet is let go of (s; at least the link timeout).
const LINK_CONNECT_S: f64 = 6.0;
/// Links not connected (yet) at most: beyond it the oldest go first.
const MAX_PENDING_LINKS: usize = 256;
/// After telling the players, the server stops its transports (netcode's disconnect packets go out), then exits (s).
const SHUTDOWN_TELL_S: f64 = 0.5;
const SHUTDOWN_STOP_S: f64 = 0.2;

/// When a link came (server time, s).
#[derive(Component, Clone, Copy, Debug)]
struct LinkedAt(f64);

/// SIGTERM, SIGINT or SIGHUP (`exit_on_signals`): the server goes down, telling the players first.
#[derive(Resource, Default)]
pub struct Shutdown {
    pub signal: Arc<AtomicBool>,
    stage: Stage,
}

/// How far the shutdown is, and since when (server time, s).
#[derive(Default, Clone, Copy, Debug)]
enum Stage {
    #[default]
    Running,
    Told(f64),
    Stopped(f64),
}

impl Shutdown {
    fn going(&self) -> bool {
        !matches!(self.stage, Stage::Running) || self.signal.load(Ordering::Relaxed)
    }
}

/// One Lightyear server for both transports: one netcode server sees every client, and the topology stays
/// valid (a server per transport was `Invalid`, lightyear#1693).
fn start_server(mut commands: Commands, opts: Res<Opts>, keys: Res<Keys>) {
    let udp_addr = SocketAddr::new(opts.udp_addr, opts.udp_port);
    let ws_addr = SocketAddr::new(opts.ws_addr, opts.ws_port);
    // A client sends one netcode packet (about a kilobyte) per message: tungstenite's defaults (16 MB frames,
    // 64 MB messages) would let anyone who opens the port make the server buffer that much per connection.
    let socket = WebSocketConfig::default()
        .max_frame_size(Some(WS_MESSAGE_MAX))
        .max_message_size(Some(WS_MESSAGE_MAX));
    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(ws_addr)
        .with_no_encryption()
        .with_socket_config(socket);
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

/// A new link, not yet authenticated: any UDP packet from a new address makes one (`reap_links`).
fn on_link(trigger: On<Add, LinkOf>, time: Res<Time<Real>>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert((LinkedAt(time.elapsed_secs_f64()), Name::new("client")));
}

/// Links not connected yet.
type Unconnected = (With<LinkOf>, Without<Connected>);

/// Links that never connect go after LINK_CONNECT_S, the oldest beyond MAX_PENDING_LINKS: UDP makes one per source
/// before any token is checked. Their UDP address and handshake go too (patched `lightyear_udp`, `lightyear_netcode`).
fn reap_links(
    time: Res<Time<Real>>,
    opts: Res<Opts>,
    links: Query<(Entity, &LinkedAt), Unconnected>,
    mut pending: Local<Vec<(f64, Entity)>>,
    mut reaped: Local<(usize, f64)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let wait = LINK_CONNECT_S.max(opts.link_timeout.map_or(0.0, f64::from));
    pending.clear();
    let mut gone = 0;
    for (link, at) in &links {
        if now - at.0 > wait {
            commands.entity(link).try_despawn();
            gone += 1;
        } else {
            pending.push((at.0, link));
        }
    }
    if pending.len() > MAX_PENDING_LINKS {
        pending.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let extra = pending.len() - MAX_PENDING_LINKS;
        for &(_, link) in pending.iter().take(extra) {
            commands.entity(link).try_despawn();
        }
        gone += extra;
    }
    // A line a minute at most (a flood would fill the log).
    reaped.0 += gone;
    if reaped.0 > 0 && now - reaped.1 >= 60.0 {
        info!("let go of {} links that never connected", reaped.0);
        *reaped = (0, now);
    }
}

/// Clients' links that are connected.
type Joined = (With<ClientOf>, With<Connected>);

/// A client whose packet was acked but whose messages were refused (too large, or a partial one expired):
/// its reliable channels would never deliver again, so it goes and comes back with a fresh link.
fn drop_broken_links(links: Query<(Entity, &Transport, &RemoteId), Joined>, mut commands: Commands) {
    for (link, transport, remote) in &links {
        if transport.receive_failed() {
            warn!("dropping {:?}: a reliable message from it was lost", remote.0);
            commands.entity(link).insert(Disconnecting);
        }
    }
}

/// Whether the server started, stopped or was unlinked (and why).
type ServerState = (Has<Started>, Has<Stopped>, Option<&'static Unlinked>);

/// The transports gave up: an accept error closes the shared `Server`. The process exits and systemd restarts it.
fn watch_server(
    servers: Query<ServerState, With<NetcodeServer>>,
    shutdown: Res<Shutdown>,
    mut started: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    for (on, stopped, unlinked) in &servers {
        *started |= on;
        if *started && (stopped || unlinked.is_some()) && !shutdown.going() {
            let reason = unlinked.map(|u| u.reason.to_string()).unwrap_or_default();
            error!("the server stopped listening ({reason}): exiting");
            exit.write(AppExit::from_code(1));
            *started = false;
        }
    }
}

/// A signal (`Shutdown`): everyone is told the server is restarting, the transports are stopped, then the
/// process exits.
fn shut_down(
    time: Res<Time<Real>>,
    mut shutdown: ResMut<Shutdown>,
    rooms: Option<ResMut<Rooms>>,
    servers: Query<Entity, With<NetcodeServer>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if !shutdown.signal.load(Ordering::Relaxed) {
        return;
    }
    let now = time.elapsed_secs_f64();
    match shutdown.stage {
        Stage::Running => {
            info!("shutting down: telling the players");
            if let Some(mut rooms) = rooms {
                rooms.hub.shutdown("Сервер перезапускается");
            }
            shutdown.stage = Stage::Told(now);
        }
        Stage::Told(at) if now - at >= SHUTDOWN_TELL_S => {
            for server in &servers {
                commands.trigger(Stop { entity: server });
            }
            shutdown.stage = Stage::Stopped(now);
        }
        Stage::Stopped(at) if now - at >= SHUTDOWN_STOP_S => {
            info!("shut down");
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// A client's link, where it is from and its connect token's data.
type Arrival = (
    &'static RemoteId,
    Option<&'static PeerAddr>,
    Option<&'static TokenUserData>,
);

/// A client is in, as the player of its connect token: the hub waits for its hello.
fn on_connected(
    trigger: On<Add, Connected>,
    links: Query<Arrival, With<ClientOf>>,
    rooms: Option<ResMut<Rooms>>,
    mut commands: Commands,
) {
    let link = trigger.entity;
    // Replication only to links that connected (Lightyear admits a link that gets its sender late): links
    // that never do (`reap_links`) cost the replication systems nothing.
    commands.entity(link).insert(ReplicationSender);
    let (Ok((remote, addr, data)), Some(mut rooms)) = (links.get(link), rooms) else {
        return;
    };
    // Through a local proxy the sealed token's address stands for the player, else all share 127.0.0.1.
    let (uid, asked_from) = data.map(|d| from_user_data(&d.0)).unwrap_or_default();
    let link_ip = addr.map(|a| a.0.ip());
    let ip = match link_ip {
        Some(ip) if !ip.is_loopback() => Some(ip),
        _ => asked_from.or(link_ip),
    };
    let from = ip.map_or_else(|| "?".to_string(), |ip| ip.to_string());
    match link_ip.filter(|l| Some(*l) != ip) {
        Some(via) => info!("connected: {:?} from {from} via {via}", remote.0),
        None => info!("connected: {:?} from {from}", remote.0),
    }
    rooms.hub.open(conn_of(link), ip, uid);
}

/// A client's link, why it went and its last stats.
type Departure = (&'static RemoteId, Option<&'static Disconnected>, Option<&'static Link>);

/// A client is gone (it left, timed out, or was let go of).
fn on_disconnected(
    trigger: On<Remove, Connected>,
    links: Query<Departure, With<ClientOf>>,
    rooms: Option<ResMut<Rooms>>,
) {
    if let Ok((remote, gone, link)) = links.get(trigger.entity) {
        let reason = gone.map_or("?".into(), |d| d.reason.to_string());
        let rtt = link.map_or(0, |l| l.stats.rtt.as_millis());
        info!("disconnected: {:?} ({reason}), rtt {rtt} ms", remote.0);
    }
    if let Some(mut rooms) = rooms {
        rooms.hub.close(conn_of(trigger.entity));
    }
}
