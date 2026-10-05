//! Connecting: a connect token from the HTTP API, then UDP (WebSocket if UDP is silent for 2 s); after a drop, again.
//! On WebSocket, `auto` checks UDP once a minute and moves back to it between rounds.
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
use crate::servers::Target;
use crate::session::Session;
use crate::settings::{Me, Player};

/// How long UDP gets to answer before `auto` goes over WebSocket.
pub const UDP_TRY_S: f32 = 2.0;
/// How often `auto` on WebSocket checks whether UDP works.
const PROBE_EVERY_S: f32 = 60.0;
/// Wait before asking the HTTP API again after a failed request.
const RETRY_S: f32 = 2.0;

type Asked = Mutex<Receiver<Result<SessionReply, String>>>;

#[derive(Resource)]
pub struct Conn {
    /// The server's HTTP API, and its WebSocket URL once a session reply named it.
    pub http: String,
    ws: Option<String>,
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
    /// The link is being closed to say hello again (into practice and back): reconnect at once, same transport.
    pub restart: bool,
    /// The link was up at least once (a drop after that is a reconnect).
    pub ever: bool,
    /// Why `auto` went over WebSocket, and when (None while on UDP or with WebSocket asked for).
    pub fallback: Option<(Fallback, f32)>,
    /// The last UDP check on WebSocket: when, and whether UDP worked.
    pub udp_check: Option<(f32, bool)>,
    /// The last failure to connect, and how many times in a row (`again`).
    failing: Option<(String, u32)>,
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
        app.add_systems(First, disconnect_hung_up);
        app.add_systems(Last, despawn_closed);
    }
}

/// A link to disconnect at the start of the next frame. What the server sent (beans, the map) goes with the link,
/// and other systems may have commands queued on it this frame: an `insert` on a despawned bean panics.
#[derive(Component)]
struct HangUp;

pub fn hang_up(commands: &mut Commands, link: Entity) {
    commands.entity(link).insert(HangUp);
}

fn disconnect_hung_up(mut commands: Commands, links: Query<(Entity, Has<Closing>), With<HangUp>>) {
    for (entity, closing) in &links {
        commands.entity(entity).remove::<HangUp>();
        commands.trigger(Disconnect { entity });
        if closing {
            commands.queue(|world: &mut World| {
                let mut q =
                    world.query_filtered::<Entity, With<bevy_replicon::client::confirm_history::ConfirmHistory>>();
                for e in q.iter(world).collect::<Vec<_>>() {
                    world.despawn(e);
                }
            });
        }
    }
}

/// A link left or down: it lives until its disconnect packets went out (`PostUpdate`), so that the server lets
/// the player go at once instead of keeping them in the room until the connection times out.
#[derive(Component)]
struct Closing;

fn despawn_closed(mut commands: Commands, closing: Query<Entity, (With<Closing>, Without<HangUp>)>) {
    for e in &closing {
        commands.entity(e).despawn();
    }
}

fn setup_prediction(mut commands: Commands) {
    let mut manager = PredictionManager::default();
    // A second: at 150 ms RTT plus jitter the default 20 ticks rejects every rollback.
    manager.rollback_policy.max_rollback_ticks = 120;
    commands.insert_resource(manager);
}

/// The WebSocket URL when nobody named one: the HTTP API's host on the WebSocket port.
fn default_ws(http: &str, port: u16) -> String {
    let rest = http.split_once("://").map_or(http, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = match authority.rfind(']') {
        Some(i) => authority.get(..=i).unwrap_or(authority),
        None => authority.split(':').next().unwrap_or(authority),
    };
    format!("ws://{host}:{port}")
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

/// Closes the link to connect again right away (the hello goes out anew).
pub fn restart(commands: &mut Commands, conn: &mut Conn) {
    if let Some(entity) = conn.entity {
        conn.restart = true;
        hang_up(commands, entity);
    }
}

/// Asks the HTTP API for a connect token on a thread of its own.
fn ask(conn: &mut Conn, now: f32) {
    let (tx, rx) = channel();
    let url = format!("{}/api/session", conn.http);
    let req = session_request(conn, conn.transport);
    std::thread::spawn(move || {
        let _ = tx.send(request(&url, &req));
    });
    conn.asking = Some(Mutex::new(rx));
    conn.next_try = None;
    conn.started = now;
}

/// A socket of the server address's family on any port (an IPv4 socket cannot send to an IPv6 address,
/// and on Windows an IPv6 one does not take IPv4).
pub fn local_addr_for(server: SocketAddr) -> LocalAddr {
    let any: IpAddr = if server.is_ipv6() {
        Ipv6Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::UNSPECIFIED.into()
    };
    LocalAddr(SocketAddr::new(any, 0))
}

fn spawn_client(
    commands: &mut Commands,
    opts: &Opts,
    conn: &Conn,
    token: ConnectToken,
    now: f32,
) -> Result<Entity, String> {
    let transport = conn.transport;
    let netcode = NetcodeClient::new(Authentication::Token(token), NetcodeConfig::default())
        .map_err(|e| format!("netcode client: {e:?}"))?;
    // The server address comes from the token (`PeerAddr` set by `NetcodeClient`).
    let local = local_addr_for(netcode.inner.server_addr());
    let mut e = commands.spawn((
        Name::new("client"),
        Client,
        ReplicationReceiver,
        Link::default().with_conditioner(conditioner(opts, transport, now).map(RecvLinkConditioner::new)),
        local,
        netcode,
    ));
    let target = if transport == Transport::Ws {
        let url = opts
            .ws_url
            .clone()
            .or_else(|| conn.ws.clone())
            .unwrap_or_else(|| default_ws(&conn.http, opts.ws_port));
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
    Ok(entity)
}

fn connect_first(mut commands: Commands, me: Me, target: Res<Target>, time: Res<Time>) {
    if let Some(http) = target.0.clone() {
        open(&mut commands, &me.opts, me.identity(), http, time.elapsed_secs());
    }
}

/// Starts connecting to the server whose HTTP API is `http`.
pub fn open(commands: &mut Commands, opts: &Opts, identity: Option<String>, http: String, now: f32) {
    let transport = if opts.transport == Transport::Ws {
        Transport::Ws
    } else {
        Transport::Udp
    };
    let mut conn = Conn {
        http,
        ws: None,
        transport,
        entity: None,
        started: 0.0,
        connected: false,
        identity,
        asking: None,
        next_try: None,
        probe: None,
        next_probe: None,
        udp_works: false,
        moving: false,
        restart: false,
        ever: false,
        fallback: None,
        udp_check: None,
        failing: None,
    };
    ask(&mut conn, now);
    commands.insert_resource(conn);
}

/// Leaves the server: the link goes, and with it everything the server sent.
pub fn close(commands: &mut Commands, conn: &Conn) {
    if let Some(entity) = conn.entity {
        hang_up(commands, entity);
        commands.entity(entity).insert(Closing);
    }
    commands.remove_resource::<Conn>();
}

/// The HTTP API's answer: connect, wait (the game is being updated), or give up (this client is out of date).
fn receive_session(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    conn: Option<ResMut<Conn>>,
    mut player: ResMut<Player>,
) {
    let Some(mut conn) = conn else { return };
    let now = time.elapsed_secs();
    if conn.next_try.is_some_and(|t| now >= t) && !session.refused {
        ask(&mut conn, now);
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
        Err(e) => return failed(&mut conn, e, now),
    };
    conn.identity = Some(reply.identity.clone());
    if reply.ws_url.is_some() {
        conn.ws = reply.ws_url.clone();
    }
    if opts.token.is_none() && player.identity != reply.identity {
        player.identity = reply.identity.clone();
        crate::settings::save_now(&mut commands);
    }
    if reply.protocol != PROTOCOL_VERSION {
        error!(
            "this client is out of date: the server speaks protocol {} (build {}), this one {PROTOCOL_VERSION}",
            reply.protocol, reply.build
        );
        session.refused = true;
        return;
    }
    let Some(token) = token_of(&reply) else {
        warn!("session: no connect token");
        conn.next_try = Some(now + RETRY_S);
        return;
    };
    match spawn_client(&mut commands, &opts, &conn, token, now) {
        Ok(entity) => {
            conn.entity = Some(entity);
            conn.started = now;
        }
        // (A token netcode does not take: ask for another one, as after any failed request.)
        Err(e) => failed(&mut conn, e, now),
    }
}

/// A session request failed.
fn failed(conn: &mut Conn, e: String, now: f32) {
    conn.next_try = Some(now + RETRY_S);
    if let Some(n) = again(&mut conn.failing, format!("session: {e}")) {
        warn!("session: {e}{n}");
    }
}

/// The same failure over and over (the server is down): said the first time, then every tenth time.
fn again(slot: &mut Option<(String, u32)>, what: String) -> Option<String> {
    let n = match slot {
        Some((last, n)) if *last == what => {
            *n += 1;
            *n
        }
        s => {
            *s = Some((what, 1));
            1
        }
    };
    match n {
        1 => Some(String::new()),
        n if n % 10 == 0 => Some(format!(" ({n} times)")),
        _ => None,
    }
}

/// Connected, or the link went down: then again from the HTTP API (`auto` moves from UDP to WebSocket).
fn watch_link(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time>,
    session: Res<Session>,
    conn: Option<ResMut<Conn>>,
    links: Query<(Has<Connected>, Option<&Disconnected>)>,
) {
    let Some(mut conn) = conn else { return };
    let now = time.elapsed_secs();
    let Some(entity) = conn.entity else { return };
    let Ok((connected, gone)) = links.get(entity) else {
        return;
    };
    let down = gone.is_some();
    if connected && !conn.connected {
        conn.connected = true;
        conn.ever = true;
        conn.failing = None;
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
    // (Its disconnect packets go out first: hung up in `First`, it is despawned in `Last`.)
    commands.entity(entity).insert(Closing);
    conn.entity = None;
    (conn.probe, conn.next_probe, conn.udp_works) = (None, None, false);
    if session.refused {
        return;
    }
    if core::mem::take(&mut conn.restart) {
        ask(&mut conn, now);
        return;
    }
    if core::mem::take(&mut conn.moving) {
        info!("UDP works again: moving back to it");
        conn.transport = Transport::Udp;
        conn.fallback = None;
        ask(&mut conn, now);
        return;
    }
    let reason = gone.map_or(String::new(), |d| d.reason.to_string());
    if was {
        warn!("connection lost over {:?}: {reason}", conn.transport);
    } else {
        let what = format!("no connection over {:?}: {reason}", conn.transport);
        if let Some(n) = again(&mut conn.failing, what.clone()) {
            warn!("{what}{n}");
        }
    }
    if opts.transport == Transport::Auto && conn.transport == Transport::Udp {
        conn.transport = Transport::Ws;
        conn.fallback = Some((if was { Fallback::Lost } else { Fallback::Silent }, now));
        ask(&mut conn, now);
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
    ask(&mut conn, now);
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
        let url = format!("{}/api/session", conn.http);
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
        hang_up(&mut commands, entity);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn socket_of_the_servers_family() {
        let l = |a: &str| super::local_addr_for(a.parse().unwrap()).0;
        assert!(l("192.168.1.10:5888").is_ipv4());
        assert!(l("[::1]:5888").is_ipv6());
        assert_eq!(l("[::1]:5888").port(), 0);
    }

    #[test]
    fn default_ws() {
        let w = |h: &str| super::default_ws(h, 5889);
        assert_eq!(w("http://192.168.1.10:5887/fallbeans"), "ws://192.168.1.10:5889");
        assert_eq!(w("https://game.example.com/fallbeans"), "ws://game.example.com:5889");
        assert_eq!(w("http://[::1]:5887/fallbeans"), "ws://[::1]:5889");
    }
}
