use std::path::PathBuf;

use bevy::prelude::Resource;
use clap::{Parser, ValueEnum};
use fb_net::{HTTP_PORT, NetSim, WS_PORT};

fn map_ids() -> clap::builder::PossibleValuesParser {
    fb_maps::GAMES.iter().map(|m| m.meta().id).collect::<Vec<_>>().into()
}

/// `--profile`: a name for the settings folder, never a path (`../x` would write outside it).
fn profile_name(s: &str) -> Result<String, String> {
    let ok = !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| s.to_string())
        .ok_or_else(|| "letters, digits, - and _ only".to_string())
}

/// `--fps`: a rate the headless loop can sleep by (0 or less would make its frame infinitely long).
fn fps(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    (1.0..=1000.0)
        .contains(&v)
        .then_some(v)
        .ok_or_else(|| "between 1 and 1000".into())
}

/// `--perf-capture`: seconds a benchmark records (NaN or infinity would never end).
fn capture_secs(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    (f32::MIN_POSITIVE..=3600.0)
        .contains(&v)
        .then_some(v)
        .ok_or_else(|| "more than 0, at most 3600".into())
}

/// `--perf-warmup`: seconds of a round before a benchmark.
fn warmup_secs(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    (0.0..=600.0)
        .contains(&v)
        .then_some(v)
        .ok_or_else(|| "between 0 and 600".into())
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// UDP, then WebSocket if UDP gets no answer within 2 s.
    Auto,
    Udp,
    Ws,
}

/// A graphics API the game draws with (`backend.rs`): 1.2 or newer for Vulkan.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    /// Windows only.
    Dx12,
}

#[derive(Parser, Resource, Clone, Debug)]
#[command(about = "Fall Beans client")]
pub struct Opts {
    /// Connect to this server at once (default without it: the server list; with `--room`, `--practice`,
    /// `--start`, `--headless` or `--offscreen`: 127.0.0.1).
    #[arg(long)]
    pub server: Option<String>,
    /// The server's HTTP API, where connect tokens come from (default http://<server>:<http-port>/fallbeans).
    #[arg(long)]
    pub http_url: Option<String>,
    #[arg(long, default_value_t = HTTP_PORT)]
    pub http_port: u16,
    #[arg(long, default_value_t = WS_PORT)]
    pub ws_port: u16,
    /// WebSocket URL (default: the one the server's session reply names, else ws://<server>:<ws-port>).
    #[arg(long)]
    pub ws_url: Option<String>,
    #[arg(long, value_enum, default_value_t = Transport::Auto)]
    pub transport: Transport,
    /// The player's name for this run (the saved one otherwise).
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub name: Option<String>,
    /// Identity from an earlier session for this run (the saved one otherwise).
    #[arg(long)]
    pub token: Option<String>,
    /// A settings file of its own (identity, name, look, options): another player on this machine.
    /// Letters, digits, `-` and `_` (it becomes part of a path).
    #[arg(long, value_parser = profile_name)]
    pub profile: Option<String>,
    /// Go straight into this room (its PIN for a private one).
    #[arg(long)]
    pub room: Option<String>,
    #[arg(long)]
    pub pin: Option<String>,
    /// A practice round of this map with bots.
    #[arg(long)]
    pub practice: Option<String>,
    /// Suit colour for this run (index into the palette).
    #[arg(long)]
    pub color: Option<u8>,
    /// Map ids: if this client hosts the room, it starts a game of these maps in turn (`a,b,c`).
    #[arg(long, value_name = "MAP", value_parser = map_ids(), value_delimiter = ',')]
    pub start: Vec<String>,
    /// With `--start`: wait until the room has this many players (default: 2, or 1 if the server runs with `--solo`; never fewer).
    #[arg(long, value_name = "N")]
    pub start_players: Option<usize>,
    /// With `--start`: number of rounds in the game.
    #[arg(long, value_name = "N", default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..=12))]
    pub start_rounds: u32,
    /// With `--start`: fill the room's empty places with bots.
    #[arg(long)]
    pub fill: bool,
    #[command(flatten)]
    pub net: NetSim,
    /// Ticks of input lead on top of RTT/2 and 4 × jitter (`clock.rs`). Lightyear's default of 1 let about 0.4%
    /// of inputs reach the server late at 150 ms RTT, 30 ms jitter, 5% loss; 3 (+17 ms) brought that under 0.05%.
    #[arg(long, default_value_t = 3.0)]
    pub input_margin: f32,
    /// Tests `auto`: everything UDP brings in is dropped for this many seconds after the start.
    #[arg(long, default_value_t = 0.0)]
    pub udp_blocked: f32,
    /// Ticks the clock's lead may be off before Lightyear jumps it (a hard resync, relabelling inputs
    /// already sent) instead of speeding up or slowing down (default: `clock::MAX_ERROR`).
    #[arg(long)]
    pub sync_max_error: Option<f32>,
    /// Tests the clock: every `--spike-every` seconds incoming packets get this much more latency for 1.5 s, ms.
    #[arg(long, default_value_t = 0)]
    pub spike: u64,
    #[arg(long, default_value_t = 10.0)]
    pub spike_every: f32,
    /// Graphics API for this run, over the settings' (default: DirectX 12 on Windows, Vulkan without a DX12 GPU;
    /// Vulkan elsewhere).
    #[arg(long, value_enum)]
    pub backend: Option<Backend>,
    /// No window and no GPU: simulation and network only, driven by the autopilot (stress runs).
    #[arg(long)]
    pub headless: bool,
    /// Drawn to an image instead of a window (`--screenshot`, `fb/shot`): checks of the graphics that put
    /// nothing on the screen.
    #[arg(long)]
    pub offscreen: bool,
    /// Frames a second without a window, `--headless` or `--offscreen` (a player's runs at the display's rate).
    #[arg(long, default_value_t = 60.0, value_parser = fps)]
    pub fps: f64,
    /// Plays by itself: circles, hops, runs for bonuses.
    #[arg(long)]
    pub autopilot: bool,
    /// Writes the own bean's input and position per predicted tick here (`cargo xtask stress` reads it).
    #[arg(long)]
    pub trace: Option<PathBuf>,
    /// Seconds between `stats:` lines in the log (default: 1 with `--headless`, which stress reads; else none).
    #[arg(long, hide = true)]
    pub stats_every: Option<f32>,
    /// Saves a screenshot a second before `--exit-after`.
    #[arg(long)]
    pub screenshot: Option<PathBuf>,
    /// Quits after this many seconds.
    #[arg(long)]
    pub exit_after: Option<f32>,
    /// Serves the Bevy Remote Protocol with the game's `fb/*` methods on 127.0.0.1 (default port 15702).
    #[cfg(feature = "brp")]
    #[arg(long, num_args = 0..=1, default_missing_value = "15702")]
    pub brp: Option<u16>,
    /// Records this many seconds of a round uncapped, saves and quits (`cargo xtask perf`; F9 stops it early).
    #[arg(long, value_name = "SECS", value_parser = capture_secs)]
    pub perf_capture: Option<f32>,
    /// Switches the graphics features off one at a time in a round, saves and quits.
    #[arg(long)]
    pub perf_sweep: bool,
    #[arg(long, value_name = "SECS", default_value_t = 5.0, value_parser = warmup_secs)]
    pub perf_warmup: f32,
    /// The spans of every system and schedule, for the F4 profiler and the CPU rows of recordings (also
    /// `FB_PROFILER=1`). Without it they are filtered out where they are made and cost nothing.
    #[arg(long)]
    pub profiler: bool,
    /// `--perf-capture` from the launch on (the menu, the lobby, every round), not from a round's warmup.
    #[arg(long)]
    pub perf_from_start: bool,
    /// Where a recording goes (default: `perf-<time>.json` in the logs folder).
    #[arg(long)]
    pub perf_out: Option<PathBuf>,
    /// No GPU timestamp and pipeline statistics queries (the GPU times of the F4 overlay).
    #[arg(long)]
    pub no_gpu_timers: bool,
    /// Borderless fullscreen on the current monitor for this run (the default unless the settings say windowed).
    #[arg(long, conflicts_with = "windowed")]
    pub fullscreen: bool,
    /// A 1280×720 window for this run, whatever the settings say (several clients on one screen).
    #[arg(long)]
    pub windowed: bool,
    /// Does not look for a newer release.
    #[arg(long)]
    pub no_update: bool,
    /// Loads every model through Bevy's glTF loader, reports and quits.
    #[arg(long)]
    pub check_assets: bool,
    /// No loading screen at the start: every map's scene and shaders are made when first needed, as the
    /// round starts (quicker to the menu for a quick look; rounds may stutter at first).
    #[arg(long)]
    pub no_warmup: bool,
    /// Tests: the warm-up on the bench too (it is off there).
    #[arg(long, hide = true)]
    pub warmup: bool,
    /// Tests: this upscaler, if the machine offers it, instead of the best one ("dlss", "fsr3", "fsr1").
    #[arg(long, hide = true, value_parser = ["dlss", "fsr3", "fsr1"])]
    pub upscaler: Option<String>,
    #[arg(long, default_value = "Fall Beans")]
    pub title: String,
    /// Writes the own bean's contacts, tackles and dives here, with the others as drawn (`cargo xtask play
    /// --trace-hits` pairs it with the server's).
    #[arg(long)]
    pub trace_hits: Option<PathBuf>,
    /// Logs every left click through window, picking, button and action, with a verdict (target `clicks`).
    #[arg(long)]
    pub trace_clicks: bool,
}

impl Opts {
    pub fn autopilot(&self) -> bool {
        self.autopilot || self.headless
    }

    /// `--profiler`, or `FB_PROFILER=1`.
    pub fn profiler(&self) -> bool {
        self.profiler || std::env::var("FB_PROFILER").is_ok_and(|v| v == "1")
    }

    /// The HTTP API to connect to at the start (None: the player picks a server from the list).
    pub fn direct(&self) -> Option<String> {
        if let Some(url) = &self.http_url {
            return Some(url.clone());
        }
        let direct = self.server.is_some()
            || self.headless
            || self.offscreen
            || self.room.is_some()
            || self.practice.is_some()
            || !self.start.is_empty();
        direct.then(|| {
            let host = self.server.as_deref().unwrap_or("127.0.0.1");
            // (An IPv6 address goes in brackets in a URL.)
            let host = match host.parse::<core::net::Ipv6Addr>() {
                Ok(v6) => format!("[{v6}]"),
                Err(_) => host.to_string(),
            };
            format!("http://{host}:{}/fallbeans", self.http_port)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_server_urls() {
        let d = |args: &[&str]| Opts::parse_from([&["fb_client"], args].concat()).direct();
        assert_eq!(d(&[]), None);
        assert_eq!(
            d(&["--room", "dev"]).as_deref(),
            Some("http://127.0.0.1:5887/fallbeans")
        );
        assert_eq!(d(&["--server", "::1"]).as_deref(), Some("http://[::1]:5887/fallbeans"));
        assert_eq!(
            d(&["--server", "192.168.1.10", "--http-port", "7000"]).as_deref(),
            Some("http://192.168.1.10:7000/fallbeans")
        );
        assert!(Opts::try_parse_from(["fb_client", "--fps", "0"]).is_err());
    }

    #[test]
    fn backends_are_vulkan_and_dx12() {
        let b = |name: &str| Opts::try_parse_from(["fb_client", "--backend", name]).map(|o| o.backend);
        assert_eq!(b("vulkan").ok(), Some(Some(Backend::Vulkan)));
        assert_eq!(b("dx12").ok(), Some(Some(Backend::Dx12)));
        assert!(b("gl").is_err());
    }

    #[test]
    fn benchmark_times_end() {
        let ok = |args: &[&str]| Opts::try_parse_from([&["fb_client"], args].concat()).is_ok();
        assert!(ok(&["--perf-capture", "30", "--perf-warmup", "0"]));
        for bad in ["nan", "inf", "-1", "0"] {
            assert!(!ok(&["--perf-capture", bad]), "{bad}");
        }
        for bad in ["nan", "inf", "-1"] {
            assert!(!ok(&["--perf-warmup", bad]), "{bad}");
        }
    }
}
