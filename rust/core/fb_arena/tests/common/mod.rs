//! Checks shared by the golden-trace tests.
use fb_sim::collider::Shape;
use fb_sim::physics::Body;
use fb_sim::world::World;
use serde_json::Value;

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

pub fn load(name: &str) -> Value {
    let path = format!("{}/tests/golden/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Shapes, flags and matrices of every collider, in build order.
pub fn check_colliders(what: &str, world: &World, cols: &Value) {
    let cols = cols.as_array().unwrap();
    assert_eq!(cols.len(), world.colliders.len(), "{what}: collider count");
    for (i, (c, g)) in world.colliders.iter().zip(cols).enumerate() {
        assert_eq!(
            c.is_static,
            g["isStatic"].as_bool().unwrap(),
            "{what}: collider {i} static"
        );
        let s = &g["shape"];
        // Sizes may differ in the last bit: Bun's two-argument Math.hypot is the platform libm's.
        let eq = |a: f64, key: &str| (a - f(&s[key])).abs() < 1e-12;
        let ok = match c.shape {
            Shape::Box { hx, hy, hz } => s["type"] == "box" && eq(hx, "hx") && eq(hy, "hy") && eq(hz, "hz"),
            Shape::Cyl { r, hh } => s["type"] == "cyl" && eq(r, "r") && eq(hh, "hh"),
            Shape::Sphere { r } => s["type"] == "sphere" && eq(r, "r"),
        };
        assert!(ok, "{what}: collider {i} shape {:?} vs {s}", c.shape);
        for (e, ge) in c.cur.0.iter().zip(g["cur"].as_array().unwrap()) {
            assert!((e - f(ge)).abs() < 1e-9, "{what}: collider {i} matrix {:?}", c.cur.0);
        }
    }
}

/// One trace row (tick, then 10 numbers per body); returns the largest difference.
pub fn check_bodies<'a>(what: &str, row: &Value, bodies: impl Iterator<Item = (u32, &'a Body)>) -> f64 {
    let row = row.as_array().unwrap();
    let k = row[0].as_i64().unwrap();
    let mut worst = 0.0f64;
    for (bi, (id, b)) in bodies.enumerate() {
        let g = &row[1 + bi * 10..1 + bi * 10 + 10];
        let got = [b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.yaw, b.tilt];
        for (j, v) in got.iter().enumerate() {
            let d = (v - f(&g[j])).abs();
            worst = worst.max(d);
            assert!(
                d < 1e-9,
                "{what}: tick {k} body {id} field {j}: rust {v} ts {} (state rust {:?} ts {})",
                f(&g[j]),
                b.state,
                g[8]
            );
        }
        assert_eq!(
            b.state as u8 as i64,
            g[8].as_i64().unwrap(),
            "{what}: tick {k} body {id} state"
        );
        assert_eq!(
            b.ground_col as i64,
            g[9].as_i64().unwrap(),
            "{what}: tick {k} body {id} ground"
        );
    }
    worst
}
