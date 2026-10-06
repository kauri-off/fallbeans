//! The client's log on disk: a file a run in the profile's `logs/` (the newest `KEEP`), where crash and F8
//! reports go too; the settings and the crash note open the folder.
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{Duration, Instant, SystemTime};

use bevy::log::BoxedLayer;
use bevy::log::tracing::{Level, Metadata};
use bevy::log::tracing_subscriber::filter::FilterFn;
use bevy::log::tracing_subscriber::fmt::MakeWriter;
use bevy::log::tracing_subscriber::{Layer, fmt};
use bevy::prelude::*;

use crate::ui::{Action, UiAction};

/// Files of each kind kept: logs, crash reports, F8 reports.
pub const KEEP: usize = 10;
/// A log file this long (bytes) moves to `<name>.1` (replacing the one before) and a new one starts.
const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Lines wait in memory at most about this long (a warning or an error is written at once).
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// The profile's `logs/` (None: tests and headless clients write no files).
#[derive(Resource, Clone, Default)]
pub struct Logs(pub Option<PathBuf>);

static FILE: OnceLock<PathBuf> = OnceLock::new();
static OUT: OnceLock<Mutex<Out>> = OnceLock::new();

/// This run's log file.
pub fn file() -> Option<&'static Path> {
    FILE.get().map(PathBuf::as_path)
}

/// This run's log file, buffered (a write a line from whichever thread logs would stall it).
struct Out {
    w: Option<BufWriter<File>>,
    bytes: u64,
    flushed: Instant,
    path: PathBuf,
}

impl Out {
    fn new(file: File, path: PathBuf) -> Out {
        let mut out = Out {
            w: Some(BufWriter::with_capacity(64 * 1024, file)),
            bytes: 0,
            flushed: Instant::now(),
            path,
        };
        out.header("");
        out
    }

    fn header(&mut self, more: &str) {
        let line = format!(
            "Fall Beans {} | {} {} | times in UTC{more}\n",
            fb_net::build(),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let _ = self.write_all(line.as_bytes());
    }

    /// The file moves to `<name>.1` and a new one starts (if it cannot move, it starts over).
    fn rotate(&mut self) {
        if let Some(mut w) = self.w.take() {
            let _ = w.flush();
        }
        let mut old = OsString::from(self.path.as_os_str());
        old.push(".1");
        let _ = fs::rename(&self.path, PathBuf::from(old));
        self.bytes = 0;
        self.w = File::create(&self.path)
            .ok()
            .map(|f| BufWriter::with_capacity(64 * 1024, f));
        self.header(" | continued: the lines before are in the .1 file");
    }

    fn flush_now(&mut self) {
        self.flushed = Instant::now();
        if let Some(w) = self.w.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes + buf.len() as u64 > MAX_BYTES && self.bytes > 0 {
            self.rotate();
        }
        let Some(w) = self.w.as_mut() else {
            return Ok(buf.len());
        };
        let n = w.write(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_now();
        Ok(())
    }
}

/// The file layer's writer: a line into the buffer, the buffer to the file now and then.
struct ToFile;

/// One event's line; written out when it is a warning or an error, or the buffer has waited long enough.
struct Line {
    out: Option<MutexGuard<'static, Out>>,
    urgent: bool,
}

impl<'a> MakeWriter<'a> for ToFile {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line {
            out: OUT.get().map(|m| m.lock().unwrap_or_else(PoisonError::into_inner)),
            urgent: false,
        }
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Line {
        let mut line = self.make_writer();
        line.urgent = *meta.level() <= Level::WARN;
        line
    }
}

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.out.as_mut() {
            Some(out) => out.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.out.as_mut() {
            Some(out) => out.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        if let Some(out) = self.out.as_mut()
            && (self.urgent || out.flushed.elapsed() >= FLUSH_EVERY)
        {
            out.flush_now();
        }
    }
}

/// Writes out what the log file has in memory (`LogsPlugin` does it every second and at the exit, `main`
/// once more after the app).
pub fn flush() {
    if let Some(m) = OUT.get() {
        m.lock().unwrap_or_else(PoisonError::into_inner).flush_now();
    }
}

/// The same from a panic: the panicking thread may hold the file itself.
fn flush_after_panic() {
    if let Some(m) = OUT.get() {
        match m.try_lock() {
            Ok(mut out) => out.flush_now(),
            Err(TryLockError::Poisoned(p)) => p.into_inner().flush_now(),
            Err(TryLockError::WouldBlock) => {}
        }
    }
}

/// For `LogPlugin::custom_layer` (after `Logs` is in): the logbook, the profiler, and this run's file.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let mut layers: Vec<BoxedLayer> = fb_net::logbook::layer(app).into_iter().collect();
    layers.push(crate::perf::profiler::layer());
    let dir = app.world().get_resource::<Logs>().and_then(|l| l.0.clone());
    let Some((file, path)) = dir.and_then(|d| create(&d, "client", "log")) else {
        return Some(Box::new(layers));
    };
    eprintln!("log: {}", path.display());
    let _ = FILE.set(path.clone());
    if OUT.set(Mutex::new(Out::new(file, path))).is_err() {
        return Some(Box::new(layers));
    }
    // (The lines before a crash reach the file; the crash report itself takes the logbook's.)
    let next = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        next(info);
        flush_after_panic();
    }));
    let to_file: BoxedLayer = Box::new(
        fmt::layer()
            .with_ansi(false)
            .with_writer(ToFile)
            .with_filter(FilterFn::new(|m| m.is_event())),
    );
    layers.push(to_file);
    Some(Box::new(layers))
}

/// A new `<prefix>-<time>-<ms>.<ext>` in `dir` (never one that is there: `-1`, `-2`… if need be); the oldest
/// of its kind beyond `KEEP` go.
pub fn create(dir: &Path, prefix: &str, ext: &str) -> Option<(File, PathBuf)> {
    fs::create_dir_all(dir).ok()?;
    let ms = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_millis() as u64;
    let base = format!("{prefix}-{}-{:03}", stamp(ms / 1000), ms % 1000);
    let (file, path) = (0..100).find_map(|n| {
        let name = if n == 0 {
            format!("{base}.{ext}")
        } else {
            format!("{base}-{n}.{ext}")
        };
        let path = dir.join(name);
        File::create_new(&path).ok().map(|f| (f, path))
    })?;
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
        app.add_systems(Last, flush_now_and_then);
    }
}

/// The log file's buffer out every second, and at the exit.
fn flush_now_and_then(time: Res<Time<Real>>, mut exit: MessageReader<AppExit>, mut at: Local<f32>) {
    let now = time.elapsed_secs();
    if exit.read().next().is_some() || now - *at >= FLUSH_EVERY.as_secs_f32() {
        *at = now;
        flush();
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

    #[test]
    fn files_made_at_once_do_not_collide() {
        let dir = std::env::temp_dir().join(format!("fb-create-{}", std::process::id()));
        let a = create(&dir, "report", "txt").unwrap().1;
        let b = create(&dir, "report", "txt").unwrap().1;
        assert_ne!(a, b);
        assert!(a.exists() && b.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
