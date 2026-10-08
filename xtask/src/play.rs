//! `cargo xtask play`: the game as a player gets it, from its menu, against a dev server on localhost.
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use clap::Args;

use crate::{cargo, dlss_features, run, start_server, target_dir, with_features};

#[derive(Args)]
pub struct PlayArgs {
    /// The client exactly as the packages ship it (`dist` profile: thin LTO, a minute or more per change) instead of `perf`.
    #[arg(long)]
    dist: bool,
    /// More arguments for the client, e.g. `-- --windowed`.
    #[arg(last = true)]
    client_arg: Vec<String>,
}

fn perf_bin(name: &str) -> PathBuf {
    target_dir()
        .join("perf")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Server and client with the `perf` profile, as `cargo xtask perf run` builds them (one cache for both).
pub fn build_perf() -> Result<()> {
    let mut c = cargo();
    c.args([
        "build",
        "--locked",
        "--profile",
        "perf",
        "-p",
        "fb_server",
        "-p",
        "fb_client",
    ]);
    with_features(&mut c, &dlss_features());
    run(&mut c)
}

pub fn play(a: &PlayArgs) -> Result<()> {
    build_perf()?;
    let client = if a.dist {
        crate::dist::build_client()?
    } else {
        perf_bin("fb_client")
    };
    if let Some(dir) = client.parent() {
        crate::perf::dxc_beside(dir);
        let _ = crate::dist::upscalers_into(dir, None);
    }
    let mut server = start_server(Command::new(perf_bin("fb_server")).args(["--dev", "--solo"]))?;
    eprintln!("server: 127.0.0.1 (add it to the client's server list once)");
    let played = run(Command::new(&client).args(&a.client_arg));
    let _ = server.kill();
    let _ = server.wait();
    played
}
