//! The HTTP API on its own thread (axum): sessions, health, debug.
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
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
use fb_shared::text::sanitize_title;
use lightyear::netcode::ConnectToken;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener as TokioListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Sleep;

use crate::auth::{
    Auth, Budget, DEBUG_COOKIE, DEBUG_TTL_S, Limiter, address_key, random_bytes, read_cookie, same_key, to_user_data,
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
const SESSIONS_PER_MINUTE: usize = 128;
/// Largest request body (the session request is a few hundred bytes).
const BODY_MAX: usize = 16 * 1024;
/// Open HTTP connections at most: beyond it the listener waits (each is a task and a file descriptor).
const MAX_CONNECTIONS: usize = 512;
/// A connection that sends nothing and takes nothing for this long is closed. axum's server has no header
/// or keep-alive timeout of its own (hyper's needs a timer it is not given): a client could hold a
/// connection forever.
const IDLE: Duration = Duration::from_secs(20);
/// A request (head and body) must be in this long after its first byte, however its bytes trickle in.
const REQUEST: Duration = Duration::from_secs(10);
/// Open connections from one address (`auth::address_key`: IPv6 per /64, this machine exempt): one host
/// must not take the MAX_CONNECTIONS everybody shares.
const CONNECTIONS_PER_ADDRESS: usize = 32;
/// Names looked up for connect tokens behind a proxy are kept this long, and so many at most.
const RESOLVED_FOR: Duration = Duration::from_secs(60);
const RESOLVED_MAX: usize = 64;

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
    /// The HTTP thread is gone: nobody can get a connect token any more (`watch_http`).
    down: AtomicBool,
}

/// Set on drop: whichever way the HTTP thread ends (its server returned, a panic), the main loop hears of it.
struct Down(Arc<Shared>);

impl Drop for Down {
    fn drop(&mut self) {
        self.0.down.store(true, Ordering::SeqCst);
    }
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
        app.add_systems(Update, (run_jobs, publish_counts, watch_http));
    }
}

/// Without its HTTP API nobody can join: the process exits with an error, and systemd starts it again.
fn watch_http(shared: Res<HttpShared>, mut exit: MessageWriter<AppExit>, mut told: Local<bool>) {
    if !*told && shared.0.down.load(Ordering::SeqCst) {
        *told = true;
        error!("http: the API stopped: exiting");
        exit.write(AppExit::from_code(1));
    }
}

/// Warnings about the setup, once per run.
#[derive(Default)]
struct Warned {
    /// A local proxy passes `X-Forwarded-Proto` but not `X-Real-IP`.
    no_real_ip: AtomicBool,
    /// Behind a proxy without `--public-host`, the token's UDP address is a guess.
    no_public_host: AtomicBool,
    /// An IPv6 address in a token while UDP listens on IPv4 only.
    v4_only: AtomicBool,
}

impl Warned {
    /// True the first time.
    fn first(flag: &AtomicBool) -> bool {
        !flag.swap(true, Ordering::Relaxed)
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
    /// UDP listens on IPv6 too (`--udp-addr ::`).
    udp_v6: bool,
    ws_port: u16,
    guesses: Arc<Mutex<Limiter>>,
    sessions: Arc<Mutex<Budget>>,
    started: Instant,
    build: Arc<str>,
    warned: Arc<Warned>,
    /// Host names looked up for connect tokens (`proxied_host`), and when.
    resolved: Arc<Mutex<BTreeMap<String, (Option<IpAddr>, Instant)>>>,
}

fn start(mut commands: Commands, opts: Res<Opts>, keys: Res<Keys>, shared: Res<HttpShared>) {
    let addr = SocketAddr::new(opts.http_addr, opts.http_port);
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| panic!("http {addr}: {e}"));
    listener.set_nonblocking(true).expect("non-blocking socket");
    let (tx, rx) = channel();
    commands.insert_resource(Jobs(Mutex::new(rx)));
    if opts.dev && !opts.http_addr.is_loopback() {
        warn!(
            "DEV SERVER ON {addr}: --dev opens the debug API to every request from this machine (a reverse proxy \
             that sends no X-Real-IP passes everyone through) and lets every room's host run dev commands. \
             Use --http-addr 127.0.0.1, or no --dev on a public server"
        );
    }
    let name = opts.name.as_deref().map(sanitize_title).filter(|n| !n.is_empty());
    if opts.name.is_some() && name.as_deref() != opts.name.as_deref() {
        warn!(name = ?name, "FB_NAME / --name cut to what players can be shown");
    }
    let api = Api {
        auth: keys.0.clone(),
        shared: shared.0.clone(),
        jobs: tx,
        dev: opts.dev,
        debug_key: std::env::var("FB_DEBUG_KEY").ok().filter(|k| !k.is_empty()),
        public_host: opts.public_host,
        public_ws_url: opts.public_ws_url.clone(),
        name,
        link_timeout: opts.link_timeout,
        udp_port: opts.udp_port,
        udp_v6: opts.udp_addr.is_ipv6(),
        ws_port: opts.ws_port,
        guesses: Arc::default(),
        sessions: Arc::new(Mutex::new(Budget::new(SESSIONS_PER_MINUTE))),
        started: Instant::now(),
        build: fb_net::build().into(),
        warned: Arc::default(),
        resolved: Arc::default(),
    };
    std::thread::Builder::new()
        .name("http".into())
        .spawn(move || {
            let _down = Down(api.shared.clone());
            // (Blocking threads only look up host names, `proxied_host`.)
            let rt = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(2)
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
        per_address: Arc::default(),
        refused: Hush::default(),
    };
    let app = Router::new()
        .route(&format!("{BASE}/health"), get(health))
        .route(&format!("{BASE}/api/session"), post(session))
        // (POST: a key in the URL would end up in proxies' access logs.)
        .route(&format!("{BASE}/api/debug/login"), post(login))
        .route(&format!("{BASE}/api/debug/{{what}}"), get(debug))
        .layer(DefaultBodyLimit::max(BODY_MAX))
        .with_state(api);
    let service = app.into_make_service_with_connect_info::<Peer>();
    // (`serve` only returns on an error; the thread ending tells the main loop, `Down`.)
    match axum::serve(listener, service).await {
        Ok(()) => error!("http: the server returned"),
        Err(e) => error!("http: {e}"),
    }
}

/// At most one warning a second (a flood would fill the log): Some(how many were left out) when to log.
#[derive(Default)]
struct Hush {
    last: Option<Instant>,
    hushed: u32,
}

impl Hush {
    fn due(&mut self) -> Option<u32> {
        if self.last.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            self.hushed += 1;
            return None;
        }
        self.last = Some(Instant::now());
        Some(core::mem::take(&mut self.hushed))
    }
}

/// The open connections per address (`auth::address_key`).
type PerAddress = Arc<Mutex<BTreeMap<String, usize>>>;

/// One connection counted against its address until it is dropped (None: this machine, not counted).
struct AddressSlot(Option<(PerAddress, String)>);

impl AddressSlot {
    /// A place for a connection from `ip`, or None when its address has CONNECTIONS_PER_ADDRESS open.
    fn take(per_address: &PerAddress, ip: IpAddr) -> Option<Self> {
        let Some(key) = address_key(&ip.to_string()) else {
            return Some(Self(None));
        };
        let mut open = per_address.lock().unwrap_or_else(|e| e.into_inner());
        let n = open.entry(key.clone()).or_insert(0);
        if *n >= CONNECTIONS_PER_ADDRESS {
            return None;
        }
        *n += 1;
        Some(Self(Some((per_address.clone(), key))))
    }
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        let Some((per_address, key)) = &self.0 else { return };
        let mut open = per_address.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = open.get_mut(key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.remove(key);
            }
        }
    }
}

/// The HTTP listener with a cap on open connections (in all and per address) and deadlines on each.
struct Guarded {
    inner: TokioListener,
    slots: Arc<Semaphore>,
    per_address: PerAddress,
    refused: Hush,
}

impl axum::serve::Listener for Guarded {
    type Io = GuardedConn;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (GuardedConn, SocketAddr) {
        loop {
            let slot = self
                .slots
                .clone()
                .acquire_owned()
                .await
                .expect("the semaphore is never closed");
            let (io, addr) = axum::serve::Listener::accept(&mut self.inner).await;
            let Some(mine) = AddressSlot::take(&self.per_address, addr.ip()) else {
                if let Some(hushed) = self.refused.due() {
                    warn!(%addr, hushed, "http: too many connections from one address");
                }
                // (Closed at once; its slot is free for the next one.)
                drop(io);
                continue;
            };
            return (GuardedConn::new(io, (slot, mine), IDLE, REQUEST), addr);
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// A connection that fails with `TimedOut` (hyper then drops it) once it has been idle for `limit`, or when
/// a request is not in `request` after its first byte (a byte at a time does not keep it open).
struct GuardedConn {
    io: TcpStream,
    limit: Duration,
    idle: Pin<Box<Sleep>>,
    request: Duration,
    /// The deadline of the request being read; None while a response is going out.
    head: Option<Pin<Box<Sleep>>>,
    /// A response went out since the last request began: the next byte read is a new request.
    answered: bool,
    _slots: (OwnedSemaphorePermit, AddressSlot),
}

impl GuardedConn {
    fn new(io: TcpStream, slots: (OwnedSemaphorePermit, AddressSlot), limit: Duration, request: Duration) -> Self {
        Self {
            io,
            limit,
            idle: Box::pin(tokio::time::sleep(limit)),
            request,
            head: Some(Box::pin(tokio::time::sleep(request))),
            answered: false,
            _slots: slots,
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
    /// A response is going out: the request it answers is in.
    fn answering<T>(&mut self, r: &Poll<io::Result<T>>) {
        if matches!(r, Poll::Ready(Ok(_))) {
            self.answered = true;
            self.head = None;
        }
    }
}

impl AsyncRead for GuardedConn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(head) = &mut this.head
            && head.as_mut().poll(cx).is_ready()
        {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        let before = buf.filled().len();
        let r = Pin::new(&mut this.io).poll_read(cx, buf);
        if this.answered && matches!(r, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            // The first bytes of the next request on this connection: it has REQUEST to come in whole.
            this.answered = false;
            let mut head = Box::pin(tokio::time::sleep(this.request));
            // (Polled once so that its waker is in place should the next read wait.)
            let _ = head.as_mut().poll(cx);
            this.head = Some(head);
        }
        this.watch(cx, r)
    }
}

impl AsyncWrite for GuardedConn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.io).poll_write(cx, buf);
        this.answering(&r);
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
        this.answering(&r);
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
        let (Some(key), Some(cookie)) = (self.debug_key.as_deref(), cookie) else {
            return false;
        };
        self.auth.valid_debug_cookie(cookie, unix_s(), key)
    }

    /// A reverse proxy on this machine that says https but not who is asking: every player looks like this
    /// machine, which no limit per address applies to (and all share one bucket of PIN guesses).
    fn check_proxy(&self, peer: SocketAddr, headers: &HeaderMap) {
        if peer.ip().is_loopback()
            && headers.contains_key("x-forwarded-proto")
            && !headers.contains_key("x-real-ip")
            && Warned::first(&self.warned.no_real_ip)
        {
            warn!(
                "a reverse proxy passes X-Forwarded-Proto but no X-Real-IP: every player is 127.0.0.1 to the \
                 server (no limits per address, one PIN limit for all). Add `proxy_set_header X-Real-IP \
                 $remote_addr;`"
            );
        }
    }

    /// The UDP address for a connect token: `--public-host`, else the IP the client asked at, else (behind
    /// a reverse proxy) what the name it asked at resolves to, else the address of our socket that took the
    /// request.
    async fn token_host(&self, headers: &HeaderMap, local: Option<SocketAddr>, proxied: bool) -> IpAddr {
        if let Some(h) = self.public_host {
            return h;
        }
        if let Some(h) = host_ip(headers) {
            return self.family_checked(h.to_canonical());
        }
        if proxied {
            if let Some(h) = self.proxied_host(headers).await {
                return self.family_checked(h);
            }
            // Our socket's address is the proxy's side: 127.0.0.1. The client tries UDP there, gets nothing,
            // and falls back to WebSocket after its probe.
            if Warned::first(&self.warned.no_public_host) {
                warn!(
                    "behind a reverse proxy without --public-host, and the name players ask at does not resolve: \
                     connect tokens cannot name the UDP address (players use WebSocket). Pass --public-host"
                );
            }
        }
        let local = local.map(|a| a.ip().to_canonical()).filter(|ip| !ip.is_unspecified());
        self.family_checked(local.unwrap_or(Ipv4Addr::LOCALHOST.into()))
    }

    /// `ip`, with a warning (once) when it is IPv6 and UDP listens on IPv4 only: a client given it falls back
    /// to WebSocket.
    fn family_checked(&self, ip: IpAddr) -> IpAddr {
        if ip.is_ipv6() && !self.udp_v6 && Warned::first(&self.warned.v4_only) {
            warn!(%ip, "a client asked over IPv6, but UDP listens on IPv4 only (--udp-addr ::): it will use WebSocket");
        }
        ip
    }

    /// What the host name the client asked at (`Host`, behind a reverse proxy) resolves to, cached RESOLVED_FOR.
    async fn proxied_host(&self, headers: &HeaderMap) -> Option<IpAddr> {
        let name = host_name(headers)?;
        let plain = !name.is_empty()
            && name.len() <= 253
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !plain {
            return None;
        }
        {
            let cache = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(&(ip, _)) = cache.get(name).filter(|(_, at)| at.elapsed() < RESOLVED_FOR) {
                return ip;
            }
        }
        let lookup = tokio::net::lookup_host((name, self.udp_port));
        let found = match tokio::time::timeout(Duration::from_secs(1), lookup).await {
            Ok(Ok(addrs)) => addrs
                .map(|a| a.ip().to_canonical())
                .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
                .min_by_key(|ip| (ip.is_ipv6() && !self.udp_v6, ip.is_ipv6())),
            _ => None,
        };
        let mut cache = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= RESOLVED_MAX {
            cache.clear();
        }
        cache.insert(name.to_string(), (found, Instant::now()));
        found
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
    let (ip, proxied) = client_ip(peer.remote, &headers);
    api.check_proxy(peer.remote, &headers);
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
        let host = api.token_host(&headers, peer.local, proxied).await;
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
    let (ip, _) = client_ip(peer.remote, &headers);
    api.check_proxy(peer.remote, &headers);
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

/// `POST login` with FB_DEBUG_KEY as the body (`curl -d "$FB_DEBUG_KEY" -c jar …/api/debug/login`; `key=…`
/// also works): the debug cookie, for a week, or until the key changes.
async fn login(
    State(api): State<Api>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let (ip, proxied) = client_ip(peer.remote, &headers);
    api.check_proxy(peer.remote, &headers);
    let key = body.trim();
    let key = key.strip_prefix("key=").unwrap_or(key);
    let now_ms = api.started.elapsed().as_millis() as u64;
    let mut guesses = api.guesses.lock().unwrap_or_else(|e| e.into_inner());
    if !guesses.allow(&ip.to_string(), "debug", now_ms) {
        return (StatusCode::TOO_MANY_REQUESTS, "too many attempts").into_response();
    }
    let Some(want) = api.debug_key.as_deref().filter(|want| same_key(key, want)) else {
        guesses.failed(&ip.to_string(), "debug", now_ms);
        warn!(%ip, "bad debug key");
        return (StatusCode::FORBIDDEN, "wrong key").into_response();
    };
    let cookie = format!(
        "{DEBUG_COOKIE}={}; Path={BASE}/; Max-Age={DEBUG_TTL_S}; HttpOnly; SameSite=Strict{}",
        api.auth.issue_debug_cookie(unix_s(), want),
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

    /// A connection to ourselves, as the listener takes it (`idle` and `request` limits), and its client end.
    async fn pair(idle: Duration, request: Duration) -> (GuardedConn, TcpStream) {
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (io, _) = listener.accept().await.unwrap();
        let slot = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        (GuardedConn::new(io, (slot, AddressSlot(None)), idle, request), client)
    }

    async fn read(conn: &mut GuardedConn) -> (Result<(), io::ErrorKind>, Duration) {
        let mut bytes = [0u8; 8];
        let started = Instant::now();
        let r = std::future::poll_fn(|cx| Pin::new(&mut *conn).poll_read(cx, &mut ReadBuf::new(&mut bytes))).await;
        (r.map_err(|e| e.kind()), started.elapsed())
    }

    async fn send_byte(client: &TcpStream, after_ms: u64) {
        tokio::time::sleep(Duration::from_millis(after_ms)).await;
        client.writable().await.unwrap();
        client.try_write(b"x").unwrap();
    }

    fn block_on(f: impl std::future::Future<Output = ()>) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(f);
    }

    #[test]
    fn closes_idle_connections_and_keeps_busy_ones() {
        block_on(async {
            let (mut conn, client) = pair(Duration::from_millis(300), Duration::from_secs(10)).await;
            // A byte every 200 ms keeps it open past its 300 ms limit.
            for _ in 0..3 {
                send_byte(&client, 200).await;
                assert_eq!(read(&mut conn).await.0, Ok(()));
            }
            let (r, waited) = read(&mut conn).await;
            assert_eq!(r, Err(io::ErrorKind::TimedOut));
            assert!(waited >= Duration::from_millis(250), "{waited:?}");
        });
    }

    #[test]
    fn a_request_trickling_in_byte_by_byte_runs_out_of_time() {
        block_on(async {
            let (mut conn, client) = pair(Duration::from_millis(300), Duration::from_millis(500)).await;
            for _ in 0..2 {
                send_byte(&client, 200).await;
                assert_eq!(read(&mut conn).await.0, Ok(()));
            }
            send_byte(&client, 200).await;
            assert_eq!(read(&mut conn).await.0, Err(io::ErrorKind::TimedOut));
        });
    }

    #[test]
    fn each_request_on_a_connection_has_its_own_deadline() {
        block_on(async {
            let (mut conn, client) = pair(Duration::from_secs(1), Duration::from_millis(500)).await;
            send_byte(&client, 300).await;
            assert_eq!(read(&mut conn).await.0, Ok(()));
            // The answer: the next request's clock starts with its first byte, past the first one's deadline.
            let wrote = std::future::poll_fn(|cx| Pin::new(&mut conn).poll_write(cx, b"ok")).await;
            assert_eq!(wrote.map_err(|e| e.kind()), Ok(2));
            send_byte(&client, 400).await;
            assert_eq!(read(&mut conn).await.0, Ok(()));
            send_byte(&client, 300).await;
            assert_eq!(read(&mut conn).await.0, Ok(()));
            send_byte(&client, 300).await;
            assert_eq!(read(&mut conn).await.0, Err(io::ErrorKind::TimedOut));
        });
    }

    #[test]
    fn caps_connections_per_address_but_not_this_machine() {
        let per: PerAddress = Arc::default();
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let mut held: Vec<AddressSlot> = (0..CONNECTIONS_PER_ADDRESS)
            .map(|_| AddressSlot::take(&per, ip).expect("a place"))
            .collect();
        assert!(AddressSlot::take(&per, ip).is_none());
        assert!(AddressSlot::take(&per, "203.0.113.10".parse().unwrap()).is_some());
        for _ in 0..40 {
            held.push(AddressSlot::take(&per, Ipv4Addr::LOCALHOST.into()).expect("this machine"));
        }
        held.remove(0);
        assert!(AddressSlot::take(&per, ip).is_some());
    }
}
