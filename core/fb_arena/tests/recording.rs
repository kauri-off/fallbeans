//! A recorded round (humans' frames, bots from the seed, dev ops in between) replays to the same state.
use fb_arena::{Arena, ArenaKind, replay};
use fb_shared::input::{BTN_JUMP, InputFrame};
use fb_sim::math::V3;

#[test]
fn replays_end_in_the_recorded_state() {
    for map in ["door-dash", "tail-tag", "hex-a-gone"] {
        let def = fb_maps::by_id(map).unwrap();
        let ids: Vec<u32> = (1..=6).collect();
        let (mut a, _) = Arena::new(def, ArenaKind::Round, 42, -360, &ids, false);
        a.record();
        for &id in &ids {
            a.add_pawn(id, id > 2);
        }
        for k in -359..=3600i64 {
            if k == 1200 {
                a.dev_knock(1, V3::new(3.0, 4.0, 0.0));
            }
            if k == 2400 {
                a.add_late_pawn(9, true, None);
            }
            a.step(k, |id| InputFrame {
                mx: ((k / 50 + id as i64) % 3 * 60 - 60) as i8,
                mz: 100,
                buttons: if k % 90 == 0 { BTN_JUMP } else { 0 },
            });
        }
        let rec = a.take_recording().unwrap();
        let json = serde_json::to_string(&rec).unwrap();
        let rec = serde_json::from_str(&json).unwrap();
        let r = replay(&rec, |_| false).unwrap();
        assert!(r.matches, "{map}: replay {} vs recorded {}", r.hash, a.state_hash());
    }
}
