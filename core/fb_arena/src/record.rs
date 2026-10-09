//! Recorded rounds and replays: enough
//! to simulate a round again tick by tick and get the same result. Human input is stored as the frames
//! the simulation actually used, run-length encoded; bots replay from the seed.
use std::collections::BTreeMap;

use fb_shared::PlayerId;
use fb_shared::hash::StateHash;
use fb_shared::input::InputFrame;
use fb_sim::math::V3;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Arena, ArenaKind};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub v: u32,
    pub game: String,
    #[serde(with = "kind_name")]
    pub kind: ArenaKind,
    pub seed: u32,
    /// The arena tick it was created at (sim time × 120; negative during the intro).
    pub tick0: i64,
    pub participants: Vec<PlayerId>,
    /// Pawns in the order they were added.
    pub pawns: Vec<PawnRec>,
    /// Human frames whenever they change.
    pub frames: BTreeMap<PlayerId, Vec<FrameRec>>,
    /// Dev and roster changes applied between ticks: (after tick, op).
    pub ops: Vec<(i64, Op)>,
    pub end_tick: i64,
    /// `state_hash()` at `end_tick`.
    pub hash: StateHash,
}

/// A pawn added at the start: `[id, bot, spawn index, tick added at]` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    from = "(PlayerId, bool, Option<usize>, i64)",
    into = "(PlayerId, bool, Option<usize>, i64)"
)]
pub struct PawnRec {
    pub id: PlayerId,
    pub bot: bool,
    pub spawn: Option<usize>,
    pub at: i64,
}

impl From<(PlayerId, bool, Option<usize>, i64)> for PawnRec {
    fn from((id, bot, spawn, at): (PlayerId, bool, Option<usize>, i64)) -> Self {
        Self { id, bot, spawn, at }
    }
}

impl From<PawnRec> for (PlayerId, bool, Option<usize>, i64) {
    fn from(p: PawnRec) -> Self {
        (p.id, p.bot, p.spawn, p.at)
    }
}

/// A human's input from `tick` on: `[tick, mx, mz, buttons]` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "[i64; 4]", into = "[i64; 4]")]
pub struct FrameRec {
    pub tick: i64,
    pub input: InputFrame,
}

impl From<[i64; 4]> for FrameRec {
    fn from([tick, mx, mz, buttons]: [i64; 4]) -> Self {
        Self {
            tick,
            input: InputFrame {
                mx: mx as i8,
                mz: mz as i8,
                buttons: buttons as u8,
            },
        }
    }
}

impl From<FrameRec> for [i64; 4] {
    fn from(f: FrameRec) -> Self {
        let i = f.input;
        [f.tick, i64::from(i.mx), i64::from(i.mz), i64::from(i.buttons)]
    }
}

/// `ArenaKind` as recordings name it.
mod kind_name {
    use super::*;

    fn name(k: ArenaKind) -> &'static str {
        match k {
            ArenaKind::Lobby => "lobby",
            ArenaKind::Round => "round",
            ArenaKind::Podium => "podium",
        }
    }

    pub fn serialize<S: Serializer>(k: &ArenaKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(name(*k))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ArenaKind, D::Error> {
        let s = String::deserialize(d)?;
        [ArenaKind::Lobby, ArenaKind::Round, ArenaKind::Podium]
            .into_iter()
            .find(|&k| name(k) == s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown arena kind {s}")))
    }
}

/// Something that changed the simulation from outside between ticks: a pawn leaving or joining late, dev tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Remove(PlayerId),
    Late {
        id: PlayerId,
        bot: bool,
        at: Option<V3>,
    },
    Teleport {
        id: PlayerId,
        pos: V3,
        yaw: Option<f64>,
    },
    Knock {
        id: PlayerId,
        v: V3,
    },
    Kill(PlayerId),
    Grab {
        actor: PlayerId,
        target: PlayerId,
        seconds: f64,
    },
    /// Bot brains run (false: bots stand still).
    Bots(bool),
    /// The results were decided (`Arena::freeze`).
    Freeze,
    /// How many ticks behind its bean a player sees the others (`Arena::set_view`).
    View {
        id: PlayerId,
        ticks: u32,
    },
    /// The arena jumped to this tick without simulating the ones between (`Arena::skip_to`).
    Skip(i64),
}

impl Recording {
    pub(crate) fn record_frame(&mut self, id: PlayerId, k: i64, input: InputFrame) {
        let list = self.frames.entry(id).or_default();
        if list.last().is_none_or(|l| l.input != input) {
            list.push(FrameRec { tick: k, input });
        }
    }
}

pub struct Replay {
    pub arena: Arena,
    pub hash: StateHash,
    /// Ran to the end and ended in the recorded state.
    pub matches: bool,
    pub ticks: i64,
}

/// Plays a recorded round again; `each` sees the arena after every tick (returning true stops).
pub fn replay(rec: &Recording, mut each: impl FnMut(&Arena) -> bool) -> Result<Replay, String> {
    let map = fb_maps::by_id(&rec.game).ok_or_else(|| format!("unknown map {}", rec.game))?;
    let (mut a, _) = Arena::new(map, rec.kind, rec.seed, rec.tick0, &rec.participants, false);
    let first = a.tick;
    for p in rec.pawns.iter().filter(|p| p.at <= first) {
        a.add_pawn_at(p.id, p.bot, p.spawn);
    }
    let mut later = rec.pawns.iter().filter(|p| p.at > first).peekable();
    let mut cursor: BTreeMap<PlayerId, usize> = BTreeMap::new();
    let mut ops = rec.ops.iter().peekable();
    let mut stopped = false;
    let mut k = first + 1;
    while k <= rec.end_tick && !stopped {
        while let Some(op) = ops.next_if(|o| o.0 < k) {
            a.apply_op(&op.1);
        }
        k = a.tick + 1;
        if k > rec.end_tick {
            break;
        }
        while let Some(p) = later.next_if(|p| p.at < k) {
            a.add_pawn_at(p.id, p.bot, p.spawn);
        }
        let mut frames: BTreeMap<PlayerId, InputFrame> = BTreeMap::new();
        for (&id, list) in &rec.frames {
            let c = cursor.entry(id).or_insert(0);
            while *c + 1 < list.len() && list[*c + 1].tick <= k {
                *c += 1;
            }
            if let Some(f) = list.get(*c).filter(|f| f.tick <= k) {
                frames.insert(id, f.input);
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
