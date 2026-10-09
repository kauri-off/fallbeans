//! A panic, or a GPU that stops answering, leaves a report in the profile's `logs/`; the next start shows
//! where it is.
use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::render::error_handler::{ErrorType, RenderError, RenderErrorPolicy};

use crate::logs::{self, Logs};

const LOG_LINES: usize = 200;
/// Holds the path of a report the player has not been told about yet.
const MARKER: &str = "last-crash";

/// The report of the previous run's crash, if there was one.
#[derive(Resource, Default)]
pub struct LastCrash(pub Option<Crash>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crash {
    pub report: PathBuf,
    /// The GPU was lost (a driver reset or a hang): the player's driver, not the game, is the first suspect.
    pub gpu_lost: bool,
}

pub struct CrashPlugin;

impl Plugin for CrashPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = app.world().resource::<Logs>().0.clone() else {
            app.init_resource::<LastCrash>();
            return;
        };
        app.insert_resource(LastCrash(take_marker(&dir)));
        let next = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            next(info);
            if !background(std::thread::current().name()) {
                write_report(&dir, &panic_text(info), device_lost());
            }
        }));
    }
}

/// A thread whose panic does not end the game: Bevy's I/O and async pools (a file loaded or saved, a face
/// drawn) and the game's own helpers named `fb-…` (an update, a request). Such a panic is no crash to tell the
/// player about, and must not take the one report of the run from a real crash after it. The main thread,
/// the systems' pool and the render thread (unnamed) are the game.
fn background(thread: Option<&str>) -> bool {
    thread.is_some_and(|n| {
        n.starts_with("fb-") || n.starts_with("IO Task Pool") || n.starts_with("Async Compute Task Pool")
    })
}

/// The report without the player's home folder (the user name in it): `~` instead.
pub fn redact(s: &str) -> String {
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    redact_home(s, &home)
}

fn redact_home(s: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    // (Too short to be a user's own folder: `/` or a drive.)
    if home.chars().count() < 4 {
        return s.to_string();
    }
    let slashed = home.replace('\\', "/");
    s.replace(home, "~").replace(&slashed, "~")
}

/// `RenderErrorHandler`: Bevy's (quit), with a report first. A lost device (a driver reset, a GPU hang) is
/// the usual one.
pub fn render_failed(error: &RenderError, main: &mut World, _: &mut World) -> RenderErrorPolicy {
    if let Some(dir) = main.get_resource::<Logs>().and_then(|l| l.0.clone()) {
        let what = format!("GPU error ({:?}): {}\n", error.ty, error.description);
        write_report(&dir, &what, error.ty == ErrorType::DeviceLost);
    }
    main.write_message(AppExit::error());
    RenderErrorPolicy::StopRendering
}

/// Bevy's render systems run on after a lost device until its handler sees it, and the first buffer they
/// create on it panics: such a panic is the lost GPU's, which Bevy has logged just before.
fn device_lost() -> bool {
    fb_net::logbook::tail(LOG_LINES)
        .iter()
        .any(|l| l.target == "bevy_render::error_handler" && l.msg.starts_with("Caught DeviceLost"))
}

/// The marker: the report's path, then `gpu-lost` on a line of its own if the GPU was lost.
fn take_marker(dir: &Path) -> Option<Crash> {
    let marker = dir.join(MARKER);
    let text = fs::read_to_string(&marker).ok()?;
    let _ = fs::remove_file(&marker);
    let mut lines = text.lines();
    let report = PathBuf::from(lines.next()?.trim());
    let gpu_lost = lines.any(|l| l.trim() == GPU_LOST);
    Some(Crash { report, gpu_lost })
}

const GPU_LOST: &str = "gpu-lost";

fn write_report(dir: &Path, what: &str, gpu_lost: bool) {
    // (Only the first: the others are usually its consequences.)
    static WRITTEN: AtomicBool = AtomicBool::new(false);
    if WRITTEN.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some((mut file, path)) = logs::create(dir, "crash", "txt") else {
        return;
    };
    let what = if gpu_lost {
        format!("cause: the GPU was lost (a driver reset or a GPU hang); anything below follows from it\n{what}")
    } else {
        what.to_string()
    };
    if let Err(e) = file.write_all(redact(&report(&what)).as_bytes()) {
        eprintln!("crash report: {}: {e}", path.display());
        return;
    }
    let mut marker = path.to_string_lossy().into_owned();
    if gpu_lost {
        marker = format!("{marker}\n{GPU_LOST}");
    }
    if let Err(e) = fs::write(dir.join(MARKER), marker.as_bytes()) {
        eprintln!("crash marker: {e}");
    }
    eprintln!("crash report: {}", path.display());
}

fn panic_text(info: &PanicHookInfo) -> String {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(not a string)".into());
    let place = info.location().map_or("?".into(), |l| l.to_string());
    let thread = std::thread::current().name().unwrap_or("?").to_string();
    format!(
        "thread: {thread}\npanic: {msg}\nat: {place}\n\nbacktrace:\n{}\n",
        Backtrace::force_capture()
    )
}

fn report(what: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Fall Beans {} crashed", fb_net::build());
    let _ = writeln!(s, "time: {} UTC", logs::stamp(logs::now_secs()));
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    if let Some(log) = logs::file() {
        let _ = writeln!(s, "log: {}", log.display());
    }
    s.push_str(what);
    let _ = writeln!(s, "\nlog (last {LOG_LINES} lines):");
    for l in fb_net::logbook::tail(LOG_LINES) {
        let _ = writeln!(s, "{}", line(&l));
    }
    s
}

/// A logbook line as in the reports: `19:03:20.123 WARN target: message` (UTC).
pub fn line(l: &fb_net::logbook::LogLine) -> String {
    let t = logs::stamp(l.at / 1000);
    let clock = t.split_once('_').map_or(t.as_str(), |(_, c)| c).replace('-', ":");
    format!("{clock}.{:03} {} {}: {}", l.at % 1000, l.level, l.target, l.msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_is_read_once() {
        let dir = std::env::temp_dir().join(format!("fb-crash-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MARKER), "/x/crash-1.txt\n").unwrap();
        let crash = |gpu_lost| {
            Some(Crash {
                report: PathBuf::from("/x/crash-1.txt"),
                gpu_lost,
            })
        };
        assert_eq!(take_marker(&dir), crash(false));
        assert_eq!(take_marker(&dir), None);
        fs::write(dir.join(MARKER), format!("/x/crash-1.txt\n{GPU_LOST}")).unwrap();
        assert_eq!(take_marker(&dir), crash(true));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn background_threads_are_no_crash() {
        assert!(background(Some("IO Task Pool (0)")));
        assert!(background(Some("Async Compute Task Pool (2)")));
        assert!(background(Some("fb-update")));
        for game in [Some("main"), Some("Compute Task Pool (1)"), None] {
            assert!(!background(game), "{game:?}");
        }
    }

    #[test]
    fn the_home_folder_is_left_out() {
        let s = "log: C:\\Users\\Боб\\AppData\\Roaming\\x.log\nat C:/Users/Боб/src/a.rs\n";
        assert_eq!(
            redact_home(s, "C:\\Users\\Боб"),
            "log: ~\\AppData\\Roaming\\x.log\nat ~/src/a.rs\n"
        );
        assert_eq!(redact_home("/home/bob/.config/x", "/home/bob/"), "~/.config/x");
        assert_eq!(redact_home("/x", "/"), "/x");
    }

    #[test]
    fn lines_read_as_a_clock() {
        let l = fb_net::logbook::LogLine {
            at: 1_759_691_000_042,
            level: bevy::log::Level::WARN,
            target: "fb_client::net".into(),
            msg: "lost".into(),
        };
        assert_eq!(line(&l), "19:03:20.042 WARN fb_client::net: lost");
    }
}
