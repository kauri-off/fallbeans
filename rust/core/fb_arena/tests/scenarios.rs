//! The bean's physics in small worlds against TS traces (`bun scripts/golden.ts` → golden/scenarios.json):
//! pushes, slopes, ice, conveyors, pads, sweeping arms, hammers, platforms, ledges, ladders, giants.
mod common;

use common::{check_bodies, check_colliders, f, load};
use fb_arena::{Stepper, tick_bodies};
use fb_shared::input::InputFrame;
use fb_shared::{DT, m};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::math::V3;
use fb_sim::physics::{Body, StepEvents};
use fb_sim::scene::pal;
use serde_json::Value;

fn floor(b: &mut Builder) {
    b.box_(0.0, -1.0, 0.0, 40.0, 2.0, 40.0, pal::BLUE, PrimOpts::default());
}

fn wall(b: &mut Builder, x: f64, y: f64, z: f64, sx: f64, sy: f64, sz: f64) {
    b.box_(x, y, z, sx, sy, sz, pal::BLUE, PrimOpts::default());
}

fn dynamic() -> PrimOpts {
    PrimOpts {
        dynamic: true,
        ..Default::default()
    }
}

fn surface(col: ColliderOpts) -> PrimOpts {
    PrimOpts {
        col,
        ..Default::default()
    }
}

/// The same worlds as `SCENARIOS` in scripts/golden.ts.
fn build(name: &str, b: &mut Builder) {
    match name {
        "moves" => {
            floor(b);
            wall(b, 0.0, 2.0, 16.0, 40.0, 4.0, 1.0);
        }
        "push" => floor(b),
        "slopes" => {
            floor(b);
            b.ramp(-6.0, 2.0, 0.0, 14.0, 4.0, 6.0, pal::BLUE, 1.0, PrimOpts::default());
            b.ramp(6.0, 2.0, 0.0, 5.0, 4.5, 6.0, pal::BLUE, 1.0, PrimOpts::default());
            let ice = surface(ColliderOpts {
                slip: 0.9,
                ..Default::default()
            });
            b.ramp(-15.0, 5.0, 0.0, 15.0, 4.0, 6.0, pal::WHITE, 1.0, ice);
        }
        "surfaces" => {
            let ice = surface(ColliderOpts {
                slip: 0.9,
                ..Default::default()
            });
            b.box_(-6.0, -1.0, 0.0, 8.0, 2.0, 40.0, pal::WHITE, ice);
            wall(b, 0.0, -1.0, 0.0, 4.0, 2.0, 40.0);
            let belt = surface(ColliderOpts {
                conveyor: Some(V3::new(0.0, 0.0, -4.0)),
                ..Default::default()
            });
            b.box_(6.0, -1.0, 0.0, 8.0, 2.0, 40.0, pal::WHITE, belt);
        }
        "rotor" => {
            floor(b);
            b.rotor(0.0, 0.6, 0.0, 8.0, 1, |t| t * 1.2, 1.0);
        }
        "hammer" => {
            floor(b);
            b.hammer(0.0, 7.0, 0.0, 2.2, m::PI / 2.0, 1.05, true);
        }
        "bounce" => {
            floor(b);
            b.bumper(0.0, 0.0, 5.0, 1.0, 13.0);
            b.pad(6.0, 0.0, 5.0, 1.4, 17.0, None);
            b.pad(-6.0, 0.0, 5.0, 1.4, 16.0, Some((0.0, 8.0)));
            b.trampoline(12.0, 0.0, 5.0, 1.8, 19.0);
            b.mushroom(-12.0, 0.0, 5.0, 1.5, 17.0);
        }
        "platforms" => {
            floor(b);
            let slider = b.box_(-6.0, 1.0, 0.0, 4.0, 0.6, 4.0, pal::BLUE, dynamic()).node;
            b.mover(move |t, ctx| ctx.node(slider).pos.x = -6.0 + m::sin(t * 1.5) * 3.0);
            let disc = b.cyl(6.0, 1.0, 0.0, 3.0, 0.6, pal::PURPLE, dynamic()).node;
            b.mover(move |t, ctx| ctx.node(disc).rot.y = t * 1.2);
            let lift = b.box_(0.0, 2.0, 8.0, 3.0, 0.6, 3.0, pal::BLUE, dynamic()).node;
            b.mover(move |t, ctx| ctx.node(lift).pos.y = 2.0 + m::sin(t * 1.3) * 1.2);
        }
        "climb" => {
            floor(b);
            wall(b, 0.0, 1.3, 7.0, 6.0, 2.6, 6.0);
            wall(b, 8.0, 2.1, 7.0, 6.0, 4.2, 6.0);
            wall(b, -8.0, 2.5, 7.0, 6.0, 5.0, 6.0);
            b.ladder(-8.0, 0.0, 4.0, 5.0, m::PI);
            wall(b, -16.0, 2.5, 7.0, 6.0, 5.0, 6.0);
            b.ladder(-16.0, 0.0, 4.0, 5.0, m::PI);
        }
        _ => panic!("unknown scenario {name}"),
    }
}

/// Input of a scenario body on tick k: the last stick change at or before k, buttons pressed on k.
fn frame(d: &Value, k: i64) -> InputFrame {
    let mut fr = InputFrame::default();
    for s in d["stick"].as_array().unwrap() {
        if s[0].as_i64().unwrap() <= k {
            fr.mx = s[1].as_i64().unwrap() as i8;
            fr.mz = s[2].as_i64().unwrap() as i8;
        }
    }
    for p in d["press"].as_array().unwrap() {
        if p[0].as_i64().unwrap() == k {
            fr.buttons = p[1].as_u64().unwrap() as u8;
        }
    }
    fr
}

fn check(name: &str) {
    let data = load("scenarios");
    let sc = data["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no scenario {name} in scenarios.json: re-export with `cargo xtask golden`"));
    let mut b = Builder::new(1, false);
    build(name, &mut b);
    let mut world = b.world;
    world.finalize(0.0);
    assert_eq!(
        world.hash(true),
        sc["staticHash"].as_str().unwrap(),
        "{name}: static hash"
    );
    check_colliders(name, &world, &sc["colliders"]);

    let defs = sc["bodies"].as_array().unwrap();
    let mut bodies: Vec<(u32, Body, StepEvents)> = defs
        .iter()
        .map(|d| {
            let id = d["id"].as_u64().unwrap() as u32;
            let at = d["at"].as_array().unwrap();
            let mut body = Body::new(id as i32);
            body.reset(V3::new(f(&at[0]), f(&at[1]), f(&at[2])), 0.0);
            let power = d["power"].as_u64().unwrap() as u8;
            if power != 0 {
                body.give_power(power, 0.0);
            }
            (id, body, StepEvents::default())
        })
        .collect();
    let hashes = sc["hashes"].as_array().unwrap();
    let mut worst = 0.0f64;
    for row in sc["frames"].as_array().unwrap() {
        let k = row[0].as_i64().unwrap();
        let mut steppers: Vec<Stepper> = bodies
            .iter_mut()
            .zip(defs)
            .map(|((id, body, ev), d)| Stepper {
                id: *id,
                body,
                ev,
                input: frame(d, k).into(),
            })
            .collect();
        tick_bodies(&mut world, k as f64 * DT, &mut steppers, &[]);
        worst = worst.max(check_bodies(name, row, bodies.iter().map(|(id, body, _)| (*id, body))));
        if let Some(h) = hashes.iter().find(|h| h[0].as_i64() == Some(k)) {
            assert_eq!(
                world.hash(false),
                h[1].as_str().unwrap(),
                "{name}: world hash at tick {k}"
            );
        }
    }
    eprintln!("{name}: {} ticks, worst difference {worst:e}", sc["ticks"]);
}

#[test]
fn moves() {
    check("moves");
}

#[test]
fn push() {
    check("push");
}

#[test]
fn slopes() {
    check("slopes");
}

#[test]
fn surfaces() {
    check("surfaces");
}

#[test]
fn rotor() {
    check("rotor");
}

#[test]
fn hammer() {
    check("hammer");
}

#[test]
fn bounce() {
    check("bounce");
}

#[test]
fn platforms() {
    check("platforms");
}

#[test]
fn climb() {
    check("climb");
}
