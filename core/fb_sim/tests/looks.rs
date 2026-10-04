//! Looks of rounds against the TS client (`lookFor` over map sets and seeds: `looks.json`, exported once).
use fb_sim::looks::{PAL_KEYS, look_for};
use serde_json::Value;

#[test]
fn rounds_pick_the_looks_of_ts() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("looks.json")).unwrap();
    for c in cases {
        let set: Vec<&str> = c["set"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let seed = c["seed"].as_u64().unwrap() as u32;
        let got = look_for(&set, seed);
        assert_eq!(got.look.id, c["id"].as_str().unwrap(), "{set:?} {seed}");
        assert_eq!(got.pattern.name(), c["pattern"].as_str().unwrap(), "{set:?} {seed}");
        for (i, k) in PAL_KEYS.iter().enumerate() {
            let want = &c["palette"][k];
            assert_eq!(got.palette[i][0], want[0].as_str().unwrap(), "{set:?} {seed} {k}");
            assert_eq!(got.palette[i][1], want[1].as_str().unwrap(), "{set:?} {seed} {k}");
        }
    }
}
