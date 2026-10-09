//! Tail tag: a tail never leaves the room with its holder, and a grab on an immune tail pays off once the
//! immunity is over.
use core::f64::consts::FRAC_PI_2;
use fb_shared::PlayerId;
use fb_sim::map::MapId;

use fb_arena::{Arena, ArenaEvent, ArenaKind};
use fb_shared::TICK_RATE;
use fb_shared::input::{BTN_GRAB, InputFrame};
use fb_sim::map::MapEvent;
use fb_sim::math::V3;

#[test]
fn a_leaving_holder_hands_the_tail_on() {
    let map = fb_maps::by_id(MapId::TailTag);
    for seed in 1..=8 {
        let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, 0, &[1, 2, 3, 4].map(PlayerId), false);
        for id in (1..=4).map(PlayerId) {
            arena.add_pawn_at(id, false, Some(id.0 as usize - 1));
        }
        arena.step(0, |_| InputFrame::IDLE);
        // Everybody but bean 4 leaves: whatever tails they held end up with it, and it scores.
        for id in (1..=3).map(PlayerId) {
            arena.remove_pawn(id);
        }
        let end = 3 * i64::from(TICK_RATE);
        for k in 1..=end {
            arena.step(k, |_| InputFrame::IDLE);
        }
        let score = arena.scores.get(&PlayerId(4)).copied().unwrap_or(0);
        assert!(score >= 2, "seed {seed}: the last bean scored {score}");
    }
}

/// Runs `ticks` ticks after tick `k`, `grab` holding the grab button; returns who got a tail, in order.
fn run(arena: &mut Arena, k: &mut i64, ticks: i64, grab: Option<PlayerId>) -> Vec<PlayerId> {
    let mut got = Vec::new();
    for _ in 0..ticks {
        *k += 1;
        let frame = |id: PlayerId| InputFrame {
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
    let map = fb_maps::by_id(MapId::TailTag);
    let (mut arena, _) = Arena::new(map, ArenaKind::Round, 3, 0, &[1, 2].map(PlayerId), false);
    for id in (1..=2).map(PlayerId) {
        arena.add_pawn_at(id, false, Some(id.0 as usize - 1));
    }
    arena.step(0, |_| InputFrame::IDLE);
    // Face to face, within reach but not touching, on flat floor between the ramps, clear of the sweepers,
    // the trampolines and the portals.
    assert!(arena.dev_teleport(PlayerId(1), V3::new(6.0, 0.05, 6.0), Some(-FRAC_PI_2)));
    assert!(arena.dev_teleport(PlayerId(2), V3::new(4.7, 0.05, 6.0), Some(FRAC_PI_2)));
    let mut k = 0;
    // Whoever scores holds the one tail.
    run(&mut arena, &mut k, i64::from(TICK_RATE) * 5 / 4, None);
    let holder = if arena.scores.get(&PlayerId(1)).copied().unwrap_or(0) > 0 {
        PlayerId(1)
    } else {
        PlayerId(2)
    };
    let chaser = PlayerId(3 - holder.0);
    // The chaser takes it (and is immune for a while)…
    assert_eq!(run(&mut arena, &mut k, 12, Some(chaser)), [chaser]);
    run(&mut arena, &mut k, 6, None);
    // …the old holder grabs it straight back and holds on: the tail is theirs once the immunity is over,
    // well before the hold would let go.
    let back = run(&mut arena, &mut k, i64::from(TICK_RATE) * 12 / 5, Some(holder));
    assert_eq!(back, [holder]);
}

/// A tail slows its holder (`MapLogic::bean`): a client's prediction of its own bean (the client's map, the
/// same step, the same hook) must match the server's every tick, or it rolls back on every snapshot.
#[test]
fn a_client_predicts_the_tail_slowing_its_holder() {
    use fb_arena::{MapRun, Stepper, build_map, tick_bodies};
    use fb_shared::DT;
    use fb_sim::physics::StepEvents;

    let map = fb_maps::by_id(MapId::TailTag);
    let (mut arena, _) = Arena::new(map, ArenaKind::Round, 5, 0, &[1].map(PlayerId), false);
    arena.add_pawn_at(PlayerId(1), false, Some(0));
    let (mut b, mut spec) = build_map(map, 5, true, &[1].map(PlayerId));
    b.world.finalize(-1e3, &*spec.logic);
    let walk = InputFrame::from_stick(0.3, 1.0, 0);
    let mut body = arena.pawn(PlayerId(1)).unwrap().body.clone();
    let mut slowed = 0;
    // (Off the rim after some 230 ticks: a respawn is not this test.)
    for k in 0..200 {
        arena.step(k, |_| walk);
        let t = k as f64 * DT;
        let (mut ev, mut scores, mut out) = (StepEvents::default(), Default::default(), Vec::new());
        let mut map = MapRun {
            logic: &mut *spec.logic,
            server: false,
            apply: false,
            me: Some(PlayerId(1)),
            scores: &mut scores,
            out: &mut out,
        };
        let mut steppers = [Stepper {
            id: PlayerId(1),
            body: &mut body,
            ev: &mut ev,
            input: walk.into(),
        }];
        tick_bodies(&mut b.world, t, &mut steppers, &[], &mut map);
        spec.logic.bean(PlayerId(1), &mut body, t);
        let server = &arena.pawn(PlayerId(1)).unwrap().body;
        assert_eq!(&body, server, "tick {k}");
        slowed += usize::from(server.slow_k < 1.0);
    }
    assert!(slowed > 180, "the only bean has a tail and is slowed: {slowed} ticks");
}
