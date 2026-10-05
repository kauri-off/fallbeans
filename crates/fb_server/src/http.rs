//! The HTTP API on its own thread (axum): sessions, health, debug.
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bevy::prelude::*;
use fb_net::PROTOCOL_ID;
use fb_proto::{SessionReply, SessionRequest};
use fb_shared::PROTOCOL_VERSION;
use lightyear::netcode::ConnectToken;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener as TokioListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Sleep;

use crate::auth::{
    Auth, Budget, DEBUG_COOKIE, DEBUG_TTL_S, Limiter, random_bytes, read_cookie, same_key, to_user_data,
};
use crate::opts::Opts;
use crate::play::Rooms;
use crate::rooms::debug::{room_state, room_trace};
use crate::rooms::room::Room;
use fb_net::logbook;

const BASE: &str = "/fallbeans";
/// A connect token is good for one connection attempt this soon.
const TOKEN_EXPIRE_S: i32 = 30;
/// Silence before netcode gives a connection up: WebSocket through a TCP tunnel stalls for seconds at a time.
const UDP_TIMEOUT_S: i32 = 3;
const WS_TIMEOUT_S: i32 = 10;
/// Metric samples kept for `/api/debug/health`.
const SAMPLES: usize = 240;
/// The main loop is taken for stuck when it has not answered for this long.
const MAIN_LOOP_PATIENCE: Duration = Duration::from_secs(2);
/// Session requests a minute per address (IPv6: per /64; this machine is exempt). A client asks once per
/// connection attempt, every 2 s at most while it retries; a script must not mint identities without end.
const SESSIONS_PER_MINUTE: usize = 60;
/// Largest request body (the session request is a few hundred bytes).
const BODY_MAX: usize = 16 * 1024;
/// Open HTTP connections at most: beyond it the listener waits (each is a task and a file descriptor).
const MAX_CONNECTIONS: usize = 512;
/// A connection that sends nothing and takes nothing for this long is closed. axum's server has no header
/// or keep-alive timeout of its own (hyper's needs a timer it is not given): a client could hold a
/// connection forever.
const IDLE: Duration = Duration::from_secs(20);

/// The server's secret: netcode keys, identities, the debug cookie.
#[derive(Resource, Clone)]
pub struct Keys(pub Arc<Auth>);

/// The counts `/health` shows, as the main loop last published them.
#[derive(Clone, Copy, Debug)]
struct Counts {
    at: Instant,
    rooms: usize,
    practice: usize,
    players: usize,
}

/// What the main loop tells the HTTP API without being asked.
#[derive(Default)]
pub struct Shared {
    samples: Mutex<VecDeque<Value>>,
    counts: Mutex<Option<Counts>>,
}

impl Shared {
    pub fn sample(&self, v: Value) {
        let mut s = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        if s.len() == SAMPLES {
            s.pop_front();
        }
        s.push_back(v);
    }

    fn samples(&self, n: usize) -> Vec<Value> {
        let s = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        s.iter().skip(s.len().saturating_sub(n)).cloned().collect()
    }
}

#[derive(Resource, Clone)]
pub struct HttpShared(pub Arc<Shared>);

/// Work on the rooms, run by the main loop between frames.
type Job = Box<dyn FnOnce(&Rooms) + Send>;

#[derive(Resource)]
struct Jobs(Mutex<Receiver<Job>>);

pub struct HttpPlugin;

impl Plugin for HttpPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start);
        app.add_systems(Update, (run_jobs, publish_counts));
    }
}

#[derive(Clone)]
struct Api {
    auth: Arc<Auth>,
    shared: Arc<Shared>,
    jobs: Sender<Job>,
    dev: bool,
    debug_key: Option<String>,
    public_host: Option<IpAddr>,
    public_ws_url: Option<String>,
    name: Option<String>,
    link_timeout: Option<i32>,
    udp_port: u16,
    ws_port: u16,
    guesses: Arc<Mutex<Limiter>>,
    sessions: Arc<Mutex<Budget>>,
    started: Instant,
    build: Arc<str>,
}

fn start(mut commands: Commands, opts: Res<Opts>, keys: Res<Keys>, shared: Res<HttpShared>) {
    let addr = SocketAddr::new(opts.http_addr, opts.http_port);
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| panic!("http {addr}: {e}"));
    listener.set_nonblocking(true).expect("non-blocking socket");
    let (tx, rx) = channel();
    commands.insert_resource(Jobs(Mutex::new(rx)));
    let api = Api {
        auth: keys.0.clone(),
        shared: shared.0.clone(),
        jobs: tx,
        dev: opts.dev,
        debug_key: std::env::var("FB_DEBUG_KEY").ok().filter(|k| !k.is_empty()),
        public_host: opts.public_host,
        public_ws_url: opts.public_ws_url.clone(),
        name: opts.name.clone(),
        link_timeout: opts.link_timeout,
        udp_port: opts.udp_port,
        ws_port: opts.ws_port,
        guesses: Arc::default(),
        sessions: Arc::new(Mutex::new(Budget::new(SESSIONS_PER_MINUTE))),
        started: Instant::now(),
        build: fb_net::build().into(),
    };
    std::thread::Builder::new()
        .name("http".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(serve(listener, api));
        })
        .expect("http thread");
    info!("http: {addr}");
}

async fn serve(listener: TcpListener, api: Api) {
    let listener = Guarded {
        inner: TokioListener::from_std(listener).expect("tokio listener"),
        slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
    };
    let app = Router::new()
        .route(&format!("{BASE}/health"), get(health))
        .route(&format!("{BASE}/api/session"), post(session))
        .route(&format!("{BASE}/api/debug/{{what}}"), get(debug))
        .layer(DefaultBodyLimit::max(BODY_MAX))
        .with_state(api);
    let service = app.into_make_service_with_connect_info::<Peer>();
    if let Err(e) = axum::serve(listener, service).await {
        error!("http: {e}");
    }
}

/// The HTTP listener with a cap on open connections and an idle timeout on each.
struct Guarded {
    inner: TokioListener,
    slots: Arc<Semaphore>,
}

impl axum::serve::Listener for Guarded {
    type Io = GuardedConn;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (GuardedConn, SocketAddr) {
        let slot = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");
        let (io, addr) = axum::serve::Listener::accept(&mut self.inner).await;
        (GuardedConn::new(io, slot, IDLE), addr)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// A connection that fails with `TimedOut` once it has been idle for `limit` (hyper then drops it).
struct GuardedConn {
    io: TcpStream,
    limit: Duration,
    idle: Pin<Box<Sleep>>,
    _slot: OwnedSemaphorePermit,
}

impl GuardedConn {
    fn new(io: TcpStream, slot: OwnedSemaphorePermit, limit: Duration) -> Self {
        Self {
            io,
            limit,
            idle: Box::pin(tokio::time::sleep(limit)),
            _slot: slot,
        }
    }

    /// What a read or write gave: activity restarts the idle clock, waiting past it is an error.
    fn watch<T>(&mut self, cx: &mut Context<'_>, r: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        match r {
            Poll::Ready(r) => {
                self.idle.as_mut().reset(tokio::time::Instant::now() + self.limit);
                Poll::Ready(r)
            }
            Poll::Pending if self.idle.as_mut().poll(cx).is_ready() => Poll::Ready(Err(io::ErrorKind::TimedOut.into())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRead for GuardedConn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.io).poll_read(cx, buf);
        this.watch(cx, r)
    }
}

impl AsyncWrite for GuardedConn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.io).poll_write(cx, buf);
        this.watch(cx, r)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.io).poll_flush(cx);
        this.watch(cx, r)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.io).poll_write_vectored(cx, bufs);
        this.watch(cx, r)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}

/// The counts for `/health`, every frame: the health check needs no work from the main loop.
fn publish_counts(rooms: Option<Res<Rooms>>, shared: Res<HttpShared>) {
    let Some(rooms) = rooms else { return };
    let listed = rooms.hub.listed().count();
    let players = rooms
        .hub
        .rooms
        .values()
        .map(|room| room.players.iter().filter(|p| !p.bot).count())
        .sum();
    *shared.0.counts.lock().unwrap_or_else(|e| e.into_inner()) = Some(Counts {
        at: Instant::now(),
        rooms: listed,
        practice: rooms.hub.rooms.len() - listed,
        players,
    });
}

fn run_jobs(jobs: Res<Jobs>, rooms: Option<Res<Rooms>>) {
    let Some(rooms) = rooms else { return };
    let rx = jobs.0.lock().unwrap_or_else(|e| e.into_inner());
    while let Ok(job) = rx.try_recv() {
        // A bug in a debug view must not take the game down: the request gets a 503.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&rooms))).is_err() {
            error!("http: a request panicked");
        }
    }
}

impl Api {
    /// Runs `f` on the rooms in the main loop and waits for the answer.
    async fn ask<T: Send + 'static>(&self, f: impl FnOnce(&Rooms) -> T + Send + 'static) -> Option<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |r| {
            let _ = tx.send(f(r));
        });
        self.jobs.send(job).ok()?;
        tokio::time::timeout(MAIN_LOOP_PATIENCE, rx).await.ok()?.ok()
    }

    fn debug_allowed(&self, ip: IpAddr, headers: &HeaderMap) -> bool {
        if self.dev && ip.is_loopback() {
            return true;
        }
        let cookie = read_cookie(headers.get(header::COOKIE).and_then(|v| v.to_str().ok()), DEBUG_COOKIE);
        self.debug_key.is_some() && cookie.is_some_and(|c| self.auth.valid_debug_cookie(c, unix_s()))
    }
}

fn unix_s() -> u64 {
    SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}

/// The client's address, and whether it came through a reverse proxy (which passes it as X-Real-IP).
fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> (IpAddr, bool) {
    let real = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    match real {
        Some(ip) if peer.ip().is_loopback() => (ip, true),
        _ => (peer.ip(), false),
    }
}

/// Both ends of a request's connection: who asked, and the address of ours they reached.
#[derive(Clone, Copy, Debug)]
struct Peer {
    remote: SocketAddr,
    local: Option<SocketAddr>,
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Guarded>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Guarded>) -> Self {
        Self {
            remote: *stream.remote_addr(),
            local: stream.io().io.local_addr().ok(),
        }
    }
}

/// The `Host` header without its port (`[v6]` keeps its brackets).
fn host_name(headers: &HeaderMap) -> Option<&str> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    if host.starts_with('[') {
        return host.find(']').and_then(|i| host.get(..=i));
    }
    Some(host.rsplit_once(':').map_or(host, |(h, _)| h))
}

/// The WebSocket URL for a client that asked with these headers.
fn ws_url(api: &Api, headers: &HeaderMap, local: Option<SocketAddr>) -> String {
    if let Some(url) = &api.public_ws_url {
        return url.clone();
    }
    let proto = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if let (Some("https"), Some(host)) = (proto, host) {
        return format!("wss://{host}{BASE}/ws");
    }
    let name = host_name(headers)
        .map(String::from)
        .or_else(|| {
            local.map(|a| match a.ip() {
                IpAddr::V6(v6) => format!("[{v6}]"),
                ip => ip.to_string(),
            })
        })
        .unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string());
    format!("ws://{name}:{}", api.ws_port)
}

/// The host the client asked at, when it is an IP address.
fn host_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    if let Ok(a) = host.parse::<SocketAddr>() {
        return Some(a.ip());
    }
    host.trim_start_matches('[').trim_end_matches(']').parse().ok()
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// JSON, or with `?format=text` the compact text of `to_text`.
fn respond(v: Value, text: bool, status: StatusCode) -> Response {
    let r = if text {
        let mut r = (status, format!("{}\n", to_text(&v, "").trim_start())).into_response();
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        r
    } else {
        (status, Json(v)).into_response()
    };
    no_store(r)
}

/// Compact, readable text (YAML-like, arrays of plain values on one line).
pub fn to_text(v: &Value, indent: &str) -> String {
    let deeper = format!("{indent}  ");
    match v {
        Value::Null => "null".into(),
        Value::String(s) => s.clone(),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Array(a) if a.iter().all(|x| !x.is_object() && !x.is_array()) => {
            let items: Vec<String> = a.iter().map(|x| to_text(x, indent)).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Array(a) => a
            .iter()
            .map(|x| format!("\n{indent}- {}", to_text(x, &deeper).trim_start()))
            .collect(),
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => o
            .iter()
            .map(|(k, x)| format!("\n{indent}{k}: {}", to_text(x, &deeper)))
            .collect(),
        other => other.to_string(),
    }
}

/// Up, its version, and how busy.
/// Anyone may ask, so it reads what the main loop published instead of queueing work for it; a main loop
/// that stopped publishing is a 503.
async fn health(State(api): State<Api>) -> Response {
    let counts = *api.shared.counts.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = counts.filter(|c| c.at.elapsed() <= MAIN_LOOP_PATIENCE) else {
        return respond(json!({ "ok": false }), false, StatusCode::SERVICE_UNAVAILABLE);
    };
    let v = json!({
        "ok": true,
        "name": api.name,
        "version": PROTOCOL_VERSION,
        "build": &*api.build,
        "rooms": c.rooms,
        "players": c.players,
        "practice": c.practice,
    });
    respond(v, false, StatusCode::OK)
}

/// The player's identity (theirs if still valid, else a new one) and a connect token for one connection.
async fn session(
    State(api): State<Api>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Json(req): Json<SessionRequest>,
) -> Response {
    let (ip, _) = client_ip(peer.remote, &headers);
    let now_ms = api.started.elapsed().as_millis() as u64;
    let allowed = api
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow(&ip.to_string(), now_ms);
    if !allowed {
        // (The client takes any error for a failed attempt and asks again in a few seconds.)
        return (StatusCode::TOO_MANY_REQUESTS, "too many sessions").into_response();
    }
    let known = req
        .identity
        .as_deref()
        .and_then(|t| Some((api.auth.identity(t)?, t.to_string())));
    let (uid, identity) = known.unwrap_or_else(|| api.auth.issue_identity());
    let mut reply = SessionReply {
        protocol: PROTOCOL_VERSION,
        build: api.build.to_string(),
        identity,
        token: None,
        ws_url: Some(ws_url(&api, &headers, peer.local)),
    };
    if req.protocol == PROTOCOL_VERSION {
        let host = api
            .public_host
            .or_else(|| host_ip(&headers))
            .or_else(|| peer.local.map(|a| a.ip()).filter(|ip| !ip.is_unspecified()))
            .unwrap_or(Ipv4Addr::LOCALHOST.into());
        let timeout = api.link_timeout.unwrap_or(if req.transport == "ws" {
            WS_TIMEOUT_S
        } else {
            UDP_TIMEOUT_S
        });
        let id = u64::from_le_bytes(random_bytes::<8>()).max(1);
        let token = ConnectToken::build(
            SocketAddr::new(host, api.udp_port),
            PROTOCOL_ID,
            id,
            api.auth.netcode_key(),
        )
        .expire_seconds(TOKEN_EXPIRE_S)
        .timeout_seconds(timeout)
        .user_data(to_user_data(&uid, Some(ip)))
        .generate();
        match token.map(|t| t.try_into_bytes()) {
            Ok(Ok(bytes)) => reply.token = Some(B64.encode(bytes)),
            Ok(Err(e)) => error!("connect token: {e}"),
            Err(e) => error!("connect token: {e:?}"),
        }
    }
    no_store(Json(reply).into_response())
}

/// `?room=` a room id or its number in the state listing (default: the first room).
fn room_at<'a>(rooms: &'a Rooms, q: Option<&str>) -> Option<&'a Room> {
    let q = q.unwrap_or("0");
    let mut all = rooms.hub.rooms.values();
    all.clone()
        .find(|r| r.id == q)
        .or_else(|| q.parse().ok().and_then(|i| all.nth(i)))
}

async fn debug(
    State(api): State<Api>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(what): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let text = q.get("format").is_some_and(|f| f == "text");
    let (ip, proxied) = client_ip(peer.remote, &headers);
    if what == "login" {
        return login(&api, ip, proxied, q.get("key").map_or("", String::as_str));
    }
    if !api.debug_allowed(ip, &headers) {
        let error = if api.dev || api.debug_key.is_some() {
            "forbidden"
        } else {
            "debug API is off"
        };
        return respond(json!({ "ok": false, "error": error }), text, StatusCode::FORBIDDEN);
    }
    let num = |k: &str, d: f64| q.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
    let room_q = q.get("room").cloned();
    let busy = || {
        respond(
            json!({ "ok": false, "error": "no answer from the rooms" }),
            text,
            StatusCode::SERVICE_UNAVAILABLE,
        )
    };
    let v = match what.as_str() {
        "state" => {
            let Some((rooms, conns, tick)) = api
                .ask(|r| {
                    let rooms: Vec<Value> = r
                        .hub
                        .rooms
                        .values()
                        .enumerate()
                        .map(|(i, room)| {
                            let mut v = room_state(room);
                            v["room"] = i.into();
                            v
                        })
                        .collect();
                    (rooms, r.hub.sessions.len(), r.hub.real_tick())
                })
                .await
            else {
                return busy();
            };
            json!({
                "server": {
                    "build": &*api.build,
                    "protocol": PROTOCOL_VERSION,
                    "dev": api.dev,
                    "pid": std::process::id(),
                    "uptime": api.started.elapsed().as_secs(),
                    "tick": tick,
                    "connections": conns,
                    "health": api.shared.samples(1).pop(),
                    "warnings": logbook::warnings(),
                },
                "rooms": rooms,
            })
        }
        "health" => json!({ "samples": api.shared.samples(num("n", 120.0) as usize) }),
        "logs" => json!({ "lines": logbook::recent(q.get("level").map(String::as_str), num("n", 100.0) as usize) }),
        "trace" => {
            let id = q.get("id").and_then(|v| v.parse().ok());
            let s = num("s", 10.0).clamp(1.0, 30.0);
            match api
                .ask(move |r| room_at(r, room_q.as_deref()).map(|room| room_trace(room, id, s)))
                .await
            {
                None => return busy(),
                Some(None) => {
                    return respond(
                        json!({ "ok": false, "error": "no such room" }),
                        text,
                        StatusCode::NOT_FOUND,
                    );
                }
                Some(Some(v)) => v,
            }
        }
        "replay" => {
            let i = q.get("i").filter(|v| *v != "current").and_then(|v| v.parse().ok());
            let Some(rec) = api.ask(move |r| room_at(r, room_q.as_deref())?.debug_replay(i)).await else {
                return busy();
            };
            let Some(rec) = rec else {
                let error = "no recording (dev rooms record rounds)";
                return respond(json!({ "ok": false, "error": error }), text, StatusCode::NOT_FOUND);
            };
            let name = format!("inline; filename=\"{}-{}.json\"", rec.game, rec.seed);
            let mut r = respond(serde_json::to_value(&rec).unwrap_or_default(), false, StatusCode::OK);
            if let Ok(v) = HeaderValue::from_str(&name) {
                r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
            }
            return r;
        }
        "maps" => {
            let maps: Vec<Value> = fb_maps::GAMES
                .iter()
                .map(|g| {
                    let m = g.meta();
                    json!({ "id": m.id, "title": m.title, "genre": format!("{:?}", m.genre), "duration": m.duration })
                })
                .collect();
            json!({ "maps": maps })
        }
        _ => {
            let endpoints = ["login", "state", "health", "logs", "trace", "replay", "maps"];
            let v = json!({ "ok": false, "error": "unknown endpoint", "endpoints": endpoints });
            return respond(v, text, StatusCode::NOT_FOUND);
        }
    };
    respond(v, text, StatusCode::OK)
}

/// `login?key=FB_DEBUG_KEY`: the debug cookie, for a week.
fn login(api: &Api, ip: IpAddr, proxied: bool, key: &str) -> Response {
    let now_ms = api.started.elapsed().as_millis() as u64;
    let mut guesses = api.guesses.lock().unwrap_or_else(|e| e.into_inner());
    if !guesses.allow(&ip.to_string(), "debug", now_ms) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many attempts").into_response();
    }
    if !api.debug_key.as_deref().is_some_and(|want| same_key(key, want)) {
        guesses.failed(&ip.to_string(), "debug", now_ms);
        warn!(%ip, "bad debug key");
        return (StatusCode::FORBIDDEN, "wrong key").into_response();
    }
    let cookie = format!(
        "{DEBUG_COOKIE}={}; Path={BASE}/; Max-Age={DEBUG_TTL_S}; HttpOnly; SameSite=Strict{}",
        api.auth.issue_debug_cookie(unix_s()),
        if proxied { "; Secure" } else { "" }
    );
    let mut r = no_store((StatusCode::OK, "ok\n").into_response());
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_compact_text() {
        let v = json!({ "a": 1, "b": [1, 2], "c": [{ "d": "x" }], "e": null });
        assert_eq!(to_text(&v, "").trim_start(), "a: 1\nb: [1, 2]\nc: \n  - d: x\ne: null");
    }

    #[test]
    fn reads_the_host_asked_at() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("192.168.1.5:5887"));
        assert_eq!(host_ip(&h), Some("192.168.1.5".parse().unwrap()));
        h.insert(header::HOST, HeaderValue::from_static("[::1]:5887"));
        assert_eq!(host_ip(&h), Some("::1".parse().unwrap()));
        h.insert(header::HOST, HeaderValue::from_static("example.org"));
        assert_eq!(host_ip(&h), None);
    }

    #[test]
    fn closes_idle_connections_and_keeps_busy_ones() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
            let (io, _) = listener.accept().await.unwrap();
            let slot = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
            let mut conn = GuardedConn::new(io, slot, Duration::from_millis(300));
            async fn read(conn: &mut GuardedConn) -> (Result<(), io::ErrorKind>, Duration) {
                let mut bytes = [0u8; 8];
                let started = Instant::now();
                let r =
                    std::future::poll_fn(|cx| Pin::new(&mut *conn).poll_read(cx, &mut ReadBuf::new(&mut bytes))).await;
                (r.map_err(|e| e.kind()), started.elapsed())
            }
            // A byte every 200 ms keeps it open past its 300 ms limit.
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                client.writable().await.unwrap();
                client.try_write(b"x").unwrap();
                assert_eq!(read(&mut conn).await.0, Ok(()));
            }
            let (r, waited) = read(&mut conn).await;
            assert_eq!(r, Err(io::ErrorKind::TimedOut));
            assert!(waited >= Duration::from_millis(250), "{waited:?}");
        });
    }
}
