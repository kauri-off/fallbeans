use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use bevy::prelude::Resource;
use clap::Parser;
use fb_net::{NetSim, UDP_PORT, WS_PORT};

#[derive(Parser, Resource, Clone, Debug)]
#[command(about = "Fall Beans server: rooms of players and bots over UDP and WebSocket")]
pub struct Opts {
    #[arg(long, default_value_t = UDP_PORT)]
    pub udp_port: u16,
    #[arg(long, default_value_t = WS_PORT)]
    pub ws_port: u16,
    /// Address the WebSocket listener binds (production: 127.0.0.1, behind nginx's wss on 443).
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED.into())]
    pub ws_addr: IpAddr,
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
    /// Rooms that stay open with nobody in them, by id (stress runs meet in them), e.g. `s1,s2`.
    #[arg(long, value_delimiter = ',')]
    pub open_rooms: Vec<String>,
    /// While this file exists the game is being updated: everybody is told and disconnected.
    #[arg(long)]
    pub maintenance_file: Option<PathBuf>,
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
