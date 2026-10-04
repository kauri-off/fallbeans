//! Project tasks: `cargo xtask <check|audit|assets|dev|stress|deploy|dist>`.
mod deploy;
mod dist;
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
    /// Map and system audits (`fb_audit`): [map…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics]
    /// [--notes] [--json].
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
    /// `--remote`: against a probe server on a server host (`--host`), over the real network.
    Stress(Box<stress::StressArgs>),
    /// Builds the server for Linux and installs it on a server host (`--host`, `--domain`).
    Deploy(deploy::DeployArgs),
    /// Release package of one kind into dist/.
    Dist(dist::DistArgs),
    /// Claude Code hook: rustfmt on the edited file (hook payload on stdin).
    #[command(hide = true)]
    FormatHook,
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
    /// The host fills the room's empty places with bots.
    #[arg(long)]
    fill: bool,
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
                "--profile",
                &format!("dev-{profile}"),
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
            if a.fill {
                c.arg("--fill");
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

/// Blender bakes and exports every model of blender/fallguys_assets.blend straight into assets/models.
fn export_models() -> bool {
    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    let ok = run(Command::new(blender).current_dir(root()).args([
        "-b",
        "blender/fallguys_assets.blend",
        "--python",
        "blender/export.py",
        "--",
        "assets/models",
    ]));
    if !ok {
        eprintln!("(Blender not found or failed: set BLENDER=/path/to/blender)");
    }
    ok
}

/// Exit code 2 shows Claude what rustfmt could not fix.
fn format_hook() -> ExitCode {
    let mut input = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    let payload: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let Some(file) = payload["tool_input"]["file_path"].as_str() else {
        return ExitCode::SUCCESS;
    };
    if !file.ends_with(".rs") || file.contains("/target/") || file.contains("\\target\\") {
        return ExitCode::SUCCESS;
    }
    let out = Command::new("rustfmt")
        .args(["--edition", "2024", "--config-path"])
        .arg(root().join("rustfmt.toml"))
        .arg(file)
        .output();
    match out {
        Ok(o) if o.status.success() => ExitCode::SUCCESS,
        Ok(o) => {
            eprintln!(
                "rustfmt: {file} still has problems:\n{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("rustfmt: {e}");
            ExitCode::SUCCESS
        }
    }
}

fn main() -> ExitCode {
    let ok = match Cli::parse().task {
        Task::FormatHook => return format_hook(),
        Task::Check => check(),
        Task::Audit { args } => run(cargo().args(["run", "--release", "-p", "fb_audit", "--"]).args(args)),
        Task::Assets { export } => {
            (!export || export_models()) && run(cargo().args(["run", "-p", "fb_client", "--", "--check-assets"]))
        }
        Task::Dev(a) => dev(&a),
        Task::Stress(a) => stress::stress(&a),
        Task::Deploy(a) => deploy::deploy(&a),
        Task::Dist(a) => dist::dist(&a),
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
