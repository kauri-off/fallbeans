//! `cargo xtask dist <kind>`: release packages into dist/ (the release workflow runs one kind per job).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};

use crate::{cargo, report, reported, root, run, target_dir};

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

fn create_dir(d: &Path) -> Result<()> {
    fs::create_dir_all(d).with_context(|| d.display().to_string())
}

fn out_dir() -> Result<PathBuf> {
    let d = root().join("dist");
    create_dir(&d)?;
    Ok(d)
}

fn stage(name: &str) -> Result<PathBuf> {
    let d = target_dir().join("pkg").join(name);
    let _ = fs::remove_dir_all(&d);
    create_dir(&d)?;
    Ok(d)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    create_dir(to)?;
    for entry in fs::read_dir(from).with_context(|| from.display().to_string())? {
        let entry = entry.with_context(|| from.display().to_string())?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).with_context(|| path.display().to_string())?;
        }
    }
    Ok(())
}

/// `from` (relative to the root, or absolute) to `to`.
fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let (from, to) = (root().join(from), to.as_ref());
    fs::copy(&from, to).with_context(|| format!("{} → {}", from.display(), to.display()))?;
    Ok(())
}

/// The client's features in the packages: no BRP probe or `--trace`, the profiler of F4, and DLSS when its SDK
/// is there (the package carries NVIDIA's library then: `upscalers_into`).
fn client_features() -> [&'static str; 3] {
    let features = if crate::dlss() { "profiler,dlss" } else { "profiler" };
    ["--no-default-features", "--features", features]
}

/// The licenses of every crate built into `package` (cargo-about, `packaging/about.toml`) as `out`; fails on a
/// license that `about.toml` does not accept.
fn third_party(package: &str, features: &[&str], out: &Path) -> Result<()> {
    let found = cargo()
        .args(["about", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        bail!("cargo-about is missing: cargo xtask setup --dist");
    }
    if let Some(dir) = out.parent() {
        create_dir(dir)?;
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
fn dxc_into(dir: &Path) -> Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    let Some(src) = env_path("FB_DXC_DIR") else {
        eprintln!("FB_DXC_DIR is not set: the package compiles DX12 shaders with FXC (slow)");
        return Ok(());
    };
    for f in ["dxcompiler.dll", "dxil.dll"] {
        copy(src.join("bin").join("x64").join(f), dir.join(f))?;
    }
    let licenses = dir.join("dxc-licenses");
    create_dir(&licenses)?;
    for f in ["LICENCE-MIT.txt", "LICENSE-LLVM.txt", "LICENSE-MS.txt"] {
        copy(src.join(f), licenses.join(f))?;
    }
    Ok(())
}

/// The packages' client, built into target/dist (`cargo xtask play --dist` runs it from there).
pub fn build_client() -> Result<PathBuf> {
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
    run(&mut c)?;
    Ok(target_dir()
        .join("dist")
        .join(format!("fb_client{}", std::env::consts::EXE_SUFFIX)))
}

/// The client packages ship every upscaler: a missing one fails the package instead of a quiet build without it.
fn upscalers_ready() -> Result<()> {
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
        bail!(
            "the client packages need {}: cargo xtask setup (cargo xtask doctor lists the rest)",
            missing.join(", ")
        );
    }
    Ok(())
}

/// The release client, staged with its assets and licenses into `dir`.
fn client_into(dir: &Path) -> Result<()> {
    upscalers_ready()?;
    let exe = build_client()?;
    third_party("fb_client", &client_features(), &dir.join(THIRD_PARTY))?;
    let name = exe.file_name().context("the client has no file name")?;
    copy(&exe, dir.join(name))?;
    copy_dir(&root().join("assets"), &dir.join("assets"))?;
    copy("LICENSE", dir.join("LICENSE"))?;
    dxc_into(dir)?;
    upscalers_into(dir, Some(&dir.join(UPSCALER_LICENSES)))
}

/// Where a package keeps the upscalers' licenses.
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
fn fidelityfx() -> Result<PathBuf> {
    let sdk = env_path("FFX_SDK").ok_or_else(|| anyhow!("FFX_SDK is not set"))?;
    if cfg!(windows) {
        let dll = sdk.join("PrebuiltSignedDLL/amd_fidelityfx_vk.dll");
        if !dll.is_file() {
            bail!("FFX_SDK has no {}", dll.display());
        }
        return Ok(dll);
    }
    fidelityfx_linux(&sdk)
}

/// `libamd_fidelityfx_vk.so` under target/fidelityfx-linux: the SDK's sources copied and patched once per
/// SDK and patch, then CMake (cmake, a C++17 compiler, the Vulkan headers and loader, glslangValidator).
fn fidelityfx_linux(sdk: &Path) -> Result<PathBuf> {
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
        bail!("FFX_SDK has no {missing}/ (the whole SDK unpacked, not only PrebuiltSignedDLL)");
    }
    let recipe = root().join("packaging/fidelityfx-linux");
    let patch = recipe.join("fidelityfx-sdk-v1.1.4.patch");
    let work = target_dir().join("fidelityfx-linux");
    let src = work.join("src");
    let stamp = work.join("patched");
    let want = format!(
        "{}\n{}",
        sdk.display(),
        fs::read_to_string(&patch).with_context(|| patch.display().to_string())?
    );
    if fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = fs::remove_dir_all(&src);
        for d in SOURCES.iter().chain([&PROCESS]) {
            copy_dir(&sdk.join(d), &src.join(d))?;
        }
        let patched = run(Command::new("patch")
            .args(["-p1", "--forward", "--batch", "-d"])
            .arg(&src)
            .arg("-i")
            .arg(&patch));
        if patched.is_err() {
            bail!("the FidelityFX patch does not apply to FFX_SDK (v1.1.4 expected)");
        }
        fs::write(&stamp, &want).with_context(|| stamp.display().to_string())?;
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
    let built =
        run(&mut configure).and_then(|()| run(Command::new("cmake").arg("--build").arg(&build).arg("--parallel")));
    let lib = build.join("libamd_fidelityfx_vk.so");
    if built.is_err() || !lib.is_file() {
        bail!("FidelityFX did not build for Linux");
    }
    Ok(lib)
}

/// AMD's FSR 3.1 library beside the client.
pub const FFX_LIB: &str = if cfg!(windows) {
    "amd_fidelityfx_vk.dll"
} else {
    "libamd_fidelityfx_vk.so"
};

/// The upscalers' libraries beside the client (without them: FSR 1): DLSS (`dlss_runtime`) and FidelityFX (FSR 3.1).
/// A missing library fails a package; failures are printed as they come, the result says only whether one was fatal.
pub fn upscalers_into(dir: &Path, licenses: Option<&Path>) -> Result<()> {
    let package = licenses.is_some();
    let mut ok = true;
    if crate::dlss() {
        match dlss_runtime() {
            Some(lib) => {
                let name = match lib.file_name() {
                    Some(n) if !cfg!(windows) => n.to_owned(),
                    _ => "nvngx_dlss.dll".into(),
                };
                ok &= report(put(&lib, &dir.join(name)));
            }
            None => {
                eprintln!("no DLSS library (DLSS_DLL or DLSS_SDK): the client cannot use DLSS");
                ok &= !package;
            }
        }
    }
    let lib = match fidelityfx() {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("{e:#}: no AMD FSR 3.1");
            return reported(ok && !package);
        }
    };
    ok &= report(put(&lib, &dir.join(FFX_LIB)));
    if let Some(l) = licenses {
        let license = env_path("FFX_SDK").unwrap_or_default().join("docs/license.md");
        if license.is_file() {
            ok &= report(create_dir(l).and_then(|()| copy(&license, l.join("AMD-FidelityFX-SDK-license.md"))));
        } else {
            eprintln!("FFX_SDK has no docs/license.md: AMD's library is not packaged without its license");
            ok = false;
        }
    }
    reported(ok)
}

/// A library copied beside the client, unless the same one is there (a running client holds its libraries).
fn put(src: &Path, to: &Path) -> Result<()> {
    let len = |p: &Path| fs::metadata(p).ok().map(|m| m.len());
    if to.is_file() && len(to) == len(src) {
        return Ok(());
    }
    fs::copy(src, to).with_context(|| format!("{} → {}", src.display(), to.display()))?;
    Ok(())
}

fn tool(var: &str, name: &str) -> Command {
    Command::new(env_path(var).unwrap_or_else(|| name.into()))
}

fn nsis() -> Result<()> {
    let s = stage("nsis")?;
    client_into(&s)?;
    // The backtrace's names on Windows: dbghelp finds the PDB beside the exe.
    copy(target_dir().join("dist").join("fb_client.pdb"), s.join("fb_client.pdb"))?;
    copy("packaging/icons/fallbeans.ico", s.join("fallbeans.ico"))?;
    let out = out_dir()?.join(format!("FallBeans-{}-setup.exe", version()));
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

fn appimage() -> Result<()> {
    let app = stage("AppDir")?;
    let bin = app.join("usr/bin");
    create_dir(&bin)?;
    client_into(&bin)?;
    let desktop = format!("{APP_ID}.desktop");
    copy(Path::new("packaging/linux").join(&desktop), app.join(&desktop))?;
    copy("packaging/icons/fallbeans-256.png", app.join(format!("{APP_ID}.png")))?;
    copy("packaging/icons/fallbeans-256.png", app.join(".DirIcon"))?;
    let apps = app.join("usr/share/applications");
    let icons = app.join("usr/share/icons/hicolor/256x256/apps");
    let meta = app.join("usr/share/metainfo");
    for d in [&apps, &icons, &meta] {
        create_dir(d)?;
    }
    copy(Path::new("packaging/linux").join(&desktop), apps.join(&desktop))?;
    copy("packaging/icons/fallbeans-256.png", icons.join(format!("{APP_ID}.png")))?;
    let metainfo = format!("{APP_ID}.metainfo.xml");
    copy(
        Path::new("packaging/linux").join(&metainfo),
        meta.join(format!("{APP_ID}.appdata.xml")),
    )?;
    let apprun = app.join("AppRun");
    fs::write(
        &apprun,
        "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/fb_client\" \"$@\"\n",
    )
    .with_context(|| apprun.display().to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&apprun, fs::Permissions::from_mode(0o755))
            .with_context(|| apprun.display().to_string())?;
    }
    let out = out_dir()?.join(format!("FallBeans-{}-x86_64.AppImage", version()));
    let mut c = tool("APPIMAGETOOL", "appimagetool");
    c.env("ARCH", "x86_64").env("APPIMAGE_EXTRACT_AND_RUN", "1");
    // A runtime of a known release (the workflow checks its sum); else appimagetool downloads the latest one.
    if let Some(runtime) = env_path("APPIMAGE_RUNTIME") {
        c.arg("--runtime-file").arg(runtime);
    }
    run(c.arg(&app).arg(&out))
}

fn flatpak() -> Result<()> {
    let dir = stage("flatpak")?;
    let s = dir.join("stage");
    create_dir(&s)?;
    client_into(&s)?;
    // (client_into staged LICENSE and THIRD_PARTY too: the manifest installs them.)
    for f in [format!("{APP_ID}.desktop"), format!("{APP_ID}.metainfo.xml")] {
        copy(Path::new("packaging/linux").join(&f), s.join(&f))?;
    }
    for f in ["fallbeans-256.png", "fallbeans-512.png"] {
        copy(Path::new("packaging/icons").join(f), s.join(f))?;
    }
    let manifest = dir.join(format!("{APP_ID}.yml"));
    copy(Path::new("packaging/flatpak").join(format!("{APP_ID}.yml")), &manifest)?;
    let repo = dir.join("repo");
    let out = out_dir()?.join(format!("FallBeans-{}-x86_64.flatpak", version()));
    run(Command::new("flatpak").args([
        "remote-add",
        "--user",
        "--if-not-exists",
        "flathub",
        "https://dl.flathub.org/repo/flathub.flatpakrepo",
    ]))?;
    run(Command::new("flatpak-builder")
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
        .arg(&manifest))?;
    run(Command::new("flatpak")
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
fn server_licenses() -> Result<()> {
    third_party(
        "fb_server",
        &[],
        &root().join("target/licenses/fb_server").join(THIRD_PARTY),
    )
}

fn deb() -> Result<()> {
    server_licenses()?;
    // `--no-strip`: the binary keeps the `dist` profile's line tables, as in the rpm.
    let mut c = stamped(cargo());
    c.args(["deb", "--locked", "--profile", "dist", "--no-strip", "-p", "fb_server"])
        .args(["--target", MUSL, "--deb-version"])
        .arg(package_version())
        .arg("-o")
        .arg(out_dir()?);
    run(&mut c).inspect_err(|_| {
        eprintln!("(needs `cargo install cargo-deb`, `rustup target add {MUSL}`, musl-tools and cmake)")
    })
}

fn rpm() -> Result<()> {
    server_licenses()?;
    let mut b = stamped(cargo());
    b.args([
        "build",
        "--locked",
        "--profile",
        "dist",
        "-p",
        "fb_server",
        "--no-default-features",
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
    .arg(out_dir()?);
    run(&mut b).and_then(|()| run(&mut c)).inspect_err(|_| {
        eprintln!("(needs `cargo install cargo-generate-rpm`, `rustup target add {MUSL}`, musl-tools and cmake)")
    })
}

pub fn dist(a: &DistArgs) -> Result<()> {
    match a.kind {
        Kind::Nsis => nsis(),
        Kind::Appimage => appimage(),
        Kind::Flatpak => flatpak(),
        Kind::Deb => deb(),
        Kind::Rpm => rpm(),
    }
}
