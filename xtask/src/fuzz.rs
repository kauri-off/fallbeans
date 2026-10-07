//! `cargo xtask fuzz-ui`: the client's monkey (`fb_client/src/monkey.rs`) for a while, several seeds at once.
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Args;

use crate::stress::wait_or_kill;
use crate::{cargo, root, target_dir};

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

const TEST: [&str; 5] = ["test", "-p", "fb_client", "--bin", "fb_client"];
/// Past its seconds of play a seed has this long to finish, then it counts as hung and is killed.
const GRACE: Duration = Duration::from_secs(120);

/// Builds the client's test binary and returns its path (run directly, so that a hung one can be killed: killing
/// `cargo test` would leave it running).
fn test_binary() -> Option<PathBuf> {
    let mut c = cargo();
    c.args(TEST).args(["--no-run", "--message-format=json"]);
    eprintln!("$ {c:?}");
    let out = c.stderr(Stdio::inherit()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["reason"] == "compiler-artifact" && v["target"]["name"] == "fb_client" && v["profile"]["test"] == true {
            v["executable"].as_str().map(PathBuf::from)
        } else {
            None
        }
    })
}

pub fn fuzz(a: &FuzzArgs) -> bool {
    let Some(exe) = test_binary() else {
        eprintln!("the client's test binary did not build");
        return false;
    };
    let dir = target_dir().join("fuzz-ui");
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("cannot create {}", dir.display());
        return false;
    }
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
            let child = Command::new(&exe)
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
    ok
}
