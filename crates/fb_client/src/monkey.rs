//! Random play on the `harness` by a seed: buttons on screen, keys, text, sliders, the mouse, two players and a
//! server that goes away. A failure prints the seed and the last actions: the path for a new scenario.
use std::time::{Duration, Instant};

use bevy::input::keyboard::Key;
use bevy::prelude::{KeyCode, MouseButton, Vec2};
use fb_proto::DevCmd;

use crate::harness::Game;
use crate::ui::Action;

/// Text a player may type or paste: Cyrillic, emoji (joined, flags, skin tones), combining marks, other
/// scripts, invisible characters, and lengths beyond every field's limit.
const TEXTS: &[&str] = &[
    "привет",
    "Ёжик ё",
    "👋",
    "👩‍👩‍👧‍👦",
    "🇷🇺🏳️‍🌈",
    "👍🏽",
    "e\u{301}",
    "a\u{308}\u{301}\u{20dd}",
    "مرحبا",
    "שלום",
    "日本語",
    "\u{200d}\u{200b}\u{feff}",
    "\u{fe0f}",
    "  ",
    "\t",
    "0000",
    "1234",
    "127.0.0.1:1",
    "http://",
    "[::1]:5887",
    "ЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁЁ",
    "🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘🫘",
];

/// Keys pressed once: menus, text editing, emotes.
fn tap_keys() -> Vec<(KeyCode, Key)> {
    vec![
        (KeyCode::Escape, Key::Escape),
        (KeyCode::Enter, Key::Enter),
        (KeyCode::Tab, Key::Tab),
        (KeyCode::Backspace, Key::Backspace),
        (KeyCode::Delete, Key::Delete),
        (KeyCode::ArrowLeft, Key::ArrowLeft),
        (KeyCode::ArrowRight, Key::ArrowRight),
        (KeyCode::ArrowUp, Key::ArrowUp),
        (KeyCode::ArrowDown, Key::ArrowDown),
        (KeyCode::Home, Key::Home),
        (KeyCode::End, Key::End),
        (KeyCode::Space, Key::Space),
        (KeyCode::Digit1, Key::Character("1".into())),
        (KeyCode::Digit2, Key::Character("2".into())),
        (KeyCode::Digit3, Key::Character("3".into())),
        (KeyCode::Digit4, Key::Character("4".into())),
        (KeyCode::KeyE, Key::Character("e".into())),
        (KeyCode::KeyQ, Key::Character("q".into())),
        (KeyCode::KeyR, Key::Character("r".into())),
        (KeyCode::F1, Key::F1),
    ]
}

/// Keys held: walking, jumping, diving, grabbing.
fn hold_keys() -> Vec<(KeyCode, Key)> {
    vec![
        (KeyCode::KeyW, Key::Character("w".into())),
        (KeyCode::KeyA, Key::Character("a".into())),
        (KeyCode::KeyS, Key::Character("s".into())),
        (KeyCode::KeyD, Key::Character("d".into())),
        (KeyCode::Space, Key::Space),
        (KeyCode::ShiftLeft, Key::Shift),
        (KeyCode::ControlLeft, Key::Control),
    ]
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() { None } else { v.get(self.below(v.len())) }
    }
}

/// The last actions, printed if the run fails.
struct Log {
    seed: u64,
    start: Instant,
    lines: Vec<String>,
}

impl Log {
    fn add(&mut self, peer: usize, what: String) {
        let t = self.start.elapsed().as_secs_f32();
        self.lines.push(format!("{t:7.2} s  p{peer}  {what}"));
        if self.lines.len() > 400 {
            self.lines.drain(..100);
        }
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "monkey seed {} failed; its last actions:\n{}",
                self.seed,
                self.lines.join("\n")
            );
        }
    }
}

/// What the monkey never presses: quitting ends the client, and the updater's buttons go out to the internet.
fn allowed(a: &Action, home: &str) -> bool {
    match a {
        Action::Quit | Action::Update | Action::ReleasePage => false,
        Action::RemoveServer(s) => s != home,
        _ => true,
    }
}

/// Plays for `secs` from `seed`: two players on one server (the first starts in the dev room).
pub fn run(seed: u64, secs: f32) {
    let mut g = Game::new(&["--room", "dev"]);
    g.add_peer(wgpu::DeviceType::IntegratedGpu, &[]);
    let home = format!("127.0.0.1:{}", g.http_port());
    // (Each player has this server in the list, as one does who came back to it.)
    for p in &mut g.peers {
        p.app
            .world_mut()
            .resource_mut::<crate::servers::Servers>()
            .list
            .push(home.clone());
    }
    let mut rng = Rng(seed);
    let mut log = Log {
        seed,
        start: Instant::now(),
        lines: Vec::new(),
    };
    let (taps, holds) = (tap_keys(), hold_keys());
    let end = Instant::now() + Duration::from_secs_f32(secs);
    let mut actions = 0;
    while Instant::now() < end {
        let roll = rng.below(1000);
        let at = g.at;
        match roll {
            0..300 => {
                let all = g.actions(|a| allowed(a, &home));
                if let Some(a) = rng.pick(&all) {
                    let want = format!("{a:?}");
                    log.add(at, format!("press {want}"));
                    g.press(1.0, &want, |b| format!("{b:?}") == want);
                }
            }
            300..450 => {
                if let Some((code, key)) = rng.pick(&taps).cloned() {
                    log.add(at, format!("key {code:?}"));
                    g.key(code, key, None);
                }
            }
            450..580 => {
                let n = 5 + rng.below(90);
                if let Some((code, key)) = rng.pick(&holds).cloned() {
                    log.add(at, format!("hold {code:?} {n} frames"));
                    g.hold(code, key, n);
                }
            }
            580..700 => {
                if g.typing() {
                    let t = *rng.pick(TEXTS).unwrap_or(&"");
                    if rng.below(3) == 0 {
                        log.add(at, format!("text at once {t:?}"));
                        g.type_at_once(t);
                    } else {
                        log.add(at, format!("type {t:?}"));
                        g.type_text(t);
                    }
                } else {
                    let fields = g.fields();
                    if let Some(f) = rng.pick(&fields).copied() {
                        log.add(at, format!("focus {f:?}"));
                        g.focus(f);
                    }
                }
            }
            700..750 => {
                let knobs = g.knobs();
                if let Some(&(k, lo, hi)) = rng.pick(&knobs) {
                    let v = lo + (hi - lo) * rng.unit();
                    log.add(at, format!("slide {k:?} {v}"));
                    g.slide(k, v);
                }
            }
            750..800 => {
                let b = if rng.below(4) == 0 {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                log.add(at, format!("click {b:?}"));
                g.click(b);
            }
            800..850 => {
                let d = Vec2::new(rng.unit() - 0.5, rng.unit() - 0.5) * 400.0;
                log.add(at, format!("look {d}"));
                g.look(d);
            }
            850..860 => {
                log.add(at, "blur".into());
                g.blur();
            }
            860..900 => {
                let p = rng.below(g.peers.len());
                log.add(p, "acts".into());
                g.as_peer(p);
            }
            900..910 => {
                // (A dev room's host: rounds come sooner than by finding «Начать игру».)
                let cmd = match rng.below(4) {
                    0 => DevCmd::SkipIntro,
                    1 => DevCmd::EndRound,
                    2 => DevCmd::Warp { s: 10.0 },
                    _ => DevCmd::Start {
                        games: vec![],
                        rounds: Some(2),
                        bots: Some(rng.below(4) as u32),
                    },
                };
                log.add(at, format!("dev {cmd:?}"));
                g.dev(cmd);
            }
            910..913 => {
                let s = 1.0 + 12.0 * rng.unit();
                log.add(at, format!("server away {s:.1} s"));
                g.server_away(s);
            }
            _ => {
                let n = 1 + rng.below(90);
                log.add(at, format!("wait {n} frames"));
                g.frames(n);
            }
        }
        actions += 1;
    }
    for (i, p) in g.peers.iter().enumerate() {
        let seen: Vec<String> = p.arenas.iter().map(|a| format!("{:?} {}", a.kind, a.game)).collect();
        eprintln!("monkey seed {seed}: p{i} went through {}", seen.join(", "));
    }
    eprintln!("monkey seed {seed}: {actions} actions in {secs} s");
}

/// Short runs in `cargo xtask check`.
#[test]
fn monkey_seed_1() {
    run(1, 25.0);
}

#[test]
fn monkey_seed_2() {
    run(2, 25.0);
}

/// `cargo xtask fuzz-ui --secs N [--seed S]`: one long run.
#[test]
#[ignore]
fn monkey_long() {
    let secs = std::env::var("FB_MONKEY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300.0);
    let seed = std::env::var("FB_MONKEY_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64)
        });
    eprintln!("monkey seed {seed}, {secs} s");
    run(seed, secs);
}
