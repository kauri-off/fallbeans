//! Headless rounds for audits, benchmarks and tools: a server arena with bots only (or idle pawns),
//! started at a fixed seed, with what it reports collected.
use fb_arena::{Arena, ArenaEvent, ArenaKind, PawnStatus};
use fb_shared::cause::Cause;
use fb_shared::input::InputFrame;
use fb_shared::{DT, PlayerId, TICK_RATE};
use fb_sim::map::{MapDef, MapEvent};
use fb_sim::math::V3;

#[derive(Clone, Debug)]
pub struct Fall {
    pub id: PlayerId,
    pub t: f64,
    pub out: bool,
    pub cause: Cause,
    pub by: Option<PlayerId>,
    /// Progress (z, or the map's progress) reached before the fall.
    pub progress: f64,
    pub pos: V3,
}

/// Intro before the start in the audits' rounds: half a second.
pub const INTRO: i64 = 60;

pub struct Opts {
    pub seed: u32,
    /// Number of pawns (ids 1…n).
    pub players: u32,
    /// Which of them are bots (the others stand idle).
    pub bots: fn(PlayerId) -> bool,
    pub kind: ArenaKind,
    /// Intro before t = 0, in ticks.
    pub intro: i64,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            seed: 11,
            players: 8,
            bots: |_| true,
            kind: ArenaKind::Round,
            intro: INTRO,
        }
    }
}

pub struct Harness {
    pub arena: Arena,
    pub map: &'static dyn MapDef,
    pub ids: Vec<PlayerId>,
    pub finishes: Vec<(PlayerId, f64)>,
    pub falls: Vec<Fall>,
    pub events: Vec<MapEvent>,
}

impl Harness {
    pub fn new(map: &'static dyn MapDef, o: &Opts) -> Self {
        let ids: Vec<PlayerId> = (1..=o.players).map(PlayerId).collect();
        let (mut arena, _) = Arena::new(map, o.kind, o.seed, -o.intro, &ids, false);
        for (i, &id) in ids.iter().enumerate() {
            arena.add_pawn_at(id, (o.bots)(id), Some(i));
        }
        Self {
            arena,
            map,
            ids,
            finishes: Vec::new(),
            falls: Vec::new(),
            events: Vec::new(),
        }
    }

    /// One tick.
    pub fn step(&mut self) {
        let k = self.arena.tick + 1;
        for e in self.arena.step(k, |_| InputFrame::IDLE) {
            match e {
                ArenaEvent::Finish { id, t } => self.finishes.push((id, t)),
                ArenaEvent::Ko(ko) => {
                    let progress = self.arena.pawn(ko.id).map_or(0.0, |p| p.progress);
                    self.falls.push(Fall {
                        id: ko.id,
                        t: k as f64 * DT,
                        out: ko.out,
                        cause: ko.cause,
                        by: ko.by,
                        progress,
                        pos: ko.pos,
                    });
                }
                ArenaEvent::Event { ev, .. } => self.events.push(ev),
                _ => {}
            }
        }
    }

    /// Advances to sim time `t` seconds (every tick simulated).
    #[expect(clippy::cast_possible_truncation, reason = "a tick number")]
    pub fn run_to(&mut self, t: f64) {
        let target = (t * f64::from(TICK_RATE) + 0.5).floor() as i64;
        while self.arena.tick < target {
            self.step();
        }
    }

    /// Pawns still in play.
    pub fn alive(&self) -> usize {
        self.arena.pawns.iter().filter(|p| p.status == PawnStatus::Play).count()
    }
}

pub fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    let mut s = xs.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    if s.len() % 2 == 1 {
        s[m]
    } else {
        (s[m - 1] + s[m]) / 2.0
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "an index below len"
)]
pub fn quantile(xs: &[f64], q: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    let mut s = xs.to_vec();
    s.sort_by(f64::total_cmp);
    s[((q * s.len() as f64).floor() as usize).min(s.len() - 1)]
}
