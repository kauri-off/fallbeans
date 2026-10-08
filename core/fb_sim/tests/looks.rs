//! Looks of rounds by map set and seed (`looks.json`): a seed keeps the look, pattern and palette it had.
//! Record again after an intended change with `FB_BLESS=1 cargo test -p fb_sim --test looks`.
use fb_sim::looks::{LookId, Swatch, look_for};
use serde_json::{Map, Value, json};

fn case(set: &[LookId], seed: u32) -> Value {
    let got = look_for(set, seed);
    let palette: Map<String, Value> = Swatch::ALL
        .into_iter()
        .map(|s| {
            let [a, b] = got.tones(s);
            (s.name().to_string(), json!([a, b]))
        })
        .collect();
    let set: Vec<&str> = set.iter().map(|l| l.name()).collect();
    json!({ "set": set, "seed": seed, "id": got.look.id.name(), "pattern": got.pattern.name(), "palette": palette })
}

#[test]
fn round_looks_are_stable() {
    let path = format!("{}/tests/looks.json", env!("CARGO_MANIFEST_DIR"));
    let want: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let got: Vec<Value> = want
        .iter()
        .map(|c| {
            let set: Vec<LookId> = c["set"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let name = v.as_str().unwrap();
                    fb_sim::looks::LOOKS
                        .iter()
                        .map(|l| l.id)
                        .find(|l| l.name() == name)
                        .unwrap()
                })
                .collect();
            case(&set, c["seed"].as_u64().unwrap() as u32)
        })
        .collect();
    if std::env::var("FB_BLESS").is_ok() {
        let lines: Vec<String> = got.iter().map(|c| c.to_string()).collect();
        std::fs::write(&path, format!("[\n{}\n]\n", lines.join(",\n"))).unwrap();
        return;
    }
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w, "the look of {} with seed {}", w["set"], w["seed"]);
    }
}
