//! Rooms as the debug API shows them.
use std::collections::BTreeMap;

use fb_arena::{JournalEntry, Pawn, TraceEntry};
use fb_proto::{Phase, PlayerId, Playlist};
use fb_shared::m;
use fb_shared::rules::RoundStats;
use serde::Serialize;
use serde_json::Value;

use super::players::Player;
use super::room::Room;

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn v3(v: [f64; 3]) -> [f64; 3] {
    v.map(r3)
}

#[derive(Serialize)]
pub struct RoomState {
    /// Its number in the listing (`?room=`).
    room: usize,
    id: String,
    title: String,
    private: bool,
    phase: Phase,
    practice: bool,
    permanent: bool,
    host: Option<PlayerId>,
    fill: bool,
    rate: f64,
    timer_in: Option<f64>,
    session: Option<SessionState>,
    round: Option<RoundState>,
    playlist: Playlist,
    players: Vec<PlayerState>,
    arena: ArenaState,
    replays: usize,
}

#[derive(Serialize)]
struct SessionState {
    plan: Vec<&'static str>,
    index: usize,
    started: usize,
}

#[derive(Serialize)]
struct RoundState {
    game: &'static str,
    index: u32,
    total: u32,
    over: bool,
}

#[derive(Serialize)]
struct PlayerState {
    id: PlayerId,
    name: String,
    bot: bool,
    owner: bool,
    connected: bool,
    rtt: u32,
    score: i64,
    crowns: u32,
    spectator: bool,
    stats: GameTotals,
}

#[derive(Serialize)]
struct GameTotals {
    falls: u32,
    kos: u32,
    grabs: u32,
    tackles: u32,
    shortcuts: u32,
    wins: u32,
    survived: f64,
}

#[derive(Serialize)]
struct ArenaState {
    id: u32,
    kind: String,
    game: &'static str,
    seed: u32,
    tick: i64,
    t: f64,
    zero_tick: i64,
    starts_in: f64,
    ends_in: Option<f64>,
    frozen: bool,
    bots_on: bool,
    finished: Vec<PlayerId>,
    out: Vec<PlayerId>,
    scores: BTreeMap<PlayerId, i64>,
    events: usize,
    static_hash: String,
    pawns: Vec<PawnState>,
}

#[derive(Serialize)]
struct PawnState {
    id: PlayerId,
    bot: bool,
    status: String,
    pos: [f64; 3],
    speed: f64,
    state: String,
    grounded: bool,
    progress: f64,
    checkpoint: Option<usize>,
    grabbing: Option<PlayerId>,
    teleports: u32,
    stats: RoundTotals,
}

#[derive(Serialize)]
struct RoundTotals {
    falls: u32,
    shortcuts: u32,
    kos: u32,
    grabs: u32,
    tackles: u32,
    finish_at: Option<f64>,
    out_at: Option<f64>,
}

impl From<&RoundStats> for RoundTotals {
    fn from(s: &RoundStats) -> Self {
        Self {
            falls: s.falls,
            shortcuts: s.shortcuts,
            kos: s.kos,
            grabs: s.grabs,
            tackles: s.tackles,
            finish_at: s.finish_at.map(r3),
            out_at: s.out_at.map(r3),
        }
    }
}

fn player_state(room: &Room, p: &Player) -> PlayerState {
    let s = &p.stats;
    PlayerState {
        id: p.id,
        name: p.name.clone(),
        bot: p.is_bot(),
        owner: room.owner.is_some() && p.uid() == room.owner.as_ref(),
        connected: p.present(),
        rtt: p.rtt,
        score: p.score,
        crowns: p.crowns,
        spectator: p.spectator,
        stats: GameTotals {
            falls: s.falls,
            kos: s.kos,
            grabs: s.grabs,
            tackles: s.tackles,
            shortcuts: s.shortcuts,
            wins: s.wins,
            survived: r3(s.survived),
        },
    }
}

fn pawn_state(p: &Pawn) -> PawnState {
    let b = &p.body;
    PawnState {
        id: p.id,
        bot: p.bot.is_some(),
        status: format!("{:?}", p.status),
        pos: v3([b.pos.x, b.pos.y, b.pos.z]),
        speed: r3(m::hypot(b.vel.x, b.vel.z)),
        state: format!("{:?}", b.state),
        grounded: b.grounded,
        progress: r3(p.progress),
        checkpoint: p.checkpoint,
        grabbing: p.grabbing,
        teleports: p.teleports,
        stats: (&p.stats).into(),
    }
}

/// Everything about the room numbered `i` for the debug API.
pub fn room_state(room: &Room, i: usize) -> RoomState {
    let a = &room.arena;
    RoomState {
        room: i,
        id: room.id.clone(),
        title: room.title.clone(),
        private: room.pin.is_some(),
        phase: room.phase(),
        practice: room.practice(),
        permanent: room.permanent(),
        host: room.host,
        fill: room.fill,
        rate: room.clock.rate,
        timer_in: room.timer_in().map(r3),
        session: room.session().map(|s| SessionState {
            plan: s.plan.iter().map(|g| g.id()).collect(),
            index: s.index,
            started: s.started,
        }),
        round: room.round().map(|r| RoundState {
            game: r.game.id(),
            index: r.index,
            total: r.total,
            over: !room.round_live(),
        }),
        playlist: room.playlist.clone(),
        players: room.players.iter().map(|p| player_state(room, p)).collect(),
        arena: ArenaState {
            id: room.arena_id,
            kind: format!("{:?}", a.kind),
            game: a.map.meta().id,
            seed: a.seed,
            tick: a.tick,
            t: r3(a.time()),
            zero_tick: room.zero_tick(),
            starts_in: r3((-a.time()).max(0.0)),
            ends_in: room.ends_in().map(r3),
            frozen: a.frozen,
            bots_on: a.bots_on,
            finished: a.finished.clone(),
            out: a.out.clone(),
            scores: a.scores.clone(),
            events: a.kept_events,
            static_hash: a.static_hash.to_string(),
            pawns: a.pawns.iter().map(pawn_state).collect(),
        },
        replays: room.replays.len(),
    }
}

#[derive(Serialize)]
pub struct RoomTrace {
    game: &'static str,
    t: f64,
    journal: Vec<JournalLine>,
    /// By bean.
    trace: BTreeMap<PlayerId, Vec<TraceLine>>,
}

#[derive(Serialize)]
struct JournalLine {
    t: f64,
    what: &'static str,
    id: Option<PlayerId>,
    data: Option<Value>,
}

impl From<&JournalEntry> for JournalLine {
    fn from(e: &JournalEntry) -> Self {
        Self {
            t: r3(e.t),
            what: e.what,
            id: e.id,
            data: e.data.clone(),
        }
    }
}

#[derive(Serialize)]
struct TraceLine {
    t: f64,
    pos: [f64; 3],
    vel: [f64; 3],
    state: String,
    grounded: bool,
    input: [i32; 3],
    grabbing: Option<PlayerId>,
    hazard: Option<fb_proto::Hazard>,
}

impl From<&TraceEntry> for TraceLine {
    fn from(e: &TraceEntry) -> Self {
        Self {
            t: r3(e.t),
            pos: v3(e.pos),
            vel: v3(e.vel),
            state: format!("{:?}", e.state),
            grounded: e.grounded,
            input: e.input,
            grabbing: e.grabbing,
            hazard: e.hazard,
        }
    }
}

/// The last `seconds` of one bean's history (or every bean's) and of the arena's journal.
pub fn room_trace(room: &Room, id: Option<PlayerId>, seconds: f64) -> RoomTrace {
    let a = &room.arena;
    let from = a.time() - seconds;
    let trace = a
        .trace
        .iter()
        .filter(|(k, _)| id.is_none_or(|id| id == **k))
        .map(|(k, list)| (*k, list.iter().filter(|e| e.t >= from).map(TraceLine::from).collect()))
        .collect();
    let journal = a
        .journal
        .iter()
        .filter(|e| e.t >= from)
        .map(JournalLine::from)
        .collect();
    RoomTrace {
        game: a.map.meta().id,
        t: r3(a.time()),
        journal,
        trace,
    }
}
