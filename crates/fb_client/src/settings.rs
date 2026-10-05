//! What the player keeps between runs (port of `settings.ts`): identity, name and look, controls, sound,
//! display. One file per profile in the system's settings directory; `--profile b` is a second player on
//! the same machine. Flags (`--name`, `--token`, `--color`) win over the file for the run and are not saved.
use core::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::settings::{
    ReflectSettingsGroup, SaveSettings, SaveSettingsDeferred, SaveSettingsSync, SettingsGroup, SettingsPlugin,
};
use fb_proto::Outfit;
use fb_shared::COLORS;
use fb_shared::outfit::{Glasses, Hat, Tint};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::keys::Bind;
use crate::opts::Opts;

const APP: &str = "io.github.kauri-off.fallbeans";

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "player")]
pub struct Player {
    /// The identity token the server gave: the same player next time (`fb_id` in TS).
    pub identity: String,
    pub name: String,
    /// The suit colour picked last (index into `COLORS`, -1 none yet): rooms give it when it is free.
    pub color: i32,
    /// The outfit, its parts by name (`Cap`, `Shades`, `Pink`; empty for none).
    pub hat: String,
    pub hat_color: String,
    pub glasses: String,
    pub belly: String,
    pub shoes: String,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            identity: String::new(),
            name: String::new(),
            color: -1,
            hat: String::new(),
            hat_color: String::new(),
            glasses: String::new(),
            belly: String::new(),
            shoes: String::new(),
        }
    }
}

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "controls")]
pub struct Controls {
    pub mouse_sensitivity: f32,
    pub invert_mouse_y: bool,
    pub stick_sensitivity: f32,
    pub invert_stick_y: bool,
    /// The camera shakes from hits and hard landings.
    pub camera_shake: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            invert_mouse_y: false,
            stick_sensitivity: 1.0,
            invert_stick_y: false,
            camera_shake: true,
        }
    }
}

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "sound")]
pub struct Sound {
    /// 0…1.
    pub volume: f32,
}

impl Default for Sound {
    fn default() -> Self {
        Self { volume: 0.8 }
    }
}

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "display")]
pub struct Display {
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Interface size on top of the one that follows the window (0.75…1.5).
    pub ui_scale: f32,
    pub show_fps: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            fov: 70.0,
            ui_scale: 1.0,
            show_fps: false,
        }
    }
}

/// Graphics (`render/quality.rs`): a preset for the hardware tier, and switches that only ever take work
/// away from it.
#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "graphics")]
pub struct Graphics {
    /// "auto" (by the hardware, lowered when frames run slow), "low", "medium" or "high".
    pub preset: String,
    pub shadows: bool,
    pub ao: bool,
    /// Anti-aliasing (TAA, SMAA or FXAA, by the preset).
    pub aa: bool,
    /// Saturation, contrast and vignette.
    pub grade: bool,
    /// Specks drifting in the air.
    pub motes: bool,
    /// Render scale with FSR 1 upscaling: "off", "ultra", "quality", "balanced", "performance".
    pub upscale: String,
    pub vsync: bool,
    /// Frames a second at most (0: no limit).
    pub fps_limit: u32,
    /// "" (wgpu's pick), "vulkan", "dx12" or "gl": takes effect on the next start.
    pub backend: String,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            preset: "auto".into(),
            shadows: true,
            ao: true,
            aa: true,
            grade: true,
            motes: true,
            upscale: String::new(),
            vsync: true,
            fps_limit: 0,
            backend: String::new(),
        }
    }
}

/// Which keys do what (`keys.rs`): key names separated by spaces.
#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "keys")]
pub struct Bindings {
    pub forward: String,
    pub back: String,
    pub left: String,
    pub right: String,
    pub jump: String,
    pub dive: String,
    pub grab: String,
}

impl Default for Bindings {
    fn default() -> Self {
        let d = |b: Bind| b.defaults().to_string();
        Self {
            forward: d(Bind::Forward),
            back: d(Bind::Back),
            left: d(Bind::Left),
            right: d(Bind::Right),
            jump: d(Bind::Jump),
            dive: d(Bind::Dive),
            grab: d(Bind::Grab),
        }
    }
}

impl Bindings {
    fn slot(&mut self, b: Bind) -> &mut String {
        match b {
            Bind::Forward => &mut self.forward,
            Bind::Back => &mut self.back,
            Bind::Left => &mut self.left,
            Bind::Right => &mut self.right,
            Bind::Jump => &mut self.jump,
            Bind::Dive => &mut self.dive,
            Bind::Grab => &mut self.grab,
        }
    }

    /// The keys of an action (its defaults if the file names none that can be bound).
    pub fn keys(&self, b: Bind) -> Vec<KeyCode> {
        let s = match b {
            Bind::Forward => &self.forward,
            Bind::Back => &self.back,
            Bind::Left => &self.left,
            Bind::Right => &self.right,
            Bind::Jump => &self.jump,
            Bind::Dive => &self.dive,
            Bind::Grab => &self.grab,
        };
        let keys = crate::keys::parse(s);
        if keys.is_empty() {
            crate::keys::parse(b.defaults())
        } else {
            keys
        }
    }

    /// Binds one key to an action; another action that had it lets it go.
    pub fn bind(&mut self, b: Bind, key: KeyCode) {
        for other in crate::keys::BINDS {
            if other != b {
                let mut keys = self.keys(other);
                if keys.contains(&key) {
                    keys.retain(|k| *k != key);
                    *self.slot(other) = crate::keys::names(&keys);
                }
            }
        }
        *self.slot(b) = crate::keys::names(&[key]);
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn to_name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn from_name<T: DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.into())).ok()
}

impl Player {
    pub fn outfit(&self) -> Outfit {
        Outfit {
            hat: from_name(&self.hat).unwrap_or(Hat::None),
            hat_color: from_name::<Tint>(&self.hat_color),
            glasses: from_name(&self.glasses).unwrap_or(Glasses::None),
            belly: from_name::<Tint>(&self.belly),
            shoes: from_name::<Tint>(&self.shoes),
        }
    }

    pub fn set_outfit(&mut self, o: &Outfit) {
        let tint = |t: &Option<Tint>| t.as_ref().map_or_else(String::new, to_name);
        self.hat = to_name(&o.hat);
        self.hat_color = tint(&o.hat_color);
        self.glasses = to_name(&o.glasses);
        self.belly = tint(&o.belly);
        self.shoes = tint(&o.shoes);
    }

    pub fn color(&self) -> Option<u8> {
        u8::try_from(self.color).ok().filter(|c| (*c as usize) < COLORS.len())
    }
}

/// The player as this run sees them: the file, with the flags on top.
#[derive(SystemParam)]
pub struct Me<'w> {
    pub player: Res<'w, Player>,
    pub opts: Res<'w, Opts>,
}

impl Me<'_> {
    pub fn name(&self) -> String {
        if self.opts.name.is_empty() {
            self.player.name.clone()
        } else {
            self.opts.name.clone()
        }
    }

    pub fn identity(&self) -> Option<String> {
        self.opts
            .token
            .clone()
            .or_else(|| Some(self.player.identity.clone()).filter(|s| !s.is_empty()))
    }

    pub fn color(&self) -> Option<u8> {
        self.opts.color.or_else(|| self.player.color())
    }
}

/// Settings changed: written a moment later (a slider being dragged writes once).
pub fn save_soon(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(500)));
}

pub fn save_now(commands: &mut Commands) {
    commands.queue(SaveSettings::IfChanged);
}

fn store_name(profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{APP}/profile-{p}"),
        None => APP.to_string(),
    }
}

/// The profile's directory in the system's settings directory (where its `settings.toml` is).
pub fn dir(profile: Option<&str>) -> Option<std::path::PathBuf> {
    bevy::platform::dirs::preferences_dir().map(|d| d.join(store_name(profile)))
}

pub struct ClientSettingsPlugin {
    pub profile: Option<String>,
    /// Without a file (headless clients of stress runs): defaults, nothing written.
    pub stored: bool,
}

impl Plugin for ClientSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Player>()
            .register_type::<Controls>()
            .register_type::<Sound>()
            .register_type::<Display>()
            .register_type::<Bindings>()
            .register_type::<Graphics>()
            .register_type::<crate::servers::Servers>();
        if self.stored {
            app.add_plugins(SettingsPlugin::new(&store_name(self.profile.as_deref())));
            app.add_systems(Last, save_on_exit);
        }
        app.init_resource::<Player>()
            .init_resource::<Controls>()
            .init_resource::<Sound>()
            .init_resource::<Display>()
            .init_resource::<Bindings>()
            .init_resource::<Graphics>()
            .init_resource::<crate::servers::Servers>();
    }
}

fn save_on_exit(mut exit: MessageReader<AppExit>, mut commands: Commands) {
    if exit.read().next().is_some() {
        commands.queue(SaveSettingsSync::IfChanged);
    }
}
