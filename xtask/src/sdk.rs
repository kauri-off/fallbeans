//! Pinned SDKs and tools (`toolchain/deps.toml`): `cargo xtask setup` puts them in target/sdk, and xtask hands
//! their paths to what it runs unless the environment names its own.
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, ensure};
use clap::Args;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{report, reported, root, run, target_dir};

const MANIFEST: &str = include_str!("../../toolchain/deps.toml");

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Os {
    Windows,
    Linux,
}

impl Os {
    fn this() -> Os {
        if cfg!(windows) { Os::Windows } else { Os::Linux }
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Group {
    /// Plain `setup`.
    Upscalers,
    /// `setup --dist`.
    Dist,
}

/// Where an entry comes from and how it is installed.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Source {
    Git {
        repo: String,
        tag: String,
        commit: String,
        #[serde(default)]
        paths: Vec<String>,
    },
    Zip {
        url: String,
        sha256: String,
        #[serde(default)]
        extract: Vec<String>,
    },
    File {
        url: String,
        sha256: String,
        name: String,
    },
    Cargo {
        #[serde(default)]
        features: Vec<String>,
    },
}

#[derive(Deserialize)]
pub struct Dep {
    os: Option<Os>,
    group: Group,
    pub env: Option<String>,
    pub version: String,
    points_to: Option<String>,
    file_sha256: Option<String>,
    #[serde(flatten)]
    source: Source,
}

pub fn this_os() -> &'static str {
    match Os::this() {
        Os::Windows => "windows",
        Os::Linux => "linux",
    }
}

/// This OS's entries, by name.
pub fn deps() -> &'static BTreeMap<String, Dep> {
    static DEPS: OnceLock<BTreeMap<String, Dep>> = OnceLock::new();
    DEPS.get_or_init(|| {
        let all: BTreeMap<String, Dep> = toml::from_str(MANIFEST).expect("toolchain/deps.toml");
        all.into_iter()
            .filter(|(_, d)| d.os.is_none_or(|os| os == Os::this()))
            .collect()
    })
}

pub fn dep(name: &str) -> Option<&'static Dep> {
    deps().get(name)
}

impl Dep {
    fn dir(&self, name: &str) -> PathBuf {
        sdk_dir().join(format!("{name}-{}", self.version))
    }

    /// Its directory in target/sdk once `setup` has finished it (CI's rust-cache can leave it emptied).
    pub fn installed(&self, name: &str) -> Option<PathBuf> {
        Some(self.dir(name)).filter(|d| has_file(d) && self.target(d).exists())
    }

    /// What `env` names: a file inside the entry, or the entry's directory.
    fn target(&self, dir: &Path) -> PathBuf {
        let file = match &self.source {
            Source::File { name, .. } => Some(name),
            _ => None,
        };
        match self.points_to.as_ref().or(file) {
            Some(p) => dir.join(p),
            None => dir.to_path_buf(),
        }
    }

    pub fn is_dist(&self) -> bool {
        self.group == Group::Dist
    }

    fn install(&self, name: &str, to: &Path) -> Result<()> {
        match &self.source {
            Source::Git {
                repo,
                tag,
                commit,
                paths,
            } => git(repo, tag, commit, paths, to),
            Source::Zip { url, sha256, extract } => zip(name, &self.version, url, sha256, extract, to),
            Source::File { url, sha256, name } => file(url, sha256, &to.join(name)),
            Source::Cargo { features } => cargo_install(name, &self.version, features, to),
        }?;
        match &self.file_sha256 {
            Some(sum) => checked(&self.target(to), sum),
            None => Ok(()),
        }
    }
}

fn has_file(dir: &Path) -> bool {
    fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        e.file_type()
            .is_ok_and(|t| t.is_file() || (t.is_dir() && has_file(&e.path())))
    })
}

fn sdk_dir() -> PathBuf {
    target_dir().join("sdk")
}

/// `var`'s path: the environment's, else the one of the entry in target/sdk that `var` names.
pub fn var(var: &str) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(var) {
        return Some(v.into());
    }
    deps()
        .iter()
        .filter(|(_, d)| d.env.as_deref() == Some(var))
        .find_map(|(n, d)| d.installed(n).map(|dir| d.target(&dir)))
}

/// What target/sdk has for a child process: the variables the environment does not set, the cargo tools on PATH,
/// and LLVM's libclang where its Windows installer puts it.
pub fn apply(c: &mut Command) {
    let mut bins = Vec::new();
    for (n, d) in deps() {
        let Some(dir) = d.installed(n) else {
            continue;
        };
        if let Some(v) = &d.env
            && std::env::var_os(v).is_none()
        {
            c.env(v, d.target(&dir));
        }
        if matches!(d.source, Source::Cargo { .. }) {
            bins.push(dir.join("bin"));
        }
    }
    if !bins.is_empty() {
        bins.extend(std::env::var_os("PATH").iter().flat_map(std::env::split_paths));
        c.env("PATH", std::env::join_paths(bins).unwrap_or_default());
    }
    if let Some(dir) = libclang_default() {
        c.env("LIBCLANG_PATH", dir);
    }
}

/// `C:\Program Files\LLVM\bin` when it has libclang and `LIBCLANG_PATH` is not set.
pub fn libclang_default() -> Option<PathBuf> {
    let dir = PathBuf::from(r"C:\Program Files\LLVM\bin");
    (cfg!(windows) && std::env::var_os("LIBCLANG_PATH").is_none() && dir.join("libclang.dll").is_file()).then_some(dir)
}

#[derive(Args)]
pub struct SetupArgs {
    /// The packaging tools of this OS too (`cargo xtask dist`).
    #[arg(long)]
    dist: bool,
    /// Again, over what target/sdk has.
    #[arg(long)]
    force: bool,
}

pub fn setup(a: &SetupArgs) -> Result<()> {
    let mut ok = true;
    for (name, d) in deps().iter().filter(|(_, d)| a.dist || !d.is_dist()) {
        let dir = d.dir(name);
        if d.installed(name).is_some() && !a.force {
            eprintln!("{name} {}: there", d.version);
            continue;
        }
        if let Some(v) = &d.env
            && std::env::var_os(v).is_some()
        {
            eprintln!("{name}: {v} is set, that one is used (unset it to use target/sdk)");
            continue;
        }
        eprintln!("{name} {}: installing", d.version);
        let part = sdk_dir().join(format!("{name}-{}.part", d.version));
        let _ = fs::remove_dir_all(&part);
        fs::create_dir_all(&part).with_context(|| part.display().to_string())?;
        let done = d.install(name, &part).and_then(|()| {
            let _ = fs::remove_dir_all(&dir);
            fs::rename(&part, &dir).with_context(|| format!("{} → {}", part.display(), dir.display()))
        });
        if report(done) {
            eprintln!("{name} {}: ok", d.version);
        } else {
            eprintln!("{name} {}: FAILED", d.version);
            ok = false;
        }
    }
    if a.dist && !cfg!(windows) {
        ok &= report(run(Command::new("rustup").current_dir(root()).args([
            "target",
            "add",
            "x86_64-unknown-linux-musl",
        ])));
    }
    reported(ok)
}

fn git(repo: &str, tag: &str, commit: &str, paths: &[String], to: &Path) -> Result<()> {
    let git = |args: &[&str]| run(Command::new("git").arg("-C").arg(to).args(args));
    run(Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--filter=blob:none",
            "--no-checkout",
            "--depth",
            "1",
            "--branch",
            tag,
            repo,
        ])
        .arg(to))?;
    if !paths.is_empty() {
        let mut args = vec!["sparse-checkout", "set", "--no-cone"];
        args.extend(paths.iter().map(String::as_str));
        git(&args)?;
    }
    git(&["checkout", "--quiet"])?;
    let head = Command::new("git")
        .arg("-C")
        .arg(to)
        .args(["rev-parse", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    ensure!(head == commit, "{tag} is at {head}, not at the pinned {commit}");
    Ok(())
}

fn download(url: &str, to: &Path, sha256: &str) -> Result<()> {
    run(Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(to)
        .arg(url))?;
    checked(to, sha256)
}

fn checked(path: &Path, want: &str) -> Result<()> {
    let got = sha256_of(path).unwrap_or_default();
    ensure!(got == want, "{}: sha256 {got}, expected {want}", path.display());
    Ok(())
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn zip(name: &str, version: &str, url: &str, sum: &str, extract: &[String], to: &Path) -> Result<()> {
    let archive = sdk_dir().join(format!("{name}-{version}.zip"));
    download(url, &archive, sum)?;
    // Windows has bsdtar as tar (it reads zip); Linux has unzip (1: warnings only, e.g. backslashes).
    let unpacked = if cfg!(windows) {
        let mut c = Command::new("tar");
        c.arg("-xf").arg(&archive).arg("-C").arg(to);
        c.args(extract.iter().map(|e| e.trim_end_matches('/')));
        run(&mut c)
    } else {
        let mut c = Command::new("unzip");
        c.arg("-q").arg(&archive);
        c.args(
            extract
                .iter()
                .map(|e| if e.ends_with('/') { format!("{e}*") } else { e.clone() }),
        );
        c.arg("-d").arg(to);
        eprintln!("$ {c:?}");
        match c.status() {
            Ok(s) => reported(matches!(s.code(), Some(0 | 1))),
            Err(e) => Err(anyhow!("cannot start unzip: {e}")),
        }
    };
    let _ = fs::remove_file(&archive);
    unpacked
}

fn file(url: &str, sum: &str, path: &Path) -> Result<()> {
    download(url, path, sum)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).with_context(|| path.display().to_string())?;
    }
    Ok(())
}

fn cargo_install(name: &str, version: &str, features: &[String], to: &Path) -> Result<()> {
    let mut c = crate::cargo();
    c.args(["install", "--locked", "--quiet", name, "--version", version, "--root"])
        .arg(to);
    if !features.is_empty() {
        c.args(["--features", &features.join(",")]);
    }
    run(&mut c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads() {
        toml::from_str::<BTreeMap<String, Dep>>(MANIFEST).unwrap();
    }
}
