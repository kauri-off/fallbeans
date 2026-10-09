//! Hex-a-gone: a tile drops a fixed time after a bean first touches it, and a bean that stands still goes down
//! through every floor and out.
use fb_arena::{Arena, ArenaEvent, ArenaKind};
use fb_shared::TICK_RATE;
use fb_shared::input::InputFrame;
use fb_sim::map::MapEvent;

const FALL_DELAY: f64 = 0.42;

#[test]
fn a_bean_standing_still_drops_through_every_floor() {
    let map = fb_maps::by_id("hex-a-gone").unwrap();
    for seed in [1, 4, 9] {
        let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, 0, &[1, 2], false);
        arena.add_pawn_at(1, false, Some(0));
        arena.add_pawn_at(2, false, Some(1));
        let mut tiles = 0;
        let mut out_at = None;
        for k in 0..30 * i64::from(TICK_RATE) {
            let events = arena.step(k, |_| InputFrame::IDLE);
            let now = arena.time();
            for e in events {
                match e {
                    ArenaEvent::Event {
                        ev: MapEvent::Tile { at, .. },
                        ..
                    } => {
                        tiles += 1;
                        assert!(
                            (at - now - FALL_DELAY).abs() < 1e-6,
                            "seed {seed}: a tile drops at {at}, now {now}"
                        );
                    }
                    ArenaEvent::Ko(ko) if ko.id == 1 && ko.out => out_at = Some(now),
                    _ => {}
                }
            }
            if out_at.is_some() {
                break;
            }
        }
        let out_at = out_at.unwrap_or_else(|| panic!("seed {seed}: bean 1 never went out"));
        assert!(tiles >= 3, "seed {seed}: {tiles} tiles dropped before bean 1 went out");
        assert!(
            out_at > 3.0 * FALL_DELAY,
            "seed {seed}: out at {out_at} s, too soon for three floors"
        );
    }
}
