//! Rooms as plain JSON for the debug API.
use fb_arena::{JournalEntry, TraceEntry};
use fb_shared::m;
use fb_shared::rules::RoundStats;
use serde_json::{Value, json};

use super::room::Room;

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn round_stats(s: &RoundStats) -> Value {
    json!({
        "falls": s.falls, "shortcuts": s.shortcuts, "kos": s.kos, "grabs": s.grabs, "tackles": s.tackles,
        "idle": r3(s.idle), "finishAt": s.finish_at.map(r3), "outAt": s.out_at.map(r3),
    })
}

/// Everything about a room for the debug API.
pub fn room_state(room: &Room) -> Value {
    let a = &room.arena;
    let players: Vec<Value> = room
        .players
        .iter()
        .map(|p| {
            let s = &p.stats;
            json!({
                "id": p.id,
                "name": p.name,
                "bot": p.bot,
                "owner": !p.bot && room.owner.as_deref() == Some(p.uid.as_str()),
                "connected": p.bot || p.conn.is_some(),
                "rtt": p.rtt,
                "score": p.score,
                "crowns": p.crowns,
                "spectator": p.spectator,
                "stats": {
                    "falls": s.falls, "kos": s.kos, "grabs": s.grabs, "tackles": s.tackles,
                    "shortcuts": s.shortcuts, "wins": s.wins, "survived": r3(s.survived),
                },
            })
        })
        .collect();
    let pawns: Vec<Value> = a
        .pawns
        .iter()
        .map(|p| {
            let b = &p.body;
            json!({
                "id": p.id,
                "bot": p.bot.is_some(),
                "status": format!("{:?}", p.status),
                "pos": [r3(b.pos.x), r3(b.pos.y), r3(b.pos.z)],
                "speed": r3(m::hypot(b.vel.x, b.vel.z)),
                "state": format!("{:?}", b.state),
                "grounded": b.grounded,
                "progress": r3(p.progress),
                "checkpoint": p.checkpoint,
                "grabbing": p.grabbing,
                "teleports": p.teleports,
                "stats": round_stats(&p.stats),
            })
        })
        .collect();
    json!({
        "id": room.id,
        "title": room.title,
        "private": room.pin.is_some(),
        "phase": format!("{:?}", room.phase),
        "practice": room.practice(),
        "permanent": room.permanent(),
        "host": room.host,
        "fill": room.fill,
        "rate": room.clock.rate,
        "timerIn": room.timer_in().map(r3),
        "session": room.session.as_ref().map(|s| json!({ "plan": s.plan, "index": s.index, "started": s.started })),
        "round": room.round.as_ref().map(|r| json!({ "game": r.game.id, "index": r.index, "total": r.total, "over": r.over })),
        "playlist": room.playlist,
        "players": players,
        "arena": {
            "id": room.arena_id,
            "kind": format!("{:?}", a.kind),
            "game": a.map.meta().id,
            "seed": a.seed,
            "tick": a.tick,
            "t": r3(a.time()),
            "zeroTick": room.zero_tick(),
            "startsIn": r3((-a.time()).max(0.0)),
            "endsIn": room.ends_in().map(r3),
            "frozen": a.frozen,
            "botsOn": a.bots_on,
            "finished": a.finished,
            "out": a.out,
            "scores": a.scores,
            "events": a.kept_events,
            "staticHash": a.static_hash,
            "pawns": pawns,
        },
        "replays": room.replays.len(),
    })
}

fn journal_entry(e: &JournalEntry) -> Value {
    json!({ "t": r3(e.t), "what": e.what, "id": e.id, "data": e.data })
}

fn trace_entry(e: &TraceEntry) -> Value {
    let v3 = |v: [f64; 3]| [r3(v[0]), r3(v[1]), r3(v[2])];
    json!({
        "t": r3(e.t), "pos": v3(e.pos), "vel": v3(e.vel), "state": format!("{:?}", e.state),
        "grounded": e.grounded, "input": e.input, "grabbing": e.grabbing, "hazard": e.hazard,
    })
}

/// The last `seconds` of one bean's history (or every bean's) and of the arena's journal.
pub fn room_trace(room: &Room, id: Option<u32>, seconds: f64) -> Value {
    let a = &room.arena;
    let from = a.time() - seconds;
    let trace: serde_json::Map<String, Value> = a
        .trace
        .iter()
        .filter(|(k, _)| id.is_none_or(|id| id == **k))
        .map(|(k, list)| {
            let recent: Vec<Value> = list.iter().filter(|e| e.t >= from).map(trace_entry).collect();
            (k.to_string(), Value::Array(recent))
        })
        .collect();
    let journal: Vec<Value> = a.journal.iter().filter(|e| e.t >= from).map(journal_entry).collect();
    json!({ "game": a.map.meta().id, "t": r3(a.time()), "journal": journal, "trace": trace })
}
