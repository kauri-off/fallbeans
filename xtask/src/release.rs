//! `cargo xtask release`: a release built on the authors' machines. `prepare` opens a draft release of this commit,
//! `build` makes this OS's packages and uploads them to the draft (Linux clients in an Ubuntu 22.04 container: the
//! glibc they are built against is the oldest they run on), `publish` adds SHA256SUMS once every package is there and
//! publishes it, which makes the tag. The self-updater reads the published release (`fb_client/src/update.rs`).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use clap::{Args, Subcommand};
use serde_json::Value;

use crate::dist::{Kind, package_version, version};
use crate::{root, run, sdk, target_dir};

#[derive(Args)]
pub struct ReleaseArgs {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Checks master, the version and the tag, and opens the draft release of this commit.
    Prepare(Opts),
    /// This OS's packages (or the ones named) into the draft: Windows nsis; Linux appimage, flatpak, deb, rpm.
    Build {
        kinds: Vec<Kind>,
        /// AppImage and Flatpak built on this machine instead of the container (they then need its glibc).
        #[arg(long)]
        native: bool,
        #[command(flatten)]
        o: Opts,
    },
    /// Every package in the draft: adds SHA256SUMS and publishes the release.
    Publish(Opts),
}

#[derive(Args)]
struct Opts {
    /// Prints what would change on GitHub instead of changing it.
    #[arg(long)]
    dry_run: bool,
}

pub fn release(a: &ReleaseArgs) -> bool {
    let ok = match &a.cmd {
        Cmd::Prepare(o) => prepare(o),
        Cmd::Build { kinds, native, o } => build(kinds, *native, o),
        Cmd::Publish(o) => publish(o),
    };
    ok.unwrap_or_else(|e| {
        eprintln!("release: {e}");
        false
    })
}

type Step = Result<bool, String>;

fn tag() -> String {
    format!("v{}", version())
}

fn output(cmd: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(cmd)
        .current_dir(root())
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

fn git(args: &[&str]) -> Result<String, String> {
    output("git", args)
}

fn head() -> Result<String, String> {
    git(&["rev-parse", "HEAD"])
}

fn clean() -> Result<(), String> {
    let status = git(&["status", "--porcelain"])?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the working tree has changes: commit or stash them first\n{status}"
        ))
    }
}

/// A change on GitHub: run, or only shown with `--dry-run`.
fn gh_change(o: &Opts, args: &[&str]) -> bool {
    if o.dry_run {
        eprintln!("(dry run) gh {}", args.join(" "));
        return true;
    }
    run(Command::new("gh").current_dir(root()).args(args))
}

struct Draft {
    draft: bool,
    target: String,
    assets: Vec<String>,
}

/// The release of this version, if there is one (a draft too).
fn release_of(tag: &str) -> Result<Option<Draft>, String> {
    match output(
        "gh",
        &["release", "view", tag, "--json", "isDraft,targetCommitish,assets"],
    ) {
        Ok(json) => {
            let v: Value = serde_json::from_str(&json).map_err(|e| format!("gh release view: {e}"))?;
            Ok(Some(Draft {
                draft: v["isDraft"].as_bool().unwrap_or(false),
                target: v["targetCommitish"].as_str().unwrap_or_default().to_string(),
                assets: v["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a["name"].as_str().map(String::from))
                    .collect(),
            }))
        }
        Err(e) if e.contains("not found") => Ok(None),
        Err(e) => Err(format!("gh release view {tag}: {e}")),
    }
}

/// The draft of this version, opened by `prepare`.
fn draft() -> Result<Draft, String> {
    match release_of(&tag())? {
        Some(d) if d.draft => Ok(d),
        Some(_) => Err(format!(
            "{} is published already: raise the version in Cargo.toml",
            tag()
        )),
        None => Err(format!("no draft of {}: cargo xtask release prepare", tag())),
    }
}

fn prepare(o: &Opts) -> Step {
    let tag = tag();
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch != "master" {
        return Err(format!("releases are made from master only (this is {branch})"));
    }
    clean()?;
    git(&["fetch", "--quiet", "origin", "master"])?;
    let (head, pushed) = (head()?, git(&["rev-parse", "origin/master"])?);
    if head != pushed {
        return Err(format!(
            "master is not origin/master ({head} against {pushed}): push or pull first"
        ));
    }
    match release_of(&tag)? {
        Some(r) if !r.draft => return Err(format!("{tag} is released already: raise the version in Cargo.toml")),
        Some(r) if r.target == head => {
            eprintln!("{tag}: the draft of this commit is there already");
            return Ok(true);
        }
        Some(r) => {
            return Err(format!(
                "the draft of {tag} is of {}: `gh release delete {tag}` to make it again of this commit",
                r.target
            ));
        }
        None => {}
    }
    // A tag left by an earlier release of this version must be on this commit (an annotated one's is `^{}`).
    let remote = git(&["ls-remote", "--tags", "origin", &format!("refs/tags/{tag}*")])?;
    let tagged = remote
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(_, r)| r.ends_with("^{}"))
        .or_else(|| remote.lines().filter_map(|l| l.split_once('\t')).next())
        .map(|(sha, _)| sha.to_string());
    if let Some(sha) = tagged
        && sha != head
    {
        return Err(format!(
            "tag {tag} is on {sha}, not on this commit: delete the tag or raise the version"
        ));
    }
    let notes = fs::read_to_string(root().join("packaging/release-notes.md"))
        .map_err(|e| format!("packaging/release-notes.md: {e}"))?
        .replace("@VERSION@", version());
    let notes_file = target_dir().join(format!("release-notes-{tag}.md"));
    fs::create_dir_all(target_dir()).map_err(|e| e.to_string())?;
    fs::write(&notes_file, notes).map_err(|e| e.to_string())?;
    let notes_arg = notes_file.to_string_lossy().into_owned();
    Ok(gh_change(
        o,
        &[
            "release",
            "create",
            &tag,
            "--draft",
            "--target",
            &head,
            "--title",
            &title(version()),
            "--notes-file",
            &notes_arg,
        ],
    ))
}

/// "Fall Beans 0.2.0 ALPHA.1" for 0.2.0-alpha.1.
fn title(version: &str) -> String {
    match version.split_once('-') {
        Some((v, pre)) => format!("Fall Beans {v} {}", pre.to_uppercase()),
        None => format!("Fall Beans {version}"),
    }
}

/// The packages this OS makes.
fn this_os_kinds() -> Vec<Kind> {
    if cfg!(windows) {
        vec![Kind::Nsis]
    } else {
        vec![Kind::Appimage, Kind::Flatpak, Kind::Deb, Kind::Rpm]
    }
}

/// The release's file of `kind`, as uploaded (`~` of the server packages' version is `.`: GitHub renames it).
fn is_asset(kind: Kind, name: &str) -> bool {
    let (v, server) = (version(), package_version().replace('~', "."));
    match kind {
        Kind::Nsis => name == format!("FallBeans-{v}-setup.exe"),
        Kind::Appimage => name == format!("FallBeans-{v}-x86_64.AppImage"),
        Kind::Flatpak => name == format!("FallBeans-{v}-x86_64.flatpak"),
        Kind::Deb => name.starts_with("fallbeans-server_") && name.contains(&server) && name.ends_with(".deb"),
        Kind::Rpm => name.starts_with("fallbeans-server-") && name.contains(&server) && name.ends_with(".rpm"),
    }
}

const ALL: [Kind; 5] = [Kind::Nsis, Kind::Appimage, Kind::Flatpak, Kind::Deb, Kind::Rpm];

fn build(kinds: &[Kind], native: bool, o: &Opts) -> Step {
    clean()?;
    let head = head()?;
    let d = draft()?;
    if d.target != head {
        return Err(format!(
            "the draft of {} is of {}, this is {head}: check that commit out",
            tag(),
            d.target
        ));
    }
    let kinds = if kinds.is_empty() {
        this_os_kinds()
    } else {
        kinds.to_vec()
    };
    let started = SystemTime::now();
    let (boxed, here): (Vec<Kind>, Vec<Kind>) = kinds
        .iter()
        .partition(|k| !cfg!(windows) && !native && matches!(k, Kind::Appimage | Kind::Flatpak));
    if !boxed.is_empty() && !in_container(&boxed) {
        return Err("the container build failed".into());
    }
    for k in &here {
        if !crate::dist::build(*k) {
            return Err(format!("{k:?} did not build"));
        }
    }
    let mut files = Vec::new();
    for k in &kinds {
        files.push(made(*k, started)?);
    }
    // (Again: the tree may have changed while this built, the draft may have been made anew of another commit.)
    clean().map_err(|e| format!("not uploaded, the packages may hold what changed: {e}"))?;
    if self::head()? != head {
        return Err("HEAD moved while this built: not uploaded".into());
    }
    let now = draft()?;
    if now.target != head {
        return Err(format!(
            "the draft of {} is of {} now, these packages are of {head}: not uploaded",
            tag(),
            now.target
        ));
    }
    let names: Vec<String> = files.iter().map(|f| f.to_string_lossy().into_owned()).collect();
    let tag = tag();
    let mut args = vec!["release", "upload", tag.as_str(), "--clobber"];
    args.extend(names.iter().map(String::as_str));
    Ok(gh_change(o, &args))
}

/// The file `kind` made in dist/ since `since`, with `~` renamed to `.` as GitHub would.
fn made(kind: Kind, since: SystemTime) -> Result<PathBuf, String> {
    let dist = root().join("dist");
    let found = fs::read_dir(&dist)
        .map_err(|e| format!("{}: {e}", dist.display()))?
        .flatten()
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since))
        .map(|e| e.path())
        .find(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().replace('~', "."))
                .unwrap_or_default();
            is_asset(kind, &name)
        })
        .ok_or_else(|| format!("{kind:?} left no file in dist/"))?;
    let name = found
        .file_name()
        .map(|n| n.to_string_lossy().replace('~', "."))
        .unwrap_or_default();
    let renamed = dist.join(&name);
    if renamed != found {
        fs::rename(&found, &renamed).map_err(|e| format!("{}: {e}", found.display()))?;
    }
    Ok(renamed)
}

const IMAGE: &str = "fallbeans-linux-build";

/// `cargo xtask dist` of `kinds` in the Ubuntu 22.04 container (packaging/linux-build), the repository at /src:
/// its own target dir (target/linux-build), this machine's SDKs (their paths moved under /src).
pub fn in_container(kinds: &[Kind]) -> bool {
    let image = root().join("packaging/linux-build");
    let built = run(Command::new("docker")
        .args(["build", "-q", "-t", IMAGE, "-f"])
        .arg(image.join("Containerfile"))
        .arg(&image));
    if !built {
        return false;
    }
    let uid = output("id", &["-u"]).unwrap_or_else(|_| "1000".into());
    let gid = output("id", &["-g"]).unwrap_or_else(|_| "1000".into());
    let work = "/src/target/linux-build";
    let mut c = Command::new("docker");
    // (`--privileged`: flatpak-builder's bubblewrap makes namespaces of its own.)
    c.args(["run", "--rm", "--privileged", "--user", &format!("{uid}:{gid}")])
        .arg("-v")
        .arg(format!("{}:/src", root().display()))
        .args(["-w", "/src"])
        .args(["-e", &format!("HOME={work}/home")])
        .args(["-e", &format!("RUSTUP_HOME={work}/rustup")])
        .args(["-e", &format!("CARGO_HOME={work}/cargo")])
        .args(["-e", "CARGO_TARGET_DIR=target/linux-build"]);
    for (var, path) in sdk::exported() {
        match inside(&path) {
            Some(p) => {
                c.args(["-e", &format!("{var}={p}")]);
            }
            None => eprintln!(
                "{var}={}: outside the repository, the container does without it",
                path.display()
            ),
        }
    }
    let dists: Vec<String> = kinds
        .iter()
        .map(|k| format!("cargo xtask dist {}", format!("{k:?}").to_lowercase()))
        .collect();
    let script = format!("cargo xtask setup --dist && {}", dists.join(" && "));
    c.args([IMAGE, "sh", "-c", &script]);
    run(&mut c)
}

/// `path` as the container sees it (the repository is /src there).
fn inside(path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root()).ok()?;
    Some(format!("/src/{}", rel.to_string_lossy()))
}

fn publish(o: &Opts) -> Step {
    let tag = tag();
    let d = draft()?;
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for k in ALL {
        match d.assets.iter().find(|a| is_asset(k, a)) {
            Some(a) => files.push(a.clone()),
            None => missing.push(format!("{k:?}")),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "the draft of {tag} has no {}: cargo xtask release build on the machine that makes it",
            missing.join(", ")
        ));
    }
    let dir = target_dir().join("release").join(&tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dir_arg = dir.to_string_lossy().into_owned();
    let mut download = vec!["release", "download", tag.as_str(), "--dir", dir_arg.as_str()];
    for f in &files {
        download.extend(["--pattern", f.as_str()]);
    }
    if !run(Command::new("gh").current_dir(root()).args(&download)) {
        return Err("the packages did not download".into());
    }
    files.sort();
    let mut sums = String::new();
    for f in &files {
        let sum = sdk::sha256_of(&dir.join(f)).ok_or_else(|| format!("{f}: unreadable"))?;
        sums += &format!("{sum}  {f}\n");
    }
    print!("{sums}");
    let sums_file = dir.join("SHA256SUMS");
    fs::write(&sums_file, &sums).map_err(|e| e.to_string())?;
    let sums_arg = sums_file.to_string_lossy().into_owned();
    Ok(gh_change(o, &["release", "upload", &tag, "--clobber", &sums_arg])
        && gh_change(o, &["release", "edit", &tag, "--draft=false", "--latest"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_assets() {
        assert_eq!(title("0.2.0-alpha.1"), "Fall Beans 0.2.0 ALPHA.1");
        assert_eq!(title("1.0.0"), "Fall Beans 1.0.0");
        let v = version();
        let server = package_version().replace('~', ".");
        assert!(is_asset(Kind::Nsis, &format!("FallBeans-{v}-setup.exe")));
        assert!(is_asset(Kind::Appimage, &format!("FallBeans-{v}-x86_64.AppImage")));
        assert!(!is_asset(Kind::Appimage, "FallBeans-0.0.1-x86_64.AppImage"));
        assert!(is_asset(Kind::Deb, &format!("fallbeans-server_{server}_amd64.deb")));
        assert!(is_asset(Kind::Rpm, &format!("fallbeans-server-{server}-1.x86_64.rpm")));
        assert!(!is_asset(Kind::Rpm, &format!("fallbeans-server_{server}_amd64.deb")));
    }
}
