//! `cargo xtask fuzz-ui`: the client's monkey (`fb_client/src/monkey.rs`) for a while, several seeds at once.
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::Args;
use serde::Deserialize;

use crate::stress::wait_or_kill;
use crate::{cargo, dev_features, dylib_env, reported, root, target_dir, with_features};

#[derive(Args)]
pub struct FuzzArgs {
    /// How long each seed plays.
    #[arg(long, default_value_t = 300)]
    secs: u32,
    /// The first seed (then +1 for each job); a new one from the clock if not given.
    #[arg(long)]
    seed: Option<u64>,
    /// Seeds played at once (each one is a server and two clients).
    #[arg(long, default_value_t = 4)]
    jobs: u32,
}

/// Past its seconds of play a seed has this long to finish, then it counts as hung and is killed.
const GRACE: Duration = Duration::from_secs(120);

/// The part of a line of cargo's `--message-format=json` that names a built binary.
#[derive(Deserialize)]
struct Artifact {
    reason: String,
    target: Option<Target>,
    profile: Option<Profile>,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
}

#[derive(Deserialize)]
struct Profile {
    test: bool,
}

/// Builds the client's test binary and returns its path (run directly, so that a hung one can be killed: killing
/// `cargo test` would leave it running).
fn test_binary() -> Option<PathBuf> {
    let mut c = cargo();
    // (The workspace and `check`'s features: its build, not one of its own.)
    c.args(["test", "--locked", "--workspace", "--bin", "fb_client"]);
    with_features(&mut c, &dev_features());
    c.args(["--no-run", "--message-format=json"]);
    eprintln!("$ {c:?}");
    let out = c.stderr(Stdio::inherit()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let a: Artifact = serde_json::from_str(l).ok()?;
        let client = a.reason == "compiler-artifact"
            && a.target.is_some_and(|t| t.name == "fb_client")
            && a.profile.is_some_and(|p| p.test);
        a.executable.filter(|_| client)
    })
}

pub fn fuzz(a: &FuzzArgs) -> Result<()> {
    let exe = test_binary().ok_or_else(|| anyhow!("the client's test binary did not build"))?;
    let dir = target_dir().join("fuzz-ui");
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let first = a.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_secs())
    });
    let runs: Vec<_> = (0..u64::from(a.jobs.max(1)))
        .filter_map(|i| {
            let seed = first + i;
            let path = dir.join(format!("{seed}.log"));
            let log = File::create(&path).ok()?;
            // (The package's folder, as under `cargo test`.)
            let mut child = Command::new(&exe);
            dylib_env(&mut child);
            let child = child
                .current_dir(root().join("crates/fb_client"))
                .args(["monkey::monkey_long", "--exact", "--ignored", "--nocapture"])
                .env("FB_MONKEY_SECS", a.secs.to_string())
                .env("FB_MONKEY_SEED", seed.to_string())
                .stdout(Stdio::from(log.try_clone().ok()?))
                .stderr(Stdio::from(log))
                .spawn()
                .ok()?;
            Some((seed, path, child))
        })
        .collect();
    eprintln!("{} seeds from {first}, {} s each", runs.len(), a.secs);
    let deadline = Instant::now() + Duration::from_secs(a.secs.into()) + GRACE;
    let mut ok = runs.len() == a.jobs.max(1) as usize;
    for (seed, path, mut child) in runs {
        let status = wait_or_kill(&mut child, deadline);
        if status.is_some_and(|s| s.success()) {
            eprintln!("seed {seed}: ok");
            continue;
        }
        ok = false;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let panic = text
            .lines()
            .find(|l| l.contains("panicked at"))
            .unwrap_or(if status.is_none() {
                "(hung: killed)"
            } else {
                "(no panic line)"
            });
        let tail: Vec<&str> = text
            .lines()
            .skip_while(|l| !l.contains("failed; its last actions:"))
            .filter(|l| l.contains(" s  p"))
            .collect();
        eprintln!(
            "seed {seed}: FAILED — {}\n  {panic}\n  last actions:\n{}",
            path.display(),
            tail.iter()
                .rev()
                .take(25)
                .rev()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    reported(ok)
}
