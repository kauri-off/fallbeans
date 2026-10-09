//! Sound effects synthesized at start into PCM buffers; other beans' sounds are panned and attenuated by distance.
use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::Volume;
use bevy::prelude::*;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_shared::PlayerId;
use fb_sim::map::MapSfx;
use lightyear::prelude::*;

use crate::game::{Cue, Map};
use crate::session::Session;

const RATE: u32 = 44_100;
/// Metres to the units of the spatial mix: a sound at full volume up to this far, a quarter at twice that.
const HEARD_FULL_M: f32 = 12.0;
/// A jingle asked for again within this many seconds (the own finish and the round's end in one tick) plays once.
const JINGLE_GAP: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    Jump,
    Dive,
    Hit,
    Grab,
    Boing,
    Break,
    Count,
    Go,
    Qualify,
    Out,
    Win,
    Pickup,
    Click,
}

impl Sfx {
    const ALL: [Sfx; 13] = [
        Sfx::Jump,
        Sfx::Dive,
        Sfx::Hit,
        Sfx::Grab,
        Sfx::Boing,
        Sfx::Break,
        Sfx::Count,
        Sfx::Go,
        Sfx::Qualify,
        Sfx::Out,
        Sfx::Win,
        Sfx::Pickup,
        Sfx::Click,
    ];
}

impl From<MapSfx> for Sfx {
    fn from(s: MapSfx) -> Self {
        match s {
            MapSfx::Break => Sfx::Break,
            MapSfx::Pickup => Sfx::Pickup,
            MapSfx::Steal => Sfx::Boing,
        }
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Triangle,
    Sawtooth,
    Square,
}

impl Wave {
    /// One period over p in 0..1, as WebAudio's oscillators.
    fn at(self, p: f32) -> f32 {
        match self {
            Wave::Sine => (p * core::f32::consts::TAU).sin(),
            Wave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Sawtooth => 2.0 * (p - (p + 0.5).floor()),
            Wave::Triangle => 1.0 - 4.0 * ((p + 0.25).fract() - 0.5).abs(),
        }
    }
}

/// A tone from `from` to `to` Hz over `dur` s, fading from `vol` to silence, starting `delay` s in.
#[derive(Clone, Copy)]
struct Tone {
    from: f32,
    to: f32,
    dur: f32,
    wave: Wave,
    vol: f32,
    delay: f32,
}

impl Tone {
    fn after(self, delay: f32) -> Self {
        Self { delay, ..self }
    }
}

fn tones(s: Sfx) -> Vec<Tone> {
    use Wave::*;
    let t = |from, to, dur, wave, vol| Tone {
        from,
        to,
        dur,
        wave,
        vol,
        delay: 0.0,
    };
    match s {
        Sfx::Jump => vec![t(330.0, 620.0, 0.14, Triangle, 0.1)],
        Sfx::Dive => vec![t(500.0, 180.0, 0.2, Triangle, 0.1)],
        Sfx::Hit => vec![t(180.0, 60.0, 0.25, Sawtooth, 0.09)],
        Sfx::Grab => vec![t(260.0, 200.0, 0.1, Triangle, 0.08)],
        Sfx::Boing => vec![t(200.0, 700.0, 0.22, Sine, 0.14)],
        Sfx::Break => vec![t(140.0, 50.0, 0.3, Square, 0.08)],
        Sfx::Pickup => vec![
            t(988.0, 988.0, 0.07, Triangle, 0.1),
            t(1319.0, 1319.0, 0.16, Triangle, 0.1).after(0.07),
        ],
        Sfx::Count => vec![t(520.0, 520.0, 0.18, Square, 0.07)],
        Sfx::Click => vec![t(700.0, 900.0, 0.05, Triangle, 0.05)],
        Sfx::Go => vec![t(880.0, 880.0, 0.45, Square, 0.08)],
        Sfx::Qualify => [523.0, 659.0, 784.0, 1046.0]
            .iter()
            .enumerate()
            .map(|(i, f)| t(*f, *f, 0.18, Triangle, 0.12).after(i as f32 * 0.09))
            .collect(),
        Sfx::Out => [440.0, 330.0, 220.0]
            .iter()
            .enumerate()
            .map(|(i, f)| t(*f, f * 0.95, 0.22, Sawtooth, 0.07).after(i as f32 * 0.14))
            .collect(),
        Sfx::Win => [523.0, 659.0, 784.0, 1046.0, 784.0, 1046.0]
            .iter()
            .enumerate()
            .map(|(i, f)| t(*f, *f, 0.25, Triangle, 0.12).after(i as f32 * 0.12))
            .collect(),
    }
}

fn synth(tones: &[Tone]) -> Vec<f32> {
    let len = tones.iter().map(|t| t.delay + t.dur + 0.02).fold(0.0, f32::max);
    let mut out = vec![0.0f32; (len * RATE as f32).ceil() as usize];
    for &Tone {
        from: f1,
        to: f2,
        dur,
        wave,
        vol,
        delay,
    } in tones
    {
        let start = (delay * RATE as f32) as usize;
        let n = ((dur + 0.02) * RATE as f32) as usize;
        let mut phase = 0.0f32;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            // Exponential ramps, held at their end values after `dur` (as WebAudio's).
            let k = (t / dur).min(1.0);
            let f = f1 * (f2 / f1).powf(k);
            let g = vol * (0.001f32 / vol).powf(k);
            phase = (phase + f / RATE as f32).fract();
            if let Some(s) = out.get_mut(start + i) {
                *s += wave.at(phase) * g;
            }
        }
    }
    out
}

/// A mono 16-bit WAV file of `samples`.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend(b"RIFF");
    b.extend((36 + data).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(RATE.to_le_bytes());
    b.extend((RATE * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data.to_le_bytes());
    for s in samples {
        b.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

#[derive(Resource)]
struct Sounds(HashMap<Sfx, Handle<AudioSource>>);

/// What the sounds follow between frames: the countdown's last number, the own bean's grab, the last jingle.
#[derive(Resource, Default)]
struct Heard {
    count: Option<i64>,
    holding: Option<PlayerId>,
    jingle: Option<(Sfx, f32)>,
    rng: u32,
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Heard {
            rng: 0x9e37_79b9,
            ..default()
        });
        app.insert_resource(bevy::audio::DefaultSpatialScale(bevy::audio::SpatialScale::new(
            1.0 / HEARD_FULL_M,
        )));
        app.add_systems(Startup, make_sounds);
        app.add_systems(Update, (volume, on_cues, countdown, own_grab));
    }
}

fn make_sounds(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    let map = Sfx::ALL
        .iter()
        .map(|s| {
            let bytes: Arc<[u8]> = wav(&synth(&tones(*s))).into();
            (*s, sources.add(AudioSource { bytes }))
        })
        .collect();
    commands.insert_resource(Sounds(map));
}

/// The player's volume over every sound.
fn volume(sound: Res<crate::settings::Sound>, mut global: ResMut<GlobalVolume>) {
    let v = Volume::Linear(sound.volume.clamp(0.0, 1.0));
    if global.volume != v {
        global.volume = v;
    }
}

fn play(commands: &mut Commands, sounds: &Sounds, heard: &mut Heard, s: Sfx, gain: f32) {
    play_at(commands, sounds, heard, s, gain, None);
}

/// A sound, from a place in the world (`at`) or from everywhere.
fn play_at(commands: &mut Commands, sounds: &Sounds, heard: &mut Heard, s: Sfx, gain: f32, at: Option<Vec3>) {
    let Some(h) = sounds.0.get(&s) else { return };
    heard.rng ^= heard.rng << 13;
    heard.rng ^= heard.rng >> 17;
    heard.rng ^= heard.rng << 5;
    // A little variation in pitch: the same sound many times over does not drone.
    let pitch = 1.0 + ((heard.rng >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.08;
    let settings = PlaybackSettings::DESPAWN
        .with_speed(pitch)
        .with_volume(Volume::Linear(gain));
    match at {
        Some(p) => commands.spawn((
            AudioPlayer(h.clone()),
            settings.with_spatial(true),
            Transform::from_translation(p),
        )),
        None => commands.spawn((AudioPlayer(h.clone()), settings)),
    };
}

fn on_cues(
    mut commands: Commands,
    sounds: Option<Res<Sounds>>,
    mut heard: ResMut<Heard>,
    session: Res<Session>,
    mut cues: MessageReader<Cue>,
    beans: Query<(&BeanId, &GlobalTransform), With<crate::beans::BeanView>>,
    time: Res<Time<Real>>,
) {
    let Some(sounds) = sounds else {
        cues.clear();
        return;
    };
    let me = session.me;
    let at = |id: PlayerId| {
        (Some(id) != me)
            .then(|| beans.iter().find(|(p, _)| p.0 == id).map(|(_, t)| t.translation()))
            .flatten()
    };
    for c in cues.read() {
        let from = match *c {
            Cue::Bonus(id) | Cue::Bell(id) => at(id),
            _ => None,
        };
        let (s, gain) = match *c {
            Cue::Jumped => (Sfx::Jump, 1.0),
            Cue::Dived => (Sfx::Dive, 1.0),
            Cue::Knocked | Cue::Hit => (Sfx::Hit, 1.0),
            Cue::Bumped(b) => (Sfx::Hit, (b / 12.0).min(1.0) * 0.6),
            Cue::Bounced | Cue::Bonus(_) => (Sfx::Boing, 1.0),
            Cue::Finish(id) if Some(id) == me => (Sfx::Qualify, 1.0),
            Cue::Ko { id, out } if Some(id) == me => (if out { Sfx::Out } else { Sfx::Hit }, 1.0),
            Cue::Results => (Sfx::Qualify, 1.0),
            Cue::Bell(id) => {
                if Some(id) == me {
                    (Sfx::Win, 1.0)
                } else {
                    (Sfx::Boing, 0.5)
                }
            }
            Cue::Sfx(s) => (s, 1.0),
            _ => continue,
        };
        if matches!(s, Sfx::Qualify | Sfx::Out | Sfx::Win) {
            let now = time.elapsed_secs();
            if heard.jingle.is_some_and(|(j, at)| j == s && now - at < JINGLE_GAP) {
                continue;
            }
            heard.jingle = Some((s, now));
        }
        play_at(&mut commands, &sounds, &mut heard, s, gain, from);
    }
}

/// 3, 2, 1, go before a round.
fn countdown(
    mut commands: Commands,
    sounds: Option<Res<Sounds>>,
    mut heard: ResMut<Heard>,
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
) {
    let (Some(sounds), Some(map)) = (sounds, map) else {
        return;
    };
    if map.round.kind != ArenaKind::Round {
        heard.count = None;
        return;
    }
    let n = (-map.time(f64::from(timeline.tick().0))).ceil() as i64;
    if heard.count == Some(n) {
        return;
    }
    if (1..=3).contains(&n) {
        play(&mut commands, &sounds, &mut heard, Sfx::Count, 1.0);
    }
    if n == 0 && heard.count == Some(1) {
        play(&mut commands, &sounds, &mut heard, Sfx::Go, 1.0);
    }
    heard.count = Some(n);
}

/// The own bean takes hold of somebody.
fn own_grab(
    mut commands: Commands,
    sounds: Option<Res<Sounds>>,
    mut heard: ResMut<Heard>,
    own: Query<&Hold, With<Predicted>>,
) {
    let Some(sounds) = sounds else { return };
    let now = own.single().ok().and_then(|h| h.target);
    if now != heard.holding {
        heard.holding = now;
        if now.is_some() {
            play(&mut commands, &sounds, &mut heard, Sfx::Grab, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_short_and_within_range() {
        for s in Sfx::ALL {
            let pcm = synth(&tones(s));
            let secs = pcm.len() as f32 / RATE as f32;
            assert!((0.05..1.0).contains(&secs), "{s:?}: {secs} s");
            let peak = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.01 && peak <= 0.5, "{s:?}: peak {peak}");
            let file = wav(&pcm);
            assert_eq!(&file[..4], b"RIFF");
            assert_eq!(file.len(), 44 + pcm.len() * 2);
        }
    }
}
