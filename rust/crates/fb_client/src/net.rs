//! Connecting: UDP first, WebSocket when UDP does not get through within 2 s (or when asked).
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::net::ToSocketAddrs;

use bevy::prelude::*;
use fb_net::*;
use lightyear::connection::client::Connected;
use lightyear::netcode::NetcodeClient;
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use lightyear::websocket::client::WebSocketTarget;

use crate::opts::{Opts, Transport};

#[derive(Resource)]
pub struct Conn {
    pub id: u64,
    pub transport: Transport,
    pub entity: Entity,
    pub started: f32,
    pub connected: bool,
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_input_delay, connect_first).chain());
        app.add_systems(Update, fallback_to_ws);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn setup_input_delay(mut commands: Commands, opts: Res<Opts>) {
    let mut manager = PredictionManager::default();
    // A second, as the TS client keeps: at 150 ms RTT plus jitter the default 20 ticks rejects every rollback.
    manager.rollback_policy.max_rollback_ticks = 120;
    commands.insert_resource(manager);
    let sync = SyncConfig {
        jitter_margin: opts.input_margin,
        ..default()
    };
    commands.insert_resource(
        InputTimelineConfig::default()
            .with_sync_config(sync)
            .with_input_delay(InputDelayConfig::no_input_delay()),
    );
}

fn resolve(host: &str, port: u16) -> SocketAddr {
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.find(|a| a.is_ipv4()))
        .unwrap_or_else(|| panic!("cannot resolve {host}"))
}

fn spawn_client(commands: &mut Commands, opts: &Opts, id: u64, transport: Transport) -> Entity {
    let port = if transport == Transport::Ws {
        opts.ws_port
    } else {
        opts.udp_port
    };
    let server_addr = resolve(&opts.server, port);
    let auth = Authentication::Manual {
        server_addr,
        client_id: id,
        private_key: DEV_KEY,
        protocol_id: PROTOCOL_ID,
    };
    let netcode = NetcodeClient::new(
        auth,
        NetcodeConfig {
            client_timeout_secs: 3,
            token_expire_secs: -1,
            ..default()
        },
    )
    .expect("netcode client");
    let mut e = commands.spawn((
        Name::new("client"),
        Client,
        ReplicationReceiver,
        Link::default().with_conditioner(opts.net.config().map(RecvLinkConditioner::new)),
        LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
        PeerAddr(server_addr),
        netcode,
    ));
    if transport == Transport::Ws {
        let url = opts
            .ws_url
            .clone()
            .unwrap_or_else(|| format!("ws://{}:{}", opts.server, opts.ws_port));
        let config = if url.starts_with("wss://") {
            aeronet_websocket::client::ClientConfig::default()
        } else {
            aeronet_websocket::client::ClientConfig::builder().with_no_encryption()
        };
        e.insert(WebSocketClientIo {
            config,
            target: WebSocketTarget::Url(url),
        });
    } else {
        e.insert(UdpIo::default());
    }
    let entity = e.id();
    commands.trigger(Connect { entity });
    info!("connecting to {server_addr} over {transport:?} as {id}");
    entity
}

fn connect_first(mut commands: Commands, opts: Res<Opts>, time: Res<Time>) {
    let id = opts.id.unwrap_or_else(|| {
        let n = std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_nanos() as u64;
        (n ^ (n >> 31) ^ std::process::id() as u64) & 0x7fff_ffff
    });
    let transport = if opts.transport == Transport::Ws {
        Transport::Ws
    } else {
        Transport::Udp
    };
    let entity = spawn_client(&mut commands, &opts, id, transport);
    commands.insert_resource(Conn {
        id,
        transport,
        entity,
        started: time.elapsed_secs(),
        connected: false,
    });
}

fn fallback_to_ws(mut commands: Commands, opts: Res<Opts>, time: Res<Time>, conn: Option<ResMut<Conn>>) {
    let Some(mut conn) = conn else { return };
    if conn.connected || opts.transport != Transport::Auto || conn.transport != Transport::Udp {
        return;
    }
    if time.elapsed_secs() - conn.started < Duration::from_secs(2).as_secs_f32() {
        return;
    }
    warn!("no answer over UDP in 2 s: trying WebSocket");
    commands.entity(conn.entity).despawn();
    conn.entity = spawn_client(&mut commands, &opts, conn.id, Transport::Ws);
    conn.transport = Transport::Ws;
    conn.started = time.elapsed_secs();
}

fn on_connected(_: On<Add, Connected>, conn: Option<ResMut<Conn>>) {
    if let Some(mut conn) = conn {
        conn.connected = true;
        info!("connected over {:?}", conn.transport);
    }
}

fn on_disconnected(_: On<Add, Disconnected>, conn: Option<ResMut<Conn>>) {
    if let Some(mut conn) = conn
        && conn.connected
    {
        conn.connected = false;
        warn!("disconnected");
    }
}
