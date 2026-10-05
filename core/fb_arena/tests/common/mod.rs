//! Checks shared by the golden-trace tests.
use fb_shared::m::MinMaxJs;
use fb_sim::collider::Shape;
use fb_sim::physics::Body;
use fb_sim::world::World;
use serde_json::Value;

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// A trace recorded by the TS version (gzipped JSON).
pub fn load(name: &str) -> Value {
    let path = format!("{}/tests/golden/{name}.json.gz", env!("CARGO_MANIFEST_DIR"));
    // (Frozen: the TS version that recorded them is gone; it is tagged `ts-final`.)
    let file =
        std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e} (golden traces cannot be re-recorded)"));
    serde_json::from_reader(std::io::BufReader::new(flate2::read::GzDecoder::new(file))).unwrap()
}

/// JSON values equal up to `tol` in every number (the TS side writes what JS computed).
#[allow(dead_code, reason = "golden.rs uses it, scenarios.rs does not")]
pub fn close(a: &Value, b: &Value, tol: f64) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            (x - y).abs() <= tol * 1f64.max_js(x.abs())
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| close(a, b, tol)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w, tol)))
        }
        _ => a == b,
    }
}

/// Shapes, flags and matrices of every collider, in build order.
pub fn check_colliders(what: &str, world: &World, cols: &Value) {
    let Some(cols) = cols.as_array() else { return };
    assert_eq!(cols.len(), world.colliders.len(), "{what}: collider count");
    for (i, (c, g)) in world.colliders.iter().zip(cols).enumerate() {
        assert_eq!(
            c.is_static,
            g["isStatic"].as_bool().unwrap(),
            "{what}: collider {i} static"
        );
        let s = &g["shape"];
        let eq = |a: f64, key: &str| a == f(&s[key]);
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
            worst = worst.max_js(d);
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
