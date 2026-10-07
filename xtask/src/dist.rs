//! `cargo xtask dist <kind>`: release packages into dist/ (the release workflow runs one kind per job).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, ValueEnum};

use crate::{cargo, root, run, target_dir};

pub const APP_ID: &str = "io.github.kauri_off.fallbeans";
const MUSL: &str = "x86_64-unknown-linux-musl";
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
    let d = target_dir().join("pkg").join(name);
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

/// The client's features in the packages: no BRP probe, the profiler of F4, and DLSS when its SDK is there (the
/// package carries NVIDIA's library then: `upscalers_into`).
fn client_features() -> [&'static str; 3] {
    let features = if crate::dlss() { "profiler,dlss" } else { "profiler" };
    ["--no-default-features", "--features", features]
}

/// The licenses of every crate built into `package` (cargo-about, `packaging/about.toml`) as `out`; fails on a
/// license that `about.toml` does not accept.
fn third_party(package: &str, features: &[&str], out: &Path) -> bool {
    let found = cargo()
        .args(["about", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("cargo-about is missing: cargo xtask setup --dist");
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
/// an unpacked DXC release in `FB_DXC_DIR` (`setup` puts it in target/sdk); without it the game falls back to FXC.
fn dxc_into(dir: &Path) {
    if !cfg!(windows) {
        return;
    }
    let Some(src) = env_path("FB_DXC_DIR") else {
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

/// The packages' client, built into target/dist (`cargo xtask play --dist` runs it from there).
pub fn build_client() -> Option<PathBuf> {
    let mut c = stamped(cargo());
    c.args(["build", "--locked", "--profile", "dist", "-p", "fb_client"])
        .args(client_features());
    // The C runtime inside the exe: a clean Windows has no VCRUNTIME140.dll and the installer does not ship it.
    // (Here, not in .cargo/config.toml: there every local build would rebuild everything for it.)
    if cfg!(windows) {
        c.env(
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS",
            "-C target-feature=+crt-static",
        );
    }
    run(&mut c).then(|| {
        target_dir()
            .join("dist")
            .join(format!("fb_client{}", std::env::consts::EXE_SUFFIX))
    })
}

/// The client packages ship every upscaler: a missing one fails the package instead of a quiet build without it.
fn upscalers_ready() -> bool {
    let mut missing = Vec::new();
    if !crate::dlss() {
        missing.push("DLSS SDK (DLSS_SDK)");
    }
    if dlss_runtime().is_none() {
        missing.push("DLSS library (DLSS_DLL)");
    }
    if env_path("FFX_SDK").is_none() {
        missing.push("FidelityFX SDK (FFX_SDK)");
    }
    if cfg!(windows) && env_path("FB_DXC_DIR").is_none() {
        missing.push("DXC (FB_DXC_DIR)");
    }
    if !missing.is_empty() {
        eprintln!(
            "the client packages need {}: cargo xtask setup (cargo xtask doctor lists the rest)",
            missing.join(", ")
        );
    }
    missing.is_empty()
}

/// The release client, staged with its assets and licenses into `dir`.
fn client_into(dir: &Path) -> bool {
    if !upscalers_ready() {
        return false;
    }
    let Some(exe) = build_client() else {
        return false;
    };
    if !third_party("fb_client", &client_features(), &dir.join(THIRD_PARTY)) {
        return false;
    }
    copy(&exe, dir.join(exe.file_name().expect("exe name")));
    copy_dir(&root().join("assets"), &dir.join("assets"));
    copy("LICENSE", dir.join("LICENSE"));
    dxc_into(dir);
    upscalers_into(dir, Some(&dir.join(UPSCALER_LICENSES)))
}

/// Where a package keeps the upscalers' notices and licenses.
const UPSCALER_LICENSES: &str = "upscaler-licenses";

fn env_path(var: &str) -> Option<PathBuf> {
    crate::sdk::var(var)
}

/// NVIDIA's DLSS library the game loads at run time: `DLSS_DLL`, else the SDK's own in `DLSS_SDK`
/// (`nvngx_dlss.dll` on Windows, `libnvidia-ngx-dlss.so.<version>` on Linux, a name NGX looks for as it is).
fn dlss_runtime() -> Option<PathBuf> {
    let from_sdk = || {
        let sdk = env_path("DLSS_SDK")?;
        if cfg!(windows) {
            return Some(sdk.join("lib/Windows_x86_64/rel/nvngx_dlss.dll"));
        }
        fs::read_dir(sdk.join("lib/Linux_x86_64/rel"))
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libnvidia-ngx-dlss.so."))
            })
    };
    env_path("DLSS_DLL").or_else(from_sdk).filter(|p| p.is_file())
}

/// AMD's FidelityFX library for FSR 3.1: the signed DLL of the SDK in `FFX_SDK` on Windows, on Linux (AMD ships
/// none) built from the same SDK's sources with `packaging/fidelityfx-linux`.
fn fidelityfx() -> Option<PathBuf> {
    let Some(sdk) = env_path("FFX_SDK") else {
        eprintln!("FFX_SDK is not set: no AMD FSR 3.1");
        return None;
    };
    if cfg!(windows) {
        let dll = sdk.join("PrebuiltSignedDLL/amd_fidelityfx_vk.dll");
        if !dll.is_file() {
            eprintln!("FFX_SDK has no {}: no AMD FSR 3.1", dll.display());
            return None;
        }
        return Some(dll);
    }
    fidelityfx_linux(&sdk)
}

/// `libamd_fidelityfx_vk.so` under target/fidelityfx-linux: the SDK's sources copied and patched once per
/// SDK and patch, then CMake (cmake, a C++17 compiler, the Vulkan headers and loader, glslangValidator).
fn fidelityfx_linux(sdk: &Path) -> Option<PathBuf> {
    const SOURCES: [&str; 7] = [
        "ffx-api/include",
        "ffx-api/src",
        "sdk/include",
        "sdk/src",
        "sdk/tools/ffx_shader_compiler/src",
        "sdk/tools/ffx_shader_compiler/libs/MD5",
        "sdk/tools/ffx_shader_compiler/libs/SPIRV-Reflect",
    ];
    const PROCESS: &str = "sdk/tools/ffx_shader_compiler/libs/tiny-process-library";
    if let Some(missing) = SOURCES.iter().chain([&PROCESS]).find(|d| !sdk.join(d).is_dir()) {
        eprintln!("FFX_SDK has no {missing}/ (the whole SDK unpacked, not only PrebuiltSignedDLL): no AMD FSR 3.1");
        return None;
    }
    let recipe = root().join("packaging/fidelityfx-linux");
    let patch = recipe.join("fidelityfx-sdk-v1.1.4.patch");
    let work = target_dir().join("fidelityfx-linux");
    let src = work.join("src");
    let stamp = work.join("patched");
    let want = format!(
        "{}\n{}",
        sdk.display(),
        fs::read_to_string(&patch).expect("the FidelityFX patch")
    );
    if fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = fs::remove_dir_all(&src);
        for d in SOURCES.iter().chain([&PROCESS]) {
            copy_dir(&sdk.join(d), &src.join(d));
        }
        let patched = run(Command::new("patch")
            .args(["-p1", "--forward", "--batch", "-d"])
            .arg(&src)
            .arg("-i")
            .arg(&patch));
        if !patched {
            eprintln!("the FidelityFX patch does not apply to FFX_SDK (v1.1.4 expected): no AMD FSR 3.1");
            return None;
        }
        fs::write(&stamp, &want).expect("stamp");
    }
    let build = work.join("build");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&recipe)
        .arg("-B")
        .arg(&build)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg(format!("-DFFX_SRC={}", src.display()));
    let built = run(&mut configure) && run(Command::new("cmake").arg("--build").arg(&build).arg("--parallel"));
    let lib = build.join("libamd_fidelityfx_vk.so");
    if !built || !lib.is_file() {
        eprintln!("FidelityFX did not build for Linux: no AMD FSR 3.1");
        return None;
    }
    Some(lib)
}

/// The upscalers' libraries beside the client (the game falls back to FSR 1 without them): NVIDIA's DLSS
/// (`dlss_runtime`) when the client is built with it, AMD's FidelityFX for FSR 3.1 (`fidelityfx`). With
/// `licenses` (a package), the notices they ship with go there, and a library without them fails the package.
pub fn upscalers_into(dir: &Path, licenses: Option<&Path>) -> bool {
    let mut ok = true;
    if crate::dlss() {
        match dlss_runtime() {
            Some(lib) => {
                let name = if cfg!(windows) {
                    "nvngx_dlss.dll".into()
                } else {
                    lib.file_name().expect("a file").to_owned()
                };
                ok &= put(&lib, &dir.join(name));
                if let Some(l) = licenses {
                    ok &= notices_into(l);
                }
            }
            None => {
                eprintln!("no DLSS library (DLSS_DLL or DLSS_SDK): the client cannot use DLSS");
                ok &= licenses.is_none();
            }
        }
    }
    let Some(lib) = fidelityfx() else {
        return ok && licenses.is_none();
    };
    let name = if cfg!(windows) {
        "amd_fidelityfx_vk.dll"
    } else {
        "libamd_fidelityfx_vk.so"
    };
    ok &= put(&lib, &dir.join(name));
    if let Some(l) = licenses {
        let license = env_path("FFX_SDK").unwrap_or_default().join("docs/license.md");
        if license.is_file() {
            fs::create_dir_all(l).unwrap_or_else(|e| panic!("{}: {e}", l.display()));
            copy(&license, l.join("AMD-FidelityFX-SDK-license.md"));
        } else {
            eprintln!("FFX_SDK has no docs/license.md: AMD's library is not packaged without its license");
            ok = false;
        }
    }
    ok
}

/// A library copied beside the client, unless the same one is there (a running client holds its libraries).
fn put(src: &Path, to: &Path) -> bool {
    let len = |p: &Path| fs::metadata(p).ok().map(|m| m.len());
    if to.is_file() && len(to) == len(src) {
        return true;
    }
    match fs::copy(src, to) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("{} → {}: {e}", src.display(), to.display());
            false
        }
    }
}

/// NVIDIA's notices the DLSS DLL ships with (section 9.5 of its programming guide, and the third-party code of
/// 9.6), from the guide as text: `DLSS_GUIDE_TEXT` (a text file of it), else the guide's PDF in `DLSS_SDK` read
/// with `pdftotext` (poppler).
fn notices_into(dir: &Path) -> bool {
    let text = match std::env::var_os("DLSS_GUIDE_TEXT") {
        Some(f) => fs::read_to_string(f).ok(),
        None => env_path("DLSS_SDK").and_then(|sdk| {
            let pdf = Path::new(&sdk).join("doc/DLSS_Programming_Guide_Release.pdf");
            let out = Command::new("pdftotext")
                .args(["-layout", "-enc", "UTF-8"])
                .arg(&pdf)
                .arg("-")
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        }),
    };
    let Some(notices) = text.as_deref().and_then(notices_of) else {
        eprintln!("no DLSS notices (DLSS_GUIDE_TEXT, or pdftotext and the guide in DLSS_SDK/doc): not packaged");
        return false;
    };
    fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    fs::write(dir.join("NVIDIA-DLSS-notices.txt"), notices).is_ok()
}

/// Sections 9.5 and 9.6 of the DLSS programming guide's text: from the last "9.5 Notices" (the contents name it
/// first) to "9.7 Linux driver compatibility".
fn notices_of(text: &str) -> Option<String> {
    let start = text.rfind("9.5 Notices")?;
    let rest = text.get(start..)?;
    let end = rest.find("9.7 Linux driver").unwrap_or(rest.len());
    let notices = rest.get(..end)?.trim_end();
    (notices.len() > 1000).then(|| format!("{notices}\n"))
}

fn tool(var: &str, name: &str) -> Command {
    Command::new(env_path(var).unwrap_or_else(|| name.into()))
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
    if let Some(runtime) = env_path("APPIMAGE_RUNTIME") {
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
            target_dir().join("flatpak-builder").display()
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

#[cfg(test)]
mod tests {
    use super::notices_of;

    #[test]
    fn the_notices_are_the_section_not_its_line_in_the_contents() {
        let body = "x".repeat(1200);
        let guide =
            format!("   9.5 Notices ........ 68\n   9.7 Linux driver ... 73\n\n9.5 Notices\n{body}\n9.7 Linux driver");
        let n = notices_of(&guide).unwrap();
        assert!(n.starts_with("9.5 Notices\nxxx") && n.ends_with("x\n"), "{n}");
        assert_eq!(notices_of("9.5 Notices, too short\n9.7 Linux driver"), None);
    }
}
