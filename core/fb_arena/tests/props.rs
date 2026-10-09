//! Properties of whole arenas on generated seeds and human input: every map builds, a round is a function
//! of its seed and input, and its recording (through JSON) replays to the same state.
use fb_arena::{Arena, ArenaKind, Recording, replay};
use fb_shared::PlayerId;
use fb_shared::input::InputFrame;
use fb_sim::map::MapId;
use proptest::prelude::*;
use proptest::test_runner::Config;

fn kind(map: MapId) -> ArenaKind {
    match map {
        MapId::Lobby => ArenaKind::Lobby,
        MapId::Podium => ArenaKind::Podium,
        _ => ArenaKind::Round,
    }
}

/// A human's input as runs of one frame, `(ticks, frame)`.
fn presses() -> impl Strategy<Value = Vec<(i64, InputFrame)>> {
    let frame = (any::<i8>(), any::<i8>(), 0u8..8)
        .prop_map(|(mx, mz, b)| InputFrame::from_stick(f64::from(mx) / 127.0, f64::from(mz) / 127.0, b));
    prop::collection::vec((1i64..90, frame), 1..24)
}

fn frame_at(runs: &[(i64, InputFrame)], k: i64) -> InputFrame {
    let mut t = 0;
    for &(len, f) in runs {
        t += len;
        if k < t {
            return f;
        }
    }
    InputFrame::IDLE
}

/// A round of two humans and two bots from `tick0` to `end`, recorded.
fn play(map: MapId, seed: u32, humans: &[Vec<(i64, InputFrame)>; 2], tick0: i64, end: i64) -> (Arena, Recording) {
    let ids: Vec<PlayerId> = (1..=4).map(PlayerId).collect();
    let (mut a, _) = Arena::new(fb_maps::by_id(map), kind(map), seed, tick0, &ids, false);
    a.record();
    for &id in &ids {
        a.add_pawn(id, id.0 > 2);
    }
    for k in tick0 + 1..=end {
        a.step(k, |id| match id.0 {
            1 | 2 => frame_at(&humans[id.0 as usize - 1], k - tick0),
            _ => InputFrame::IDLE,
        });
    }
    let rec = a.take_recording().expect("recording");
    (a, rec)
}

proptest! {
    #![proptest_config(Config::with_cases(24))]

    #[test]
    fn every_map_builds_with_any_seed(seed: u32) {
        for map in MapId::ALL {
            let ids: Vec<PlayerId> = (1..=8).map(PlayerId).collect();
            let (mut a, _) = Arena::new(fb_maps::by_id(map), kind(map), seed, -360, &ids, false);
            for &id in &ids {
                a.add_pawn(id, true);
            }
            for c in &a.world.colliders {
                prop_assert!(c.cur.is_finite() && c.radius.is_finite(), "{map} seed {seed}: collider {}", c.index);
            }
            for p in &a.pawns {
                prop_assert!(p.body.pos.is_finite(), "{map} seed {seed}: {} spawns at {}", p.id, p.body.pos);
            }
        }
    }

    #[test]
    fn rounds_are_functions_of_seed_and_input(
        map in prop::sample::select(MapId::ALL.to_vec()),
        seed: u32,
        humans in [presses(), presses()],
    ) {
        let (a, rec) = play(map, seed, &humans, -30, 1200);
        for p in &a.pawns {
            prop_assert!(p.body.pos.is_finite() && p.body.vel.is_finite(), "{map}: {} at {}", p.id, p.body.pos);
        }
        let (b, _) = play(map, seed, &humans, -30, 1200);
        prop_assert_eq!(a.state_hash(), b.state_hash(), "{} seed {}", map, seed);
        let json = serde_json::to_string(&rec).expect("serialized");
        let back: Recording = serde_json::from_str(&json).expect("parsed");
        prop_assert_eq!(&back, &rec);
        let r = replay(&back, |_| false);
        prop_assert!(r.matches, "{map} seed {seed}: replay {} vs recorded {}", r.hash, a.state_hash());
    }
}
