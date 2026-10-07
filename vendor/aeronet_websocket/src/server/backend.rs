use {
    super::{ServerConfig, ServerError, ToConnected, ToOpen},
    crate::{
        server::{HandshakeHandler, ToConnecting},
        session::SessionError,
    },
    aeronet_io::{connection::DisconnectReason, server::CloseReason},
    bevy_ecs::prelude::*,
    core::{
        mem,
        net::{IpAddr, Ipv6Addr, SocketAddr},
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    },
    std::{
        collections::BTreeMap,
        sync::{Arc, Mutex, PoisonError},
    },
    futures::{
        FutureExt, SinkExt, StreamExt,
        channel::{mpsc, oneshot},
        never::Never,
    },
    tokio::{
        io::{AsyncRead, AsyncWrite, ReadBuf},
        net::{TcpListener, TcpStream},
    },
    tokio_rustls::TlsAcceptor,
    tokio_tungstenite::tungstenite::{
        handshake::server::{Request, Response},
        protocol::WebSocketConfig,
    },
    tracing::{Instrument, debug, debug_span, warn},
};

/// fallbeans patch: how long a connection may take to finish the TLS and WebSocket handshakes.
const HANDSHAKE_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);

/// fallbeans patch: connections open at once (each is a task, a socket and a session entity), in all and
/// from one address (IPv6: its /64). This machine is exempt from the second: a reverse proxy's connections
/// all come from it.
const MAX_CONNECTIONS: usize = 512;
const MAX_PER_ADDRESS: usize = 32;

pub async fn start(
    config: ServerConfig,
    tx_next: oneshot::Sender<ToOpen>,
) -> Result<Never, CloseReason> {
    let tls_acceptor = config.tls.map(TlsAcceptor::from);
    let listener = TcpListener::bind(config.bind_address)
        .await
        .map_err(ServerError::BindSocket)?;
    debug!("Listening on {}", config.bind_address);

    let (tx_connecting, rx_connecting) = mpsc::channel::<ToConnecting>(1);
    let (tx_dropped, mut rx_dropped) = mpsc::channel::<()>(0);

    let local_addr = listener.local_addr().map_err(SessionError::GetLocalAddr)?;
    let next = ToOpen {
        local_addr,
        rx_connecting,
        tx_dropped,
    };
    tx_next
        .send(next)
        .map_err(|_| SessionError::FrontendClosed)?;

    debug!("Starting server loop");
    let limits = Arc::new(Limits::default());
    let mut hush = Hush::default();
    loop {
        let result = futures::select! {
            x = listener.accept().fuse() => x,
            _ = rx_dropped.next() => {
                return Err(CloseReason::ByError(SessionError::FrontendClosed.into()));
            }
        };
        // fallbeans patch: a failed accept used to end the loop, and with it the server: every link of the
        // Lightyear server it belongs to, UDP too. Out of file descriptors (EMFILE, ENFILE) it waits a second
        // for some to close; after other errors (a connection reset before it was taken) a moment.
        let (stream, peer_addr) = match result {
            Ok(accepted) => accepted,
            Err(err) => {
                let pause = if out_of_descriptors(&err) { 1000 } else { 50 };
                if let Some(hushed) = hush.due() {
                    warn!("WebSocket accept failed ({hushed} more not logged): {err}");
                }
                tokio::time::sleep(Duration::from_millis(pause)).await;
                continue;
            }
        };
        // fallbeans patch: beyond the caps a connection is closed at once.
        let slot = match limits.take(peer_addr.ip()) {
            Ok(slot) => slot,
            Err(why) => {
                if let Some(hushed) = hush.due() {
                    warn!("WebSocket connection from {peer_addr} refused: {why} ({hushed} more not logged)");
                }
                drop(stream);
                continue;
            }
        };

        tokio::spawn({
            let tx_connecting = tx_connecting.clone();
            let tls_acceptor = tls_acceptor.clone();
            let handshake_handler = config.handshake_handler.clone();
            async move {
                // (Counted until the session is over.)
                let _slot = slot;
                if let Err(err) = accept_session(
                    stream,
                    peer_addr,
                    config.socket,
                    tls_acceptor,
                    tx_connecting,
                    handshake_handler,
                )
                .await
                {
                    debug!("Failed to accept session: {err:?}");
                }
            }
        });
    }
}

/// fallbeans patch: the process is out of file descriptors (EMFILE, ENFILE; WSAEMFILE on Windows).
fn out_of_descriptors(err: &std::io::Error) -> bool {
    if cfg!(windows) {
        err.raw_os_error() == Some(10024)
    } else {
        matches!(err.raw_os_error(), Some(23 | 24))
    }
}

/// fallbeans patch: at most one warning a second about failed or refused connections (a flood would fill the
/// log).
#[derive(Default)]
struct Hush {
    last: Option<std::time::Instant>,
    hushed: u32,
}

impl Hush {
    /// Whether to log now: Some(how many were left out since the last line).
    fn due(&mut self) -> Option<u32> {
        let now = std::time::Instant::now();
        if self
            .last
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1))
        {
            self.hushed += 1;
            return None;
        }
        self.last = Some(now);
        Some(mem::take(&mut self.hushed))
    }
}

/// fallbeans patch: who a cap per address counts against: the address, or for IPv6 its /64 (one subscriber
/// has billions of addresses); None for this machine.
fn address_key(ip: IpAddr) -> Option<IpAddr> {
    match ip.to_canonical() {
        ip if ip.is_loopback() || ip.is_unspecified() => None,
        IpAddr::V6(v6) => {
            let s = v6.segments();
            Some(IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0)))
        }
        v4 => Some(v4),
    }
}

/// fallbeans patch: the connections open now, in all and per address (`address_key`).
#[derive(Default)]
struct Limits {
    open: Mutex<(usize, BTreeMap<IpAddr, usize>)>,
}

impl Limits {
    fn take(self: &Arc<Self>, ip: IpAddr) -> Result<Slot, &'static str> {
        let key = address_key(ip);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.0 >= MAX_CONNECTIONS {
            return Err("too many connections");
        }
        if let Some(k) = key
            && open.1.get(&k).copied().unwrap_or(0) >= MAX_PER_ADDRESS
        {
            return Err("too many connections from this address");
        }
        open.0 += 1;
        if let Some(k) = key {
            *open.1.entry(k).or_insert(0) += 1;
        }
        Ok(Slot {
            limits: Arc::clone(self),
            key,
        })
    }
}

/// fallbeans patch: one open connection, counted until it is dropped.
struct Slot {
    limits: Arc<Limits>,
    key: Option<IpAddr>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut open = self.limits.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.0 = open.0.saturating_sub(1);
        if let Some(k) = self.key
            && let Some(n) = open.1.get_mut(&k)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.1.remove(&k);
            }
        }
    }
}

async fn accept_session(
    stream: TcpStream,
    peer_addr: SocketAddr,
    socket_config: WebSocketConfig,
    tls_acceptor: Option<TlsAcceptor>,
    mut tx_connecting: mpsc::Sender<ToConnecting>,
    handshake_handler: Option<HandshakeHandler>,
) -> Result<(), DisconnectReason> {
    let (tx_session_entity, rx_session_entity) = oneshot::channel::<Entity>();
    let (tx_dc_reason, rx_dc_reason) = oneshot::channel::<DisconnectReason>();
    let (tx_next, rx_next) = oneshot::channel::<ToConnected>();
    tx_connecting
        .send(ToConnecting {
            peer_addr,
            tx_session_entity,
            rx_dc_reason,
            rx_next,
        })
        .await
        .map_err(|_| SessionError::FrontendClosed)?;
    let session = rx_session_entity
        .await
        .map_err(|_| SessionError::FrontendClosed)?;

    let Err(dc_reason) = handle_session(
        stream,
        peer_addr,
        socket_config,
        tls_acceptor,
        tx_next,
        handshake_handler,
    )
    .instrument(debug_span!("session", %session))
    .await;
    _ = tx_dc_reason.send(dc_reason);
    Ok(())
}

async fn handle_session(
    stream: TcpStream,
    peer_addr: SocketAddr,
    socket_config: WebSocketConfig,
    tls_acceptor: Option<TlsAcceptor>,
    tx_next: oneshot::Sender<ToConnected>,
    handshake_handler: Option<HandshakeHandler>,
) -> Result<Never, DisconnectReason> {
    // fallbeans patch: small messages many times a second; Nagle's algorithm would hold each one
    // back until the previous one is acknowledged (the client side has `disable_nagle`, the server none).
    if let Err(err) = stream.set_nodelay(true) {
        debug!("Failed to set TCP_NODELAY: {err:?}");
    }
    debug!("Performing session handshake");

    // fallbeans patch: a client that connects and never finishes the handshake would hold its session
    // (entity, task, socket) for ever.
    let handshake = async {
        let stream = if let Some(tls_acceptor) = tls_acceptor {
            tls_acceptor
                .accept(stream)
                .await
                .map(MaybeTlsStream::Rustls)
                .map_err(ServerError::TlsHandshake)?
        } else {
            MaybeTlsStream::Plain(stream)
        };
        tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            #[expect(
                clippy::result_large_err,
                reason = "this `Result` is what `tokio_tungstenite` asks for"
            )]
            |req: &Request, resp: Response| match &handshake_handler {
                Some(h) => h.handle(req, resp),
                None => Ok(resp),
            },
            Some(socket_config),
        )
        .await
        .map_err(ServerError::AcceptClient)
    };
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake)
        .await
        .map_err(|_| ServerError::AcceptConnection(std::io::ErrorKind::TimedOut.into()))??;

    let (frontend, backend) = crate::session::backend::native::split(stream);
    let connected = ToConnected {
        peer_addr,
        frontend,
    };
    debug!("Connected");

    tx_next
        .send(connected)
        .map_err(|_| SessionError::FrontendClosed)?;

    debug!("Starting session loop");
    backend.start().await
}

#[derive(Debug)]
#[expect(clippy::large_enum_variant, reason = "most users will use `Rustls`")]
enum MaybeTlsStream<S> {
    Plain(S),
    Rustls(tokio_rustls::server::TlsStream<S>),
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for MaybeTlsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Rustls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for MaybeTlsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Rustls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Rustls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Rustls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
