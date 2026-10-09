//! Tail tag: a tail never leaves the room with its holder, a grab on an immune tail pays off once the
//! immunity is over, and a client's prediction slows its bean by the tails of each tick.
use fb_shared::PlayerId;
use fb_sim::map::MapId;

use fb_arena::{Arena, ArenaEvent, ArenaKind};
use fb_shared::TICK_RATE;
use fb_shared::input::{BTN_GRAB, InputFrame};
use fb_sim::map::{Bodies, Cx, MapEvent, MapOut, Role};
use fb_sim::math::V3;
use fb_sim::physics::Body;

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

/// The map's logic alone, as the server runs it, for two beans a metre apart: no world, no physics, no
/// geometry for a test to depend on.
struct Logic {
    b: fb_sim::builder::Builder,
    spec: fb_sim::map::MapSpec,
    beans: Pair,
    scores: std::collections::BTreeMap<PlayerId, i64>,
    holder: PlayerId,
    chaser: PlayerId,
}

struct Pair([(PlayerId, Body); 2]);

impl Bodies for Pair {
    fn ids(&self) -> Vec<PlayerId> {
        self.0.iter().map(|b| b.0).collect()
    }
    fn get(&self, id: PlayerId) -> Option<&Body> {
        self.0.iter().find(|b| b.0 == id).map(|b| &b.1)
    }
    fn get_mut(&mut self, id: PlayerId) -> Option<&mut Body> {
        self.0.iter_mut().find(|b| b.0 == id).map(|b| &mut b.1)
    }
}

impl Logic {
    fn new() -> Self {
        let ids = [1, 2].map(PlayerId);
        let (b, spec) = fb_arena::build_map(fb_maps::by_id(MapId::TailTag), 3, false, &ids);
        let at = |x: f64| {
            let mut body = Body::new(0);
            body.pos = V3::new(x, 0.0, 0.0);
            body
        };
        let mut l = Logic {
            b,
            spec,
            beans: Pair([(ids[0], at(0.0)), (ids[1], at(1.0))]),
            scores: Default::default(),
            holder: ids[0],
            chaser: ids[1],
        };
        if !l.has(ids[0]) {
            (l.holder, l.chaser) = (ids[1], ids[0]);
        }
        assert!(l.has(l.holder) && !l.has(l.chaser), "one tail between two beans");
        l
    }

    /// `id` has a tail now (it slows them).
    fn has(&self, id: PlayerId) -> bool {
        let mut body = Body::new(0);
        self.spec.logic.bean(id, &mut body, 1e6);
        body.slow_k < 1.0
    }

    /// Tick k: `grab` grabs, `holding` holds on (renewing its slow-down, as fb_arena's hold does); returns who
    /// got a tail.
    fn step(&mut self, k: i64, grab: Option<(PlayerId, PlayerId)>, holding: Option<PlayerId>) -> Vec<PlayerId> {
        let t = k as f64 * fb_shared::DT;
        if let Some(body) = holding.and_then(|id| self.beans.get_mut(id)) {
            body.slow_until = t + 0.15;
        }
        let mut out = Vec::new();
        let mut cx = Cx::new(
            Role::SERVER,
            t,
            &mut self.b.world,
            &mut self.beans,
            &mut self.scores,
            &mut out,
        );
        if let Some((actor, target)) = grab {
            self.spec.logic.grab(&mut cx, actor, target);
        }
        self.spec.logic.tick(&mut cx, t);
        out.into_iter()
            .filter_map(|o| match o {
                MapOut::Event {
                    ev: MapEvent::Tails { by, .. },
                    ..
                } => Some(by),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_tail_held_through_its_immunity_changes_hands_as_it_ends() {
    let mut l = Logic::new();
    let (holder, chaser) = (l.holder, l.chaser);
    let start = i64::from(TICK_RATE);
    // The chaser takes it (and is immune for a while)…
    assert_eq!(l.step(start, Some((chaser, holder)), Some(chaser)), [chaser]);
    // …the old holder grabs straight back and holds on…
    assert!(l.step(start + 6, Some((holder, chaser)), Some(holder)).is_empty());
    let back = (start + 7..start + 4 * i64::from(TICK_RATE))
        .find(|&k| match l.step(k, None, Some(holder)).as_slice() {
            [] => false,
            got => {
                assert_eq!(got, [holder], "tick {k}");
                true
            }
        })
        .expect("the tail goes back to a holder who held on");
    // …and it is theirs once the immunity is over, not on the grab.
    assert!(
        back - start > i64::from(TICK_RATE) / 2,
        "back after {} ticks",
        back - start
    );
    assert!(l.has(holder) && !l.has(chaser));
}

#[test]
fn a_grab_let_go_during_the_immunity_takes_nothing() {
    let mut l = Logic::new();
    let (holder, chaser) = (l.holder, l.chaser);
    let start = i64::from(TICK_RATE);
    assert_eq!(l.step(start, Some((chaser, holder)), None), [chaser]);
    // Grabbed back and let go at once: no hold renews the grabber's slow-down.
    assert!(l.step(start + 6, Some((holder, chaser)), None).is_empty());
    for k in start + 7..start + 4 * i64::from(TICK_RATE) {
        assert!(l.step(k, None, None).is_empty(), "tick {k}");
    }
    assert!(l.has(chaser));
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

/// A client learns of a tail passed to it a round trip late, and rolls back to a server state from before
/// the pass: replayed, its bean has no tail until the tick it got one, and the server's after it.
#[test]
fn a_client_replays_a_pass_at_its_tick() {
    use fb_arena::{MapRun, Stepper, build_map, client_event, tick_bodies};
    use fb_shared::DT;
    use fb_sim::map::MapSpec;
    use fb_sim::physics::StepEvents;
    use fb_sim::world::World;

    fn predict(world: &mut World, spec: &mut MapSpec, me: PlayerId, body: &mut Body, k: i64) {
        let t = k as f64 * DT;
        let (mut ev, mut scores, mut out) = (StepEvents::default(), Default::default(), Vec::new());
        let mut map = MapRun {
            logic: &mut *spec.logic,
            server: false,
            apply: false,
            me: Some(me),
            scores: &mut scores,
            out: &mut out,
        };
        let mut steppers = [Stepper {
            id: me,
            body: &mut *body,
            ev: &mut ev,
            input: InputFrame::IDLE.into(),
        }];
        tick_bodies(world, t, &mut steppers, &[], &mut map);
        spec.logic.bean(me, body, t);
    }

    const LATE: i64 = 24;
    const BACK: i64 = 12;
    let map = fb_maps::by_id(MapId::TailTag);
    let ids = [1, 2].map(PlayerId);
    let (mut arena, _) = Arena::new(map, ArenaKind::Round, 3, 0, &ids, false);
    for id in ids {
        arena.add_pawn_at(id, false, Some(id.0 as usize - 1));
    }
    let mut k = 0;
    run(&mut arena, &mut k, i64::from(TICK_RATE) * 5 / 4, None);
    let holder = if arena.scores.get(&PlayerId(1)).copied().unwrap_or(0) > 0 {
        PlayerId(1)
    } else {
        PlayerId(2)
    };
    let me = PlayerId(3 - holder.0);
    let (mut b, mut spec) = build_map(map, 3, true, &ids);
    b.world.finalize(-1e3, &*spec.logic);
    let mut server = vec![(k, arena.pawn(me).unwrap().body.clone())];
    let mut pass = None;
    for i in 0..2 * BACK + 3 * LATE {
        // The holder drops off: nobody knocked it, so the tail goes to the only other bean.
        if i == 2 * BACK {
            assert!(arena.dev_kill(holder));
        }
        k += 1;
        for e in arena.step(k, |_| InputFrame::IDLE) {
            if let ArenaEvent::Event {
                ev: ev @ MapEvent::Tails { by, .. },
                ..
            } = e
                && by == me
            {
                pass = Some((k, ev));
            }
        }
        server.push((k, arena.pawn(me).unwrap().body.clone()));
    }
    let (at, ev) = pass.expect("the tail passes");
    let body_at = |k: i64| server.iter().find(|s| s.0 == k).map(|s| s.1.clone()).unwrap();
    assert!(
        body_at(at).slow_k < 1.0 && body_at(at - 1).slow_k >= 1.0,
        "slowed from the pass on"
    );

    // Predicted without the tail until the event comes…
    let mut body = body_at(server[0].0);
    for k in server[0].0 + 1..=at + LATE {
        predict(&mut b.world, &mut spec, me, &mut body, k);
    }
    let (mut scores, mut out) = (Default::default(), Vec::new());
    client_event(&mut b.world, &mut spec, &mut scores, Some(me), &ev, &mut out);
    // …then rolled back to before the pass and replayed: every tick as the server had it.
    let mut body = body_at(at - BACK);
    for k in at - BACK + 1..=at + 2 * LATE {
        predict(&mut b.world, &mut spec, me, &mut body, k);
        assert_eq!(body, body_at(k), "tick {k} (the pass at {at})");
    }
}
