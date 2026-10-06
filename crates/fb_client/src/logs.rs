//! The client's log on disk: a file a run in the profile's `logs/` (the newest `KEEP`), where crash and F8
//! reports go too; the settings and the crash note open the folder.
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use bevy::log::BoxedLayer;
use bevy::log::tracing_subscriber::filter::FilterFn;
use bevy::log::tracing_subscriber::{Layer, fmt};
use bevy::prelude::*;

use crate::ui::{Action, UiAction};

/// Files of each kind kept: logs, crash reports, F8 reports.
pub const KEEP: usize = 10;

/// The profile's `logs/` (None: tests and headless clients write no files).
#[derive(Resource, Clone, Default)]
pub struct Logs(pub Option<PathBuf>);

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// This run's log file.
pub fn file() -> Option<&'static Path> {
    FILE.get().map(PathBuf::as_path)
}

/// For `LogPlugin::custom_layer` (after `Logs` is in): the logbook, the profiler, and this run's file.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let mut layers: Vec<BoxedLayer> = fb_net::logbook::layer(app).into_iter().collect();
    layers.push(crate::perf::profiler::layer());
    let dir = app.world().get_resource::<Logs>().and_then(|l| l.0.clone());
    let Some((mut file, path)) = dir.and_then(|d| create(&d, "client", "log")) else {
        return Some(Box::new(layers));
    };
    let _ = writeln!(
        file,
        "Fall Beans {} | {} {} | times in UTC",
        fb_net::build(),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    eprintln!("log: {}", path.display());
    let _ = FILE.set(path);
    let to_file: BoxedLayer = Box::new(
        fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .with_filter(FilterFn::new(|m| m.is_event())),
    );
    layers.push(to_file);
    Some(Box::new(layers))
}

/// A new `<prefix>-<time>.<ext>` in `dir`; the oldest of its kind beyond `KEEP` go.
pub fn create(dir: &Path, prefix: &str, ext: &str) -> Option<(File, PathBuf)> {
    fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{prefix}-{}.{ext}", stamp(now_secs())));
    let file = File::create(&path).ok()?;
    prune(dir, prefix);
    Some((file, path))
}

pub fn now_secs() -> u64 {
    SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}

/// `2026-10-05_19-03-20` (UTC): names that sort by time.
pub fn stamp(secs: u64) -> String {
    let (days, s) = ((secs / 86_400) as i64, secs % 86_400);
    // (Howard Hinnant's `civil_from_days`.)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// Keeps the newest `KEEP` files named `<prefix>-…`.
pub fn prune(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let head = format!("{prefix}-");
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&head)))
        .collect();
    files.sort();
    for old in files.iter().rev().skip(KEEP) {
        let _ = fs::remove_file(old);
    }
}

pub struct LogsPlugin;

impl Plugin for LogsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, open_folder);
    }
}

fn open_folder(logs: Res<Logs>, mut actions: MessageReader<UiAction>) {
    for UiAction(a) in actions.read() {
        if let (Action::OpenLogs, Some(dir)) = (a, &logs.0) {
            open_dir(dir);
        }
    }
}

/// Shows a folder in the system's file manager.
fn open_dir(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let r = if cfg!(windows) {
        std::process::Command::new("explorer").arg(dir).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(dir).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(dir).spawn()
    };
    if let Err(e) = r {
        warn!("{}: {e}", dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "1970-01-01_00-00-00");
        assert_eq!(stamp(951_782_400), "2000-02-29_00-00-00");
        assert_eq!(stamp(1_759_691_000), "2025-10-05_19-03-20");
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("fb-prune-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 3 {
            fs::write(dir.join(format!("crash-{}.txt", stamp(1_000_000 + i as u64))), "").unwrap();
        }
        fs::write(dir.join("client-x.log"), "").unwrap();
        prune(&dir, "crash");
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP + 1);
        assert_eq!(left[0], "client-x.log");
        assert_eq!(left[1], format!("crash-{}.txt", stamp(1_000_003)));
        fs::remove_dir_all(&dir).unwrap();
    }
}
