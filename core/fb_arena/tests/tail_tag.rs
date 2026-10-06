//! Tail tag: a tail never leaves the room with its holder, and a grab on an immune tail pays off once the
//! immunity is over.
use core::f64::consts::FRAC_PI_2;

use fb_arena::{Arena, ArenaEvent, ArenaKind};
use fb_shared::TICK_RATE;
use fb_shared::input::{BTN_GRAB, InputFrame};
use fb_sim::map::MapEvent;
use fb_sim::math::V3;

#[test]
fn a_leaving_holder_hands_the_tail_on() {
    let map = fb_maps::by_id("tail-tag").unwrap();
    for seed in 1..=8 {
        let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, 0, &[1, 2, 3, 4], false);
        for id in 1..=4 {
            arena.add_pawn_at(id, false, Some(id as usize - 1));
        }
        arena.step(0, |_| InputFrame::IDLE);
        // Everybody but bean 4 leaves: whatever tails they held end up with it, and it scores.
        for id in 1..=3 {
            arena.remove_pawn(id);
        }
        let end = 3 * i64::from(TICK_RATE);
        for k in 1..=end {
            arena.step(k, |_| InputFrame::IDLE);
        }
        let score = arena.scores.get(&4).copied().unwrap_or(0.0);
        assert!(score >= 2.0, "seed {seed}: the last bean scored {score}");
    }
}

/// Runs `ticks` ticks after tick `k`, `grab` holding the grab button; returns who got a tail, in order.
fn run(arena: &mut Arena, k: &mut i64, ticks: i64, grab: Option<u32>) -> Vec<u32> {
    let mut got = Vec::new();
    for _ in 0..ticks {
        *k += 1;
        let frame = |id| InputFrame {
            buttons: if Some(id) == grab { BTN_GRAB } else { 0 },
            ..InputFrame::IDLE
        };
        for e in arena.step(*k, frame) {
            if let ArenaEvent::Event {
                ev: MapEvent::Tails { by, .. },
                ..
            } = e
            {
                got.push(by);
            }
        }
    }
    got
}

#[test]
fn a_tail_held_through_its_immunity_changes_hands_as_it_ends() {
    let map = fb_maps::by_id("tail-tag").unwrap();
    let (mut arena, _) = Arena::new(map, ArenaKind::Round, 3, 0, &[1, 2], false);
    for id in 1..=2 {
        arena.add_pawn_at(id, false, Some(id as usize - 1));
    }
    arena.step(0, |_| InputFrame::IDLE);
    // Face to face, within reach but not touching, on flat floor between the ramps, clear of the sweepers,
    // the trampolines and the portals.
    assert!(arena.dev_teleport(1, V3::new(6.0, 0.05, 6.0), Some(-FRAC_PI_2)));
    assert!(arena.dev_teleport(2, V3::new(4.7, 0.05, 6.0), Some(FRAC_PI_2)));
    let mut k = 0;
    // Whoever scores holds the one tail.
    run(&mut arena, &mut k, i64::from(TICK_RATE) * 5 / 4, None);
    let holder = if arena.scores.get(&1).copied().unwrap_or(0.0) > 0.0 {
        1
    } else {
        2
    };
    let chaser = 3 - holder;
    // The chaser takes it (and is immune for a while)…
    assert_eq!(run(&mut arena, &mut k, 12, Some(chaser)), [chaser]);
    run(&mut arena, &mut k, 6, None);
    // …the old holder grabs it straight back and holds on: the tail is theirs once the immunity is over,
    // well before the hold would let go.
    let back = run(&mut arena, &mut k, i64::from(TICK_RATE) * 12 / 5, Some(holder));
    assert_eq!(back, [holder]);
}
