//! Project tasks: `cargo xtask <check|play|doctor|setup|audit|assets|dev|stress|perf|fuzz-ui|dist>`.
mod check;
mod dist;
mod doctor;
mod fuzz;
mod perf;
mod play;
mod sdk;
mod stress;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};

use anyhow::{Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Fall Beans project tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// fmt, clippy (warnings are errors), tests; a summary on screen, everything in target/check.log.
    Check(check::CheckArgs),
    /// The game from its menu as a player gets it (`perf` build, `--dist`: the packages' client), a dev server on
    /// localhost.
    Play(play::PlayArgs),
    /// What this machine has for each task (build, upscalers, packages), and how to get what it lacks.
    Doctor,
    /// The pinned SDKs and tools of toolchain/deps.toml into target/sdk (`--dist`: the packaging tools too).
    Setup(sdk::SetupArgs),
    /// Map and system audits (`fb_audit`): [map…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics]
    /// [--notes] [--json]; `--baseline` / `--bless-baseline`: the game's feel against `baseline.json`.
    Audit {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Loads every model in the client; `--export` first re-exports them from blender/ (needs Blender).
    Assets {
        #[arg(long)]
        export: bool,
    },
    /// Server plus windowed clients on this machine.
    Dev(DevArgs),
    /// Server plus headless clients under a simulated network; checks prediction, traffic and tick cost.
    Stress(Box<stress::StressArgs>),
    /// Render performance: `run` (a benchmark round on this machine, `--sweep`), `show`, `compare`, `csv`.
    Perf(perf::PerfArgs),
    /// The whole client played at random (`fb_client` monkey): seeds in parallel, logs in target/fuzz-ui.
    FuzzUi(fuzz::FuzzArgs),
    /// Release package of one kind into dist/.
    Dist(dist::DistArgs),
}

/// What both server and clients take.
#[derive(Args, Clone, Debug, Default)]
pub struct Shared {
    /// One-way latency on every incoming packet, ms (RTT ≈ 2 × lag).
    #[arg(long, default_value_t = 0)]
    pub lag: u64,
    #[arg(long, default_value_t = 0)]
    pub jitter: u64,
    #[arg(long, default_value_t = 0.0)]
    pub loss: f32,
    /// The map the clients play (the room's host starts a game of it).
    #[arg(long, default_value = "jump-club")]
    pub map: String,
    #[arg(long)]
    pub seed: Option<u32>,
    /// Builds and runs with the release profile.
    #[arg(long)]
    pub release: bool,
}

impl Shared {
    pub fn net_args(&self) -> Vec<String> {
        vec![
            "--lag".into(),
            self.lag.to_string(),
            "--jitter".into(),
            self.jitter.to_string(),
            "--loss".into(),
            self.loss.to_string(),
        ]
    }

    pub fn server_args(&self) -> Vec<String> {
        let mut a = self.net_args();
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        a
    }

    /// A client goes into `room` and, as its host, starts a game of the map once `players` are in.
    pub fn play_args(&self, room: &str, players: u32) -> Vec<String> {
        vec![
            "--room".into(),
            room.into(),
            "--start".into(),
            self.map.clone(),
            "--start-players".into(),
            players.to_string(),
        ]
    }

    pub fn bin(&self, name: &str) -> PathBuf {
        target_dir()
            .join(if self.release { "release" } else { "debug" })
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    pub fn build(&self) -> Result<()> {
        let mut c = cargo();
        c.args(["build", "-p", "fb_server", "-p", "fb_client"]);
        let features = if self.release {
            c.arg("--release");
            dlss_features()
        } else {
            dev_features()
        };
        with_features(&mut c, &features);
        run(&mut c)?;
        if let Some(dir) = self.bin("fb_client").parent() {
            let _ = dist::upscalers_into(dir, None);
        }
        Ok(())
    }

    /// The built `name` to run: a debug build finds Bevy's and Rust's shared libraries, as under `cargo run`.
    pub fn command(&self, name: &str) -> Command {
        let mut c = Command::new(self.bin(name));
        if !self.release {
            dylib_env(&mut c);
        }
        c
    }
}

#[derive(Args)]
struct DevArgs {
    #[arg(long, default_value_t = 2)]
    clients: u32,
    /// Clients play by themselves.
    #[arg(long)]
    autopilot: bool,
    /// The host fills the room's empty places with bots.
    #[arg(long)]
    fill: bool,
    /// Clients log every left click (`--trace-clicks`).
    #[arg(long)]
    trace_clicks: bool,
    #[command(flatten)]
    shared: Shared,
}

/// Bevy's and Rust's shared libraries of a dev build (`dynamic`) on the loader's path, as `cargo run` puts them.
pub fn dylib_env(c: &mut Command) {
    let var = if cfg!(windows) {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let sysroot = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .current_dir(root())
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let mut dirs = vec![target_dir().join("debug").join("deps")];
    if let Some(s) = sysroot {
        dirs.push(s.join("bin"));
        dirs.push(s.join("lib"));
        let targets = std::fs::read_dir(s.join("lib").join("rustlib"))
            .into_iter()
            .flatten()
            .flatten();
        dirs.extend(targets.map(|t| t.path().join("lib")).filter(|d| d.is_dir()));
    }
    if let Some(p) = std::env::var_os(var) {
        dirs.extend(std::env::split_paths(&p));
    }
    c.env(var, std::env::join_paths(dirs).unwrap_or_default());
}

/// `fb_client/dlss` when its SDK is there.
pub fn dlss_features() -> Vec<&'static str> {
    if dlss() { vec!["fb_client/dlss"] } else { Vec::new() }
}

/// Every dev-profile build, tests too: Bevy as a shared library (relinks in seconds), one feature set, one cache.
/// No DLSS: Bevy's shared library cannot link the SDK's static NGX library (`perf` and `dist` builds have it).
pub fn dev_features() -> Vec<&'static str> {
    vec!["fb_client/dynamic", "fb_server/dynamic"]
}

pub fn with_features(c: &mut Command, features: &[&str]) {
    if !features.is_empty() {
        c.args(["--features", &features.join(",")]);
    }
}

/// Starts a server and gives it half a second: gone by then (its ports held by another one), it is an error.
pub fn start_server(c: &mut Command) -> Result<Child> {
    let mut server = c.spawn().map_err(|e| anyhow!("cannot start the server: {e}"))?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Ok(Some(status)) = server.try_wait() {
        bail!("the server exited at once ({status}): another one on its ports?");
    }
    Ok(server)
}

/// The client's `dlss` feature (NVIDIA DLSS 4.5 before AMD FSR 3.1): on when the DLSS SDK is there to build it
/// with (`DLSS_SDK`; the build needs the Vulkan headers, `VULKAN_SDK`, and clang too: `README.md`).
pub fn dlss() -> bool {
    let Some(sdk) = sdk::var("DLSS_SDK") else {
        return false;
    };
    if !sdk.join("include").join("nvsdk_ngx.h").is_file() {
        eprintln!("DLSS_SDK has no include/nvsdk_ngx.h: the client is built without DLSS");
        return false;
    }
    if sdk::var("VULKAN_SDK").is_none() {
        eprintln!("DLSS_SDK is set but VULKAN_SDK is not: the DLSS build needs the Vulkan headers");
    }
    true
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Cargo's target directory: `CARGO_TARGET_DIR` (relative to the root), else `target`.
pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), |d| root().join(d))
}

/// A failure the user has seen already (the tool's own output, `check`'s summary): nothing more to print.
#[derive(Debug)]
pub struct Reported;

impl fmt::Display for Reported {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("failed")
    }
}

impl std::error::Error for Reported {}

/// `Err(Reported)` unless `ok`.
pub fn reported(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Reported.into()) }
}

/// Prints a failure the user has not seen yet; whether `r` went well.
pub fn report(r: Result<()>) -> bool {
    r.inspect_err(|e| {
        if !e.is::<Reported>() {
            eprintln!("{e:#}");
        }
    })
    .is_ok()
}

/// Runs `cmd`, whose own output tells why it failed.
pub fn run(cmd: &mut Command) -> Result<()> {
    eprintln!("$ {cmd:?}");
    let status = cmd
        .status()
        .map_err(|e| anyhow!("cannot start {}: {e}", cmd.get_program().display()))?;
    reported(status.success())
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    c.current_dir(root());
    sdk::apply(&mut c);
    c
}

/// A client's tag from its index, as spreadsheet columns: a…z, aa, ab, …
fn letters(mut i: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'a' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn dev(a: &DevArgs) -> Result<()> {
    a.shared.build()?;
    let mut server = start_server(
        a.shared
            .command("fb_server")
            .args(a.shared.server_args())
            .args(["--dev", "--solo"]),
    )?;
    let clients: Vec<_> = (0..a.clients)
        .filter_map(|i| {
            let mut c = a.shared.command("fb_client");
            let profile = letters(i);
            c.args([
                "--profile",
                &format!("dev-{profile}"),
                "--name",
                &format!("Боб {profile}"),
                "--title",
                &format!("Fall Beans — {profile}"),
            ])
            .args(a.shared.net_args())
            .args(a.shared.play_args("dev", a.clients));
            // (Several clients share the screen: windows. One plays fullscreen, as a player would.)
            if a.clients > 1 {
                c.arg("--windowed");
            }
            if a.autopilot {
                c.arg("--autopilot");
            }
            if a.fill {
                c.arg("--fill");
            }
            if a.trace_clicks {
                c.arg("--trace-clicks");
            }
            c.spawn().ok()
        })
        .collect();
    for mut c in clients {
        let _ = c.wait();
    }
    let _ = server.kill();
    let _ = server.wait();
    Ok(())
}

/// Blender bakes and exports every model of blender/models.blend straight into assets/models.
fn export_models() -> Result<()> {
    let blender = std::env::var_os("BLENDER").unwrap_or_else(|| "blender".into());
    run(Command::new(blender).current_dir(root()).args([
        "-b",
        "blender/models.blend",
        "--python",
        "blender/export.py",
        "--",
        "assets/models",
    ]))
    .inspect_err(|_| eprintln!("(Blender not found or failed: set BLENDER=/path/to/blender)"))
}

/// Loads every model in a dev build of the client (the build `dev` and `check` share), exported first with `export`.
fn assets(export: bool) -> Result<()> {
    if export {
        export_models()?;
    }
    let s = Shared::default();
    s.build()?;
    run(s.command("fb_client").arg("--check-assets"))
}

/// The text of a line without its terminal colors and other escape sequences.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn main() -> ExitCode {
    let done = match Cli::parse().task {
        Task::Check(a) => check::check(&a),
        Task::Play(a) => play::play(&a),
        Task::Doctor => doctor::doctor(),
        Task::Setup(a) => sdk::setup(&a),
        Task::Audit { args } => run(cargo().args(["run", "--release", "-p", "fb_audit", "--"]).args(args)),
        Task::Assets { export } => assets(export),
        Task::Dev(a) => dev(&a),
        Task::Stress(a) => stress::stress(&a),
        Task::Perf(a) => perf::perf(&a),
        Task::FuzzUi(a) => fuzz::fuzz(&a),
        Task::Dist(a) => dist::dist(&a),
    };
    if report(done) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
