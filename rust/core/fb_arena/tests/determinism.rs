//! Same records, same state on every OS: full rounds of four scripted beans and four bots end in a
//! recorded hash. Regenerate after an intended change with `FB_BLESS=1 cargo test -p fb_arena --test determinism`.
use fb_arena::{Arena, ArenaKind};
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};

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
const SEEDS: [u32; 3] = [1, 777, 123_456_789];

fn script(id: u32, k: i64) -> InputFrame {
    let d = DIRS[((k.div_euclid(90 + id as i64 * 7) * 5 + id as i64 * 3).rem_euclid(DIRS.len() as i64)) as usize];
    let mut buttons = 0;
    if k.rem_euclid(70 + id as i64) == id as i64 * 7 {
        buttons |= BTN_JUMP;
    }
    if k.rem_euclid(250) == id as i64 * 31 {
        buttons |= BTN_DIVE;
    }
    InputFrame {
        mx: d.0,
        mz: d.1,
        buttons,
    }
}

fn run(map: &str, seed: u32) -> String {
    let def = fb_maps::by_id(map).unwrap();
    let ids: Vec<u32> = (1..=8).collect();
    let (mut arena, _) = Arena::new(def, ArenaKind::Round, seed, -720, &ids, false);
    for id in 1..=8 {
        arena.add_pawn(id, id > 4);
    }
    let end = (def.meta().duration * 120.0) as i64;
    let mut events = 0;
    for k in -719..=end {
        events += arena.step(k, |id| script(id, k)).len();
    }
    format!("{map} {seed} {} {events}", arena.state_hash())
}

#[test]
fn rounds_end_in_the_recorded_state() {
    let got: Vec<String> = fb_maps::MAPS
        .iter()
        .flat_map(|m| SEEDS.map(|s| run(m.meta().id, s)))
        .collect();
    let path = format!("{}/tests/golden/determinism.txt", env!("CARGO_MANIFEST_DIR"));
    let text = got.join("\n") + "\n";
    println!("{text}");
    if std::env::var("FB_BLESS").is_ok() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("no recorded hashes: run with FB_BLESS=1");
    assert_eq!(
        text,
        want.replace("\r\n", "\n"),
        "the simulation is not the same as recorded"
    );
}
