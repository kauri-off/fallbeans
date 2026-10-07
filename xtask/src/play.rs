//! `cargo xtask play`: the game as a player gets it, from its menu, against a dev server on localhost.
use std::path::PathBuf;
use std::process::Command;

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
pub fn build_perf() -> bool {
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

pub fn play(a: &PlayArgs) -> bool {
    if !build_perf() {
        return false;
    }
    let client = if a.dist {
        let Some(exe) = crate::dist::build_client() else {
            return false;
        };
        exe
    } else {
        perf_bin("fb_client")
    };
    if let Some(dir) = client.parent() {
        crate::perf::dxc_beside(dir);
        crate::dist::upscalers_into(dir, None);
    }
    let Some(mut server) = start_server(Command::new(perf_bin("fb_server")).args(["--dev", "--solo"])) else {
        return false;
    };
    eprintln!("server: 127.0.0.1 (add it to the client's server list once)");
    let mut c = Command::new(&client);
    c.args(&a.client_arg);
    let ok = run(&mut c);
    let _ = server.kill();
    let _ = server.wait();
    ok
}
