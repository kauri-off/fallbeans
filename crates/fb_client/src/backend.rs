//! The graphics API, chosen before Bevy starts its renderer. Windows: DirectX 12, Vulkan 1.2+ when it has no
//! DX12 GPU. Elsewhere: Vulkan 1.2+. A real GPU wins over a software one (WARP, lavapipe) on either API. The
//! player may force one (`--backend`, the settings). A saved one whose start drew no frame is not tried again:
//! a marker written before the renderer starts and cleared a few frames in outlives a start that crashes or
//! hangs, and the next start takes the automatic choice.
use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::settings::{Backends, InstanceFlags, WgpuSettings};
use bevy::tasks::block_on;

use crate::opts::{Backend, Opts};
use crate::settings::Graphics;

/// The marker in the profile's directory: the saved backend a start is trying.
const TRIAL: &str = "backend-trial";

impl Backend {
    /// Its name in the settings.
    pub fn setting(self) -> &'static str {
        match self {
            Backend::Vulkan => "vulkan",
            Backend::Dx12 => "dx12",
        }
    }

    /// A saved choice: "" and the names this system cannot run (an old "gl", "dx12" off Windows) are "auto".
    pub fn from_setting(s: &str) -> Option<Backend> {
        [Backend::Vulkan, Backend::Dx12]
            .into_iter()
            .find(|b| b.setting() == s && b.available())
    }

    pub fn available(self) -> bool {
        self != Backend::Dx12 || cfg!(target_os = "windows")
    }

    fn name(self) -> &'static str {
        match self {
            Backend::Vulkan => "Vulkan",
            Backend::Dx12 => "DirectX 12",
        }
    }

    fn wgpu(self) -> Backends {
        match self {
            Backend::Vulkan => Backends::VULKAN,
            Backend::Dx12 => Backends::DX12,
        }
    }
}

/// The automatic choice, in order.
fn auto_order() -> &'static [Backend] {
    if cfg!(target_os = "windows") {
        &[Backend::Dx12, Backend::Vulkan]
    } else {
        &[Backend::Vulkan]
    }
}

/// An adapter a backend offers.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    /// The CPU draws (WARP, llvmpipe/lavapipe): tier T0.
    software: bool,
    /// The device's packed Vulkan version (Vulkan adapters only).
    vulkan: Option<u32>,
}

impl Found {
    /// Vulkan before 1.2 is not enough.
    fn usable(&self) -> bool {
        self.vulkan.is_none_or(|v| vk_version(v) >= (1, 2))
    }
}

/// Major and minor of a packed Vulkan version (the variant in the top 3 bits apart).
fn vk_version(v: u32) -> (u32, u32) {
    ((v >> 22) & 0x7f, (v >> 12) & 0x3ff)
}

#[derive(Clone, Debug, PartialEq)]
struct Pick {
    backend: Backend,
    adapter: String,
    software: bool,
}

/// The first backend in `order` with a real GPU, else the first with a software one.
fn pick(order: &[Backend], mut survey: impl FnMut(Backend) -> Vec<Found>) -> Option<Pick> {
    let mut soft = None;
    for &b in order {
        for f in survey(b).into_iter().filter(Found::usable) {
            let p = Pick {
                backend: b,
                adapter: f.name,
                software: f.software,
            };
            if !p.software {
                return Some(p);
            }
            if soft.is_none() {
                soft = Some(p);
            }
        }
    }
    soft
}

/// The adapters of one backend, from an instance of its own (Bevy makes another for the renderer).
fn survey(b: Backend) -> Vec<Found> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: b.wgpu(),
        flags: wgpu::InstanceFlags::empty().with_env(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    block_on(instance.enumerate_adapters(b.wgpu()))
        .iter()
        .map(|a| {
            let info = a.get_info();
            Found {
                name: info.name,
                software: info.device_type == wgpu::DeviceType::Cpu,
                vulkan: if b == Backend::Vulkan { vulkan_version(a) } else { None },
            }
        })
        .collect()
}

/// The device's Vulkan version: wgpu shows it only on its hal adapter.
#[cfg(any(windows, target_os = "linux", target_os = "android", target_os = "freebsd"))]
#[expect(unsafe_code, reason = "the Vulkan version is only on wgpu-hal's adapter")]
fn vulkan_version(adapter: &wgpu::Adapter) -> Option<u32> {
    // SAFETY: reads the properties wgpu-hal queried when it listed the adapter; nothing is created or destroyed,
    // and the guard is dropped before the adapter.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    Some(hal.physical_device_capabilities().properties().api_version)
}

/// (wgpu has no Vulkan of its own here: the version is not checked.)
#[cfg(not(any(windows, target_os = "linux", target_os = "android", target_os = "freebsd")))]
fn vulkan_version(_: &wgpu::Adapter) -> Option<u32> {
    None
}

#[derive(Debug, PartialEq)]
enum Note {
    Info(String),
    Warn(String),
}

/// What the player asked for.
struct Wish {
    backend: Option<Backend>,
    /// From the settings (not the flag): a start with it leaves the marker.
    saved: bool,
    /// The saved choice goes back to "auto".
    reset: bool,
}

/// The flag, else the saved choice unless this system cannot run it or its last start drew no frame (`failed`:
/// the backend that start tried).
fn wish(flag: Option<Backend>, saved: &str, failed: Option<&str>, notes: &mut Vec<Note>) -> Wish {
    let mut kept = Backend::from_setting(saved);
    let mut reset = false;
    if kept.is_none() && !saved.is_empty() {
        notes.push(Note::Warn(format!(
            "the saved graphics API {saved:?} is not available here: automatic choice"
        )));
        reset = true;
    }
    if let Some(f) = failed {
        notes.push(Note::Warn(format!(
            "the last start with {f:?} drew no frame: automatic choice"
        )));
        reset |= kept.is_some();
        kept = None;
    }
    match flag {
        Some(b) if b.available() => Wish {
            backend: Some(b),
            saved: false,
            reset,
        },
        _ => {
            if let Some(b) = flag {
                notes.push(Note::Warn(format!("--backend {}: not on this system", b.setting())));
            }
            Wish {
                backend: kept,
                saved: kept.is_some(),
                reset,
            }
        }
    }
}

/// A marker left by a start that drew no frame: the backend it tried. Read once.
fn take_trial(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let _ = fs::remove_file(path);
    Some(s.trim().to_string())
}

fn begin_trial(path: &Path, b: Backend) -> bool {
    path.parent().is_some_and(|d| fs::create_dir_all(d).is_ok()) && fs::write(path, b.setting()).is_ok()
}

/// The renderer's settings for this start (`build`, before `RenderPlugin`). Without a GPU it can draw with, the
/// game ends here, with a message to the player.
pub fn choose(app: &mut App, opts: &Opts) -> WgpuSettings {
    let trial = crate::settings::dir(opts.profile.as_deref()).map(|d| d.join(TRIAL));
    let failed = trial.as_deref().and_then(take_trial);
    // (The log is not up yet: what to say waits for `report`.)
    let mut notes = Vec::new();
    let saved = app.world().resource::<Graphics>().backend.clone();
    let wish = wish(opts.backend, &saved, failed.as_deref(), &mut notes);
    let mut surveyed: Vec<(Backend, Vec<Found>)> = Vec::new();
    let mut look = |b: Backend| {
        if let Some((_, f)) = surveyed.iter().find(|(x, _)| *x == b) {
            return f.clone();
        }
        let f = survey(b);
        surveyed.push((b, f.clone()));
        f
    };
    let chosen = match wish.backend {
        Some(b) => pick(&[b], &mut look).or_else(|| {
            notes.push(Note::Warn(format!("{}: no usable GPU: automatic choice", b.name())));
            pick(auto_order(), &mut look)
        }),
        None => pick(auto_order(), &mut look),
    };
    for (_, found) in &surveyed {
        for f in found.iter().filter(|f| !f.usable()) {
            let (major, minor) = vk_version(f.vulkan.unwrap_or_default());
            notes.push(Note::Warn(format!(
                "{}: Vulkan {major}.{minor}, the game needs 1.2",
                f.name
            )));
        }
    }
    let Some(p) = chosen else {
        no_gpu(!opts.offscreen);
    };
    let forced = wish.backend == Some(p.backend);
    let why = match (forced, wish.saved) {
        (false, _) => "automatic",
        (true, true) => "the settings",
        (true, false) => "--backend",
    };
    let soft = if p.software { ", a software device" } else { "" };
    notes.push(Note::Info(format!("{} ({why}): {}{soft}", p.backend.name(), p.adapter)));
    if forced
        && wish.saved
        && let Some(t) = &trial
        && begin_trial(t, p.backend)
    {
        app.insert_resource(Trial(t.clone()));
        app.add_systems(Last, trial_passed.run_if(resource_exists::<Trial>));
    }
    app.insert_resource(Chosen {
        notes,
        reset: wish.reset,
    });
    app.add_systems(Startup, report);
    let mut wgpu = WgpuSettings {
        backends: Some(p.backend.wgpu()),
        ..default()
    };
    // (Bevy keeps wgpu's indirect-call validation in release builds only where DX12 may run, which needs it;
    // its default reckons with every backend.)
    if !cfg!(debug_assertions) && p.backend != Backend::Dx12 {
        wgpu.instance_flags.remove(InstanceFlags::VALIDATION_INDIRECT_CALL);
    }
    wgpu
}

#[derive(Resource)]
struct Chosen {
    notes: Vec<Note>,
    reset: bool,
}

/// The choice into the log, and a saved one that cannot run back to "auto".
fn report(mut chosen: ResMut<Chosen>, mut gfx: ResMut<Graphics>, mut commands: Commands) {
    for n in chosen.notes.drain(..) {
        match n {
            Note::Info(s) => info!("graphics: {s}"),
            Note::Warn(s) => warn!("graphics: {s}"),
        }
    }
    if chosen.reset {
        gfx.backend.clear();
        crate::settings::save_soon(&mut commands);
    }
}

/// The marker of this start.
#[derive(Resource)]
struct Trial(PathBuf);

/// A few frames in, the backend works: the marker goes. (The third: with pipelined rendering the render world
/// draws a frame behind the main one.)
fn trial_passed(mut commands: Commands, trial: Res<Trial>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames >= 3 {
        let _ = fs::remove_file(&trial.0);
        commands.remove_resource::<Trial>();
    }
}

/// No GPU the game can draw with: said on stderr and, with a window, in a dialog.
fn no_gpu(dialog: bool) -> ! {
    let needs = if cfg!(target_os = "windows") {
        "DirectX 12 or Vulkan 1.2"
    } else {
        "Vulkan 1.2"
    };
    eprintln!("graphics: no GPU with {needs}: the game cannot start");
    eprintln!("{}", crate::ui::text::NO_GPU);
    if dialog {
        tell(crate::ui::text::NO_GPU);
    }
    std::process::exit(1)
}

/// A message box (user32, which winit links anyway).
#[cfg(windows)]
#[expect(unsafe_code, reason = "one call into user32: no crate for it")]
fn tell(text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(owner: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide("Fall Beans"));
    // (MB_ICONERROR | MB_SETFOREGROUND.)
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner window.
    unsafe {
        MessageBoxW(core::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10 | 0x1_0000);
    }
}

/// Whichever dialog the desktop has (GNOME's, then KDE's); without either, stderr only.
#[cfg(not(windows))]
fn tell(text: &str) {
    use std::process::Command;
    let zenity = Command::new("zenity")
        .args(["--error", "--title=Fall Beans"])
        .arg(format!("--text={text}"))
        .status();
    if zenity.is_err() {
        let _ = Command::new("kdialog")
            .args(["--title", "Fall Beans", "--error", text])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, software: bool, vulkan: Option<(u32, u32)>) -> Found {
        Found {
            name: name.into(),
            software,
            vulkan: vulkan.map(|(major, minor)| (major << 22) | (minor << 12)),
        }
    }

    fn on(dx12: &[Found], vulkan: &[Found]) -> Option<(Backend, String)> {
        let order = [Backend::Dx12, Backend::Vulkan];
        pick(&order, |b| match b {
            Backend::Dx12 => dx12.to_vec(),
            Backend::Vulkan => vulkan.to_vec(),
        })
        .map(|p| (p.backend, p.adapter))
    }

    #[test]
    fn a_real_gpu_wins_and_vulkan_needs_1_2() {
        let warp = gpu("WARP", true, None);
        let radeon = gpu("Radeon", false, None);
        let new = gpu("Arc", false, Some((1, 3)));
        let old = gpu("HD 4000", false, Some((1, 0)));
        let lavapipe = gpu("llvmpipe", true, Some((1, 4)));
        let to = |b: Backend, name: &str| Some((b, name.to_string()));
        assert_eq!(
            on(&[warp.clone(), radeon], std::slice::from_ref(&new)),
            to(Backend::Dx12, "Radeon")
        );
        assert_eq!(on(std::slice::from_ref(&warp), &[new]), to(Backend::Vulkan, "Arc"));
        assert_eq!(on(&[warp], std::slice::from_ref(&old)), to(Backend::Dx12, "WARP"));
        assert_eq!(on(&[], &[old.clone(), lavapipe]), to(Backend::Vulkan, "llvmpipe"));
        assert_eq!(on(&[], &[old]), None);
        assert_eq!(on(&[], &[]), None);
        // (The variant bits are not the major version.)
        assert_eq!(vk_version((1 << 29) | (1 << 22) | (3 << 12) | 5), (1, 3));
    }

    #[test]
    fn saved_choices_that_cannot_run_are_auto() {
        let mut notes = Vec::new();
        let w = |flag, saved, failed, notes: &mut Vec<Note>| {
            let w = wish(flag, saved, failed, notes);
            (w.backend, w.saved, w.reset)
        };
        assert_eq!(w(None, "", None, &mut notes), (None, false, false));
        assert!(notes.is_empty());
        assert_eq!(w(None, "gl", None, &mut notes), (None, false, true));
        assert_eq!(notes.len(), 1);
        assert_eq!(
            w(None, "vulkan", None, &mut notes),
            (Some(Backend::Vulkan), true, false)
        );
        assert_eq!(w(None, "vulkan", Some("vulkan"), &mut notes), (None, false, true));
        assert_eq!(
            w(Some(Backend::Vulkan), "", None, &mut notes),
            (Some(Backend::Vulkan), false, false)
        );
        assert_eq!(
            w(Some(Backend::Vulkan), "vulkan", Some("vulkan"), &mut notes),
            (Some(Backend::Vulkan), false, true)
        );
        assert_eq!(Backend::from_setting("dx12").is_some(), cfg!(target_os = "windows"));
        assert_eq!(Backend::from_setting("vulkan"), Some(Backend::Vulkan));
    }

    #[test]
    fn the_trial_marker_is_read_once() {
        let dir = std::env::temp_dir().join(format!("fb-backend-{}", std::process::id()));
        let path = dir.join(TRIAL);
        assert!(begin_trial(&path, Backend::Vulkan));
        assert_eq!(take_trial(&path).as_deref(), Some("vulkan"));
        assert_eq!(take_trial(&path), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
