//! `cargo xtask deploy`: builds the server for Linux (a static musl binary: it runs on any glibc), packs it
//! with `deploy/` and installs it on the host (`deploy/remote-install.sh`: checks, release switch with a
//! health check, rollback on failure, checks from outside).
//!
//! On Windows the build runs in WSL (`--wsl-distro`, default Ubuntu), which needs Rust with the
//! `x86_64-unknown-linux-musl` target and `build-essential cmake musl-tools`.
use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;

use crate::{root, run};

pub const LINUX_TARGET: &str = "x86_64-unknown-linux-musl";

/// A server host over SSH (`deploy`, `stress --remote`); no defaults: flags or DEPLOY_* variables.
#[derive(Args, Clone, Debug)]
pub struct Host {
    /// SSH target, e.g. `user@203.0.113.7` (default DEPLOY_HOST).
    #[arg(long)]
    pub host: Option<String>,
    /// SSH key (default DEPLOY_KEY, else ssh's own choice).
    #[arg(long)]
    pub key: Option<PathBuf>,
    /// Domain whose nginx site serves the game's https and wss (default DEPLOY_DOMAIN).
    #[arg(long)]
    pub domain: Option<String>,
    /// The address players reach UDP at (default DEPLOY_PUBLIC_IP, else the SSH target when it is an IP).
    #[arg(long)]
    pub public_ip: Option<IpAddr>,
}

impl Host {
    pub fn complete(&self) -> bool {
        let missing: Vec<&str> = [
            ("--host / DEPLOY_HOST", self.try_target().is_some()),
            ("--domain / DEPLOY_DOMAIN", self.try_domain().is_some()),
            ("--public-ip / DEPLOY_PUBLIC_IP", self.try_public_ip().is_some()),
        ]
        .into_iter()
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| name)
        .collect();
        if !missing.is_empty() {
            eprintln!("no server host given: {}", missing.join(", "));
            return false;
        }
        if let Some(k) = self.try_key()
            && !k.exists()
        {
            eprintln!("SSH key not found: {}", k.display());
            return false;
        }
        true
    }

    fn try_target(&self) -> Option<String> {
        self.host.clone().or_else(|| std::env::var("DEPLOY_HOST").ok())
    }

    fn try_key(&self) -> Option<PathBuf> {
        self.key
            .clone()
            .or_else(|| std::env::var_os("DEPLOY_KEY").map(PathBuf::from))
    }

    fn try_domain(&self) -> Option<String> {
        self.domain.clone().or_else(|| std::env::var("DEPLOY_DOMAIN").ok())
    }

    fn try_public_ip(&self) -> Option<IpAddr> {
        self.public_ip
            .or_else(|| std::env::var("DEPLOY_PUBLIC_IP").ok()?.parse().ok())
            .or_else(|| {
                let t = self.try_target()?;
                t.rsplit('@').next()?.parse().ok()
            })
    }

    pub fn target(&self) -> String {
        self.try_target().expect("Host::complete")
    }

    pub fn domain(&self) -> String {
        self.try_domain().expect("Host::complete")
    }

    pub fn public_ip(&self) -> IpAddr {
        self.try_public_ip().expect("Host::complete")
    }

    fn opts(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .try_key()
            .map(|k| vec!["-i".into(), k.to_string_lossy().into()])
            .unwrap_or_default();
        v.extend(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"].map(String::from));
        v
    }

    pub fn ssh(&self, remote_cmd: &str) -> Command {
        let mut c = Command::new("ssh");
        c.args(self.opts()).arg(self.target()).arg(remote_cmd);
        c
    }

    /// `scp` from `from` to `to`; a remote side is written `:path` (the host is added).
    pub fn scp(&self, from: &str, to: &str) -> Command {
        let side = |s: &str| match s.strip_prefix(':') {
            Some(p) => format!("{}:{p}", self.target()),
            None => s.to_string(),
        };
        let mut c = Command::new("scp");
        c.args(self.opts()).arg(side(from)).arg(side(to));
        c
    }
}

#[derive(Args)]
pub struct DeployArgs {
    #[command(flatten)]
    host: Host,
    /// Build and pack (target/deploy/fallbeans-update.tar.gz) without uploading.
    #[arg(long)]
    pack_only: bool,
    /// Do not ask for confirmation.
    #[arg(long)]
    yes: bool,
    /// Skip `cargo xtask check`.
    #[arg(long)]
    skip_checks: bool,
    /// WSL distribution the Linux build runs in (Windows only).
    #[arg(long, default_value = "Ubuntu")]
    wsl_distro: String,
}

/// `C:\Users\…` → `/mnt/c/Users/…`.
fn wsl_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    match s.split_once(":/") {
        Some((drive, rest)) if drive.len() == 1 => format!("/mnt/{}/{rest}", drive.to_ascii_lowercase()),
        _ => s,
    }
}

/// Builds `fb_server` for Linux and copies it to `out`.
pub fn build_linux_server(distro: &str, out: &Path) -> bool {
    let copy = format!(
        "cp \"$CARGO_TARGET_DIR/{LINUX_TARGET}/release/fb_server\" \"{}\"",
        wsl_path(out)
    );
    let build = format!(
        "cargo build --release -p fb_server --target {LINUX_TARGET} --config profile.release.strip='\"debuginfo\"'"
    );
    if cfg!(windows) {
        // The build directory lives in WSL's own file system (fast); the sources stay where they are.
        let script = format!(
            "set -e; . \"$HOME/.cargo/env\"; cd \"{}\"; export CARGO_TARGET_DIR=\"$HOME/fb-target\"; {build}; {copy}",
            wsl_path(&root())
        );
        let mut c = Command::new("wsl");
        c.args(["-d", distro, "--exec", "bash", "-lc", &script]);
        // Git Bash would rewrite the /mnt/… paths.
        c.env("MSYS_NO_PATHCONV", "1");
        run(&mut c)
    } else {
        let script = format!(
            "set -e; export CARGO_TARGET_DIR=\"{}\"; {build}; {copy}",
            root().join("target").display()
        );
        run(Command::new("bash").args(["-c", &script]).current_dir(root()))
    }
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

pub fn deploy(a: &DeployArgs) -> bool {
    if !a.pack_only && !a.host.complete() {
        return false;
    }
    if !a.pack_only
        && !a.yes
        && !confirm(&format!(
            "[deploy] Update Fall Beans on {} ({})?",
            a.host.target(),
            a.host.domain()
        ))
    {
        eprintln!("[deploy] cancelled");
        return false;
    }
    if !a.skip_checks && !crate::check() {
        eprintln!("[deploy] checks failed");
        return false;
    }
    let stage = root().join("target").join("deploy");
    let bundle = stage.join("bundle");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(bundle.join("nginx")).expect("target/deploy");
    eprintln!("[deploy] Linux build");
    if !build_linux_server(&a.wsl_distro, &bundle.join("fb_server")) {
        eprintln!("[deploy] the Linux build failed");
        return false;
    }
    let src = root().join("deploy");
    for f in [
        "remote-install.sh",
        "fallbeans.service",
        "nginx/fallbeans.conf",
        "nginx/fallbeans.ws",
        "nginx/fallbeans.http",
    ] {
        fs::copy(src.join(f), bundle.join(f)).unwrap_or_else(|e| panic!("deploy/{f}: {e}"));
    }
    let version = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .current_dir(root())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    fs::write(bundle.join("VERSION"), format!("{version}\n")).expect("VERSION");
    // Relative paths: GNU tar would read "C:\…" as a remote host.
    if !run(Command::new("tar")
        .args(["-czf", "../fallbeans-update.tar.gz", "."])
        .current_dir(&bundle))
    {
        return false;
    }
    let archive = stage.join("fallbeans-update.tar.gz");
    let kb = fs::metadata(&archive).map_or(0, |m| m.len() / 1024);
    eprintln!("[deploy] bundle: {} ({kb} KB, {version})", archive.display());
    if a.pack_only {
        return true;
    }
    let stamp = std::time::SystemTime::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_secs());
    let dir = format!("fallbeans-update-{stamp}");
    let (domain, ip) = (a.host.domain(), a.host.public_ip());
    run(&mut a.host.scp(&archive.to_string_lossy(), &format!(":{dir}.tar.gz")))
        && run(&mut a.host.ssh(&format!(
            "mkdir -p ~/{dir} && tar -xzf ~/{dir}.tar.gz -C ~/{dir} && sudo bash ~/{dir}/remote-install.sh '{domain}' '{ip}'; code=$?; rm -rf ~/{dir} ~/{dir}.tar.gz; exit $code"
        )))
}
