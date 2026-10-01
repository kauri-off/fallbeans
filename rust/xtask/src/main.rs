//! Project tasks: `cargo xtask <check|golden|assets|dev|stress>`.
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
    /// Re-exports the TS golden traces (needs bun) and runs the comparison test.
    Golden,
    /// Re-exports the models for Bevy (needs bun) and loads them in the client.
    Assets,
    /// Server plus windowed clients on this machine.
    Dev(DevArgs),
    /// Server plus headless clients under a simulated network; checks prediction, traffic and tick cost.
    Stress(stress::StressArgs),
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
        a.extend(["--map".into(), self.map.clone()]);
        if let Some(s) = self.seed {
            a.extend(["--seed".into(), s.to_string()]);
        }
        a
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

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn bun(args: &[&str]) -> Command {
    let mut c = Command::new("bun");
    c.args(args).current_dir(root().parent().unwrap());
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
        .spawn()
    else {
        eprintln!("cannot start the server");
        return false;
    };
    std::thread::sleep(std::time::Duration::from_millis(500));
    let clients: Vec<_> = (0..a.clients)
        .filter_map(|i| {
            let mut c = Command::new(a.shared.bin("fb_client"));
            c.args([
                "--id",
                &(1000 + i).to_string(),
                "--title",
                &format!("Fall Beans — {}", (b'a' + i as u8) as char),
            ])
            .args(a.shared.net_args());
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
        Task::Golden => {
            run(&mut bun(&["scripts/golden.ts"])) && run(cargo().args(["test", "-p", "fb_arena", "--test", "golden"]))
        }
        Task::Assets => {
            run(&mut bun(&["scripts/assets.ts", "--bevy"]))
                && run(cargo().args(["run", "-p", "fb_client", "--", "--check-assets"]))
        }
        Task::Dev(a) => dev(&a),
        Task::Stress(a) => stress::stress(&a),
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
