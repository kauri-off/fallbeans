//! `cargo xtask dist <kind>`: release packages into dist/ (the release workflow runs one kind per job).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, ValueEnum};

use crate::{cargo, root, run};

pub const APP_ID: &str = "io.github.kauri_off.fallbeans";
const MUSL: &str = "x86_64-unknown-linux-musl";
/// The cargo-about the release workflow installs (`release.yml`).
const CARGO_ABOUT: &str = "0.9.2";
const THIRD_PARTY: &str = "THIRD-PARTY-LICENSES.html";

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Windows installer (makensis).
    Nsis,
    /// Linux client (appimagetool, or APPIMAGETOOL=path).
    Appimage,
    /// Linux client bundle (flatpak-builder).
    Flatpak,
    /// Server package for Debian/Ubuntu (cargo-deb, musl target).
    Deb,
    /// Server package for Fedora/RHEL/openSUSE (cargo-generate-rpm, musl target).
    Rpm,
}

#[derive(Args)]
pub struct DistArgs {
    kind: Kind,
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The commit the build says it is (`fb_net::build`).
fn commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn stamped(mut c: Command) -> Command {
    c.env("FB_COMMIT", commit());
    c
}

fn out_dir() -> PathBuf {
    let d = root().join("dist");
    fs::create_dir_all(&d).expect("dist/");
    d
}

fn stage(name: &str) -> PathBuf {
    let d = root().join("target").join("pkg").join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("stage");
    d
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|e| panic!("{}: {e}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|e| panic!("{}: {e}", from.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            fs::copy(&path, &dest).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}

fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) {
    let (from, to) = (root().join(from), to.as_ref());
    fs::copy(&from, to).unwrap_or_else(|e| panic!("{} → {}: {e}", from.display(), to.display()));
}

/// The client's features in the packages: no BRP probe, the profiler of F4.
const CLIENT_FEATURES: [&str; 3] = ["--no-default-features", "--features", "profiler"];

/// The licenses of every crate built into `package` (cargo-about, `packaging/about.toml`) as `out`; fails on a
/// license that `about.toml` does not accept.
fn third_party(package: &str, features: &[&str], out: &Path) -> bool {
    let found = cargo()
        .args(["about", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("cargo-about is missing: cargo install --locked cargo-about --version {CARGO_ABOUT}");
        return false;
    }
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
    run(cargo()
        .args(["about", "generate", "--locked", "-c", "packaging/about.toml", "-m"])
        .arg(Path::new("crates").join(package).join("Cargo.toml"))
        .args(features)
        .arg("-o")
        .arg(out)
        .arg("packaging/about.hbs"))
}

/// DirectX's shader compiler for the Windows package (`backend.rs`: DX12 compiles with it instead of FXC), from
/// an unpacked DXC release in `FB_DXC_DIR` (release.yml downloads it); without it the game falls back to FXC.
fn dxc_into(dir: &Path) {
    if !cfg!(windows) {
        return;
    }
    let Some(src) = std::env::var_os("FB_DXC_DIR").map(PathBuf::from) else {
        eprintln!("FB_DXC_DIR is not set: the package compiles DX12 shaders with FXC (slow)");
        return;
    };
    for f in ["dxcompiler.dll", "dxil.dll"] {
        copy(src.join("bin").join("x64").join(f), dir.join(f));
    }
    let licenses = dir.join("dxc-licenses");
    fs::create_dir_all(&licenses).unwrap_or_else(|e| panic!("{}: {e}", licenses.display()));
    for f in ["LICENCE-MIT.txt", "LICENSE-LLVM.txt", "LICENSE-MS.txt"] {
        copy(src.join(f), licenses.join(f));
    }
}

/// The release client, staged with its assets and licenses into `dir`.
fn client_into(dir: &Path) -> bool {
    let mut c = stamped(cargo());
    c.args(["build", "--locked", "--profile", "dist", "-p", "fb_client"])
        .args(CLIENT_FEATURES);
    // The C runtime inside the exe: a clean Windows has no VCRUNTIME140.dll and the installer does not ship it.
    // (Here, not in .cargo/config.toml: there every local build would rebuild everything for it.)
    if cfg!(windows) {
        c.env(
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS",
            "-C target-feature=+crt-static",
        );
    }
    if !run(&mut c) || !third_party("fb_client", &CLIENT_FEATURES, &dir.join(THIRD_PARTY)) {
        return false;
    }
    let exe = format!("fb_client{}", std::env::consts::EXE_SUFFIX);
    copy(Path::new("target/dist").join(&exe), dir.join(&exe));
    copy_dir(&root().join("assets"), &dir.join("assets"));
    copy("LICENSE", dir.join("LICENSE"));
    dxc_into(dir);
    true
}

fn tool(var: &str, name: &str) -> Command {
    Command::new(std::env::var(var).unwrap_or_else(|_| name.into()))
}

fn nsis() -> bool {
    let s = stage("nsis");
    if !client_into(&s) {
        return false;
    }
    copy("packaging/icons/fallbeans.ico", s.join("fallbeans.ico"));
    let out = out_dir().join(format!("FallBeans-{}-setup.exe", version()));
    let default = if cfg!(windows) && Path::new(r"C:\Program Files (x86)\NSIS\makensis.exe").exists() {
        r"C:\Program Files (x86)\NSIS\makensis.exe"
    } else {
        "makensis"
    };
    run(tool("MAKENSIS", default)
        .arg(format!("-DVERSION={}", version()))
        .arg(format!(
            "-DVI_VERSION={}.0",
            version().split('-').next().unwrap_or("0.0.0")
        ))
        .arg(format!("-DSRC={}", s.display()))
        .arg(format!("-DOUT={}", out.display()))
        .arg(root().join("packaging/windows/installer.nsi")))
}

fn appimage() -> bool {
    let app = stage("AppDir");
    let bin = app.join("usr/bin");
    fs::create_dir_all(&bin).expect("AppDir");
    if !client_into(&bin) {
        return false;
    }
    let desktop = format!("{APP_ID}.desktop");
    copy(Path::new("packaging/linux").join(&desktop), app.join(&desktop));
    copy("packaging/icons/fallbeans-256.png", app.join(format!("{APP_ID}.png")));
    copy("packaging/icons/fallbeans-256.png", app.join(".DirIcon"));
    let apps = app.join("usr/share/applications");
    let icons = app.join("usr/share/icons/hicolor/256x256/apps");
    let meta = app.join("usr/share/metainfo");
    for d in [&apps, &icons, &meta] {
        fs::create_dir_all(d).expect("AppDir");
    }
    copy(Path::new("packaging/linux").join(&desktop), apps.join(&desktop));
    copy("packaging/icons/fallbeans-256.png", icons.join(format!("{APP_ID}.png")));
    let metainfo = format!("{APP_ID}.metainfo.xml");
    copy(
        Path::new("packaging/linux").join(&metainfo),
        meta.join(format!("{APP_ID}.appdata.xml")),
    );
    let apprun = app.join("AppRun");
    fs::write(
        &apprun,
        "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/fb_client\" \"$@\"\n",
    )
    .expect("AppRun");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&apprun, fs::Permissions::from_mode(0o755)).expect("AppRun");
    }
    let out = out_dir().join(format!("FallBeans-{}-x86_64.AppImage", version()));
    let mut c = tool("APPIMAGETOOL", "appimagetool");
    c.env("ARCH", "x86_64").env("APPIMAGE_EXTRACT_AND_RUN", "1");
    // A runtime of a known release (the workflow checks its sum); else appimagetool downloads the latest one.
    if let Ok(runtime) = std::env::var("APPIMAGE_RUNTIME") {
        c.arg("--runtime-file").arg(runtime);
    }
    run(c.arg(&app).arg(&out))
}

fn flatpak() -> bool {
    let dir = stage("flatpak");
    let s = dir.join("stage");
    fs::create_dir_all(&s).expect("stage");
    if !client_into(&s) {
        return false;
    }
    // (client_into staged LICENSE and THIRD_PARTY too: the manifest installs them.)
    for f in [format!("{APP_ID}.desktop"), format!("{APP_ID}.metainfo.xml")] {
        copy(Path::new("packaging/linux").join(&f), s.join(&f));
    }
    for f in ["fallbeans-256.png", "fallbeans-512.png"] {
        copy(Path::new("packaging/icons").join(f), s.join(f));
    }
    let manifest = dir.join(format!("{APP_ID}.yml"));
    copy(Path::new("packaging/flatpak").join(format!("{APP_ID}.yml")), &manifest);
    let repo = dir.join("repo");
    let out = out_dir().join(format!("FallBeans-{}-x86_64.flatpak", version()));
    run(Command::new("flatpak").args([
        "remote-add",
        "--user",
        "--if-not-exists",
        "flathub",
        "https://dl.flathub.org/repo/flathub.flatpakrepo",
    ])) && run(Command::new("flatpak-builder")
        .args([
            "--user",
            "--install-deps-from=flathub",
            "--force-clean",
            "--disable-rofiles-fuse",
        ])
        // Its cache (often gigabytes) under target/, not as .flatpak-builder/ in the repository.
        .arg(format!(
            "--state-dir={}",
            root().join("target/flatpak-builder").display()
        ))
        .arg(format!("--repo={}", repo.display()))
        .arg(dir.join("build"))
        .arg(&manifest))
        && run(Command::new("flatpak")
            .args([
                "build-bundle",
                "--runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo",
            ])
            .arg(&repo)
            .arg(&out)
            .arg(APP_ID))
}

/// Debian versions sort `~` before anything: 0.1.0~alpha < 0.1.0. RPM does the same.
fn package_version() -> String {
    version().replacen('-', "~", 1)
}

/// The server's third-party licenses, where its package metadata takes them from (`fb_server/Cargo.toml`).
fn server_licenses() -> bool {
    third_party(
        "fb_server",
        &[],
        &root().join("target/licenses/fb_server").join(THIRD_PARTY),
    )
}

fn deb() -> bool {
    if !server_licenses() {
        return false;
    }
    // `--no-strip`: the binary keeps the `dist` profile's line tables, as in the rpm.
    let mut c = stamped(cargo());
    c.args(["deb", "--locked", "--profile", "dist", "--no-strip", "-p", "fb_server"])
        .args(["--target", MUSL, "--deb-version"])
        .arg(package_version())
        .arg("-o")
        .arg(out_dir());
    let ok = run(&mut c);
    if !ok {
        eprintln!("(needs `cargo install cargo-deb`, `rustup target add {MUSL}`, musl-tools and cmake)");
    }
    ok
}

fn rpm() -> bool {
    if !server_licenses() {
        return false;
    }
    let mut b = stamped(cargo());
    b.args([
        "build",
        "--locked",
        "--profile",
        "dist",
        "-p",
        "fb_server",
        "--target",
        MUSL,
    ]);
    let mut c = cargo();
    c.args([
        "generate-rpm",
        "--profile",
        "dist",
        "-p",
        "crates/fb_server",
        "--target",
        MUSL,
        "-s",
    ])
    .arg(format!("version = \"{}\"", package_version()))
    .arg("-o")
    .arg(out_dir());
    let ok = run(&mut b) && run(&mut c);
    if !ok {
        eprintln!("(needs `cargo install cargo-generate-rpm`, `rustup target add {MUSL}`, musl-tools and cmake)");
    }
    ok
}

pub fn dist(a: &DistArgs) -> bool {
    match a.kind {
        Kind::Nsis => nsis(),
        Kind::Appimage => appimage(),
        Kind::Flatpak => flatpak(),
        Kind::Deb => deb(),
        Kind::Rpm => rpm(),
    }
}
