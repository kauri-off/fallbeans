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
use bevy::render::error_handler::{RenderError, RenderErrorPolicy};

use crate::logs::{self, Logs};

const LOG_LINES: usize = 200;
/// Holds the path of a report the player has not been told about yet.
const MARKER: &str = "last-crash";

/// The report of the previous run's crash, if there was one.
#[derive(Resource, Default)]
pub struct LastCrash(pub Option<PathBuf>);

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
            write_report(&dir, &panic_text(info));
        }));
    }
}

/// `RenderErrorHandler`: Bevy's (quit), with a report first. A lost device (a driver reset, a GPU hang) is
/// the usual one.
pub fn render_failed(error: &RenderError, main: &mut World, _: &mut World) -> RenderErrorPolicy {
    if let Some(dir) = main.get_resource::<Logs>().and_then(|l| l.0.clone()) {
        write_report(&dir, &format!("GPU error ({:?}): {}\n", error.ty, error.description));
    }
    main.write_message(AppExit::error());
    RenderErrorPolicy::StopRendering
}

fn take_marker(dir: &Path) -> Option<PathBuf> {
    let marker = dir.join(MARKER);
    let path = fs::read_to_string(&marker).ok()?;
    let _ = fs::remove_file(&marker);
    Some(PathBuf::from(path.trim()))
}

fn write_report(dir: &Path, what: &str) {
    // (Only the first: the others are usually its consequences.)
    static WRITTEN: AtomicBool = AtomicBool::new(false);
    if WRITTEN.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some((mut file, path)) = logs::create(dir, "crash", "txt") else {
        return;
    };
    if file.write_all(report(what).as_bytes()).is_ok() {
        let _ = fs::write(dir.join(MARKER), path.to_string_lossy().as_bytes());
        eprintln!("crash report: {}", path.display());
    }
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
        assert_eq!(take_marker(&dir), Some(PathBuf::from("/x/crash-1.txt")));
        assert_eq!(take_marker(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lines_read_as_a_clock() {
        let l = fb_net::logbook::LogLine {
            at: 1_759_691_000_042,
            level: "WARN",
            target: "fb_client::net".into(),
            msg: "lost".into(),
        };
        assert_eq!(line(&l), "19:03:20.042 WARN fb_client::net: lost");
    }
}
