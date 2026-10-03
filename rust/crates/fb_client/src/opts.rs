use std::path::PathBuf;

use bevy::prelude::Resource;
use clap::{Parser, ValueEnum};
use fb_net::{NetSim, UDP_PORT, WS_PORT};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// UDP, then WebSocket if UDP gets no answer within 2 s.
    Auto,
    Udp,
    Ws,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    Dx12,
    Gl,
}

#[derive(Parser, Resource, Clone, Debug)]
#[command(about = "Fall Beans client")]
pub struct Opts {
    #[arg(long, default_value = "127.0.0.1")]
    pub server: String,
    #[arg(long, default_value_t = UDP_PORT)]
    pub udp_port: u16,
    #[arg(long, default_value_t = WS_PORT)]
    pub ws_port: u16,
    /// WebSocket URL (default ws://<server>:<ws-port>; wss:// behind a proxy).
    #[arg(long)]
    pub ws_url: Option<String>,
    #[arg(long, value_enum, default_value_t = Transport::Auto)]
    pub transport: Transport,
    /// Netcode client id (default: random). Phase 0 trusts it; the session endpoint will issue tokens.
    #[arg(long)]
    pub id: Option<u64>,
    /// The player's name.
    #[arg(long, default_value = "")]
    pub name: String,
    /// Identity token from an earlier session (the same player again).
    #[arg(long)]
    pub token: Option<String>,
    /// Go straight into this room (its PIN for a private one).
    #[arg(long)]
    pub room: Option<String>,
    #[arg(long)]
    pub pin: Option<String>,
    /// A practice round of this map with bots.
    #[arg(long)]
    pub practice: Option<String>,
    /// Suit colour (index into the palette).
    #[arg(long)]
    pub color: Option<u8>,
    /// As the room's host, start a game of this map (all rounds) once `--start-players` are in.
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub start_players: Option<usize>,
    #[command(flatten)]
    pub net: NetSim,
    /// Ticks of input lead on top of Lightyear's jitter margin. Its default of 1 let about 0.4% of inputs
    /// reach the server late at 150 ms RTT, 30 ms jitter, 5% loss; 3 (+17 ms) brought that under 0.05%.
    #[arg(long, default_value_t = 3.0)]
    pub input_margin: f32,
    /// Ticks the clock's lead may be off before Lightyear jumps it (a hard resync, relabelling inputs
    /// already sent) instead of speeding up or slowing down (Lightyear's default: 10).
    #[arg(long)]
    pub sync_max_error: Option<f32>,
    /// Graphics API (default: wgpu's choice, or WGPU_BACKEND).
    #[arg(long, value_enum)]
    pub backend: Option<Backend>,
    /// No window and no GPU: simulation and network only, driven by the autopilot (stress runs, CI).
    #[arg(long)]
    pub headless: bool,
    /// Frames a second without a window (a player's client runs at the display's rate).
    #[arg(long, default_value_t = 60.0)]
    pub fps: f64,
    /// Plays by itself: circles, hops, runs for bonuses.
    #[arg(long)]
    pub autopilot: bool,
    /// Writes the own bean's input and position per predicted tick here (`cargo xtask stress` reads it).
    #[arg(long)]
    pub trace: Option<PathBuf>,
    /// Saves a screenshot a second before `--exit-after`.
    #[arg(long)]
    pub screenshot: Option<PathBuf>,
    /// Quits after this many seconds.
    #[arg(long)]
    pub exit_after: Option<f32>,
    /// Loads every model through Bevy's glTF loader, reports and quits.
    #[arg(long)]
    pub check_assets: bool,
    #[arg(long, default_value = "Fall Beans")]
    pub title: String,
}

impl Opts {
    pub fn autopilot(&self) -> bool {
        self.autopilot || self.headless
    }
}
