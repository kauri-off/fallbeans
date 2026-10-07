//! The bean's physics by its properties: the collider grid, walking,
//! collision substeps, sweeping arms, dives, ledges, portals, giants.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use fb_arena::{Stepper, tick_bodies, touch_hook};
use fb_shared::m::MinMax;
use fb_shared::rng::Rng;
use fb_shared::{DT, m};
use fb_sim::builder::{Builder, PortalEnd, PortalOpts, PrimOpts};
use fb_sim::collider::{ColliderOpts, Contact};
use fb_sim::map::Touches;
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyInput, BodyState, GIANT_MASS, GIANT_SIZE, StepEvents, power};
use fb_sim::scene::pal;
use fb_sim::world::World;

const IDLE: BodyInput = BodyInput {
    mx: 0.0,
    mz: 0.0,
    jump: false,
    dive: false,
};

const FORWARD: BodyInput = BodyInput { mz: 1.0, ..IDLE };

fn block(b: &mut Builder, x: f64, y: f64, z: f64, sx: f64, sy: f64, sz: f64) {
    b.box_(x, y, z, sx, sy, sz, pal::BLUE, PrimOpts::default());
}

struct Sim {
    world: World,
    touches: Touches,
    body: Body,
    ev: StepEvents,
}

impl Sim {
    fn new(mut b: Builder) -> Self {
        let touches = core::mem::take(&mut b.touches);
        let mut world = b.world;
        world.finalize(0.0);
        Self {
            world,
            touches,
            body: Body::new(1),
            ev: StepEvents::default(),
        }
    }

    fn reset(&mut self, x: f64, y: f64, z: f64) {
        self.body.reset(V3::new(x, y, z), 0.0);
    }

    /// One tick at time t (the world moves first, as on the server).
    fn tick(&mut self, t: f64, input: BodyInput) {
        let (mut scores, mut out) = (BTreeMap::new(), Vec::new());
        let mut touch = touch_hook(&mut self.touches, true, t, None, &mut scores, &mut out);
        let mut steppers = [Stepper {
            id: 1,
            body: &mut self.body,
            ev: &mut self.ev,
            input,
        }];
        tick_bodies(&mut self.world, t, &mut steppers, &[], &mut touch);
    }

    /// n ticks after t0.
    fn run(&mut self, t0: f64, n: u32, input: BodyInput) {
        for i in 1..=n {
            self.tick(t0 + i as f64 * DT, input);
        }
    }

    /// Deepest overlap of the body's spheres with sweeping arms right now.
    fn arm_overlap(&self) -> f64 {
        let mut hit = Contact::default();
        let mut worst = 0.0f64;
        for col in &self.world.colliders {
            for i in 0..2 {
                if col.sweep && col.contact(self.body.sphere(i), 0.5, &mut hit) {
                    worst = worst.at_least(hit.depth);
                }
            }
        }
        worst
    }
}

#[test]
fn grid_finds_every_collider_a_brute_force_search_finds() {
    let mut rng = Rng::new(5);
    let mut b = Builder::new(1, false);
    for i in 0..300 {
        let x = (rng.next() - 0.5) * 120.0;
        let z = (rng.next() - 0.5) * 120.0;
        if i % 3 == 0 {
            let (y, r, h) = (rng.next() * 5.0, 0.5 + rng.next() * 3.0, 1.0 + rng.next() * 2.0);
            b.cyl(x, y, z, r, h, pal::BLUE, PrimOpts::default());
        } else {
            let (y, sx, sz) = (rng.next() * 5.0, 0.5 + rng.next() * 10.0, 0.5 + rng.next() * 10.0);
            let rot = Some(V3::new(0.0, rng.next() * 6.0, rng.next() * 0.4));
            b.box_(
                x,
                y,
                z,
                sx,
                1.0,
                sz,
                pal::BLUE,
                PrimOpts {
                    rot,
                    ..Default::default()
                },
            );
        }
    }
    let mut world = b.world;
    world.finalize(0.0);
    let mut got = Vec::new();
    for _ in 0..200 {
        let x = (rng.next() - 0.5) * 130.0;
        let z = (rng.next() - 0.5) * 130.0;
        let r = 1.7;
        world.query(x, z, r, &mut got);
        for (i, c) in world.colliders.iter().enumerate() {
            let (ex, ez) = c.extent_xz();
            if (c.center.x - x).abs() <= ex + r && (c.center.z - z).abs() <= ez + r {
                assert!(got.contains(&(i as u32)), "collider {i} missed at ({x}, {z})");
            }
        }
    }
}

#[test]
fn lands_walks_jumps_and_falls_off_edges() {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 10.0, 2.0, 10.0);
    let mut s = Sim::new(b);
    s.reset(0.0, 2.0, 0.0);
    s.run(0.0, 120, IDLE);
    assert!(s.body.grounded);
    assert!(s.body.pos.y.abs() < 0.005, "{}", s.body.pos.y);
    s.run(1.0, 1, BodyInput { jump: true, ..IDLE });
    assert!(s.ev.jumped);
    s.run(1.0 + DT, 240, FORWARD);
    assert!(s.body.pos.z > 6.0, "{}", s.body.pos.z);
    assert!(s.body.pos.y < -1.0, "{}", s.body.pos.y);
}

#[test]
fn does_not_pass_through_a_thin_wall_at_any_speed() {
    let mut b = Builder::new(1, false);
    block(&mut b, 3.0, 0.0, 0.0, 0.1, 10.0, 10.0);
    let mut s = Sim::new(b);
    // 1.25 m a tick, from where whole ticks would land at x = 2.40 and 3.65: either side of the wall.
    s.reset(-0.05, 1.0, 0.0);
    s.body.vel = V3::new(150.0, 0.0, 0.0);
    s.run(0.0, 12, IDLE);
    assert!(s.body.pos.x < 3.0, "{}", s.body.pos.x);
}

#[test]
fn carries_on_from_the_full_body_state() {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 10.0, 2.0, 10.0);
    let mut a = Sim::new(b);
    a.reset(0.0, 0.5, 0.0);
    for i in 1..=60 {
        let input = BodyInput {
            mx: 0.5,
            mz: 1.0,
            jump: i == 31,
            dive: false,
        };
        a.tick(i as f64 * DT, input);
    }
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 10.0, 2.0, 10.0);
    let mut c = Sim::new(b);
    c.body = a.body.clone();
    for i in 61..=120 {
        let input = BodyInput {
            mx: 1.0,
            dive: i == 91,
            ..IDLE
        };
        a.tick(i as f64 * DT, input);
        c.tick(i as f64 * DT, input);
    }
    assert_eq!(c.body, a.body);
}

#[test]
fn falls_through_a_fake_pane_keeping_its_speed() {
    let mut b = Builder::new(1, false);
    let pane = b
        .box_(
            0.0,
            -0.15,
            0.0,
            3.0,
            0.3,
            3.0,
            pal::BLUE,
            PrimOpts {
                col: ColliderOpts {
                    trigger: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .col();
    let touched = Arc::new(AtomicU32::new(0));
    let count = touched.clone();
    b.on_touch(pane, move |_, _, _, _| {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let mut s = Sim::new(b);
    s.reset(0.0, 2.0, 0.0);
    s.body.vel = V3::new(0.0, -6.0, 4.0);
    let mut jumped = false;
    for i in 1..=40 {
        s.tick(i as f64 * DT, BodyInput { jump: true, ..FORWARD });
        jumped |= s.ev.jumped;
    }
    assert!(touched.load(Ordering::Relaxed) > 0);
    assert!(!jumped);
    assert!(s.body.pos.y < -0.5, "{}", s.body.pos.y);
    assert!(s.body.vel.y < -6.0, "{}", s.body.vel.y);
    assert!(s.body.vel.z > 3.0, "{}", s.body.vel.z);
}

fn arm_course(speed: f64) -> Sim {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 40.0, 2.0, 40.0);
    b.rotor(0.0, 0.6, 0.0, 8.0, 1, move |t| t * speed, 1.0);
    Sim::new(b)
}

#[test]
fn a_sweeping_arm_knocks_a_bean_over_and_never_passes_through_it() {
    let mut s = arm_course(1.2);
    // A quarter turn ahead of the arm (which starts along +x and turns towards −z).
    s.reset(0.5, 0.02, -5.0);
    let start = s.body.pos;
    let (mut knocked, mut drag, mut overlap) = (false, 0.0f64, 0.0f64);
    for i in 1..=360 {
        s.tick(i as f64 * DT, IDLE);
        knocked |= s.ev.knocked;
        drag = drag.at_least(m::sqrt(s.body.pos.distance_squared(start)));
        overlap = overlap.at_least(s.arm_overlap());
    }
    assert!(knocked);
    // Shoved a few metres, not carried round with the arm.
    assert!(drag < 8.0, "{drag}");
    assert!(overlap < 0.35, "{overlap}");
}

#[test]
fn a_sweeping_arm_tosses_a_lying_bean_up_and_over_itself() {
    let mut s = arm_course(1.2);
    s.reset(0.5, 0.02, -5.0);
    s.body.knock(&mut s.ev, 0.2, 0.0, 0.0, 3.0, false);
    let start = s.body.pos;
    let (mut top, mut overlap, mut drag) = (0.0f64, 0.0f64, 0.0f64);
    for i in 1..=240 {
        s.tick(i as f64 * DT, IDLE);
        if i > 30 {
            overlap = overlap.at_least(s.arm_overlap());
        }
        top = top.at_least(s.body.pos.y);
        drag = drag.at_least(m::hypot(s.body.pos.x - start.x, s.body.pos.z - start.z));
    }
    assert!(top > 0.7, "{top}");
    assert!(overlap < 0.35, "{overlap}");
    assert!(drag < 6.0, "{drag}");
}

#[test]
fn a_sweeping_arm_is_no_ride() {
    let mut s = arm_course(1.2);
    // On top of the arm (along +x at t = 0; its top is at 0.96).
    s.reset(5.0, 1.0, 0.0);
    s.run(0.0, 120, IDLE);
    // Carried along, it would be 6 m round the circle by now.
    assert!(m::hypot(s.body.pos.x - 5.0, s.body.pos.z) < 1.5, "{:?}", s.body.pos);
    assert!(s.body.pos.y < 0.1, "{}", s.body.pos.y);
}

#[test]
fn running_into_the_back_of_a_leaving_arm_does_not_knock_over() {
    let mut s = arm_course(0.5);
    // Behind the arm (it turns towards −z), running after it.
    s.reset(3.0, 0.02, 1.5);
    let mut knocked = false;
    for i in 1..=96 {
        s.tick(i as f64 * DT, BodyInput { mz: -1.0, ..IDLE });
        knocked |= s.ev.knocked;
    }
    assert!(!knocked);
    assert_eq!(s.body.state, BodyState::Normal);
}

#[test]
fn a_dive_lays_the_body_along_its_flight_and_stays_out_of_a_wall() {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 20.0, 2.0, 40.0);
    block(&mut b, 0.0, 2.0, 6.0, 20.0, 4.0, 1.0);
    let mut s = Sim::new(b);
    s.reset(0.0, 0.02, 0.0);
    s.run(0.0, 20, FORWARD);
    s.run(20.0 * DT, 1, BodyInput { dive: true, ..FORWARD });
    s.run(21.0 * DT, 12, FORWARD);
    assert_eq!(s.body.state, BodyState::Dive);
    assert!(s.body.tilt > 0.8, "{}", s.body.tilt);
    let mut deepest = -9.0f64;
    for i in 0..120 {
        s.run((33 + i) as f64 * DT, 1, FORWARD);
        deepest = deepest
            .at_least(s.body.sphere(1).z + 0.5 - 5.5)
            .at_least(s.body.pos.z + 0.5 - 5.5);
    }
    assert!(deepest < 0.02, "{deepest}");
}

/// A 30° slope of the given grip from y = 6 down to the floor at z = 10.4.
fn slope_course(slip: f64) -> Sim {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 20.0, 12.0, 2.0, 60.0);
    let opts = PrimOpts {
        col: ColliderOpts {
            slip,
            ..Default::default()
        },
        ..Default::default()
    };
    b.ramp(0.0, 0.0, 6.0, 10.4, 0.0, 4.0, pal::WHITE, 0.5, opts);
    Sim::new(b)
}

/// Down the slope from near its top: the share of ticks on the ground on the way and the speed at its foot.
fn down_the_slope(s: &mut Sim, input: BodyInput) -> (f64, f64) {
    s.reset(0.0, 5.7, 0.8);
    let (mut on, mut all, mut speed) = (0, 0, 0.0);
    for i in 1..=360 {
        s.tick(i as f64 * DT, input);
        let z = s.body.pos.z;
        if (2.0..9.5).contains(&z) {
            all += 1;
            on += u32::from(s.body.grounded);
            speed = m::hypot(s.body.vel.x, s.body.vel.z);
        }
    }
    assert!(all > 0, "never went down");
    (on as f64 / all as f64, speed)
}

#[test]
fn keeps_its_feet_on_a_slope_and_slides_down_ice() {
    // Running down: on the ground all the way, not hopping (air control steered it and ice did nothing).
    let (grip_on, grip_speed) = down_the_slope(&mut slope_course(0.0), FORWARD);
    assert!(grip_on > 0.95, "{grip_on}");
    assert!(grip_speed < 9.5, "{grip_speed}");
    let (ice_on, ice_speed) = down_the_slope(&mut slope_course(1.0), FORWARD);
    assert!(ice_on > 0.95, "{ice_on}");
    assert!(ice_speed > 12.0, "{ice_speed}");
    // Standing still on ice: no footing, it slides to the bottom on the ground.
    let mut s = slope_course(1.0);
    let (idle_on, _) = down_the_slope(&mut s, IDLE);
    assert!(idle_on > 0.95, "{idle_on}");
    assert!(s.body.pos.z > 10.4 && s.body.grounded, "{:?}", s.body.pos);
    // Up the icy slope from its foot: a run-up carries it a little way, but there is no grip to climb on.
    let mut s = slope_course(1.0);
    s.reset(0.0, 0.0, 13.0);
    let mut top = 0.0f64;
    for i in 1..=360 {
        s.tick(i as f64 * DT, BodyInput { mz: -1.0, ..IDLE });
        top = top.at_least(s.body.pos.y);
    }
    assert!(top < 3.0, "{top}");
}

fn ledge_course(height: f64) -> Sim {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 20.0, 2.0, 40.0);
    block(&mut b, 0.0, height / 2.0, 7.0, 20.0, height, 6.0);
    Sim::new(b)
}

/// Runs at the block and jumps a little before it, holding `hold`; the climbing phases seen.
fn jump_at(s: &mut Sim, hold: BodyInput) -> (bool, Vec<bool>) {
    s.reset(0.0, 0.02, 0.0);
    let mut t = 0.0;
    let mut step = |s: &mut Sim, n: u32, input: BodyInput| {
        s.run(t, n, input);
        t += n as f64 * DT;
    };
    step(s, 24, FORWARD);
    step(s, 1, BodyInput { jump: true, ..FORWARD });
    let mut climbed = false;
    let mut phases = Vec::new();
    for _ in 0..360 {
        step(s, 1, hold);
        if s.body.state == BodyState::Climb {
            climbed = true;
            phases.push(s.body.climbing_over());
        } else if climbed {
            break;
        }
    }
    // Settle where it got to.
    step(s, 30, IDLE);
    (climbed, phases)
}

#[test]
fn catches_a_ledge_out_of_reach_of_a_jump_and_climbs_onto_it() {
    let mut s = ledge_course(2.6);
    let (climbed, phases) = jump_at(&mut s, FORWARD);
    assert!(climbed);
    // Pulling up, then over the edge: one change of phase (what others see as the pose).
    assert_eq!(phases.first(), Some(&false));
    assert_eq!(phases.last(), Some(&true));
    assert_eq!(phases.windows(2).filter(|w| w[0] != w[1]).count(), 1);
    assert_eq!(s.body.state, BodyState::Normal);
    assert!((s.body.pos.y - 2.6).abs() < 0.05, "{}", s.body.pos.y);
    assert!(s.body.pos.z > 4.3, "{}", s.body.pos.z);
}

#[test]
fn does_not_catch_a_ledge_too_high_or_when_not_pushing_towards_it() {
    let mut high = ledge_course(4.2);
    assert!(!jump_at(&mut high, FORWARD).0);
    assert!(high.body.pos.y < 0.1, "{}", high.body.pos.y);
    let mut low = ledge_course(2.6);
    assert!(!jump_at(&mut low, IDLE).0);
}

#[test]
fn carries_on_climbing_from_a_full_state_exactly() {
    let mut a = ledge_course(2.6);
    a.reset(0.0, 0.02, 0.0);
    let mut k = 0i64;
    for i in 0..300 {
        if a.body.state == BodyState::Climb {
            break;
        }
        k += 1;
        a.tick(
            k as f64 * DT,
            BodyInput {
                jump: i == 24,
                ..FORWARD
            },
        );
    }
    assert_eq!(a.body.state, BodyState::Climb);
    let mut c = ledge_course(2.6);
    c.body = a.body.clone();
    for _ in 0..120 {
        k += 1;
        a.tick(k as f64 * DT, FORWARD);
        c.tick(k as f64 * DT, FORWARD);
    }
    assert_eq!(c.body, a.body);
}

#[test]
fn portals_send_a_bean_out_of_the_other_end_facing_its_way() {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 60.0, 2.0, 60.0);
    let end = |x, z, yaw| PortalEnd { x, y: 0.0, z, yaw };
    b.portal(
        end(0.0, 5.0, m::PI),
        end(20.0, 0.0, m::PI / 2.0),
        "#a66bff",
        PortalOpts::default(),
    );
    let mut s = Sim::new(b);
    s.reset(0.0, 0.02, 0.0);
    // In, half a second inside (out of play), then out at the other end.
    let mut out = false;
    for i in 0..240 {
        s.run(i as f64 * DT, 1, FORWARD);
        out = s.ev.portal_out;
        if out {
            break;
        }
    }
    assert!(out);
    assert!(s.body.pos.x > 21.0, "{}", s.body.pos.x);
    assert!(s.body.vel.x > 5.0, "{}", s.body.vel.x);
}

#[test]
fn a_giant_is_bigger_heavier_and_shrugs_off_knocks() {
    let mut b = Builder::new(1, false);
    block(&mut b, 0.0, -1.0, 0.0, 20.0, 2.0, 20.0);
    let mut s = Sim::new(b);
    s.reset(0.0, 0.02, 0.0);
    s.body.give_power(power::GIANT, 0.0);
    s.run(0.0, 10, IDLE);
    assert_eq!(s.body.size, GIANT_SIZE);
    assert_eq!(s.body.mass(), GIANT_MASS);
    s.body.knock(&mut s.ev, 10.0, 0.0, 5.0, 1.0, false);
    assert_ne!(s.body.state, BodyState::Tumble);
    assert!(s.body.vel.x < 4.0, "{}", s.body.vel.x);
    // It wears off.
    s.run(0.1, 120 * 10, IDLE);
    assert_eq!(s.body.size, 1.0);
}
