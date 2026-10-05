//! A panic leaves a report in the profile's `crashes/`; the next start shows where it is.
use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::fs;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use bevy::prelude::*;

const KEEP: usize = 10;
const LOG_LINES: usize = 200;
/// Holds the path of a report the player has not been told about yet.
const MARKER: &str = "last";

/// The report of the previous run's panic, if there was one.
#[derive(Resource, Default)]
pub struct LastCrash(pub Option<PathBuf>);

pub struct CrashPlugin {
    pub dir: Option<PathBuf>,
}

impl Plugin for CrashPlugin {
    fn build(&self, app: &mut App) {
        let Some(dir) = self.dir.clone().map(|d| d.join("crashes")) else {
            app.init_resource::<LastCrash>();
            return;
        };
        app.insert_resource(LastCrash(take_marker(&dir)));
        let next = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            next(info);
            write_report(&dir, info);
        }));
    }
}

fn take_marker(dir: &Path) -> Option<PathBuf> {
    let marker = dir.join(MARKER);
    let path = fs::read_to_string(&marker).ok()?;
    let _ = fs::remove_file(&marker);
    Some(PathBuf::from(path.trim()))
}

fn write_report(dir: &Path, info: &PanicHookInfo) {
    // (Only the first panic: the others are usually its consequences.)
    static WRITTEN: AtomicBool = AtomicBool::new(false);
    if WRITTEN.swap(true, Ordering::Relaxed) || fs::create_dir_all(dir).is_err() {
        return;
    }
    let now = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default();
    let path = dir.join(format!("crash-{}.txt", now.as_secs()));
    if fs::write(&path, report(info, now.as_secs())).is_ok() {
        let _ = fs::write(dir.join(MARKER), path.to_string_lossy().as_bytes());
        eprintln!("crash report: {}", path.display());
    }
    prune(dir);
}

fn report(info: &PanicHookInfo, at: u64) -> String {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(not a string)".into());
    let place = info.location().map_or("?".into(), |l| l.to_string());
    let thread = std::thread::current().name().unwrap_or("?").to_string();
    let mut s = String::new();
    let _ = writeln!(s, "Fall Beans {} crashed", fb_net::build());
    let _ = writeln!(s, "time: {at}");
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(s, "thread: {thread}");
    let _ = writeln!(s, "panic: {msg}");
    let _ = writeln!(s, "at: {place}");
    let _ = writeln!(s, "\nbacktrace:\n{}", Backtrace::force_capture());
    let _ = writeln!(s, "log (last {LOG_LINES} lines):");
    for l in fb_net::logbook::tail(LOG_LINES) {
        let _ = writeln!(
            s,
            "{}.{:03} {} {}: {}",
            l.at / 1000,
            l.at % 1000,
            l.level,
            l.target,
            l.msg
        );
    }
    s
}

/// Keeps the newest `KEEP` reports (their names sort by time).
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut reports: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("crash-")))
        .collect();
    reports.sort();
    for old in reports.iter().rev().skip(KEEP) {
        let _ = fs::remove_file(old);
    }
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
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("fb-prune-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 3 {
            fs::write(dir.join(format!("crash-{}.txt", 1_000_000 + i)), "").unwrap();
        }
        fs::write(dir.join(MARKER), "").unwrap();
        prune(&dir);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP + 1);
        assert_eq!(left[0], "crash-1000003.txt");
        assert_eq!(left[KEEP], MARKER);
        fs::remove_dir_all(&dir).unwrap();
    }
}
