//! `cargo xtask doctor`: what this machine has for each task, and how to get what it lacks.
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{cargo, root, sdk};

type Check = Result<String, String>;

#[derive(Default)]
struct Report {
    required_missing: bool,
}

impl Report {
    fn group(&self, title: &str) {
        eprintln!("\n{title}");
    }

    fn item(&mut self, name: &str, required: bool, check: Check) {
        match check {
            Ok(detail) => eprintln!("  ok    {name}: {detail}"),
            Err(hint) => {
                self.required_missing |= required;
                eprintln!("  {}  {name}: {hint}", if required { "MISS" } else { "--  " });
            }
        }
    }
}

/// The first line a tool prints for `args` (stdout, else stderr), when it runs and succeeds.
fn tool(cmd: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).current_dir(root()).args(args).output().ok()?;
    if !o.status.success() {
        return None;
    }
    let text = if o.stdout.is_empty() { o.stderr } else { o.stdout };
    Some(
        String::from_utf8_lossy(&text)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
    )
}

#[derive(PartialEq)]
enum Distro {
    Arch,
    Debian,
    Fedora,
    Other,
}

fn distro() -> Distro {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let ids: String = release
        .lines()
        .filter(|l| l.starts_with("ID=") || l.starts_with("ID_LIKE="))
        .collect::<Vec<_>>()
        .join(" ");
    if ids.contains("arch") {
        Distro::Arch
    } else if ids.contains("debian") || ids.contains("ubuntu") {
        Distro::Debian
    } else if ids.contains("fedora") || ids.contains("rhel") {
        Distro::Fedora
    } else {
        Distro::Other
    }
}

/// The install command for this distribution: packages for Arch, Debian/Ubuntu and Fedora.
fn pkg(arch: &str, debian: &str, fedora: &str) -> String {
    match distro() {
        Distro::Arch => format!("sudo pacman -S --needed {arch}"),
        Distro::Debian => format!("sudo apt install {debian}"),
        Distro::Fedora => format!("sudo dnf install {fedora}"),
        Distro::Other => format!("install {debian} (Debian names)"),
    }
}

fn present(path: Option<PathBuf>, inside: &str) -> Option<PathBuf> {
    path.filter(|p| {
        if inside.is_empty() {
            p.exists()
        } else {
            p.join(inside).exists()
        }
    })
}

fn shown(p: &Path) -> String {
    p.strip_prefix(root()).unwrap_or(p).display().to_string()
}

/// A pinned dependency: where its path comes from and its version, else how to get it.
fn pinned(name: &str, var: &str, inside: &str, setup: &str) -> Check {
    let from_env = std::env::var_os(var).is_some();
    match present(sdk::var(var), inside) {
        Some(p) if from_env => Ok(format!("{} (from {var})", shown(&p))),
        Some(p) => Ok(format!(
            "{} {}",
            sdk::dep(name).map_or("", |d| d.version.as_str()),
            shown(&p)
        )),
        None if from_env => Err(format!("{var} has no {inside}")),
        None => Err(setup.to_string()),
    }
}

fn on_path(name: &str, args: &[&str], hint: String) -> Check {
    tool(name, args).ok_or(hint)
}

pub fn doctor() -> bool {
    let mut r = Report::default();
    let windows = cfg!(windows);

    r.group("build: check, dev, play, perf");
    r.item(
        "rust",
        true,
        tool("rustc", &["-V"]).ok_or_else(|| "rustup: https://rustup.rs".into()),
    );
    r.item(
        "git",
        true,
        on_path(
            "git",
            &["--version"],
            if windows {
                "winget install Git.Git".into()
            } else {
                pkg("git", "git", "git")
            },
        ),
    );
    if windows {
        r.item("MSVC", true, msvc());
        r.item("Windows SDK", true, windows_sdk());
    } else {
        r.item(
            "C compiler",
            true,
            on_path("cc", &["--version"], pkg("base-devel", "build-essential", "gcc")),
        );
        r.item("system libraries", true, linux_libs());
    }

    r.group("upscalers (optional; DLSS 4.5, FSR 3.1): cargo xtask setup");
    let setup = "cargo xtask setup";
    r.item(
        "DLSS SDK",
        false,
        pinned(
            &format!("dlss-sdk-{}", sdk::this_os()),
            "DLSS_SDK",
            "include/nvsdk_ngx.h",
            setup,
        ),
    );
    r.item(
        "DLSS library",
        false,
        pinned(&format!("dlss-runtime-{}", sdk::this_os()), "DLSS_DLL", "", setup),
    );
    r.item(
        "Vulkan headers",
        false,
        pinned("vulkan-headers", "VULKAN_SDK", "include/vulkan/vulkan.h", setup),
    );
    r.item("libclang (DLSS bindings)", false, libclang());
    let ffx_inside = if windows {
        "PrebuiltSignedDLL/amd_fidelityfx_vk.dll"
    } else {
        "ffx-api/include"
    };
    r.item(
        "FidelityFX SDK",
        false,
        pinned(&format!("fidelityfx-{}", sdk::this_os()), "FFX_SDK", ffx_inside, setup),
    );
    if windows {
        r.item(
            "DXC",
            false,
            pinned("dxc", "FB_DXC_DIR", "bin/x64/dxcompiler.dll", setup),
        );
    } else {
        // FSR 3.1 on Linux: `dist` builds AMD's library from the SDK's sources.
        for (name, cmd, args, hint) in [
            (
                "cmake (FSR 3.1)",
                "cmake",
                &["--version"][..],
                pkg("cmake", "cmake", "cmake"),
            ),
            (
                "glslang (FSR 3.1)",
                "glslangValidator",
                &["--version"][..],
                pkg("glslang", "glslang-tools", "glslang"),
            ),
            (
                "patch (FSR 3.1)",
                "patch",
                &["--version"][..],
                pkg("patch", "patch", "patch"),
            ),
            (
                "C++ compiler (FSR 3.1)",
                "c++",
                &["--version"][..],
                pkg("base-devel", "build-essential", "gcc-c++"),
            ),
        ] {
            r.item(name, false, on_path(cmd, args, hint));
        }
    }

    r.group("packages (cargo xtask dist): the upscalers above, and cargo xtask setup --dist");
    let setup_dist = "cargo xtask setup --dist";
    r.item("cargo-about", false, cargo_tool("about", setup_dist));
    r.item("DLSS notices (pdftotext)", false, notices_text());
    if windows {
        r.item("nsis: makensis", false, makensis());
    } else {
        r.item(
            "appimage: appimagetool",
            false,
            sdk::var("APPIMAGETOOL")
                .filter(|p| p.is_file())
                .map(|p| shown(&p))
                .or_else(|| tool("appimagetool", &["--version"]))
                .ok_or_else(|| setup_dist.into()),
        );
        r.item(
            "appimage: runtime",
            false,
            sdk::var("APPIMAGE_RUNTIME")
                .filter(|p| p.is_file())
                .map(|p| shown(&p))
                .ok_or_else(|| format!("{setup_dist} (else appimagetool downloads the latest one)")),
        );
        r.item("appimage: glibc", false, glibc());
        r.item(
            "flatpak: flatpak-builder",
            false,
            on_path(
                "flatpak-builder",
                &["--version"],
                pkg("flatpak-builder", "flatpak flatpak-builder", "flatpak-builder"),
            ),
        );
        r.item("deb: cargo-deb", false, cargo_tool("deb", setup_dist));
        r.item("rpm: cargo-generate-rpm", false, cargo_tool("generate-rpm", setup_dist));
        r.item("deb, rpm: musl target", false, musl_target());
        r.item(
            "deb, rpm: musl-gcc",
            false,
            on_path("musl-gcc", &["--version"], pkg("musl", "musl-tools", "musl-gcc")),
        );
    }

    r.group("assets --export");
    let blender = std::env::var("BLENDER").unwrap_or_else(|_| "blender".into());
    r.item(
        "blender",
        false,
        tool(&blender, &["--version"]).ok_or_else(|| "Blender on PATH, or BLENDER=/path/to/blender".into()),
    );

    eprintln!();
    if r.required_missing {
        eprintln!("doctor: the build needs what is marked MISS");
    } else {
        eprintln!("doctor: the build has what it needs; `--` marks what only some tasks need");
    }
    !r.required_missing
}

fn linux_libs() -> Check {
    let hint = pkg(
        "alsa-lib systemd-libs wayland libxkbcommon pkgconf",
        "libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev pkg-config",
        "alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel pkgconf",
    );
    if tool("pkg-config", &["--version"]).is_none() {
        return Err(format!("no pkg-config: {hint}"));
    }
    let missing: Vec<_> = ["alsa", "libudev", "wayland-client", "xkbcommon"]
        .into_iter()
        .filter(|l| tool("pkg-config", &["--exists", l]).is_none())
        .collect();
    if missing.is_empty() {
        Ok("alsa, libudev, wayland, xkbcommon".into())
    } else {
        Err(format!("no {}: {hint}", missing.join(", ")))
    }
}

const BUILD_TOOLS: &str = "winget install Microsoft.VisualStudio.2022.BuildTools --override \
    \"--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\"";

fn msvc() -> Check {
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    tool(
        vswhere,
        &[
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "displayName",
        ],
    )
    .filter(|s| !s.is_empty())
    .ok_or_else(|| BUILD_TOOLS.into())
}

/// The newest Windows SDK's x64 bin with `rc.exe` (the exe's icon and version).
fn windows_sdk() -> Check {
    std::fs::read_dir(r"C:\Program Files (x86)\Windows Kits\10\bin")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("x64"))
        .filter(|d| d.join("rc.exe").is_file())
        .max()
        .map(|d| d.display().to_string())
        .ok_or_else(|| format!("no rc.exe: {BUILD_TOOLS}"))
}

fn libclang() -> Check {
    if let Some(p) = std::env::var_os("LIBCLANG_PATH") {
        return Ok(format!("{} (from LIBCLANG_PATH)", PathBuf::from(p).display()));
    }
    if cfg!(windows) {
        return sdk::libclang_default()
            .map(|d| d.display().to_string())
            .ok_or_else(|| "winget install LLVM.LLVM".into());
    }
    let ldconfig = Command::new("ldconfig").arg("-p").output().ok();
    let found = ldconfig
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .and_then(|t| {
            t.lines()
                .find(|l| l.contains("libclang.so"))
                .map(|l| l.trim().to_string())
        });
    found
        .or_else(|| tool("llvm-config", &["--libdir"]))
        .ok_or_else(|| pkg("clang", "clang libclang-dev", "clang clang-devel"))
}

fn cargo_tool(sub: &str, hint: &str) -> Check {
    let o = cargo().args([sub, "--version"]).output().ok();
    o.filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or_else(|| hint.into())
}

fn notices_text() -> Check {
    if let Some(p) = std::env::var_os("DLSS_GUIDE_TEXT") {
        return Ok(format!("{} (from DLSS_GUIDE_TEXT)", PathBuf::from(p).display()));
    }
    tool("pdftotext", &["-v"]).ok_or_else(|| {
        if cfg!(windows) {
            "scoop install poppler, or DLSS_GUIDE_TEXT=<the guide as text>".into()
        } else {
            pkg("poppler", "poppler-utils", "poppler-utils")
        }
    })
}

fn makensis() -> Check {
    let default = r"C:\Program Files (x86)\NSIS\makensis.exe";
    let cmd = std::env::var("MAKENSIS").unwrap_or_else(|_| {
        if Path::new(default).exists() {
            default.into()
        } else {
            "makensis".into()
        }
    });
    tool(&cmd, &["/VERSION"]).ok_or_else(|| "winget install NSIS.NSIS".into())
}

/// The AppImage needs at least the glibc it is built against: releases build it on Ubuntu 22.04 (2.35).
fn glibc() -> Check {
    let line = tool("ldd", &["--version"]).unwrap_or_default();
    let version = line.rsplit(' ').next().unwrap_or("").to_string();
    let newer = version
        .split_once('.')
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .is_some_and(|v| v > (2, 35));
    if newer {
        Err(format!(
            "{version}: an AppImage built here needs glibc {version}+ (release.yml builds it on 2.35)"
        ))
    } else {
        Ok(version)
    }
}

fn musl_target() -> Check {
    let installed = Command::new("rustup")
        .current_dir(root())
        .args(["target", "list", "--installed"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("x86_64-unknown-linux-musl"));
    if installed {
        Ok("x86_64-unknown-linux-musl".into())
    } else {
        Err("cargo xtask setup --dist (rustup target add x86_64-unknown-linux-musl)".into())
    }
}
