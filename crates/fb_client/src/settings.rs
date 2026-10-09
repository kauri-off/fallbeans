//! What the player keeps between runs: name and look, controls, sound, display (`settings.toml`), and the
//! identity each server gave (`identity.json`). One folder per profile in the system's settings directory;
//! `--profile b` is a second player on the same machine. Flags (`--name`, `--token`, `--color`) win over the
//! files for the run and are not saved.
use core::time::Duration;
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SaveSettingsDeferred, SaveSettingsSync, SettingsGroup, SettingsPlugin};
use fb_proto::Outfit;
use fb_shared::COLORS;
use fb_shared::outfit::{Glasses, Hat, Tint};
use serde::de::IntoDeserializer;
use serde::de::value::StrDeserializer;
use serde::{Deserialize, Serialize};

use crate::backend::BackendSetting;
use crate::keys::{Bind, Keys};
use crate::opts::Opts;
use crate::render::quality::{Preset, Upscale};
use crate::render::upscale::UpscalerSetting;
use crate::servers::{Servers, Target};

const APP: &str = "io.github.kauri-off.fallbeans";
/// The identities, beside `settings.toml`.
const IDENTITY_FILE: &str = "identity.json";

/// What the sliders allow (`ui::settings_tab`); a file with anything else is brought back into them.
pub const SENS_RANGE: (f32, f32) = (0.2, 3.0);
pub const FOV_RANGE: (f32, f32) = (55.0, 100.0);
pub const UI_SCALE_RANGE: (f32, f32) = (0.75, 1.5);

#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "player")]
pub struct Player {
    /// The one identity token of older versions, whichever server gave it last: moved into `Identities` at
    /// the start (`migrate`), then empty.
    pub identity: String,
    pub name: String,
    /// The suit colour picked last (index into `COLORS`): rooms give it when it is free.
    pub color: ColorSetting,
    pub hat: HatSetting,
    pub hat_color: TintSetting,
    pub glasses: GlassesSetting,
    pub belly: TintSetting,
    pub shoes: TintSetting,
}

/// The suit colour in the file: -1 for none yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct ColorSetting(pub Option<u8>);

impl Serialize for ColorSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.0.map_or(-1, i64::from))
    }
}

impl<'de> Deserialize<'de> for ColorSetting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(u8::try_from(i64::deserialize(d)?).ok()))
    }
}

/// The hat by name (`Cap`); one not known does not load: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(transparent)]
pub struct HatSetting(pub Hat);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(transparent)]
pub struct GlassesSetting(pub Glasses);

/// A part's tint by name (`Pink`); "" (and any name not known) for the part's own colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct TintSetting(pub Option<Tint>);

impl Serialize for TintSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Some(t) => t.serialize(s),
            None => s.serialize_str(""),
        }
    }
}

impl<'de> Deserialize<'de> for TintSetting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        let de: StrDeserializer<'_, D::Error> = name.as_str().into_deserializer();
        Ok(Self(Tint::deserialize(de).ok()))
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
    /// Borderless fullscreen on the current monitor (F11 or Alt+Enter); off: a 1280×720 window.
    pub fullscreen: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            fov: 70.0,
            ui_scale: 1.0,
            show_fps: false,
            fullscreen: true,
        }
    }
}

/// Graphics (`render/quality.rs`): one of two presets, the upscaler and its mode, the frame pacing and the API. The
/// rest is not the player's: upscaling is always on, and the switches below are only for the perf sweep, which
/// takes the features away one at a time (`perf/capture.rs`); they are never saved, and an old file's are ignored.
#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "graphics")]
pub struct Graphics {
    /// Anything else in the file (an old "auto" or "medium") does not load: High.
    pub preset: Preset,
    pub vsync: bool,
    /// Frames a second at most (0: no limit).
    pub fps_limit: u32,
    /// Takes effect on the next start; one this system cannot run (an old "gl") is automatic (`backend.rs`).
    pub backend: BackendSetting,
    /// Switched in play; one this machine does not offer is automatic (`render/upscale.rs`).
    pub upscaler: UpscalerSetting,
    /// Switched in play; the sweep tries `Performance`, which the file does not keep.
    pub upscale: Upscale,
    #[reflect(ignore)]
    pub shadows: bool,
    /// Anti-aliasing (SMAA on High, FXAA on Low).
    #[reflect(ignore)]
    pub aa: bool,
    /// Saturation, contrast, vignette and bloom.
    #[reflect(ignore)]
    pub grade: bool,
    /// Specks drifting in the air.
    #[reflect(ignore)]
    pub motes: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            preset: Preset::default(),
            vsync: true,
            fps_limit: 0,
            backend: BackendSetting::default(),
            upscaler: UpscalerSetting::default(),
            upscale: Upscale::default(),
            shadows: true,
            aa: true,
            grade: true,
            motes: true,
        }
    }
}

/// Which keys do what (`keys.rs`).
#[derive(Resource, SettingsGroup, Reflect, Clone, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "keys")]
pub struct Bindings {
    pub forward: Keys,
    pub back: Keys,
    pub left: Keys,
    pub right: Keys,
    pub jump: Keys,
    pub dive: Keys,
    pub grab: Keys,
}

impl Default for Bindings {
    fn default() -> Self {
        let d = |b: Bind| Keys(b.defaults().to_vec());
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
    fn slot(&mut self, b: Bind) -> &mut Vec<KeyCode> {
        match b {
            Bind::Forward => &mut self.forward.0,
            Bind::Back => &mut self.back.0,
            Bind::Left => &mut self.left.0,
            Bind::Right => &mut self.right.0,
            Bind::Jump => &mut self.jump.0,
            Bind::Dive => &mut self.dive.0,
            Bind::Grab => &mut self.grab.0,
        }
    }

    /// The keys of an action (none if it was left without one).
    pub fn keys(&self, b: Bind) -> &[KeyCode] {
        let k = match b {
            Bind::Forward => &self.forward,
            Bind::Back => &self.back,
            Bind::Left => &self.left,
            Bind::Right => &self.right,
            Bind::Jump => &self.jump,
            Bind::Dive => &self.dive,
            Bind::Grab => &self.grab,
        };
        &k.0
    }

    /// Binds one key to an action; another action that had it lets it go. One left without a key takes the one
    /// this action had (the two swap), or none at all.
    pub fn bind(&mut self, b: Bind, key: KeyCode) {
        let old = self.keys(b).to_vec();
        for other in crate::keys::BINDS {
            if other == b {
                continue;
            }
            let keys = self.slot(other);
            if !keys.contains(&key) {
                continue;
            }
            keys.retain(|k| *k != key);
            if keys.is_empty() {
                keys.extend(old.iter().copied().filter(|k| *k != key).take(1));
            }
        }
        *self.slot(b) = vec![key];
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Player {
    pub fn outfit(&self) -> Outfit {
        Outfit {
            hat: self.hat.0,
            hat_color: self.hat_color.0,
            glasses: self.glasses.0,
            belly: self.belly.0,
            shoes: self.shoes.0,
        }
    }

    pub fn set_outfit(&mut self, o: &Outfit) {
        self.hat = HatSetting(o.hat);
        self.hat_color = TintSetting(o.hat_color);
        self.glasses = GlassesSetting(o.glasses);
        self.belly = TintSetting(o.belly);
        self.shoes = TintSetting(o.shoes);
    }

    pub fn color(&self) -> Option<u8> {
        self.color.0.filter(|c| usize::from(*c) < COLORS.len())
    }
}

/// The player's settings and this run's options, to change.
#[derive(SystemParam)]
pub struct Profile<'w> {
    pub player: ResMut<'w, Player>,
    pub opts: ResMut<'w, Opts>,
}

/// The player as this run sees them: the files, with the flags on top.
#[derive(SystemParam)]
pub struct Me<'w> {
    pub player: Res<'w, Player>,
    pub opts: Res<'w, Opts>,
    pub ids: Res<'w, Identities>,
    target: Res<'w, Target>,
}

impl Me<'_> {
    pub fn name(&self) -> String {
        self.opts.name.clone().unwrap_or_else(|| self.player.name.clone())
    }

    /// The identity to send the server being played on (`Target`): `--token`, or the one that server gave.
    pub fn identity(&self) -> Option<String> {
        self.opts
            .token
            .clone()
            .or_else(|| self.target.0.as_deref().and_then(|http| self.ids.get(http)))
    }

    pub fn color(&self) -> Option<u8> {
        self.opts.color.or_else(|| self.player.color())
    }
}

/// Which server an identity belongs to: the host (lower case), the port unless it is the scheme's own or the
/// game's HTTP port, and the path unless it is the usual `/fallbeans`. A domain's two candidates
/// (`https://host/fallbeans`, `http://host:5887/fallbeans`) are the same server.
pub fn server_key(http: &str) -> String {
    let http = http.trim();
    let (scheme, rest) = http.split_once("://").unwrap_or(("http", http));
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match rest.find('/') {
        Some(i) => (rest.get(..i).unwrap_or(rest), rest.get(i..).unwrap_or("")),
        None => (rest, ""),
    };
    let authority = authority.to_ascii_lowercase();
    let (host, port) = match authority.find(']') {
        // `[v6]` or `[v6]:port`.
        Some(end) => (
            authority.get(..=end).unwrap_or(&authority),
            authority.get(end + 1..).and_then(|p| p.strip_prefix(':')),
        ),
        None => match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority.as_str(), None),
        },
    };
    let port = port.and_then(|p| p.parse::<u16>().ok());
    let implied = match (scheme.as_str(), port) {
        (_, None) | ("https", Some(443)) => true,
        ("http", Some(p)) => p == 80 || p == fb_net::HTTP_PORT,
        _ => false,
    };
    let mut key = host.to_string();
    if let Some(p) = port.filter(|_| !implied) {
        key.push_str(&format!(":{p}"));
    }
    let path = path.trim_end_matches('/');
    if !path.is_empty() && path != "/fallbeans" {
        key.push_str(path);
    }
    key
}

/// The identity tokens servers gave, one per server (`server_key`): each server is sent only its own, so one
/// server never sees (and cannot play as) the player's identity on another, and a visit to one keeps the
/// player's room on the other. Kept in `identity.json` of the profile, written whole and atomically: a broken
/// `settings.toml` cannot take it.
#[derive(Resource, Default)]
pub struct Identities {
    by_server: BTreeMap<String, String>,
    /// None: nothing is written (tests, headless clients).
    file: Option<PathBuf>,
}

/// Why the identity file was not written.
#[derive(Debug, thiserror::Error)]
enum SaveError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{}: {error}", .path.display())]
    Write { path: PathBuf, error: std::io::Error },
}

#[derive(Serialize, Deserialize, Default)]
struct IdentityFile {
    servers: BTreeMap<String, String>,
}

impl Identities {
    /// The token the server whose HTTP API is `http` gave.
    pub fn get(&self, http: &str) -> Option<String> {
        self.by_server.get(&server_key(http)).filter(|t| !t.is_empty()).cloned()
    }

    /// Keeps the token the server at `http` gave (written at once when it is new).
    pub fn set(&mut self, http: &str, token: &str) {
        let key = server_key(http);
        if token.is_empty() || self.by_server.get(&key).is_some_and(|t| t == token) {
            return;
        }
        self.by_server.insert(key, token.to_string());
        if let Err(e) = self.save() {
            warn!("identity: {e}");
        }
    }

    fn save(&self) -> Result<(), SaveError> {
        let Some(path) = &self.file else { return Ok(()) };
        let file = IdentityFile {
            servers: self.by_server.clone(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        write_atomic(path, json.as_bytes()).map_err(|error| SaveError::Write {
            path: path.clone(),
            error,
        })
    }

    /// The file at `path` (none yet: no identities); one that cannot be read is set aside, said in `notes`.
    fn load(path: PathBuf, notes: &mut Vec<String>) -> Self {
        let by_server = match fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<IdentityFile>(&s) {
                Ok(f) => f.servers,
                Err(e) => {
                    notes.push(set_aside(&path, &e.to_string()));
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                notes.push(set_aside(&path, &e.to_string()));
                BTreeMap::new()
            }
        };
        Self {
            by_server,
            file: Some(path),
        }
    }
}

/// `bytes` as the whole of `path`, or the file as it was: a temporary file, flushed to the disk, renamed over it.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written?;
    // (The rename itself, on the disk.)
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// Moves a file that cannot be read out of the way (`<name>.bad-<time>`) before defaults are written over it.
fn set_aside(path: &Path, why: &str) -> String {
    let mut to = path.as_os_str().to_owned();
    to.push(format!(".bad-{}", crate::logs::stamp(crate::logs::now_secs())));
    let to = PathBuf::from(to);
    match fs::rename(path, &to) {
        Ok(()) => format!("{} cannot be read ({why}): kept as {}", path.display(), to.display()),
        Err(e) => format!("{} cannot be read ({why}), nor set aside: {e}", path.display()),
    }
}

/// A broken `settings.toml` (a write cut short, an edit by hand): bevy-settings would load defaults and write
/// them over it at the next save, so it is set aside first.
fn keep_unreadable(path: &Path) -> Option<String> {
    let why = match fs::read_to_string(path) {
        Ok(s) => match s.parse::<toml::Table>() {
            Ok(_) => return None,
            Err(e) => e.to_string(),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => e.to_string(),
    };
    Some(set_aside(path, &why))
}

/// The single identity of older versions goes to the server it most likely came from: the one played on last,
/// else the only one in the list. With neither it is dropped (the player is new there next time): sent to a
/// server that did not issue it, it would only hand that server's operator a token for another server.
fn migrate(world: &mut World, notes: &mut Vec<String>) {
    let old = world.resource::<Player>().identity.clone();
    if old.is_empty() {
        return;
    }
    let servers = world.resource::<Servers>();
    let from = servers
        .last
        .clone()
        .or_else(|| servers.list.first().cloned().filter(|_| servers.list.len() == 1));
    let base = from.and_then(|s| crate::servers::candidates(&s).into_iter().next());
    if let Some(base) = base {
        let key = server_key(&base);
        let mut ids = world.resource_mut::<Identities>();
        // (Already there: moved on an earlier start that did not get to save the settings.)
        if !ids.by_server.contains_key(&key) {
            ids.by_server.insert(key.clone(), old);
            if let Err(e) = ids.save() {
                // Left in the settings until it can be written.
                notes.push(format!("identity: {e}"));
                return;
            }
        }
        notes.push(format!("identity: the old one is {key}'s now"));
    } else {
        notes.push("identity: the old one names no server, dropped".into());
    }
    world.resource_mut::<Player>().identity.clear();
}

/// Values no slider gives (a NaN, a huge sensitivity written by hand) back into range: the camera and the sound
/// take them as they are, and `clamp` lets a NaN through.
fn sanitize(world: &mut World) {
    let fit = |v: f32, (lo, hi): (f32, f32), default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
    let (dc, ds, dd) = (Controls::default(), Sound::default(), Display::default());
    let c = world.resource::<Controls>().clone();
    let fixed = Controls {
        mouse_sensitivity: fit(c.mouse_sensitivity, SENS_RANGE, dc.mouse_sensitivity),
        stick_sensitivity: fit(c.stick_sensitivity, SENS_RANGE, dc.stick_sensitivity),
        ..c
    };
    if fixed != c {
        *world.resource_mut::<Controls>() = fixed;
    }
    let s = world.resource::<Sound>().clone();
    let fixed = Sound {
        volume: fit(s.volume, (0.0, 1.0), ds.volume),
    };
    if fixed != s {
        *world.resource_mut::<Sound>() = fixed;
    }
    let d = world.resource::<Display>().clone();
    let fixed = Display {
        fov: fit(d.fov, FOV_RANGE, dd.fov),
        ui_scale: fit(d.ui_scale, UI_SCALE_RANGE, dd.ui_scale),
        ..d
    };
    if fixed != d {
        *world.resource_mut::<Display>() = fixed;
    }
    // (Older files say "" for none.)
    if let Some(mut s) = world.get_resource_mut::<Servers>()
        && s.last.as_deref() == Some("")
    {
        s.last = None;
    }
    if !Upscale::PICKS.contains(&world.resource::<Graphics>().upscale) {
        world.resource_mut::<Graphics>().upscale = Upscale::default();
    }
}

/// What loading the files found (a broken file set aside, the identity moved): logged once the log is up.
#[derive(Resource, Default)]
struct LoadNotes(Vec<String>);

fn tell_notes(notes: Res<LoadNotes>) {
    for n in &notes.0 {
        warn!("settings: {n}");
    }
}

/// Settings changed: written a moment later (a slider being dragged writes once).
pub fn save_soon(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(500)));
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
        let mut notes = Vec::new();
        let dir = dir(self.profile.as_deref()).filter(|_| self.stored);
        if self.stored {
            if let Some(n) = dir.as_ref().and_then(|d| keep_unreadable(&d.join("settings.toml"))) {
                notes.push(n);
            }
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
        sanitize(app.world_mut());
        let ids = match &dir {
            Some(d) => Identities::load(d.join(IDENTITY_FILE), &mut notes),
            None => Identities::default(),
        };
        app.insert_resource(ids);
        migrate(app.world_mut(), &mut notes);
        app.insert_resource(LoadNotes(notes));
        app.add_systems(Startup, tell_notes);
    }
}

pub fn save_on_exit(mut exit: MessageReader<AppExit>, mut commands: Commands) {
    if exit.read().next().is_some() {
        commands.queue(SaveSettingsSync::IfChanged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for one test (removed at the end).
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fb-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn set_aside_files(dir: &Path, name: &str) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("{name}.bad-")))
            .count()
    }

    #[test]
    fn server_keys() {
        let k = server_key;
        assert_eq!(k("http://127.0.0.1:7000/fallbeans"), "127.0.0.1:7000");
        assert_eq!(k("http://127.0.0.1:5887/fallbeans"), "127.0.0.1");
        assert_eq!(k("https://Game.Example.com/fallbeans"), "game.example.com");
        assert_eq!(k("http://game.example.com:5887/fallbeans/"), "game.example.com");
        assert_eq!(k("http://[::1]:7000/fallbeans"), "[::1]:7000");
        assert_eq!(k("http://[::1]:5887/fallbeans"), "[::1]");
        assert_eq!(k("https://h:8443/x/fallbeans"), "h:8443/x/fallbeans");
        assert_ne!(k("http://h:7000/fallbeans"), k("http://h:7001/fallbeans"));
    }

    /// #4: each server is sent only the token it gave, and a visit to another keeps it.
    #[test]
    fn an_identity_per_server() {
        let mut ids = Identities::default();
        let (a, b) = ("http://127.0.0.1:7000/fallbeans", "http://127.0.0.1:7001/fallbeans");
        ids.set(a, "token-a");
        assert_eq!(ids.get(b), None, "server B is not sent A's token");
        ids.set(b, "token-b");
        assert_eq!(ids.get(a).as_deref(), Some("token-a"));
        assert_eq!(ids.get(b).as_deref(), Some("token-b"));
        // A domain's two addresses are one server.
        ids.set("https://game.example.com/fallbeans", "token-c");
        assert_eq!(
            ids.get("http://game.example.com:5887/fallbeans").as_deref(),
            Some("token-c")
        );
    }

    #[test]
    fn identities_survive_a_broken_file() {
        let dir = scratch("ids");
        let path = dir.join(IDENTITY_FILE);
        fs::write(&path, "{\"servers\": {\"127.0.0.1:70").unwrap();
        let mut notes = Vec::new();
        let mut ids = Identities::load(path.clone(), &mut notes);
        assert_eq!(ids.get("http://127.0.0.1:7000/fallbeans"), None);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(set_aside_files(&dir, IDENTITY_FILE), 1, "the broken file is kept");
        ids.set("http://127.0.0.1:7000/fallbeans", "t");
        let again = Identities::load(path, &mut notes);
        assert_eq!(again.get("http://127.0.0.1:7000").as_deref(), Some("t"));
        assert_eq!(notes.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A settings file cut short is set aside before bevy-settings writes defaults over it.
    #[test]
    fn a_broken_settings_file_is_kept() {
        let dir = scratch("toml");
        let path = dir.join("settings.toml");
        assert_eq!(keep_unreadable(&path), None, "no file yet");
        fs::write(&path, "[player]\nname = \"Боб\"\n").unwrap();
        assert_eq!(keep_unreadable(&path), None);
        fs::write(&path, "[player]\nname = \"Бо").unwrap();
        assert!(keep_unreadable(&path).is_some());
        assert!(!path.exists());
        assert_eq!(set_aside_files(&dir, "settings.toml"), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn world_with(identity: &str, servers: Servers) -> World {
        let mut w = World::new();
        w.insert_resource(Player {
            identity: identity.into(),
            ..default()
        });
        w.insert_resource(servers);
        w.insert_resource(Identities::default());
        w
    }

    #[test]
    fn the_old_identity_goes_to_the_last_server() {
        let mut w = world_with(
            "old",
            Servers {
                list: vec!["10.0.0.1:7000".into(), "10.0.0.2".into()],
                last: Some("10.0.0.1:7000".into()),
            },
        );
        migrate(&mut w, &mut Vec::new());
        let ids = w.resource::<Identities>();
        assert_eq!(ids.get("http://10.0.0.1:7000/fallbeans").as_deref(), Some("old"));
        assert_eq!(ids.get("http://10.0.0.2:5887/fallbeans"), None);
        assert_eq!(w.resource::<Player>().identity, "");

        // No server played on last and several in the list: nobody to give it to.
        let mut w = world_with(
            "old",
            Servers {
                list: vec!["10.0.0.1".into(), "10.0.0.2".into()],
                last: None,
            },
        );
        migrate(&mut w, &mut Vec::new());
        assert_eq!(w.resource::<Identities>().get("http://10.0.0.1:5887/fallbeans"), None);
        assert_eq!(w.resource::<Player>().identity, "");
    }

    #[test]
    fn settings_out_of_range_are_brought_back() {
        let mut w = World::new();
        w.insert_resource(Controls {
            mouse_sensitivity: f32::NAN,
            stick_sensitivity: 1e9,
            ..default()
        });
        w.insert_resource(Sound { volume: f32::INFINITY });
        w.insert_resource(Display {
            fov: -5.0,
            ui_scale: f32::NAN,
            show_fps: true,
            fullscreen: false,
        });
        w.insert_resource(Graphics {
            upscale: Upscale::Performance,
            ..default()
        });
        w.insert_resource(Servers {
            last: Some(String::new()),
            ..default()
        });
        sanitize(&mut w);
        let c = w.resource::<Controls>();
        assert_eq!((c.mouse_sensitivity, c.stick_sensitivity), (1.0, SENS_RANGE.1));
        assert_eq!(w.resource::<Sound>().volume, Sound::default().volume);
        let d = w.resource::<Display>();
        assert_eq!((d.fov, d.ui_scale, d.show_fps), (FOV_RANGE.0, 1.0, true));
        assert_eq!(w.resource::<Graphics>().upscale, Upscale::Ultra);
        assert_eq!(w.resource::<Servers>().last, None);
        w.insert_resource(Graphics {
            upscale: Upscale::Balanced,
            ..default()
        });
        sanitize(&mut w);
        assert_eq!(w.resource::<Graphics>().upscale, Upscale::Balanced);
    }

    /// Old files keep loading: a name no longer known fails alone, and bevy-settings keeps that field's default.
    #[test]
    fn old_graphics_names() {
        let de = |s: &str| toml::Value::String(s.into());
        assert_eq!(Preset::deserialize(de("low")).ok(), Some(Preset::Low));
        assert!(Preset::deserialize(de("auto")).is_err());
        assert!(Preset::deserialize(de("medium")).is_err());
        assert_eq!(Upscale::deserialize(de("balanced")).ok(), Some(Upscale::Balanced));
        assert_eq!(UpscalerSetting::deserialize(de("")).ok(), Some(UpscalerSetting(None)));
        assert_eq!(BackendSetting::deserialize(de("gl")).ok(), Some(BackendSetting(None)));
        assert_eq!(
            BackendSetting::deserialize(de("vulkan")).ok(),
            Some(BackendSetting(Some(crate::opts::Backend::Vulkan)))
        );
    }

    #[test]
    fn old_player_values() {
        let de = |s: &str| toml::Value::String(s.into());
        assert_eq!(TintSetting::deserialize(de("")).ok(), Some(TintSetting(None)));
        assert_eq!(
            TintSetting::deserialize(de("Pink")).ok(),
            Some(TintSetting(Some(Tint::Pink)))
        );
        assert_eq!(HatSetting::deserialize(de("Cap")).ok(), Some(HatSetting(Hat::Cap)));
        assert_eq!(
            GlassesSetting::deserialize(de("None")).ok(),
            Some(GlassesSetting(Glasses::None))
        );
        assert_eq!(
            ColorSetting::deserialize(toml::Value::Integer(-1)).ok(),
            Some(ColorSetting(None))
        );
        assert_eq!(
            ColorSetting::deserialize(toml::Value::Integer(3)).ok(),
            Some(ColorSetting(Some(3)))
        );
    }

    /// The perf sweep's switches are never written to the file, and an old file's are not read.
    #[test]
    fn graphics_switches_are_not_saved() {
        let mut registry = bevy::reflect::TypeRegistry::new();
        registry.register::<Graphics>();
        let g = Graphics {
            shadows: false,
            upscale: Upscale::Balanced,
            upscaler: UpscalerSetting(Some(crate::render::upscale::Upscaler::Fsr3)),
            ..default()
        };
        let ser = bevy::reflect::serde::TypedReflectSerializer::new(g.as_partial_reflect(), &registry);
        let text = toml::to_string(&ser).unwrap();
        assert!(text.contains("preset") && text.contains("vsync"), "{text}");
        assert!(text.contains("upscaler = \"fsr3\""), "{text}");
        assert!(text.contains("upscale = \"balanced\""), "{text}");
        let key = |l: &str| l.split('=').next().unwrap_or("").trim().to_string();
        for gone in ["shadows", "aa", "grade", "motes", "ao"] {
            assert!(!text.lines().any(|l| key(l) == gone), "{gone} in {text}");
        }
    }

    /// Taking an action's only key gives it the key the other action had (it used to get its defaults back,
    /// the key just taken among them: Q both dived and grabbed).
    #[test]
    fn a_taken_key_is_swapped() {
        let mut b = Bindings::default();
        b.bind(Bind::Jump, KeyCode::KeyQ);
        assert_eq!(b.keys(Bind::Jump), [KeyCode::KeyQ]);
        assert_eq!(b.keys(Bind::Grab), [KeyCode::Space], "grab takes jump's key");
        // An action with keys to spare just lets the one go.
        b.bind(Bind::Dive, KeyCode::KeyW);
        assert_eq!(b.keys(Bind::Forward), [KeyCode::ArrowUp]);
        assert_eq!(b.keys(Bind::Dive), [KeyCode::KeyW]);
        // Nothing to swap: none at all, not the defaults.
        b.jump = Keys::default();
        assert!(b.keys(Bind::Jump).is_empty());
        b.bind(Bind::Jump, KeyCode::Space);
        assert!(b.keys(Bind::Grab).is_empty(), "{:?}", b.keys(Bind::Grab));
        assert_eq!(b.keys(Bind::Jump), [KeyCode::Space]);
        for k in [KeyCode::Space, KeyCode::KeyW] {
            let owners = crate::keys::BINDS.iter().filter(|x| b.keys(**x).contains(&k)).count();
            assert_eq!(owners, 1, "{k:?}");
        }
    }
}
