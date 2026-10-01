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
    #[command(flatten)]
    pub net: NetSim,
    /// Ticks of input lead on top of Lightyear's jitter margin. Its default of 1 let about 0.4% of inputs
    /// reach the server late at 150 ms RTT, 30 ms jitter, 5% loss; 3 (+17 ms) brought that under 0.05%.
    #[arg(long, default_value_t = 3.0)]
    pub input_margin: f32,
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
