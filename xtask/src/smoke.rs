//! `cargo xtask smoke`: the client on lavapipe (Mesa's software Vulkan), offscreen, through a practice round with
//! each upscaler. The tests' noop device makes no pipelines: shaders, pipelines and the upscalers only fail on a
//! driver.
use std::fs::File;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::Args;

use crate::stress::wait_or_kill;
use crate::{Shared, reported, start_server, strip_ansi, target_dir};

#[derive(Args)]
pub struct SmokeArgs {
    /// Seconds each run lasts from the launch: the warm-up of every map first, slow on the CPU.
    #[arg(long, default_value_t = 240)]
    secs: u32,
    /// The map of the practice round.
    #[arg(long, default_value = "jump-club")]
    map: String,
    /// Upscalers, one run each (default: FSR 3.1 when its library is built, and FSR 1).
    #[arg(long, value_delimiter = ',', value_parser = ["fsr3", "fsr1"])]
    upscaler: Vec<String>,
}

/// Past its seconds a run has this long to quit, then it counts as hung and is killed.
const GRACE: Duration = Duration::from_secs(60);

/// lavapipe's ICD manifest: `FB_LAVAPIPE`, else the Vulkan loader's usual folders.
pub fn lavapipe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FB_LAVAPIPE") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let mut dirs: Vec<PathBuf> = ["/etc/vulkan", "/usr/local/share/vulkan", "/usr/share/vulkan"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Some(d) = std::env::var_os("XDG_DATA_DIRS") {
        dirs.extend(std::env::split_paths(&d).map(|d| d.join("vulkan")));
    }
    let found: Vec<PathBuf> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(d.join("icd.d")).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("lvp_icd") && n.ends_with(".json"))
        })
        .collect();
    let arch = std::env::consts::ARCH;
    found
        .iter()
        .find(|p| p.to_string_lossy().contains(arch))
        .or(found.first())
        .cloned()
}

fn name(upscaler: &str) -> &'static str {
    match upscaler {
        "fsr3" => "AMD FSR 3.1",
        _ => "AMD FSR 1",
    }
}

pub fn smoke(a: &SmokeArgs) -> Result<()> {
    let icd = lavapipe().ok_or_else(|| {
        anyhow!("no lavapipe (lvp_icd*.json; FB_LAVAPIPE=/path/to/it): `cargo xtask doctor` says how to get it")
    })?;
    let s = Shared::default();
    s.build()?;
    let client = s.bin("fb_client");
    let upscalers = if a.upscaler.is_empty() {
        let fsr3 = client.parent().is_some_and(|d| d.join(crate::dist::FFX_LIB).is_file());
        if !fsr3 {
            eprintln!("smoke: no AMD FSR 3.1 library beside the client (`cargo xtask setup`): FSR 1 only");
        }
        [fsr3.then_some("fsr3"), Some("fsr1")]
            .into_iter()
            .flatten()
            .map(String::from)
            .collect()
    } else {
        a.upscaler.clone()
    };
    let dir = target_dir().join("smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let log = |n: &str| -> Result<File> {
        File::create(dir.join(n)).with_context(|| format!("cannot create {n} in {}", dir.display()))
    };
    eprintln!("smoke: lavapipe {}, logs in {}", icd.display(), dir.display());
    let server_log = log("server.log")?;
    let mut server = s.command("fb_server");
    server
        .args(["--dev", "--solo"])
        .stdout(Stdio::from(server_log.try_clone()?))
        .stderr(Stdio::from(server_log));
    let mut server = start_server(&mut server)?;
    let mut ok = true;
    for up in &upscalers {
        let shot = dir.join(format!("{}-{up}.png", a.map));
        let log_name = format!("client-{up}.log");
        let mut c = s.command("fb_client");
        c.env("VK_DRIVER_FILES", &icd)
            .env("VK_ICD_FILENAMES", &icd)
            .env_remove("VK_ADD_DRIVER_FILES")
            .args([
                "--profile",
                "smoke",
                "--offscreen",
                "--no-update",
                "--backend",
                "vulkan",
            ])
            .args(["--practice", &a.map, "--upscaler", up])
            .args(["--exit-after", &a.secs.to_string()])
            .arg("--screenshot")
            .arg(&shot)
            .stdout(Stdio::null())
            .stderr(Stdio::from(log(&log_name)?));
        eprintln!("smoke: {} on {}, {} s…", name(up), a.map, a.secs);
        let mut child = c.spawn().map_err(|e| anyhow!("cannot start the client: {e}"))?;
        let status = wait_or_kill(&mut child, Instant::now() + Duration::from_secs(a.secs.into()) + GRACE);
        let text = std::fs::read_to_string(dir.join(&log_name)).unwrap_or_default();
        let lines: Vec<String> = text.lines().map(strip_ansi).collect();
        let mut problems = Vec::new();
        match status {
            None => problems.push("hung: killed".to_string()),
            Some(st) if !st.success() => problems.push(format!("exited with {st}")),
            Some(_) => {}
        }
        let has = |parts: &[&str]| lines.iter().any(|l| parts.iter().all(|p| l.contains(p)));
        if !has(&["graphics: a software device"]) {
            problems.push("not on a software device: lavapipe was not the one used".into());
        }
        if !has(&["warm-up: ", " pipelines"]) {
            problems.push("the warm-up did not finish".into());
        }
        if !has(&["arena ", " seed "]) {
            problems.push(format!("no round within {} s (--secs)", a.secs));
        }
        if !has(&[&format!("upscaling: {} at", name(up))]) {
            let why = lines
                .iter()
                .find(|l| l.contains(&format!("{} not offered", name(up))))
                .map_or("", |l| l.as_str());
            problems.push(format!("not upscaled with {}: {why}", name(up)));
        }
        problems.extend(
            lines
                .iter()
                .filter(|l| l.contains(" ERROR ") || l.contains("panicked at"))
                .take(10)
                .cloned(),
        );
        if !std::fs::metadata(&shot).is_ok_and(|m| m.len() > 0) {
            problems.push("no screenshot".into());
        }
        if problems.is_empty() {
            eprintln!("  ok: {}", shot.display());
        } else {
            ok = false;
            eprintln!("  FAILED — {}", dir.join(&log_name).display());
            for p in &problems {
                eprintln!("    {p}");
            }
        }
    }
    let _ = server.kill();
    let _ = server.wait();
    reported(ok)
}
