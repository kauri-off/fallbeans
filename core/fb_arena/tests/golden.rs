//! Whole rounds of the Rust arena against the TS server arena (traces recorded by the TS version on `master`): scripted players
//! and bots, compared by state hash every tick, full rows every `rowEvery` ticks, events and stats.
mod common;

use common::{check_bodies, check_colliders, close, f, load};
use fb_arena::{Arena, ArenaEvent, ArenaKind, PawnStatus};
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_shared::m::MinMaxJs;
use serde_json::Value;

const DIRS: [(i8, i8); 9] = [
    (127, 0),
    (90, 90),
    (0, 127),
    (-90, 90),
    (-127, 0),
    (-90, -90),
    (0, -127),
    (90, -90),
    (0, 0),
];

fn script(id: u32, k: i64) -> InputFrame {
    let d = DIRS[((k.div_euclid(90) * 5 + id as i64 * 3).rem_euclid(DIRS.len() as i64)) as usize];
    let mut buttons = 0;
    if k >= 0 && k.rem_euclid(70) == id as i64 * 7 {
        buttons |= BTN_JUMP;
    }
    if k >= 0 && k.rem_euclid(250) == id as i64 * 31 {
        buttons |= BTN_DIVE;
    }
    InputFrame {
        mx: d.0,
        mz: d.1,
        buttons,
    }
}

fn ids(v: &Value) -> Vec<u32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u32)
        .collect()
}

const TOL: f64 = 1e-9;

/// An arena event as the TS hooks record it (without the tick).
fn same_event(e: &ArenaEvent, w: &Value) -> bool {
    let num = |v: &Value, x: f64| v.as_f64().is_some_and(|y| (x - y).abs() <= TOL * 1f64.max_js(x.abs()));
    match e {
        ArenaEvent::Bonus(b) => {
            let d = &w["data"];
            w["e"] == "@bonus" && d["i"] == b.i && d["id"] == b.id && num(&d["at"], b.at)
        }
        ArenaEvent::Finish { id, t } => w["e"] == "finish" && w["id"] == *id && num(&w["t"], *t),
        ArenaEvent::Ko(ko) => {
            w["e"] == "ko"
                && w["id"] == ko.id
                && w["out"] == ko.out
                && w["cause"] == ko.cause
                && w["shortcut"] == ko.shortcut
                && w["by"].as_u64().map(|b| b as u32) == ko.by
        }
        ArenaEvent::Emote { id, e } => w["e"] == "emote" && w["id"] == *id && w["emote"] == *e,
        ArenaEvent::Event { name, data, .. } => w["e"] == name.as_str() && close(data, &w["data"], TOL),
        ArenaEvent::Score { id, v } => w["e"] == "score" && w["id"] == *id && num(&w["v"], *v),
    }
}

fn kind(v: &Value) -> ArenaKind {
    match v.as_str() {
        Some("lobby") => ArenaKind::Lobby,
        Some("podium") => ArenaKind::Podium,
        _ => ArenaKind::Round,
    }
}

fn check_map(id: &str) {
    let data = load(id);
    let map = fb_maps::by_id(id).unwrap_or_else(|| panic!("no map {id} in fb_maps"));
    for run in data["runs"].as_array().unwrap() {
        let seed = run["seed"].as_u64().unwrap() as u32;
        let what = format!("{id} seed {seed}");
        let tick0 = run["tick0"].as_i64().unwrap();
        let end = run["end"].as_i64().unwrap();
        let humans = ids(&run["humans"]);
        let bots = ids(&run["bots"]);
        let participants: Vec<u32> = humans.iter().chain(&bots).copied().collect();
        let (mut arena, _) = Arena::new(map, kind(&run["kind"]), seed, tick0, &participants, false);
        assert_eq!(
            arena.static_hash,
            run["staticHash"].as_str().unwrap(),
            "{what}: static hash"
        );
        check_colliders(&what, &arena.world, &run["colliders"]);

        let bonuses = run["bonuses"].as_array().unwrap();
        assert_eq!(bonuses.len(), arena.bonuses.list.len(), "{what}: bonus count");
        for (b, g) in arena.bonuses.list.iter().zip(bonuses) {
            assert_eq!(b.kind as u64, g["kind"].as_u64().unwrap());
            assert_eq!(b.appear_at, f(&g["appearAt"]));
            // libm and V8 may differ in the last bit of sin/cos.
            assert!((b.pos.x - f(&g["x"])).abs() < 1e-12 && (b.pos.z - f(&g["z"])).abs() < 1e-12);
        }

        for &pid in &participants {
            arena.add_pawn(pid, bots.contains(&pid));
        }
        let states = run["states"].as_array().unwrap();
        assert_eq!(states.len() as i64, end - tick0, "{what}: one state hash per tick");
        let want = run["events"].as_array().unwrap();
        let mut rows = run["rows"].as_array().unwrap().iter().peekable();
        let mut worlds = run["worlds"].as_array().unwrap().iter().peekable();
        let mut events: Vec<(i64, ArenaEvent)> = Vec::new();
        let mut worst = 0.0f64;
        for (n, k) in (tick0 + 1..=end).enumerate() {
            let before = events.len();
            events.extend(arena.step(k, |pid| script(pid, k)).into_iter().map(|e| (k, e)));
            for (j, (k, e)) in events.iter().enumerate().skip(before) {
                let w = want.get(j);
                assert!(
                    w.is_some_and(|w| w["k"].as_i64() == Some(*k) && same_event(e, w)),
                    "{what}: event {j} at tick {k}: rust {e:?}, ts {}",
                    w.map_or("none".to_string(), Value::to_string)
                );
            }
            if let Some(row) = rows.next_if(|r| r[0].as_i64() == Some(k)) {
                assert_eq!(
                    row.as_array().unwrap().len(),
                    1 + arena.pawns.len() * 12,
                    "{what}: tick {k} row length"
                );
                worst = worst.max_js(check_bodies(&what, row, arena.pawns.iter().map(|p| (p.id, &p.body))));
                let extra = &row.as_array().unwrap()[1 + arena.pawns.len() * 10..];
                for (p, g) in arena.pawns.iter().zip(extra.chunks(2)) {
                    let status = match p.status {
                        PawnStatus::Play => 0,
                        PawnStatus::Finished => 1,
                        PawnStatus::Out => 2,
                    };
                    assert_eq!(status, g[0].as_i64().unwrap(), "{what}: tick {k} pawn {} status", p.id);
                    assert_eq!(
                        p.grabbing.map_or(-1, |g| g as i64),
                        g[1].as_i64().unwrap(),
                        "{what}: tick {k} pawn {} grabbing",
                        p.id
                    );
                }
            }
            assert_eq!(
                arena.state_hash(),
                states[n].as_str().unwrap(),
                "{what}: state hash at tick {k} (compare the rows around it)"
            );
            if let Some(w) = worlds.next_if(|w| w[0].as_i64() == Some(k)) {
                assert_eq!(
                    arena.world.hash(false),
                    w[1].as_str().unwrap(),
                    "{what}: world hash at tick {k}"
                );
            }
        }
        // (Rows and world hashes are in tick order: one left over was never compared.)
        assert!(rows.next().is_none(), "{what}: a row outside the run");
        assert!(worlds.next().is_none(), "{what}: a world hash outside the run");
        assert_eq!(events.len(), want.len(), "{what}: number of events");
        assert_eq!(arena.finished, ids(&run["finished"]), "{what}: finishing order");
        assert_eq!(arena.out, ids(&run["out"]), "{what}: elimination order");
        let stats = run["stats"].as_array().unwrap();
        assert_eq!(stats.len(), arena.pawns.len(), "{what}: stats per pawn");
        for (p, s) in arena.pawns.iter().zip(stats) {
            let st = &p.stats;
            let got = [st.falls, st.shortcuts, st.kos, st.grabs, st.tackles];
            let want = ["falls", "shortcuts", "kos", "grabs", "tackles"].map(|k| s[k].as_u64().unwrap() as u32);
            assert_eq!(
                got, want,
                "{what}: stats of pawn {} (falls, shortcuts, kos, grabs, tackles)",
                p.id
            );
            assert!((st.idle - f(&s["idle"])).abs() < 1e-9, "{what}: idle of pawn {}", p.id);
            assert_eq!(st.out_at, s["outAt"].as_f64(), "{what}: outAt of pawn {}", p.id);
            assert_eq!(
                st.finish_at,
                s["finishAt"].as_f64(),
                "{what}: finishAt of pawn {}",
                p.id
            );
        }
        let scores: Vec<(u32, f64)> = arena.scores.iter().map(|(&k, &v)| (k, v)).collect();
        let want_scores: Vec<(u32, f64)> = run["scores"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_u64().unwrap() as u32, f(&p[1])))
            .collect();
        assert_eq!(scores, want_scores, "{what}: scores");
        eprintln!(
            "{what}: {} ticks, {} events, out {:?}, worst difference {worst:e}",
            end - tick0,
            events.len(),
            arena.out
        );
    }
}

macro_rules! golden {
    ($($name:ident: $id:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                check_map($id);
            }
        )*
    };
}

golden! {
    door_dash: "door-dash",
    hammer_swing: "hammer-swing",
    ball_hill: "ball-hill",
    hidden_bridge: "hidden-bridge",
    drum_roll: "drum-roll",
    jump_club: "jump-club",
    roll_out: "roll-out",
    wall_rush: "wall-rush",
    hex_a_gone: "hex-a-gone",
    crown_peak: "crown-peak",
    plate_drop: "plate-drop",
    portal_panic: "portal-panic",
    bounce_park: "bounce-park",
    cliff_climb: "cliff-climb",
    frost_sky: "frost-sky",
    tail_tag: "tail-tag",
    star_fall: "star-fall",
    lobby: "lobby",
    podium: "podium",
}
