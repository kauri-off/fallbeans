//! `cargo xtask fuzz-ui`: the client's monkey (`fb_client/src/monkey.rs`) for a while, several seeds at once.
use std::fs::File;
use std::process::Stdio;

use clap::Args;

use crate::{cargo, root, run};

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

pub fn fuzz(a: &FuzzArgs) -> bool {
    if !run(cargo().args(TEST).arg("--no-run")) {
        return false;
    }
    let dir = root().join("target").join("fuzz-ui");
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
            let child = cargo()
                .args(TEST)
                .args(["--", "monkey::monkey_long", "--exact", "--ignored", "--nocapture"])
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
    let mut ok = runs.len() == a.jobs.max(1) as usize;
    for (seed, path, mut child) in runs {
        if child.wait().is_ok_and(|s| s.success()) {
            eprintln!("seed {seed}: ok");
            continue;
        }
        ok = false;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let panic = text
            .lines()
            .find(|l| l.contains("panicked at"))
            .unwrap_or("(no panic line)");
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
