//! Client prediction replays ticks from a server state after its world has run ahead: the replay must give
//! the same bodies, bit for bit, as ticking straight through (a bean riding a moving, turning platform).
use fb_arena::{Stepper, no_touch, tick_bodies};
use fb_shared::{DT, m};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyInput, StepEvents};
use fb_sim::scene::pal;
use fb_sim::world::World;

fn platform_world() -> World {
    let mut b = Builder::new(1, false);
    let p = b.box_(
        0.0,
        -0.5,
        0.0,
        8.0,
        1.0,
        8.0,
        pal::BLUE,
        PrimOpts {
            dynamic: true,
            ..Default::default()
        },
    );
    let node = p.node;
    b.mover(move |t, ctx| {
        let n = ctx.node(node);
        n.pos.x = m::sin(t * 0.8) * 3.0;
        n.rot.y = t * 0.6;
    });
    b.world.finalize(0.0);
    b.world
}

fn input(k: i64) -> BodyInput {
    BodyInput {
        // Mostly standing (riding along), a short walk now and then.
        mx: if k % 90 < 6 { 0.4 } else { 0.0 },
        mz: if k % 90 < 6 { -0.3 } else { 0.0 },
        jump: k % 150 == 0,
        dive: false,
    }
}

fn step(world: &mut World, body: &mut Body, k: i64) {
    let mut ev = StepEvents::default();
    let mut s = [Stepper {
        id: 1,
        body,
        ev: &mut ev,
        input: input(k),
    }];
    tick_bodies(world, k as f64 * DT, &mut s, &[], &mut no_touch);
}

#[test]
fn replay_after_running_ahead_matches() {
    let mut world = platform_world();
    let mut body = Body::new(1);
    body.reset(V3::new(0.5, 0.05, 0.5), 0.0);
    let mut live = Vec::new();
    for k in 1..=360 {
        step(&mut world, &mut body, k);
        live.push(body.clone());
    }
    let riding = live.iter().filter(|b| b.grounded && b.ground_col == Some(0)).count();
    assert!(
        riding > 150,
        "the bean should ride the platform: {riding} ticks, last {:?}",
        live.last().unwrap().pos
    );

    // Rollbacks to ticks 118…126 (for some of them k·DT − DT is not (k − 1)·DT): the world is at 360, the
    // body back at the state after that tick.
    for from in 118..=126i64 {
        let mut replay = live[from as usize - 1].clone();
        for k in from + 1..=360 {
            step(&mut world, &mut replay, k);
            assert_eq!(replay, live[k as usize - 1], "rollback to {from}, tick {k}");
        }
    }
    assert!(
        (118..=126i64).any(|k| k as f64 * DT - DT != (k - 1) as f64 * DT),
        "no tick here where the two times differ"
    );
}
