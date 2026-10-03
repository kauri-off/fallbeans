//! Recorded rounds and replays (port of `Recording` in `arena.ts` and `server/rooms/replay.ts`): enough
//! to simulate a round again tick by tick and get the same result. Human input is stored as the frames
//! the simulation actually used, run-length encoded; bots replay from the seed.
use std::collections::BTreeMap;

use fb_shared::input::InputFrame;
use fb_sim::map::Value;
use serde::{Deserialize, Serialize};

use crate::{Arena, ArenaKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub v: u32,
    pub game: String,
    pub kind: String,
    pub seed: u32,
    /// The arena tick it was created at (sim time × 120; negative during the intro).
    pub tick0: i64,
    pub participants: Vec<u32>,
    /// Pawns in the order they were added: id, bot, spawn index, tick added at.
    pub pawns: Vec<(u32, bool, Option<usize>, i64)>,
    /// Human frames: [tick, mx, mz, buttons] whenever they change.
    pub frames: BTreeMap<u32, Vec<[i64; 4]>>,
    /// Dev and roster changes applied between ticks: (after tick, op, args).
    pub ops: Vec<(i64, String, Value)>,
    pub end_tick: i64,
    /// `state_hash()` at `end_tick`.
    pub hash: String,
}

impl Recording {
    pub(crate) fn record_frame(&mut self, id: u32, k: i64, f: InputFrame) {
        let list = self.frames.entry(id).or_default();
        let row = [k, i64::from(f.mx), i64::from(f.mz), i64::from(f.buttons)];
        if list.last().is_none_or(|l| l[1..] != row[1..]) {
            list.push(row);
        }
    }
}

pub(crate) fn kind_name(k: ArenaKind) -> &'static str {
    match k {
        ArenaKind::Lobby => "lobby",
        ArenaKind::Round => "round",
        ArenaKind::Podium => "podium",
    }
}

fn kind_of(name: &str) -> Option<ArenaKind> {
    match name {
        "lobby" => Some(ArenaKind::Lobby),
        "round" => Some(ArenaKind::Round),
        "podium" => Some(ArenaKind::Podium),
        _ => None,
    }
}

pub struct Replay {
    pub arena: Arena,
    pub hash: String,
    /// Ran to the end and ended in the recorded state.
    pub matches: bool,
    pub ticks: i64,
}

/// Plays a recorded round again; `each` sees the arena after every tick (returning true stops).
pub fn replay(rec: &Recording, mut each: impl FnMut(&Arena) -> bool) -> Result<Replay, String> {
    let map = fb_maps::by_id(&rec.game).ok_or_else(|| format!("unknown map {}", rec.game))?;
    let kind = kind_of(&rec.kind).ok_or_else(|| format!("unknown arena kind {}", rec.kind))?;
    let (mut a, _) = Arena::new(map, kind, rec.seed, rec.tick0, &rec.participants, false);
    let first = a.tick;
    for &(id, bot, spawn, at) in &rec.pawns {
        if at <= first {
            a.add_pawn_at(id, bot, spawn);
        }
    }
    let later: Vec<_> = rec.pawns.iter().filter(|p| p.3 > first).collect();
    let mut cursor: BTreeMap<u32, usize> = BTreeMap::new();
    let mut ops = rec.ops.iter().peekable();
    let mut stopped = false;
    let mut k = first + 1;
    while k <= rec.end_tick && !stopped {
        while let Some(op) = ops.next_if(|o| o.0 < k) {
            a.apply_op(&op.1, &op.2);
        }
        for p in &later {
            if p.3 == k - 1 {
                a.add_pawn_at(p.0, p.1, p.2);
            }
        }
        let mut frames: BTreeMap<u32, InputFrame> = BTreeMap::new();
        for (&id, list) in &rec.frames {
            let c = cursor.entry(id).or_insert(0);
            while *c + 1 < list.len() && list[*c + 1][0] <= k {
                *c += 1;
            }
            if let Some(f) = list.get(*c).filter(|f| f[0] <= k) {
                frames.insert(
                    id,
                    InputFrame {
                        mx: f[1] as i8,
                        mz: f[2] as i8,
                        buttons: f[3] as u8,
                    },
                );
            }
        }
        a.step(k, |id| frames.get(&id).copied().unwrap_or(InputFrame::IDLE));
        stopped = each(&a);
        k += 1;
    }
    let hash = a.state_hash();
    let ticks = a.tick - first;
    Ok(Replay {
        matches: !stopped && hash == rec.hash,
        hash,
        ticks,
        arena: a,
    })
}
