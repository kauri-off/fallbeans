//! A recorded round (humans' frames and views, bots from the seed, dev ops in between) replays to the same state.
use fb_arena::{Arena, ArenaKind, replay};
use fb_shared::PlayerId;
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_sim::map::MapId;
use fb_sim::math::V3;

#[test]
fn replays_end_in_the_recorded_state() {
    for map in [MapId::DoorDash, MapId::TailTag, MapId::HexAGone] {
        let def = fb_maps::by_id(map);
        let ids: Vec<PlayerId> = (1..=6).map(PlayerId).collect();
        let (mut a, _) = Arena::new(def, ArenaKind::Round, 42, -360, &ids, false);
        a.record();
        for &id in &ids {
            a.add_pawn(id, id.0 > 2);
        }
        for k in -359..=3600i64 {
            if k == 1200 {
                a.dev_knock(PlayerId(1), V3::new(3.0, 4.0, 0.0));
            }
            if k == 2400 {
                a.add_late_pawn(PlayerId(9), true, None);
            }
            if k == 600 || k == 1800 {
                a.set_view(PlayerId(1), (k / 40) as u32);
                a.set_view(PlayerId(2), 7);
            }
            a.step(k, |id| InputFrame {
                mx: ((k / 50 + i64::from(id.0)) % 3 * 60 - 60) as i8,
                mz: 100,
                buttons: if k % 90 == 0 {
                    BTN_JUMP
                } else if k % 170 == i64::from(id.0) * 20 {
                    BTN_DIVE
                } else {
                    0
                },
            });
        }
        let rec = a.take_recording().unwrap();
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains(&format!("\"game\":\"{map}\"")), "{map}");
        let rec = serde_json::from_str(&json).unwrap();
        let r = replay(&rec, |_| false);
        assert!(r.matches, "{map}: replay {} vs recorded {}", r.hash, a.state_hash());
    }
}

#[test]
fn ticks_the_server_skipped_are_skipped_on_replay() {
    let def = fb_maps::by_id(MapId::DoorDash);
    let ids: Vec<PlayerId> = (1..=4).map(PlayerId).collect();
    let (mut a, _) = Arena::new(def, ArenaKind::Round, 5, -360, &ids, false);
    a.record();
    for &id in &ids {
        a.add_pawn(id, id.0 > 1);
    }
    let mut k = -359;
    while k <= 1500 {
        if k == 600 {
            a.skip_to(700);
            k = 701;
        }
        a.step(k, |_| InputFrame {
            mx: 40,
            mz: 100,
            buttons: 0,
        });
        k += 1;
    }
    let rec = a.take_recording().unwrap();
    assert!(replay(&rec, |_| false).matches);
    let mut unskipped = rec;
    unskipped.ops.retain(|o| !matches!(o.1, fb_arena::Op::Skip(_)));
    assert!(!replay(&unskipped, |_| false).matches);
}

#[test]
fn a_traced_round_logs_dives_and_tackles_and_leaves_the_state_alone() {
    let def = fb_maps::by_id(MapId::DoorDash);
    let ids: Vec<PlayerId> = (1..=8).map(PlayerId).collect();
    let run = |traced: bool| {
        let (mut a, _) = Arena::new(def, ArenaKind::Round, 7, -360, &ids, false);
        if traced {
            a.hit_log = Some(Vec::new());
        }
        for &id in &ids {
            a.add_pawn(id, true);
        }
        for k in -359..=3000i64 {
            a.step(k, |_| InputFrame::IDLE);
        }
        (a.state_hash(), a.hit_log.unwrap_or_default())
    };
    let (plain, _) = run(false);
    let (hash, log) = run(true);
    assert_eq!(plain, hash);
    let has = |what: &str| log.iter().any(|l| l.split(' ').nth(2) == Some(what));
    assert!(has("dive") && has("tackle"), "{:?}", &log[..log.len().min(20)]);
}
