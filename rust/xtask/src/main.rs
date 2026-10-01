//! Project tasks: `cargo xtask <check|golden|assets|dev|smoke>`.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn run(cmd: &mut Command) -> bool {
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

/// fmt, clippy (warnings are errors), tests.
fn check() -> bool {
    run(cargo().args(["fmt", "--all", "--check"]))
        && run(cargo().args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))
        && run(cargo().args(["test", "--workspace"]))
}

/// Server plus clients (`--clients n`, default 2) with the given extra flags for both.
fn dev(args: &[String]) -> bool {
    let mut n = 2;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--clients" {
            n = it.next().and_then(|v| v.parse().ok()).unwrap_or(2);
        } else {
            rest.push(a.clone());
        }
    }
    if !run(cargo().args(["build", "-p", "fb_server", "-p", "fb_client"])) {
        return false;
    }
    let bin = root().join("target/debug");
    let mut server = Command::new(bin.join("fb_server")).args(&rest).spawn().expect("server");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let clients: Vec<Child> = (0..n)
        .map(|i| {
            Command::new(bin.join("fb_client"))
                .args(["--id", &(1000 + i).to_string(), "--title", &format!("Fall Beans — {}", (b'a' + i as u8) as char)])
                .args(&rest)
                .spawn()
                .expect("client")
        })
        .collect();
    for mut c in clients {
        let _ = c.wait();
    }
    let _ = server.kill();
    true
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ok = match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("golden") => run(bun(&["scripts/golden.ts"])) && run(cargo().args(["test", "-p", "fb_arena", "--test", "golden"])),
        Some("assets") => run(bun(&["scripts/assets.ts", "--bevy"])) && run(cargo().args(["run", "-p", "fb_client", "--", "--check-assets"])),
        Some("dev") => dev(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <check|golden|assets|dev [--clients n] [flags for server and clients]>");
            false
        }
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
