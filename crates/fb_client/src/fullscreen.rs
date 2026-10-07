//! Fullscreen: borderless on the current monitor by default (`Display::fullscreen`), F11 or Alt+Enter switch it,
//! and `--windowed` / `--fullscreen` decide for one run without touching the setting.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};

use crate::opts::Opts;
use crate::settings::Display;

pub struct FullscreenPlugin;

impl Plugin for FullscreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (keys, apply).chain());
    }
}

/// The window mode at the start: the run's flags first, then the setting.
pub fn mode(opts: &Opts, display: Option<&Display>) -> WindowMode {
    if wanted(opts, display.is_none_or(|d| d.fullscreen)) {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    }
}

fn wanted(opts: &Opts, setting: bool) -> bool {
    !opts.windowed && (opts.fullscreen || setting)
}

/// F11 or Alt+Enter flips the setting (saved like any other).
fn keys(keys: Res<ButtonInput<KeyCode>>, mut display: ResMut<Display>, mut commands: Commands) {
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if keys.just_pressed(KeyCode::F11) || (alt && keys.just_pressed(KeyCode::Enter)) {
        display.fullscreen ^= true;
        crate::settings::save_soon(&mut commands);
    }
}

/// The setting as it changes (the key, the settings tab). A run with `--windowed` or `--fullscreen` keeps its
/// mode until the setting is changed in it.
fn apply(
    display: Res<Display>,
    opts: Res<Opts>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut seen: Local<Option<bool>>,
) {
    let first = seen.is_none();
    if seen.replace(display.fullscreen) == Some(display.fullscreen) {
        return;
    }
    let on = if first {
        wanted(&opts, display.fullscreen)
    } else {
        display.fullscreen
    };
    let mode = if on {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    for mut w in &mut windows {
        if w.mode != mode {
            w.mode = mode;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn flags_over_the_setting() {
        let o = |args: &[&str]| Opts::parse_from([&["fb_client"], args].concat());
        assert!(wanted(&o(&[]), true));
        assert!(!wanted(&o(&[]), false));
        assert!(!wanted(&o(&["--windowed"]), true));
        assert!(wanted(&o(&["--fullscreen"]), false));
        assert!(Opts::try_parse_from(["fb_client", "--windowed", "--fullscreen"]).is_err());
    }
}
