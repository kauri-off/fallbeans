//! `cargo xtask check`: fmt, clippy, tests; only what needs reading on screen, everything in target/check.log.
use std::fs::File;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;

use anyhow::Result;
use clap::Args;

use crate::{Reported, cargo, dev_features, reported, strip_ansi, target_dir, with_features};

#[derive(Args)]
pub struct CheckArgs {
    /// All of cargo's output, as it comes.
    #[arg(long, short)]
    verbose: bool,
}

/// Client tests stepping a real server against the wall clock: a busy machine can time them out.
const RETRIED: &str = "scenarios::";

#[derive(Default)]
struct Tests {
    passed: u32,
    failed: Vec<String>,
    ignored: u32,
}

impl Tests {
    fn read(&mut self, line: &str) {
        if let Some(name) = line.strip_prefix("test ").and_then(|l| l.strip_suffix(" ... FAILED")) {
            self.failed.push(name.to_string());
        } else if let Some(counts) = line.strip_prefix("test result: ") {
            let count = |what: &str| {
                counts
                    .split([';', '.'])
                    .find_map(|p| p.trim().strip_suffix(what)?.trim().parse::<u32>().ok())
                    .unwrap_or(0)
            };
            self.passed += count(" passed");
            self.ignored += count(" ignored");
        }
    }
}

struct Check {
    verbose: bool,
    color: bool,
    log: Option<File>,
}

impl Check {
    /// Runs `c`, logs every line, shows the ones that matter; `on_line` sees each line without colors.
    fn step(&mut self, name: &str, c: &mut Command, mut on_line: impl FnMut(&str)) -> bool {
        let shown: Vec<String> = std::iter::once("cargo".to_string())
            .chain(c.get_args().map(|a| a.to_string_lossy().into_owned()))
            .collect();
        eprintln!("== {name}: {}", shown.join(" "));
        if let Some(log) = &mut self.log {
            let _ = writeln!(log, "== {name}: {}", shown.join(" "));
        }
        if self.color {
            c.env("CARGO_TERM_COLOR", "always");
        }
        let started = Instant::now();
        let Ok(mut child) = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() else {
            eprintln!("{name}: cannot start cargo");
            return false;
        };
        let (tx, rx) = mpsc::channel();
        let readers: Vec<_> = [
            child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>),
            child.stderr.take().map(|e| Box::new(e) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|pipe| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            })
        })
        .collect();
        drop(tx);
        let mut blank = true;
        for line in rx {
            let plain = strip_ansi(&line);
            if let Some(log) = &mut self.log {
                let _ = writeln!(log, "{plain}");
            }
            on_line(&plain);
            if self.verbose || !noise(&plain) {
                let is_blank = plain.trim().is_empty();
                if !(is_blank && blank) {
                    eprintln!("{line}");
                }
                blank = is_blank;
            }
        }
        for r in readers {
            let _ = r.join();
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        let secs = started.elapsed().as_secs();
        eprintln!("== {name}: {} ({secs} s)", if ok { "ok" } else { "FAILED" });
        ok
    }
}

/// Progress, passing tests and the game's log lines (interleaved from parallel tests): only the log keeps them.
fn noise(line: &str) -> bool {
    if is_log_line(line) {
        return true;
    }
    let l = line.trim_start();
    const PREFIXES: [&str; 8] = [
        "Compiling ",
        "Checking ",
        "Finished ",
        "Running ",
        "Doc-tests ",
        "Fresh ",
        "Blocking waiting",
        "test result: ok.",
    ];
    PREFIXES.iter().any(|p| l.starts_with(p))
        || (l.starts_with("running ") && (l.ends_with(" tests") || l.ends_with(" test")))
        || (l.starts_with("test ") && (l.ends_with(" ... ok") || l.ends_with(" ... ignored")))
}

/// `2026-10-07T19:34:37.049912Z  INFO …`, or a `Tick(n): …` line of lightyear's input buffer dumps.
fn is_log_line(line: &str) -> bool {
    let b = line.as_bytes();
    let stamped = b.len() > 20 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-' && b[10] == b'T';
    let tick = line
        .trim_start()
        .strip_prefix("Tick(")
        .and_then(|l| l.split_once("): "))
        .is_some_and(|(n, _)| n.bytes().all(|c| c.is_ascii_digit()));
    stamped || tick
}

pub fn check(a: &CheckArgs) -> Result<()> {
    let path = target_dir().join("check.log");
    let _ = std::fs::create_dir_all(target_dir());
    let mut ch = Check {
        verbose: a.verbose,
        color: std::io::stderr().is_terminal(),
        log: File::create(&path).ok(),
    };
    let features = dev_features();
    let summary = |parts: &[String]| {
        eprintln!("\ncheck: {} (log: {})", parts.join(" · "), path.display());
    };
    if !ch.step("fmt", cargo().args(["fmt", "--all", "--check"]), |_| {}) {
        summary(&["fmt FAILED (cargo fmt --all)".into()]);
        return Err(Reported.into());
    }
    let mut clippy = cargo();
    clippy.args(["clippy", "--locked", "--workspace", "--all-targets"]);
    // (DLSS's code too when its SDK is there: clippy links nothing, so `dynamic` does not get in the way.)
    with_features(
        &mut clippy,
        &[features.as_slice(), &crate::dlss_features()[..]].concat(),
    );
    clippy.args(["--", "-D", "warnings"]);
    if !ch.step("clippy", &mut clippy, |_| {}) {
        summary(&["fmt ok".into(), "clippy FAILED".into()]);
        return Err(Reported.into());
    }
    // Without `traces`, as packages build (Bevy's features as above; no `--all-targets`: tests bring `traces` back).
    let mut bare = cargo();
    bare.args([
        "clippy",
        "--locked",
        "-p",
        "fb_client",
        "-p",
        "fb_server",
        "--no-default-features",
    ]);
    with_features(
        &mut bare,
        &[
            features.as_slice(),
            &crate::dlss_features()[..],
            &["fb_client/brp", "fb_client/profiler"],
        ]
        .concat(),
    );
    bare.args(["--", "-D", "warnings"]);
    if !ch.step("clippy (no traces)", &mut bare, |_| {}) {
        summary(&["fmt ok".into(), "clippy (no traces) FAILED".into()]);
        return Err(Reported.into());
    }
    let test_cmd = |extra: &[&str]| {
        let mut c = cargo();
        c.args(["test", "--locked", "--workspace", "--no-fail-fast"]);
        with_features(&mut c, &features);
        c.args(extra);
        c
    };
    let mut tests = Tests::default();
    let mut ok = ch.step("test", &mut test_cmd(&[]), |l| tests.read(l));
    let mut flaky = Vec::new();
    let retry = !ok && !tests.failed.is_empty() && tests.failed.iter().all(|t| t.starts_with(RETRIED));
    if retry {
        let mut again = Tests::default();
        let mut extra = vec!["--tests", "--", "--exact"];
        extra.extend(tests.failed.iter().map(String::as_str));
        if ch.step("test (retry)", &mut test_cmd(&extra), |l| again.read(l)) && again.failed.is_empty() {
            ok = true;
            flaky = std::mem::take(&mut tests.failed);
            tests.passed += flaky.len() as u32;
        }
    }
    let mut parts = vec!["fmt ok".to_string(), "clippy ok".into()];
    let mut t = format!("tests {} passed", tests.passed);
    if tests.ignored > 0 {
        t += &format!(", {} ignored", tests.ignored);
    }
    if !flaky.is_empty() {
        t += &format!(", flaky (passed on retry): {}", flaky.join(", "));
    }
    if !tests.failed.is_empty() {
        t += &format!(", FAILED: {}", tests.failed.join(", "));
    } else if !ok {
        t += ", FAILED (see above)";
    }
    parts.push(t);
    summary(&parts);
    reported(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_failures() {
        let mut t = Tests::default();
        for l in [
            "test a::b ... ok",
            "test scenarios::x ... FAILED",
            "test result: FAILED. 12 passed; 1 failed; 2 ignored; 0 measured; 0 filtered out; finished in 1.0s",
            "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s",
        ] {
            t.read(l);
        }
        assert_eq!((t.passed, t.ignored), (15, 2));
        assert_eq!(t.failed, ["scenarios::x"]);
    }

    #[test]
    fn noise_and_colors() {
        assert!(noise("   Compiling bevy v0.19.1"));
        assert!(noise("test a::b ... ok"));
        assert!(noise("running 12 tests"));
        assert!(!noise("test a::b ... FAILED"));
        assert!(!noise("error[E0308]: mismatched types"));
        assert!(noise("2026-10-07T19:34:37.049912Z  INFO fb_client::session: arena 5"));
        assert!(!noise(
            "thread 'scenarios::x' panicked at crates/fb_client/src/scenarios.rs:1:1:"
        ));
        assert!(noise(
            "Tick(3042): ActionState(FbInput { mx: -65, mz: 109, buttons: 0 })"
        ));
        assert_eq!(strip_ansi("\x1b[1m\x1b[32mCompiling\x1b[0m x"), "Compiling x");
    }
}
