use std::path::PathBuf;

use bevy::prelude::Resource;
use clap::Parser;
use fb_net::{NetSim, UDP_PORT, WS_PORT};

#[derive(Parser, Resource, Clone, Debug)]
#[command(about = "Fall Beans server: one room playing rounds of one map over UDP and WebSocket")]
pub struct Opts {
    #[arg(long, default_value_t = UDP_PORT)]
    pub udp_port: u16,
    #[arg(long, default_value_t = WS_PORT)]
    pub ws_port: u16,
    /// Seed of every round (default: a new one per round).
    #[arg(long)]
    pub seed: Option<u32>,
    /// Seconds before a round starts.
    #[arg(long, default_value_t = 3.0)]
    pub intro: f64,
    #[arg(long, default_value = "jump-club")]
    pub map: String,
    #[command(flatten)]
    pub net: NetSim,
    /// Writes every pawn's input and position per tick here (`cargo xtask stress` compares it with clients').
    #[arg(long)]
    pub trace: Option<PathBuf>,
    /// Seconds between metric lines in the log (0: none).
    #[arg(long, default_value_t = 5.0)]
    pub metrics_every: f64,
    /// Quits after this many seconds (stress runs).
    #[arg(long)]
    pub exit_after: Option<f64>,
}
