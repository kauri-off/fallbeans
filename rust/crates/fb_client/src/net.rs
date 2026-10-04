//! Connecting: a connect token from the HTTP API, then UDP (WebSocket if UDP is silent for 2 s); after a drop, again.
//! On WebSocket, `auto` checks UDP once a minute and moves back to it between rounds.
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bevy::prelude::*;
use fb_proto::{Phase, SessionReply, SessionRequest};
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
pub const UDP_TRY_S: f32 = 2.0;
/// How often `auto` on WebSocket checks whether UDP works.
const PROBE_EVERY_S: f32 = 60.0;
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
    /// `auto` on WebSocket: the UDP check under way, when the next one is due, and that UDP worked.
    probe: Option<Mutex<Receiver<bool>>>,
    next_probe: Option<f32>,
    udp_works: bool,
    /// The link is being closed to come back over UDP.
    moving: bool,
    /// Why `auto` went over WebSocket, and when (None while on UDP or with WebSocket asked for).
    pub fallback: Option<(Fallback, f32)>,
    /// The last UDP check on WebSocket: when, and whether UDP worked.
    pub udp_check: Option<(f32, bool)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// No answer over UDP within `UDP_TRY_S` of connecting.
    Silent,
    /// The UDP connection went down mid-game.
    Lost,
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_prediction, connect_first).chain());
        app.add_systems(
            Update,
            (receive_session, watch_link, fallback_to_ws, back_to_udp, spike_link).chain(),
        );
    }
}

fn setup_prediction(mut commands: Commands) {
    let mut manager = PredictionManager::default();
    // A second, as the TS client keeps: at 150 ms RTT plus jitter the default 20 ticks rejects every rollback.
    manager.rollback_policy.max_rollback_ticks = 120;
    commands.insert_resource(manager);
}

fn http_url(opts: &Opts) -> String {
    opts.http_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}/fallbeans", opts.server, opts.http_port))
}

fn session_request(conn: &Conn, transport: Transport) -> SessionRequest {
    SessionRequest {
        identity: conn.identity.clone(),
        protocol: PROTOCOL_VERSION,
        transport: if transport == Transport::Ws { "ws" } else { "udp" }.into(),
    }
}

/// `POST <url>` of the session API (blocking).
pub fn request(url: &str, req: &SessionRequest) -> Result<SessionReply, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    agent
        .post(url)
        .send_json(req)
        .and_then(|mut r| r.body_mut().read_json::<SessionReply>())
        .map_err(|e| format!("{url}: {e}"))
}

pub fn token_of(reply: &SessionReply) -> Option<ConnectToken> {
    let bytes = B64.decode(reply.token.as_ref()?).ok()?;
    ConnectToken::try_from_bytes(&bytes).ok()
}

/// What a link over `transport` receives through: `--udp-blocked` drops everything UDP brings for a while,
/// `--spike` adds latency in bursts.
fn conditioner(opts: &Opts, transport: Transport, now: f32) -> Option<LinkConditionerConfig> {
    if transport != Transport::Ws && now < opts.udp_blocked {
        return Some(LinkConditionerConfig::default().with_fixed_loss(1.0));
    }
    let base = opts.net.config();
    if opts.spike == 0 {
        return base;
    }
    let base = base.unwrap_or_default();
    let extra = if in_spike(opts, now) { opts.spike } else { 0 };
    let lag = base.incoming_latency + Duration::from_millis(extra);
    Some(base.with_incoming_latency(lag))
}

/// How long a `--spike` lasts.
const SPIKE_S: f32 = 1.5;

fn in_spike(opts: &Opts, now: f32) -> bool {
    opts.spike > 0 && now.rem_euclid(opts.spike_every) >= opts.spike_every - SPIKE_S
}

/// `--spike`: swaps the link's conditioner as a burst starts and ends (packets already delayed keep their time).
fn spike_link(
    opts: Res<Opts>,
    time: Res<Time>,
    conn: Option<Res<Conn>>,
    mut links: Query<&mut Link>,
    mut was: Local<bool>,
) {
    let now = time.elapsed_secs();
    let on = in_spike(&opts, now);
    if on == *was {
        return;
    }
    *was = on;
    let Some(conn) = conn else { return };
    let Some(mut link) = conn.entity.and_then(|e| links.get_mut(e).ok()) else {
        return;
    };
    let Some(config) = conditioner(&opts, conn.transport, now) else {
        return;
    };
    let mut next = RecvLinkConditioner::new(config);
    if let Some(old) = link.recv.conditioner.take() {
        next.time_queue = old.time_queue;
    }
    link.recv.conditioner = Some(next);
}

/// Asks the HTTP API for a connect token on a thread of its own.
fn ask(conn: &mut Conn, opts: &Opts, now: f32) {
    let (tx, rx) = channel();
    let url = format!("{}/api/session", http_url(opts));
    let req = session_request(conn, conn.transport);
    std::thread::spawn(move || {
        let _ = tx.send(request(&url, &req));
    });
    conn.asking = Some(Mutex::new(rx));
    conn.next_try = None;
    conn.started = now;
}

fn spawn_client(commands: &mut Commands, opts: &Opts, token: ConnectToken, transport: Transport, now: f32) -> Entity {
    let netcode = NetcodeClient::new(Authentication::Token(token), NetcodeConfig::default()).expect("netcode client");
    // The server address comes from the token (`PeerAddr` set by `NetcodeClient`).
    let mut e = commands.spawn((
        Name::new("client"),
        Client,
        ReplicationReceiver,
        Link::default().with_conditioner(conditioner(opts, transport, now).map(RecvLinkConditioner::new)),
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
        probe: None,
        next_probe: None,
        udp_works: false,
        moving: false,
        fallback: None,
        udp_check: None,
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
    conn.identity = Some(reply.identity.clone());
    if reply.protocol != PROTOCOL_VERSION {
        error!(
            "this client is out of date: the server speaks protocol {} (build {}), this one {PROTOCOL_VERSION}",
            reply.protocol, reply.build
        );
        session.refused = true;
        return;
    }
    let Some(token) = token_of(&reply) else {
        if reply.updating {
            info!("the game is being updated: waiting");
        } else {
            warn!("session: no connect token");
        }
        conn.next_try = Some(now + if reply.updating { UPDATING_RETRY_S } else { RETRY_S });
        return;
    };
    let transport = conn.transport;
    conn.entity = Some(spawn_client(&mut commands, &opts, token, transport, now));
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
        if opts.transport == Transport::Auto && conn.transport == Transport::Ws {
            conn.next_probe = Some(now + PROBE_EVERY_S);
        }
    }
    // (A new link is `Disconnected` until `Connect` is applied.)
    if !down || now - conn.started < 0.2 {
        return;
    }
    let was = core::mem::take(&mut conn.connected);
    commands.entity(entity).despawn();
    conn.entity = None;
    (conn.probe, conn.next_probe, conn.udp_works) = (None, None, false);
    if session.refused {
        return;
    }
    if core::mem::take(&mut conn.moving) {
        info!("UDP works again: moving back to it");
        conn.transport = Transport::Udp;
        conn.fallback = None;
        ask(&mut conn, &opts, now);
        return;
    }
    if was {
        warn!("connection lost over {:?}", conn.transport);
    } else {
        warn!("no connection over {:?}", conn.transport);
    }
    if opts.transport == Transport::Auto && conn.transport == Transport::Udp {
        conn.transport = Transport::Ws;
        conn.fallback = Some((if was { Fallback::Lost } else { Fallback::Silent }, now));
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
    conn.fallback = Some((Fallback::Silent, now));
    ask(&mut conn, &opts, now);
}

/// `auto` on WebSocket: checks UDP every minute; once it works, closes the link at a moment that costs nothing
/// (between rounds, or outside a room: a drop takes the player out of a lobby) and reconnects over UDP, back
/// into the room. Should UDP fail after all, `auto` goes back to WebSocket as on any start.
fn back_to_udp(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time>,
    session: Res<Session>,
    conn: Option<ResMut<Conn>>,
) {
    let Some(mut conn) = conn else { return };
    let Some(entity) = conn.entity else { return };
    if !conn.connected || conn.moving || opts.transport != Transport::Auto || conn.transport != Transport::Ws {
        return;
    }
    let now = time.elapsed_secs();
    if let Some(probe) = &conn.probe {
        let got = probe.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match got {
            Err(TryRecvError::Empty) => {}
            Ok(works) => {
                conn.probe = None;
                conn.udp_works = works;
                conn.udp_check = Some((now, works));
                if !works {
                    info!("UDP still does not work: staying on WebSocket");
                    conn.next_probe = Some(now + PROBE_EVERY_S);
                }
            }
            Err(TryRecvError::Disconnected) => {
                conn.probe = None;
                conn.next_probe = Some(now + PROBE_EVERY_S);
            }
        }
    } else if conn.next_probe.is_some_and(|t| now >= t) {
        conn.next_probe = None;
        let url = format!("{}/api/session", http_url(&opts));
        let req = session_request(&conn, Transport::Udp);
        conn.probe = Some(Mutex::new(crate::probe::start(
            url,
            req,
            conditioner(&opts, Transport::Udp, now),
        )));
    }
    let calm = session.room.is_none()
        || session
            .lobby
            .as_ref()
            .is_some_and(|l| matches!(l.phase, Phase::Results | Phase::Podium));
    if conn.udp_works && calm {
        conn.moving = true;
        commands.trigger(Disconnect { entity });
    }
}
