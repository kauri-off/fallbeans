//! Project tasks: `cargo xtask <check|audit|assets|dev|stress|perf|fuzz-ui|dist>`.
mod dist;
mod fuzz;
mod perf;
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
        target_dir()
            .join(if self.release { "release" } else { "debug" })
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    pub fn build(&self) -> bool {
        let mut c = cargo();
        c.args(["build", "-p", "fb_server", "-p", "fb_client"]);
        if self.release {
            c.arg("--release");
        } else {
            // (Bevy as a DLL: relinks in seconds. `command` puts the DLLs on the PATH.)
            c.args(["--features", "fb_client/dynamic,fb_server/dynamic"]);
        }
        run(&mut c)
    }

    /// The built `name` to run: a debug build finds Bevy's DLL (target/debug/deps) and Rust's own (the
    /// toolchain's bin) on the PATH, as `cargo run` would give it.
    pub fn command(&self, name: &str) -> Command {
        let mut c = Command::new(self.bin(name));
        if !self.release {
            c.env("PATH", dll_path());
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

/// The PATH with the debug build's deps and the toolchain's bin in front (Bevy's and Rust's DLLs).
pub fn dll_path() -> std::ffi::OsString {
    let sysroot = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .current_dir(root())
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let mut dirs = vec![target_dir().join("debug").join("deps")];
    if let Some(s) = sysroot {
        dirs.push(s.join("bin"));
        dirs.push(s.join("lib"));
    }
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Cargo's target directory: `CARGO_TARGET_DIR` (relative to the root), else `target`.
pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), |d| root().join(d))
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
        && run(cargo().args([
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]))
        && run(cargo().args(["test", "--locked", "--workspace"]))
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

fn dev(a: &DevArgs) -> bool {
    if !a.shared.build() {
        return false;
    }
    let Ok(mut server) = a
        .shared
        .command("fb_server")
        .args(a.shared.server_args())
        .args(["--dev", "--solo"])
        .spawn()
    else {
        eprintln!("cannot start the server");
        return false;
    };
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Gone already (its ports held by a server left running): the windows would join that one instead.
    if let Ok(Some(status)) = server.try_wait() {
        eprintln!("the server exited at once ({status})");
        return false;
    }
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
    true
}

/// Blender bakes and exports every model of blender/models.blend straight into assets/models.
fn export_models() -> bool {
    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    let ok = run(Command::new(blender).current_dir(root()).args([
        "-b",
        "blender/models.blend",
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

fn main() -> ExitCode {
    let ok = match Cli::parse().task {
        Task::Check => check(),
        Task::Audit { args } => run(cargo().args(["run", "--release", "-p", "fb_audit", "--"]).args(args)),
        Task::Assets { export } => {
            (!export || export_models()) && run(cargo().args(["run", "-p", "fb_client", "--", "--check-assets"]))
        }
        Task::Dev(a) => dev(&a),
        Task::Stress(a) => stress::stress(&a),
        Task::Perf(a) => perf::perf(&a),
        Task::FuzzUi(a) => fuzz::fuzz(&a),
        Task::Dist(a) => dist::dist(&a),
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
