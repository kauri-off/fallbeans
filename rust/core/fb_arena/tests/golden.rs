//! The Rust port against golden traces exported from the TS build (`bun scripts/golden.ts`).
use fb_arena::{Arena, MapEvent};
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_sim::collider::Shape;
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
    if k.rem_euclid(70) == id as i64 * 7 && k >= 0 {
        buttons |= BTN_JUMP;
    }
    if k.rem_euclid(250) == id as i64 * 31 && k >= 0 {
        buttons |= BTN_DIVE;
    }
    InputFrame {
        mx: d.0,
        mz: d.1,
        buttons,
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn check_map(id: &str) {
    let path = format!("{}/tests/golden/{id}.json", env!("CARGO_MANIFEST_DIR"));
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let map = fb_maps::by_id(id).unwrap();
    for run in data["runs"].as_array().unwrap() {
        let seed = run["seed"].as_u64().unwrap() as u32;
        let intro = run["intro"].as_i64().unwrap();
        let (mut arena, _) = Arena::new(map, seed, -intro, false);
        assert_eq!(
            arena.static_hash,
            run["staticHash"].as_str().unwrap(),
            "{id} seed {seed}: static hash"
        );

        let cols = run["colliders"].as_array().unwrap();
        assert_eq!(
            cols.len(),
            arena.world.colliders.len(),
            "{id} seed {seed}: collider count"
        );
        for (i, (c, g)) in arena.world.colliders.iter().zip(cols).enumerate() {
            assert_eq!(c.is_static, g["isStatic"].as_bool().unwrap(), "collider {i} static");
            let s = &g["shape"];
            let ok = match c.shape {
                Shape::Box { hx, hy, hz } => {
                    s["type"] == "box" && hx == f(&s["hx"]) && hy == f(&s["hy"]) && hz == f(&s["hz"])
                }
                Shape::Cyl { r, hh } => s["type"] == "cyl" && r == f(&s["r"]) && hh == f(&s["hh"]),
                Shape::Sphere { r } => s["type"] == "sphere" && r == f(&s["r"]),
            };
            assert!(ok, "{id} seed {seed}: collider {i} shape {:?} vs {s}", c.shape);
            for (e, ge) in c.cur.0.iter().zip(g["cur"].as_array().unwrap()) {
                assert!(
                    (e - f(ge)).abs() < 1e-9,
                    "{id} seed {seed}: collider {i} matrix {:?}",
                    c.cur.0
                );
            }
        }

        let bonuses = run["bonuses"].as_array().unwrap();
        assert_eq!(bonuses.len(), arena.bonuses.list.len(), "{id} seed {seed}: bonus count");
        for (b, g) in arena.bonuses.list.iter().zip(bonuses) {
            assert_eq!(b.kind as u64, g["kind"].as_u64().unwrap());
            assert_eq!(b.appear_at, f(&g["appearAt"]));
            // libm and V8 may differ in the last bit of sin/cos.
            assert!((b.pos.x - f(&g["x"])).abs() < 1e-12 && (b.pos.z - f(&g["z"])).abs() < 1e-12);
        }

        let ids: Vec<u32> = run["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        for &pid in &ids {
            arena.add_pawn(pid);
        }
        let frames = run["frames"].as_array().unwrap();
        let hashes: Vec<(i64, String)> = run["hashes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| (h[0].as_i64().unwrap(), h[1].as_str().unwrap().to_string()))
            .collect();
        let mut events = Vec::new();
        let mut worst = 0.0f64;
        for row in frames {
            let row = row.as_array().unwrap();
            let k = row[0].as_i64().unwrap();
            for e in arena.step(k, |pid| script(pid, k)) {
                let MapEvent::Bonus(b) = e;
                events.push((k, b));
            }
            for (pi, p) in arena.pawns.iter().enumerate() {
                let g = &row[1 + pi * 10..1 + pi * 10 + 10];
                let b = &p.body;
                let got = [b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.yaw, b.tilt];
                for (j, v) in got.iter().enumerate() {
                    let d = (v - f(&g[j])).abs();
                    worst = worst.max(d);
                    assert!(
                        d < 1e-9,
                        "{id} seed {seed}: tick {k} body {} field {j}: rust {v} ts {} (state rust {:?} ts {})",
                        p.id,
                        f(&g[j]),
                        b.state,
                        g[8]
                    );
                }
                assert_eq!(
                    b.state as u8 as i64,
                    g[8].as_i64().unwrap(),
                    "{id} seed {seed}: tick {k} body {} state",
                    p.id
                );
                assert_eq!(
                    b.ground_col as i64,
                    g[9].as_i64().unwrap(),
                    "{id} seed {seed}: tick {k} body {} ground",
                    p.id
                );
            }
            if let Some((_, h)) = hashes.iter().find(|(hk, _)| *hk == k) {
                assert_eq!(&arena.world.hash(false), h, "{id} seed {seed}: world hash at tick {k}");
            }
        }
        let want = run["events"].as_array().unwrap();
        assert_eq!(events.len(), want.len(), "{id} seed {seed}: bonus events");
        for ((k, e), w) in events.iter().zip(want) {
            assert_eq!(*k, w["k"].as_i64().unwrap());
            assert_eq!(
                (e.i as u64, e.id as u64),
                (w["i"].as_u64().unwrap(), w["id"].as_u64().unwrap())
            );
        }
        eprintln!("{id} seed {seed}: {} ticks, worst difference {worst:e}", frames.len());
    }
}

#[test]
fn jump_club_matches_ts() {
    check_map("jump-club");
}
