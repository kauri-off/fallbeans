//! Project tasks: `cargo xtask <check|golden|audit|assets|dev|stress|deploy>`.
mod audit;
mod deploy;
mod stress;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Fall Beans project tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// fmt, clippy (warnings are errors), tests.
    Check,
    /// Re-exports the TS golden traces (needs bun and the wasm32 target) and runs the comparison tests.
    Golden(GoldenArgs),
    /// Map and system audits (`fb_audit`); `--vs-ts` compares them with the TS audits.
    Audit(audit::AuditArgs),
    /// Re-exports the models for Bevy (needs bun) and loads them in the client.
    Assets,
    /// Server plus windowed clients on this machine.
    Dev(DevArgs),
    /// Server plus headless clients under a simulated network; checks prediction, traffic and tick cost.
    /// `--remote`: against a probe server on a server host (`--host`), over the real network.
    Stress(Box<stress::StressArgs>),
    /// Builds the server for Linux and installs it on a server host (`--host`, `--domain`).
    Deploy(deploy::DeployArgs),
}

/// What both server and clients take.
#[derive(Args, Clone, Debug)]
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
        root()
            .join("target")
            .join(if self.release { "release" } else { "debug" })
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    pub fn build(&self) -> bool {
        let mut c = cargo();
        c.args(["build", "-p", "fb_server", "-p", "fb_client"]);
        if self.release {
            c.arg("--release");
        }
        run(&mut c)
    }
}

#[derive(Args)]
struct DevArgs {
    #[arg(long, default_value_t = 2)]
    clients: u32,
    /// Clients play by themselves.
    #[arg(long)]
    autopilot: bool,
    #[command(flatten)]
    shared: Shared,
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

pub fn run(cmd: &mut Command) -> bool {
    eprintln!("$ {cmd:?}");
    cmd.status().is_ok_and(|s| s.success())
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

pub fn bun(args: &[&str]) -> Command {
    let mut c = Command::new("bun");
    c.args(args).current_dir(root().parent().unwrap());
    c
}

#[derive(Args, Clone, Debug)]
struct GoldenArgs {
    /// Only these maps (ids; `scenarios` for the physics scenarios). Default: all.
    maps: Vec<String>,
}

/// The TS side computes sin, cos, atan2, exp and pow with the same libm as Rust (`tools/golden_libm`
/// built for wasm32, loaded by scripts/golden-math.ts): otherwise the last bits differ and rounds drift
/// apart. The module is kept in scripts/ so bun alone can re-export.
fn golden(a: &GoldenArgs) -> bool {
    if !build_golden_libm() {
        return false;
    }
    let mut args = vec!["--preload", "./scripts/golden-math.ts", "scripts/golden.ts"];
    args.extend(a.maps.iter().map(String::as_str));
    run(&mut bun(&args)) && run(cargo().args(["test", "-p", "fb_arena", "--test", "golden", "--test", "scenarios"]))
}

/// Builds `tools/golden_libm` for wasm32 and puts it where scripts/golden-math.ts loads it.
fn build_golden_libm() -> bool {
    let wasm = root().join("target/wasm32-unknown-unknown/release/golden_libm.wasm");
    let built = run(cargo().args([
        "build",
        "-p",
        "golden_libm",
        "--release",
        "--target",
        "wasm32-unknown-unknown",
    ]));
    if !built {
        eprintln!("(needs `rustup target add wasm32-unknown-unknown`)");
        return false;
    }
    let to = root().parent().unwrap().join("scripts/golden-libm.wasm");
    if let Err(e) = std::fs::copy(&wasm, &to) {
        eprintln!("{}: {e}", to.display());
        return false;
    }
    true
}

fn check() -> bool {
    run(cargo().args(["fmt", "--all", "--check"]))
        && run(cargo().args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))
        && run(cargo().args(["test", "--workspace"]))
}

fn dev(a: &DevArgs) -> bool {
    if !a.shared.build() {
        return false;
    }
    let Ok(mut server) = Command::new(a.shared.bin("fb_server"))
        .args(a.shared.server_args())
        .args(["--dev", "--solo"])
        .spawn()
    else {
        eprintln!("cannot start the server");
        return false;
    };
    std::thread::sleep(std::time::Duration::from_millis(500));
    let clients: Vec<_> = (0..a.clients)
        .filter_map(|i| {
            let mut c = Command::new(a.shared.bin("fb_client"));
            let profile = (b'a' + i as u8) as char;
            c.args([
                "--name",
                &format!("Боб {profile}"),
                "--title",
                &format!("Fall Beans — {profile}"),
            ])
            .args(a.shared.net_args())
            .args(a.shared.play_args("dev", a.clients));
            if a.autopilot {
                c.arg("--autopilot");
            }
            c.spawn().ok()
        })
        .collect();
    for mut c in clients {
        let _ = c.wait();
    }
    let _ = server.kill();
    let _ = server.wait();
    true
}

fn main() -> ExitCode {
    let ok = match Cli::parse().task {
        Task::Check => check(),
        Task::Golden(a) => golden(&a),
        Task::Audit(a) => audit::audit(&a),
        Task::Assets => {
            run(&mut bun(&["scripts/assets.ts", "--bevy"]))
                && run(cargo().args(["run", "-p", "fb_client", "--", "--check-assets"]))
        }
        Task::Dev(a) => dev(&a),
        Task::Stress(a) => stress::stress(&a),
        Task::Deploy(a) => deploy::deploy(&a),
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
