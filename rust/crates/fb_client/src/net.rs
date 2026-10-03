//! Connecting: a connect token from the HTTP API, then UDP (WebSocket if UDP is silent for 2 s); after a drop, again.
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bevy::prelude::*;
use fb_proto::{SessionReply, SessionRequest};
use fb_shared::PROTOCOL_VERSION;
use lightyear::connection::client::Connected;
use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::netcode::{ConnectToken, NetcodeClient};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use lightyear::websocket::client::WebSocketTarget;

use crate::opts::{Opts, Transport};
use crate::session::Session;

/// How long UDP gets to answer before `auto` goes over WebSocket.
const UDP_TRY_S: f32 = 2.0;
/// Waits before asking the HTTP API again: after a failed request, and while the game is being updated.
const RETRY_S: f32 = 2.0;
const UPDATING_RETRY_S: f32 = 5.0;

type Asked = Mutex<Receiver<Result<SessionReply, String>>>;

#[derive(Resource)]
pub struct Conn {
    pub transport: Transport,
    /// The link (None while there is no connection attempt).
    pub entity: Option<Entity>,
    /// When the link was made (or the last request sent).
    pub started: f32,
    pub connected: bool,
    /// The player's identity from the HTTP API, kept for later requests: the same player after a reconnect.
    pub identity: Option<String>,
    asking: Option<Asked>,
    /// When to ask the HTTP API for a connection.
    next_try: Option<f32>,
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_input_delay, connect_first).chain());
        app.add_systems(Update, (receive_session, watch_link, fallback_to_ws).chain());
    }
}

fn setup_input_delay(mut commands: Commands, opts: Res<Opts>) {
    let mut manager = PredictionManager::default();
    // A second, as the TS client keeps: at 150 ms RTT plus jitter the default 20 ticks rejects every rollback.
    manager.rollback_policy.max_rollback_ticks = 120;
    commands.insert_resource(manager);
    let mut sync = SyncConfig {
        jitter_margin: opts.input_margin,
        ..default()
    };
    if let Some(m) = opts.sync_max_error {
        sync.max_error_margin = m;
    }
    commands.insert_resource(
        InputTimelineConfig::default()
            .with_sync_config(sync)
            .with_input_delay(InputDelayConfig::no_input_delay()),
    );
}

fn http_url(opts: &Opts) -> String {
    opts.http_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}/fallbeans", opts.server, opts.http_port))
}

/// Asks the HTTP API for a connect token on a thread of its own.
fn ask(conn: &mut Conn, opts: &Opts, now: f32) {
    let (tx, rx) = channel();
    let url = format!("{}/api/session", http_url(opts));
    let req = SessionRequest {
        identity: conn.identity.clone(),
        protocol: PROTOCOL_VERSION,
        transport: if conn.transport == Transport::Ws { "ws" } else { "udp" }.into(),
    };
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        let reply = agent
            .post(&url)
            .send_json(&req)
            .and_then(|mut r| r.body_mut().read_json::<SessionReply>())
            .map_err(|e| format!("{url}: {e}"));
        let _ = tx.send(reply);
    });
    conn.asking = Some(Mutex::new(rx));
    conn.next_try = None;
    conn.started = now;
}

fn spawn_client(commands: &mut Commands, opts: &Opts, token: ConnectToken, transport: Transport) -> Entity {
    let netcode = NetcodeClient::new(Authentication::Token(token), NetcodeConfig::default()).expect("netcode client");
    // The server address comes from the token (`PeerAddr` set by `NetcodeClient`).
    let mut e = commands.spawn((
        Name::new("client"),
        Client,
        ReplicationReceiver,
        Link::default().with_conditioner(opts.net.config().map(RecvLinkConditioner::new)),
        LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
        netcode,
    ));
    let target = if transport == Transport::Ws {
        let url = opts
            .ws_url
            .clone()
            .unwrap_or_else(|| format!("ws://{}:{}", opts.server, opts.ws_port));
        let builder = aeronet_websocket::client::ClientConfig::builder;
        let config = if url.starts_with("wss://") {
            builder().with_native_certs()
        } else {
            builder().with_no_encryption()
        };
        // Small packets many times a second: Nagle's algorithm (aeronet's default) would hold them
        // back for an ACK, tens of milliseconds each time.
        let config = config.disable_nagle();
        e.insert(WebSocketClientIo {
            config,
            target: WebSocketTarget::Url(url.clone()),
        });
        url
    } else {
        e.insert(UdpIo::default());
        "the token's UDP address".into()
    };
    let entity = e.id();
    commands.trigger(Connect { entity });
    info!("connecting to {target} over {transport:?}");
    entity
}

fn connect_first(mut commands: Commands, opts: Res<Opts>, time: Res<Time>) {
    let transport = if opts.transport == Transport::Ws {
        Transport::Ws
    } else {
        Transport::Udp
    };
    let mut conn = Conn {
        transport,
        entity: None,
        started: 0.0,
        connected: false,
        identity: opts.token.clone(),
        asking: None,
        next_try: None,
    };
    ask(&mut conn, &opts, time.elapsed_secs());
    commands.insert_resource(conn);
}

/// The HTTP API's answer: connect, wait (the game is being updated), or give up (this client is out of date).
fn receive_session(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    conn: Option<ResMut<Conn>>,
) {
    let Some(mut conn) = conn else { return };
    let now = time.elapsed_secs();
    if conn.next_try.is_some_and(|t| now >= t) && !session.refused {
        ask(&mut conn, &opts, now);
    }
    let Some(asking) = &conn.asking else { return };
    let got = asking.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
    let reply = match got {
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("the request thread is gone".to_string()),
        Ok(r) => r,
    };
    conn.asking = None;
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            warn!("session: {e}");
            conn.next_try = Some(now + RETRY_S);
            return;
        }
    };
    conn.identity = Some(reply.identity);
    if reply.protocol != PROTOCOL_VERSION {
        error!(
            "this client is out of date: the server speaks protocol {} (build {}), this one {PROTOCOL_VERSION}",
            reply.protocol, reply.build
        );
        session.refused = true;
        return;
    }
    let token = reply
        .token
        .and_then(|t| B64.decode(t).ok())
        .and_then(|b| ConnectToken::try_from_bytes(&b).ok());
    let Some(token) = token else {
        if reply.updating {
            info!("the game is being updated: waiting");
        } else {
            warn!("session: no connect token");
        }
        conn.next_try = Some(now + if reply.updating { UPDATING_RETRY_S } else { RETRY_S });
        return;
    };
    let transport = conn.transport;
    conn.entity = Some(spawn_client(&mut commands, &opts, token, transport));
    conn.started = now;
}

/// Connected, or the link went down: then again from the HTTP API (`auto` moves from UDP to WebSocket).
fn watch_link(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time>,
    session: Res<Session>,
    conn: Option<ResMut<Conn>>,
    links: Query<(Has<Connected>, Has<Disconnected>)>,
) {
    let Some(mut conn) = conn else { return };
    let now = time.elapsed_secs();
    let Some(entity) = conn.entity else { return };
    let Ok((connected, down)) = links.get(entity) else {
        return;
    };
    if connected && !conn.connected {
        conn.connected = true;
        info!("connected over {:?}", conn.transport);
    }
    // (A new link is `Disconnected` until `Connect` is applied.)
    if !down || now - conn.started < 0.2 {
        return;
    }
    let was = core::mem::take(&mut conn.connected);
    commands.entity(entity).despawn();
    conn.entity = None;
    if session.refused {
        return;
    }
    if was {
        warn!("connection lost over {:?}", conn.transport);
    } else {
        warn!("no connection over {:?}", conn.transport);
    }
    if opts.transport == Transport::Auto && conn.transport == Transport::Udp {
        conn.transport = Transport::Ws;
        ask(&mut conn, &opts, now);
    } else {
        conn.next_try = Some(now + if was { 0.0 } else { RETRY_S });
    }
}

fn fallback_to_ws(mut commands: Commands, opts: Res<Opts>, time: Res<Time>, conn: Option<ResMut<Conn>>) {
    let Some(mut conn) = conn else { return };
    let Some(entity) = conn.entity else { return };
    if conn.connected || opts.transport != Transport::Auto || conn.transport != Transport::Udp {
        return;
    }
    let now = time.elapsed_secs();
    if now - conn.started < UDP_TRY_S {
        return;
    }
    warn!("no answer over UDP in {UDP_TRY_S} s: trying WebSocket");
    commands.entity(entity).despawn();
    conn.entity = None;
    conn.transport = Transport::Ws;
    ask(&mut conn, &opts, now);
}
