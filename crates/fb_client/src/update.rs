//! Self-update from the latest GitHub release (NSIS and AppImage install it; Flatpak links to it).
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use semver::Version;
use sha2::{Digest, Sha256};

use crate::opts::Opts;

const REPO: &str = "kauri-off/fallbeans";
const SUMS: &str = "SHA256SUMS";

/// How this copy of the game was installed: what can update it.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// The NSIS installer's folder (its uninstaller next to the game).
    Nsis,
    /// The AppImage file the game runs from.
    AppImage(PathBuf),
    Flatpak,
}

impl Install {
    fn detect() -> Option<Self> {
        if std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists() {
            return Some(Self::Flatpak);
        }
        let exe = std::env::current_exe().ok()?;
        // (`APPIMAGE` alone may be inherited: a build started from an IDE that runs as an AppImage would
        // overwrite the IDE. This process must run from the image's own mount.)
        if let (Some(image), Some(dir)) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR")) {
            let dir = PathBuf::from(dir);
            let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
            let exe = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
            if exe.starts_with(&dir) {
                return Some(Self::AppImage(image.into()));
            }
        }
        (cfg!(windows) && exe.with_file_name("uninstall.exe").exists()).then_some(Self::Nsis)
    }

    /// The release asset this install updates from.
    fn asset_suffix(&self) -> Option<&'static str> {
        match self {
            Self::Nsis => Some("-setup.exe"),
            Self::AppImage(_) => Some(".AppImage"),
            Self::Flatpak => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub page: String,
    /// The file for this install and its size, and the checksums' URL.
    asset: Option<(String, String, u64)>,
    sums: Option<String>,
}

impl Release {
    pub fn installable(&self) -> bool {
        self.asset.is_some() && self.sums.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Available(Release),
    /// Bytes so far and in all.
    Downloading(u64, u64),
    Failed(String),
    Restarting,
}

#[derive(Resource)]
pub struct Update {
    pub install: Option<Install>,
    pub state: State,
    checking: Option<Mutex<Receiver<Option<Release>>>>,
    working: Option<Mutex<Receiver<Result<(), String>>>>,
    got: Arc<AtomicU64>,
    total: u64,
    release: Option<Release>,
}

pub struct UpdatePlugin;

impl Plugin for UpdatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start);
        app.add_systems(Update, poll);
    }
}

/// The game's own threads that may fail without ending it are named so (`crash::background`).
fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(format!("fb-{name}")).spawn(f) {
        warn!("update: no thread for {name}: {e}");
    }
}

fn start(mut commands: Commands, opts: Res<Opts>) {
    let install = if opts.no_update { None } else { Install::detect() };
    if install == Some(Install::Nsis) {
        spawn_named("update-cleanup", remove_old_installers);
    }
    let checking = install.clone().map(|i| {
        let (tx, rx) = channel();
        spawn_named("update-check", move || {
            let r = latest(&i);
            if let Err(e) = &r {
                warn!("update check: {e}");
            }
            let _ = tx.send(r.ok().flatten());
        });
        Mutex::new(rx)
    });
    commands.insert_resource(Update {
        install,
        state: State::Idle,
        checking,
        working: None,
        got: Arc::default(),
        total: 0,
        release: None,
    });
}

/// An installer is downloaded to `%TEMP%` (`fetch_and_install`) and runs from there while this game is closed:
/// the next start removes it (one still running is in use and stays until the start after).
fn remove_old_installers() {
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in dir.filter_map(Result::ok) {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if is_installer(&name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A release's installer as `fetch_and_install` names it (`FallBeans-0.2.0-setup.exe`).
fn is_installer(name: &str) -> bool {
    name.starts_with("FallBeans-") && name.ends_with("-setup.exe")
}

/// `body`: how long a response's body may take (None: as long as the whole call may).
fn agent(timeout: Option<Duration>, body: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(timeout)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(body)
        .user_agent(concat!("fallbeans/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// How long the download of `size` bytes may take: a stalled one ends with an error instead of
/// "downloading" for ever (ureq has no idle timeout), a slow line (32 KB/s) still gets through.
fn download_budget(size: u64) -> Duration {
    Duration::from_secs(120 + size / 32_768)
}

/// Whether `tag` (`v0.2.0`, `0.2.0-alpha.2`) is a newer version than this build.
pub fn newer(tag: &str, current: &str) -> bool {
    let parse = |s: &str| Version::parse(s.trim_start_matches('v'));
    matches!((parse(tag), parse(current)), (Ok(t), Ok(c)) if t > c)
}

/// The latest release, if newer than this build.
fn latest(install: &Install) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let v: serde_json::Value = agent(Some(Duration::from_secs(10)), None)
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut r| r.body_mut().read_json())
        .map_err(|e| format!("{url}: {e}"))?;
    let tag = v["tag_name"].as_str().unwrap_or_default();
    if !newer(tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            pred(name).then(|| {
                (
                    name.to_string(),
                    a["browser_download_url"].as_str().unwrap_or_default().to_string(),
                    a["size"].as_u64().unwrap_or(0),
                )
            })
        })
    };
    let asset = install
        .asset_suffix()
        .and_then(|suffix| find(&|n: &str| n.ends_with(suffix)));
    let sums = find(&|n: &str| n == SUMS).map(|(_, url, _)| url);
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        asset,
        sums,
    }))
}

fn poll(mut update: ResMut<Update>, mut actions: MessageReader<crate::ui::UiAction>, mut exit: MessageWriter<AppExit>) {
    for crate::ui::UiAction(a) in actions.read() {
        match a {
            crate::ui::Action::Update => update.install(),
            crate::ui::Action::ReleasePage => {
                let page = update.release.as_ref().map_or_else(
                    || format!("https://github.com/{REPO}/releases/latest"),
                    |r| r.page.clone(),
                );
                open_url(&page);
            }
            _ => {}
        }
    }
    let u = &mut *update;
    if let Some(rx) = &u.checking {
        let got = rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match got {
            Err(TryRecvError::Empty) => {}
            Ok(r) => {
                u.checking = None;
                if let Some(r) = r {
                    info!("update: {} is out", r.version);
                    u.release = Some(r.clone());
                    u.state = State::Available(r);
                }
            }
            Err(TryRecvError::Disconnected) => u.checking = None,
        }
    }
    if let Some(rx) = &u.working {
        let got = rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match got {
            Err(TryRecvError::Empty) => u.state = State::Downloading(u.got.load(Ordering::Relaxed), u.total),
            Ok(Ok(())) => {
                u.working = None;
                u.state = State::Restarting;
                exit.write(AppExit::Success);
            }
            Ok(Err(e)) => {
                warn!("update: {e}");
                u.working = None;
                u.state = State::Failed(e);
            }
            Err(TryRecvError::Disconnected) => {
                u.working = None;
                u.state = State::Failed("the update thread is gone".into());
            }
        }
    }
}

impl Update {
    /// The release found can be installed here (again, after a failed try).
    pub fn can_install(&self) -> bool {
        self.install.is_some() && self.release.as_ref().is_some_and(Release::installable)
    }

    /// Downloads and installs the release found; the game restarts into it (`State::Restarting`: quit now).
    pub fn install(&mut self) {
        // (Two clicks before the next frame would start two downloads into one file.)
        if self.working.is_some() {
            return;
        }
        let (Some(install), Some(release)) = (self.install.clone(), self.release.clone()) else {
            return;
        };
        let (Some((name, url, size)), Some(sums)) = (release.asset, release.sums) else {
            return;
        };
        let (tx, rx) = channel();
        self.got.store(0, Ordering::Relaxed);
        self.total = size;
        let got = self.got.clone();
        spawn_named("update", move || {
            let _ = tx.send(fetch_and_install(&install, &name, &url, size, &sums, &got));
        });
        self.working = Some(Mutex::new(rx));
        self.state = State::Downloading(0, size);
    }
}

/// The SHA-256 the checksums file gives `name` (`sha256sum` format).
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

fn fetch_and_install(
    install: &Install,
    name: &str,
    url: &str,
    size: u64,
    sums_url: &str,
    got: &AtomicU64,
) -> Result<(), String> {
    let sums = agent(Some(Duration::from_secs(30)), None)
        .get(sums_url)
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("{sums_url}: {e}"))?;
    let want = expected_sum(&sums, name).ok_or_else(|| format!("{name} is not in {SUMS}"))?;
    let path = match install {
        Install::AppImage(p) => {
            let mut n = p.as_os_str().to_owned();
            n.push(".new");
            PathBuf::from(n)
        }
        // (The release's name for the file, never a path of its own.)
        _ => std::path::Path::new(name)
            .file_name()
            .map(|n| std::env::temp_dir().join(n))
            .ok_or_else(|| format!("{name}: not a file name"))?,
    };
    let mut resp = agent(None, Some(download_budget(size)))
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let mut file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let fetched = (|| {
        let mut body = resp.body_mut().as_reader();
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| format!("{}: {e}", path.display()))?;
            total += n as u64;
            got.store(total, Ordering::Relaxed);
        }
        file.sync_all().map_err(|e| format!("{}: {e}", path.display()))
    })();
    drop(file);
    let have: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    // A broken or wrong download leaves nothing behind (an AppImage's `.new` would sit next to the game).
    let checked = fetched.and_then(|()| {
        (total == size && have == want)
            .then_some(())
            .ok_or_else(|| format!("{name}: the download does not match the release"))
    });
    if let Err(e) = checked {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    match install {
        Install::Nsis => {
            // The installer waits for this process to end, installs quietly and starts the game again, with
            // this run's arguments (`--profile`: the same player).
            let mut cmd = std::process::Command::new(&path);
            cmd.args(["/S", "/UPDATE"]);
            let args: Vec<String> = std::env::args_os()
                .skip(1)
                .filter_map(|a| a.into_string().ok())
                .collect();
            if let Some(restart) = restart_args(&args) {
                // (As it is: NSIS reads the backticks itself, Rust's quoting would hide them.)
                #[cfg(windows)]
                std::os::windows::process::CommandExt::raw_arg(&mut cmd, restart);
                #[cfg(not(windows))]
                cmd.arg(restart);
            }
            cmd.spawn().map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Install::AppImage(target) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            }
            std::fs::rename(&path, target).map_err(|e| format!("{}: {e}", target.display()))?;
            std::process::Command::new(target)
                .args(std::env::args_os().skip(1))
                .spawn()
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
        Install::Flatpak => return Err("a Flatpak updates from its file".into()),
    }
    Ok(())
}

/// The installer's `/ARGS=` option: the command line it starts the updated game with, in backticks (none
/// inside them: an argument with one is left out). None without arguments.
fn restart_args(args: &[String]) -> Option<String> {
    let line: Vec<String> = args.iter().filter(|a| !a.contains('`')).map(|a| quote(a)).collect();
    (!line.is_empty()).then(|| format!("/ARGS=`{}`", line.join(" ")))
}

/// One argument of a Windows command line, quoted as `CommandLineToArgvW` reads it back.
fn quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_string();
    }
    let mut s = String::from('"');
    let mut slashes = 0;
    for c in a.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, and the quote escaped.
                s.extend(core::iter::repeat_n('\\', slashes * 2 + 1));
                slashes = 0;
                s.push('"');
            }
            _ => {
                s.extend(core::iter::repeat_n('\\', slashes));
                slashes = 0;
                s.push(c);
            }
        }
    }
    // (Before the closing quote: doubled.)
    s.extend(core::iter::repeat_n('\\', slashes * 2));
    s.push('"');
    s
}

/// Opens a page in the system's browser (web pages only: `explorer` and `xdg-open` open files too).
pub fn open_url(url: &str) {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        warn!("not a web page: {url}");
        return;
    }
    let r = if cfg!(windows) {
        std::process::Command::new("explorer").arg(url).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = r {
        warn!("{url}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.1.0", "0.1.0-alpha"));
        assert!(newer("v0.1.0-alpha.2", "0.1.0-alpha"));
        assert!(newer("0.2.0-alpha", "0.1.0"));
        assert!(!newer("v0.1.0-alpha", "0.1.0-alpha"));
        assert!(!newer("v0.0.9", "0.1.0-alpha"));
        assert!(!newer("nightly", "0.1.0-alpha"));
    }

    #[test]
    fn a_slow_download_still_fits() {
        // 100 MB at 32 KB/s: under an hour, and a stall ends.
        let b = download_budget(100 << 20);
        assert!(
            b >= Duration::from_secs(3200) && b <= Duration::from_secs(3600),
            "{b:?}"
        );
    }

    #[test]
    fn sums() {
        let s = "abc123  FallBeans-0.2.0-setup.exe\nDEF456 *FallBeans-0.2.0-x86_64.AppImage\n";
        assert_eq!(expected_sum(s, "FallBeans-0.2.0-setup.exe").as_deref(), Some("abc123"));
        assert_eq!(
            expected_sum(s, "FallBeans-0.2.0-x86_64.AppImage").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_sum(s, "other"), None);
    }

    #[test]
    fn installers() {
        assert!(is_installer("FallBeans-0.2.0-setup.exe"));
        assert!(!is_installer("FallBeans-0.2.0-x86_64.AppImage"));
        assert!(!is_installer("other-setup.exe"));
    }

    #[test]
    fn the_restart_keeps_the_arguments() {
        let a = |v: &[&str]| restart_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&[]), None);
        assert_eq!(a(&["--profile", "b"]).as_deref(), Some("/ARGS=`--profile b`"));
        assert_eq!(
            a(&["--name", "Боб Ёж", "--x", "a`b"]).as_deref(),
            Some("/ARGS=`--name \"Боб Ёж\" --x`")
        );
        assert_eq!(quote(r#"C:\a b\"#), r#""C:\a b\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(""), "\"\"");
    }
}
