//! Pinned SDKs and tools (`toolchain/deps.toml`): `cargo xtask setup` puts them in target/sdk, and xtask hands
//! their paths to what it runs unless the environment names its own.
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use clap::Args;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{root, run, target_dir};

const MANIFEST: &str = include_str!("../../toolchain/deps.toml");

#[derive(Deserialize)]
pub struct Dep {
    os: Option<String>,
    group: String,
    pub env: Option<String>,
    pub version: String,
    kind: String,
    repo: Option<String>,
    tag: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    url: Option<String>,
    sha256: Option<String>,
    #[serde(default)]
    extract: Vec<String>,
    name: Option<String>,
    points_to: Option<String>,
    file_sha256: Option<String>,
    #[serde(default)]
    features: Vec<String>,
}

pub fn this_os() -> &'static str {
    if cfg!(windows) { "windows" } else { "linux" }
}

/// This OS's entries, by name.
pub fn deps() -> &'static BTreeMap<String, Dep> {
    static DEPS: OnceLock<BTreeMap<String, Dep>> = OnceLock::new();
    DEPS.get_or_init(|| {
        let all: BTreeMap<String, Dep> = toml::from_str(MANIFEST).expect("toolchain/deps.toml");
        all.into_iter()
            .filter(|(_, d)| d.os.as_deref().is_none_or(|os| os == this_os()))
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
        match self.points_to.as_ref().or(self.name.as_ref()) {
            Some(p) => dir.join(p),
            None => dir.to_path_buf(),
        }
    }

    pub fn is_dist(&self) -> bool {
        self.group == "dist"
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
        if d.kind == "cargo" {
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

pub fn setup(a: &SetupArgs) -> bool {
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
        if let Err(e) = fs::create_dir_all(&part) {
            eprintln!("{}: {e}", part.display());
            return false;
        }
        let done = match d.kind.as_str() {
            "git" => git(d, &part),
            "zip" => zip(name, d, &part),
            "file" => file(d, &part),
            "cargo" => cargo_install(name, d, &part),
            k => {
                eprintln!("{name}: unknown kind {k}");
                false
            }
        } && d.file_sha256.as_ref().is_none_or(|sum| checked(&d.target(&part), sum));
        if done {
            let _ = fs::remove_dir_all(&dir);
        }
        if done && fs::rename(&part, &dir).is_ok() {
            eprintln!("{name} {}: ok", d.version);
        } else {
            eprintln!("{name} {}: FAILED", d.version);
            ok = false;
        }
    }
    if a.dist && !cfg!(windows) {
        ok &= run(Command::new("rustup")
            .current_dir(root())
            .args(["target", "add", "x86_64-unknown-linux-musl"]));
    }
    ok
}

fn git(d: &Dep, to: &Path) -> bool {
    let (Some(repo), Some(tag)) = (&d.repo, &d.tag) else {
        return false;
    };
    let git = |args: &[&str]| run(Command::new("git").arg("-C").arg(to).args(args));
    let cloned = run(Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--filter=blob:none",
            "--no-checkout",
            "--depth",
            "1",
            "--branch",
        ])
        .args([tag, repo])
        .arg(to));
    let sparse = d.paths.is_empty() || {
        let mut args = vec!["sparse-checkout", "set", "--no-cone"];
        args.extend(d.paths.iter().map(String::as_str));
        git(&args)
    };
    if !(cloned && sparse && git(&["checkout", "--quiet"])) {
        return false;
    }
    let Some(want) = &d.commit else {
        return true;
    };
    let head = Command::new("git")
        .arg("-C")
        .arg(to)
        .args(["rev-parse", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if &head != want {
        eprintln!("{tag} is at {head}, not at the pinned {want}");
    }
    &head == want
}

fn download(url: &str, to: &Path, sha256: &str) -> bool {
    run(Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(to)
        .arg(url))
        && checked(to, sha256)
}

fn checked(path: &Path, want: &str) -> bool {
    let got = sha256_of(path).unwrap_or_default();
    if got != want {
        eprintln!("{}: sha256 {got}, expected {want}", path.display());
    }
    got == want
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn zip(name: &str, d: &Dep, to: &Path) -> bool {
    let (Some(url), Some(sum)) = (&d.url, &d.sha256) else {
        return false;
    };
    let archive = sdk_dir().join(format!("{name}-{}.zip", d.version));
    if !download(url, &archive, sum) {
        return false;
    }
    // Windows has bsdtar as tar (it reads zip); Linux has unzip (1: warnings only, e.g. backslashes).
    let ok = if cfg!(windows) {
        let mut c = Command::new("tar");
        c.arg("-xf").arg(&archive).arg("-C").arg(to);
        c.args(d.extract.iter().map(|e| e.trim_end_matches('/')));
        run(&mut c)
    } else {
        let mut c = Command::new("unzip");
        c.arg("-q").arg(&archive);
        c.args(
            d.extract
                .iter()
                .map(|e| if e.ends_with('/') { format!("{e}*") } else { e.clone() }),
        );
        c.arg("-d").arg(to);
        eprintln!("$ {c:?}");
        c.status().is_ok_and(|s| matches!(s.code(), Some(0 | 1)))
    };
    let _ = fs::remove_file(&archive);
    ok
}

fn file(d: &Dep, to: &Path) -> bool {
    let (Some(url), Some(sum), Some(name)) = (&d.url, &d.sha256, &d.name) else {
        return false;
    };
    let path = to.join(name);
    if !download(url, &path, sum) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).is_err() {
            return false;
        }
    }
    true
}

fn cargo_install(name: &str, d: &Dep, to: &Path) -> bool {
    let mut c = crate::cargo();
    c.args([
        "install",
        "--locked",
        "--quiet",
        name,
        "--version",
        &d.version,
        "--root",
    ])
    .arg(to);
    if !d.features.is_empty() {
        c.args(["--features", &d.features.join(",")]);
    }
    run(&mut c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads_and_each_kind_has_its_fields() {
        let all: BTreeMap<String, Dep> = toml::from_str(MANIFEST).unwrap();
        for (n, d) in &all {
            let ok = match d.kind.as_str() {
                "git" => d.repo.is_some() && d.tag.is_some() && d.commit.is_some(),
                "zip" => d.url.is_some() && d.sha256.is_some(),
                "file" => d.url.is_some() && d.sha256.is_some() && d.name.is_some(),
                "cargo" => d.env.is_none(),
                _ => false,
            };
            assert!(ok, "{n}");
            assert!(matches!(d.group.as_str(), "upscalers" | "dist"), "{n}");
            assert!(d.os.as_deref().is_none_or(|os| os == "windows" || os == "linux"), "{n}");
        }
    }
}
