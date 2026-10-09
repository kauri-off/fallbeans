//! Rare one-off bonuses on the course: spots and kinds follow from the seed; the server decides who took one.
use fb_shared::PlayerId;
use fb_shared::rng::Rng;

use crate::m::MinMax;
use crate::math::{V3, dist_xz};
use crate::physics::{Body, Power};

const REACH: f64 = 1.25;

#[derive(Clone, Debug, PartialEq)]
pub struct Bonus {
    pub i: u32,
    pub pos: V3,
    pub kind: Power,
    /// Sim time it shows up (0 on race courses).
    pub appear_at: f64,
    /// Who took it, and when.
    pub taken: Option<(PlayerId, f64)>,
}

/// A bonus was taken: the authoritative map event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonusTaken {
    pub i: u32,
    pub id: PlayerId,
    pub at: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Bonuses {
    pub list: Vec<Bonus>,
}

impl Bonuses {
    pub fn new(spots: &[V3], seed: u32, arena: bool, duration: f64) -> Self {
        let mut list = Vec::new();
        if spots.is_empty() {
            return Self { list };
        }
        let mut rng = Rng::new(seed ^ 0x5bd1_e995);
        let roll = rng.unit();
        let count = spots.len().min(if roll < 0.16 {
            0
        } else if roll < 0.7 {
            1
        } else {
            2
        });
        let mut pool: Vec<usize> = (0..spots.len()).collect();
        let kinds = Power::ALL;
        for i in (0u32..).take(count) {
            let pick = pool.remove(rng.index(pool.len()));
            let s = spots[pick];
            let kind = kinds[rng.index(kinds.len())];
            let appear_at = if arena {
                12.0 + rng.unit() * (duration * 0.55 - 12.0).at_least(1.0)
            } else {
                0.0
            };
            list.push(Bonus {
                i,
                pos: s,
                kind,
                appear_at,
                taken: None,
            });
        }
        Self { list }
    }

    pub fn available(&self, t: f64) -> impl Iterator<Item = &Bonus> {
        self.list.iter().filter(move |x| x.taken.is_none() && t >= x.appear_at)
    }

    /// Server: bodies (id, body) that touch a bonus take it.
    pub fn check(&mut self, t: f64, bodies: &mut [(PlayerId, &mut Body)]) -> Vec<BonusTaken> {
        let mut out = Vec::new();
        for x in &mut self.list {
            if x.taken.is_some() || t < x.appear_at {
                continue;
            }
            for (id, body) in bodies.iter_mut() {
                let id = *id;
                let dy = body.pos.y - x.pos.y;
                if !(-0.8..=2.0).contains(&dy) {
                    continue;
                }
                if dist_xz(body.pos, x.pos) > REACH + 0.3 * body.size {
                    continue;
                }
                body.give_power(x.kind, t);
                x.taken = Some((id, t));
                out.push(BonusTaken { i: x.i, id, at: t });
                break;
            }
        }
        out
    }

    /// Both sides: a bonus was taken.
    pub fn on_event(&mut self, e: BonusTaken) -> Option<&Bonus> {
        let x = self.list.get_mut(e.i as usize)?;
        if x.taken.is_some() {
            return None;
        }
        x.taken = Some((e.id, e.at));
        Some(x)
    }
}
