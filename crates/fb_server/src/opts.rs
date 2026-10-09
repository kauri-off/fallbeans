use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use bevy::prelude::Resource;
use clap::Parser;
use fb_net::{HTTP_PORT, NetSim, UDP_PORT, WS_PORT};

#[derive(Parser, Resource, Clone, Debug)]
#[command(about = "Fall Beans server: rooms of players and bots over UDP and WebSocket")]
pub struct Opts {
    #[arg(long, default_value_t = UDP_PORT)]
    pub udp_port: u16,
    /// Address the UDP socket binds. `::` takes IPv6 too where the system makes it dual-stack (Linux by
    /// default; not Windows), which is why it is not the default.
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED.into())]
    pub udp_addr: IpAddr,
    #[arg(long, default_value_t = WS_PORT)]
    pub ws_port: u16,
    /// Address the WebSocket listener binds (127.0.0.1 behind a reverse proxy's wss).
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED.into())]
    pub ws_addr: IpAddr,
    /// Port of the HTTP API: sessions (connect tokens), health, debug.
    #[arg(long, default_value_t = HTTP_PORT)]
    pub http_port: u16,
    /// Address the HTTP API binds (127.0.0.1 behind a reverse proxy's https).
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED.into())]
    pub http_addr: IpAddr,
    /// The address players reach the UDP port at, put into connect tokens (and checked by netcode). Default:
    /// the address the client asked the HTTP API at when it is an IP, else the address of our socket that
    /// took the request (127.0.0.1 when unknown), and no check.
    #[arg(long)]
    pub public_host: Option<IpAddr>,
    /// The WebSocket URL players are given (default: wss://<host>/fallbeans/ws when asked through an https
    /// proxy, else ws://<host they asked at>:<ws-port>).
    #[arg(long)]
    pub public_ws_url: Option<String>,
    /// The server's name in the players' server lists.
    #[arg(long, env = "FB_NAME")]
    pub name: Option<String>,
    /// Rooms open at once at most (practice rooms aside; up to 16).
    #[arg(long, default_value_t = crate::rooms::hub::DEFAULT_MAX_ROOMS)]
    pub max_rooms: usize,
    /// Seed of the rooms' random choices (games, map seeds, spawn order; default: a new one per room).
    #[arg(long)]
    pub seed: Option<u32>,
    /// Seconds before a round starts.
    #[arg(long, default_value_t = fb_shared::INTRO_S)]
    pub intro: f64,
    /// Development: dev commands are accepted, and the room `dev` is always open.
    #[arg(long)]
    pub dev: bool,
    /// A game may start with one player (otherwise two).
    #[arg(long)]
    pub solo: bool,
    /// A fall in a survival round is a respawn, not the end of the round (stress runs keep everybody in play).
    #[arg(long)]
    pub respawn: bool,
    /// Rooms that stay open with nobody in them, by id (stress runs meet in them), e.g. `s1,s2`.
    #[arg(long, value_delimiter = ',')]
    pub open_rooms: Vec<String>,
    #[command(flatten)]
    pub net: NetSim,
    /// Writes every pawn's input and position per tick here (`cargo xtask stress` compares it with clients').
    #[arg(long)]
    pub trace: Option<PathBuf>,
    /// Writes contacts, tackles (with where the tackler's player saw the others) and dives here.
    #[arg(long)]
    pub trace_hits: Option<PathBuf>,
    /// Seconds between metric lines in the log (0: none).
    #[arg(long, default_value_t = 5.0)]
    pub metrics_every: f64,
    /// Seconds of silence before a connection is given up, both transports (default: 3 UDP, 10 WebSocket). The
    /// client's test harness steps server and clients in one thread: a busy test machine stalls both at once.
    #[arg(long, hide = true)]
    pub link_timeout: Option<i32>,
    /// Quits after this many seconds (stress runs).
    #[arg(long)]
    pub exit_after: Option<f64>,
}
