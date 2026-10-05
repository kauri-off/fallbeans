//! Rare one-off bonuses on the course: spots and kinds follow from the seed; the server decides who took one.
use crate::m::{self, MinMaxJs};
use crate::math::V3;
use crate::physics::{Body, power};
use fb_shared::rng::Rng;

const REACH: f64 = 1.25;

#[derive(Clone, Debug, PartialEq)]
pub struct Bonus {
    pub i: u32,
    pub pos: V3,
    pub kind: u8,
    /// Sim time it shows up (0 on race courses).
    pub appear_at: f64,
    pub taken_by: Option<u32>,
    pub taken_at: f64,
}

/// A bonus was taken: the authoritative map event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonusTaken {
    pub i: u32,
    pub id: u32,
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
        let roll = rng.next();
        let count = spots.len().min(if roll < 0.16 {
            0
        } else if roll < 0.7 {
            1
        } else {
            2
        });
        let mut pool: Vec<usize> = (0..spots.len()).collect();
        let kinds = [power::GIANT, power::JUMP, power::SPEED];
        for k in 0..count {
            let pick = pool.remove((rng.next() * pool.len() as f64).floor() as usize);
            let s = spots[pick];
            let kind = kinds[(rng.next() * kinds.len() as f64).floor() as usize];
            let appear_at = if arena {
                12.0 + rng.next() * (duration * 0.55 - 12.0).max_js(1.0)
            } else {
                0.0
            };
            list.push(Bonus {
                i: k as u32,
                pos: s,
                kind,
                appear_at,
                taken_by: None,
                taken_at: 0.0,
            });
        }
        Self { list }
    }

    pub fn available(&self, t: f64) -> impl Iterator<Item = &Bonus> {
        self.list
            .iter()
            .filter(move |x| x.taken_by.is_none() && t >= x.appear_at)
    }

    /// Server: bodies (id, body) that touch a bonus take it.
    pub fn check(&mut self, t: f64, bodies: &mut [(u32, &mut Body)]) -> Vec<BonusTaken> {
        let mut out = Vec::new();
        for x in &mut self.list {
            if x.taken_by.is_some() || t < x.appear_at {
                continue;
            }
            for (id, body) in bodies.iter_mut() {
                let id = *id;
                let dy = body.pos.y - x.pos.y;
                if !(-0.8..=2.0).contains(&dy) {
                    continue;
                }
                if m::hypot(body.pos.x - x.pos.x, body.pos.z - x.pos.z) > REACH + 0.3 * body.size {
                    continue;
                }
                body.give_power(x.kind, t);
                x.taken_by = Some(id);
                x.taken_at = t;
                out.push(BonusTaken { i: x.i, id, at: t });
                break;
            }
        }
        out
    }

    /// Both sides: a bonus was taken.
    pub fn on_event(&mut self, e: BonusTaken) -> Option<&Bonus> {
        let x = self.list.get_mut(e.i as usize)?;
        if x.taken_by.is_some() {
            return None;
        }
        x.taken_by = Some(e.id);
        x.taken_at = e.at;
        Some(x)
    }
}
