//! The bean's physics in small worlds: pushes, slopes, ice, conveyors, pads, sweeping arms, hammers,
//! platforms, ledges, ladders, giants. The beans of each world follow a script; where they are every quarter
//! second must stay within `TOL` of the recorded path (`tests/paths/<world>.txt`). Record again after an
//! intended change with `FB_BLESS=1 cargo test -p fb_arena --test scenarios`.
use fb_arena::{Stepper, tick_plain};
use fb_shared::input::InputFrame;
use fb_shared::rgb;
use fb_shared::{DT, m};
use fb_sim::builder::{Builder, PortalEnd, PortalOpts, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::map::NoLogic;
use fb_sim::math::V3;
use fb_sim::physics::{Body, Power, StepEvents};
use fb_sim::scene::pal;

/// How far a bean may stray from its recorded path, m: further, and the physics feels different.
const TOL: f64 = 0.05;
/// Ticks between samples of the path.
const EVERY: i64 = 30;

/// A scripted bean: where it starts, its bonus, its stick from a tick on and the buttons pressed on a tick.
struct Bean {
    at: [f64; 3],
    power: Option<Power>,
    stick: &'static [(i64, i8, i8)],
    press: &'static [(i64, u8)],
}

const fn bean(at: [f64; 3], stick: &'static [(i64, i8, i8)], press: &'static [(i64, u8)]) -> Bean {
    Bean {
        at,
        power: None,
        stick,
        press,
    }
}

const fn powered(power: Power, b: Bean) -> Bean {
    Bean {
        power: Some(power),
        ..b
    }
}

const ON: f64 = 0.02;
const ROTOR_HOPS: &[(i64, u8)] = &[
    (10, 1),
    (55, 1),
    (100, 1),
    (145, 1),
    (190, 1),
    (235, 1),
    (280, 1),
    (325, 1),
    (370, 1),
    (415, 1),
    (460, 1),
    (505, 1),
    (550, 1),
    (595, 1),
    (640, 1),
];

/// Ticks a world runs, and its beans (ids from 1).
fn script(name: &str) -> (i64, Vec<Bean>) {
    match name {
        "moves" => (
            720,
            vec![
                bean([0.0, ON, 0.0], &[(1, 0, 127), (200, 0, 0)], &[(30, 1), (100, 2)]),
                powered(
                    Power::Jump,
                    bean([-8.0, ON, 0.0], &[(1, 0, 0)], &[(20, 1), (200, 1), (400, 1)]),
                ),
                powered(
                    Power::Speed,
                    bean(
                        [8.0, ON, -15.0],
                        &[(1, 0, 127), (100, 127, 0), (150, 0, -127), (250, -127, 0), (350, 0, 0)],
                        &[],
                    ),
                ),
                bean(
                    [-14.0, ON, -15.0],
                    &[(1, 0, 127), (500, 0, 0)],
                    &[(40, 1), (60, 2), (300, 1), (302, 2)],
                ),
            ],
        ),
        "push" => (
            420,
            vec![
                bean([0.0, ON, -3.0], &[(1, 0, 127), (180, 0, 0)], &[]),
                bean([0.0, ON, 3.0], &[(1, 0, -127), (180, 0, 0)], &[]),
                powered(Power::Giant, bean([6.0, ON, -4.0], &[(1, 0, 127), (240, 0, 0)], &[])),
                bean([6.0, ON, 1.0], &[(1, 0, 0)], &[]),
                bean([-6.0, ON, -3.0], &[(1, 0, 127), (200, 0, 0)], &[(30, 2)]),
                bean([-6.0, ON, 1.0], &[(1, 0, 0)], &[]),
            ],
        ),
        "slopes" => (
            480,
            vec![
                bean(
                    [-6.0, ON, 0.0],
                    &[(1, 0, 127), (220, 0, 0), (300, 0, -127), (400, 0, 0)],
                    &[],
                ),
                bean([6.0, ON, 0.0], &[(1, 0, 127), (200, 0, 0)], &[(60, 1), (120, 1)]),
                bean([7.0, 5.0, 3.5], &[(1, 0, 0)], &[]),
                bean([-15.0, ON, 3.0], &[(1, 0, 127), (150, 0, 0)], &[]),
            ],
        ),
        "surfaces" => (
            480,
            vec![
                bean(
                    [-6.0, ON, -15.0],
                    &[(1, 0, 127), (90, 0, 0), (200, 127, 0), (240, 0, 0)],
                    &[],
                ),
                bean(
                    [0.0, ON, -15.0],
                    &[(1, 0, 127), (90, 0, 0), (200, 127, 0), (240, 0, 0)],
                    &[],
                ),
                bean([6.0, ON, 0.0], &[(120, 0, 127), (360, 0, 0)], &[]),
            ],
        ),
        "rotor" => (
            720,
            vec![
                bean([0.5, ON, -5.0], &[(1, 0, 0)], &[]),
                bean([3.0, ON, 1.5], &[(1, 0, -127), (97, 0, 0)], &[]),
                bean([5.0, 1.0, 0.0], &[(1, 0, 0)], &[]),
                bean([-5.0, ON, -3.0], &[(1, 0, 0)], ROTOR_HOPS),
            ],
        ),
        "hammer" => (
            600,
            vec![
                bean([0.0, ON, -0.6], &[(1, 0, 0)], &[]),
                bean([0.0, ON, -8.0], &[(1, 0, 127), (300, 0, 0)], &[]),
                powered(Power::Giant, bean([0.0, ON, 1.6], &[(1, 0, 0)], &[])),
            ],
        ),
        "bounce" => (
            480,
            vec![
                bean([0.0, ON, 0.0], &[(1, 0, 127), (90, 0, 0)], &[]),
                bean([6.0, ON, 0.0], &[(1, 0, 127), (90, 0, 0)], &[]),
                bean([-6.0, ON, 0.0], &[(1, 0, 127), (90, 0, 0)], &[]),
                bean([12.0, ON, 0.0], &[(1, 0, 127), (90, 0, 0)], &[]),
                bean([-12.0, 6.0, 5.0], &[(1, 0, 0)], &[]),
            ],
        ),
        "platforms" => (
            600,
            vec![
                bean([-6.0, 1.32, 0.0], &[(400, 0, 127), (460, 0, 0)], &[]),
                bean([8.0, 1.32, 0.0], &[(1, 0, 0)], &[(300, 1)]),
                bean([0.0, 2.32, 8.0], &[(450, 127, 0)], &[]),
            ],
        ),
        "climb" => (
            720,
            vec![
                bean([0.0, ON, 0.0], &[(1, 0, 127), (200, 0, 0)], &[(25, 1)]),
                bean([8.0, ON, 0.0], &[(1, 0, 127), (200, 0, 0)], &[(25, 1)]),
                bean([-8.0, ON, 0.0], &[(1, 0, 127), (330, 0, 0)], &[]),
                bean(
                    [-16.0, ON, 0.0],
                    &[(1, 0, 127), (160, 0, -127), (220, 0, 0)],
                    &[(150, 1)],
                ),
            ],
        ),
        "portal" => (
            600,
            vec![
                bean([0.0, ON, 0.0], &[(1, 0, 127), (200, 0, 0)], &[]),
                bean([0.5, ON, -2.5], &[(1, 0, 127), (300, 0, 0)], &[]),
                bean([-10.0, ON, 0.0], &[(1, 0, 127), (100, 0, 0)], &[]),
                bean([-10.0, ON, 24.0], &[(1, 0, -127), (160, 0, 0)], &[]),
            ],
        ),
        _ => panic!("unknown scenario {name}"),
    }
}

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

/// The worlds.
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
            b.mushroom(-12.0, 0.0, 5.0, 1.5, 17.0, None);
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
            b.ladder(-8.0, 0.0, 4.0, 5.0, m::PI, rgb(0xffb347));
            wall(b, -16.0, 2.5, 7.0, 6.0, 5.0, 6.0);
            b.ladder(-16.0, 0.0, 4.0, 5.0, m::PI, rgb(0xffb347));
        }
        "portal" => {
            b.box_(0.0, -1.0, 0.0, 60.0, 2.0, 60.0, pal::BLUE, PrimOpts::default());
            let end = |x, z, yaw| PortalEnd { x, y: 0.0, z, yaw };
            b.portal(
                end(0.0, 5.0, m::PI),
                end(20.0, 0.0, m::PI / 2.0),
                rgb(0xa66bff),
                PortalOpts::default(),
            );
            let o = PortalOpts {
                one_way: true,
                speed: 9.0,
                lift: Some(6.0),
                ..Default::default()
            };
            b.portal(end(-10.0, 5.0, m::PI), end(-10.0, 20.0, 0.0), rgb(0xffffff), o);
        }
        _ => panic!("unknown scenario {name}"),
    }
}

/// Input of a bean on tick k: the last stick change at or before k, buttons pressed on k.
fn frame(b: &Bean, k: i64) -> InputFrame {
    let mut fr = InputFrame::default();
    for &(at, mx, mz) in b.stick {
        if at <= k {
            fr.mx = mx;
            fr.mz = mz;
        }
    }
    for &(at, buttons) in b.press {
        if at == k {
            fr.buttons = buttons;
        }
    }
    fr
}

/// Lines `tick id x y z`, every `EVERY` ticks.
fn path(name: &str) -> String {
    let (ticks, beans) = script(name);
    let mut b = Builder::new(1, false);
    build(name, &mut b);
    let mut world = b.world;
    world.finalize(0.0, &NoLogic);
    let mut bodies: Vec<(u32, Body, StepEvents)> = beans
        .iter()
        .zip(1..)
        .map(|(d, id)| {
            let mut body = Body::new(id as i32);
            body.reset(V3::new(d.at[0], d.at[1], d.at[2]), 0.0);
            if let Some(power) = d.power {
                body.give_power(power, 0.0);
            }
            (id, body, StepEvents::default())
        })
        .collect();
    let mut lines = String::new();
    for k in 1..=ticks {
        let mut steppers: Vec<Stepper> = bodies
            .iter_mut()
            .zip(&beans)
            .map(|((id, body, ev), d)| Stepper {
                id: *id,
                body,
                ev,
                input: frame(d, k).into(),
            })
            .collect();
        let t = k as f64 * DT;
        tick_plain(&mut world, t, &mut steppers, &[]);
        if k % EVERY == 0 {
            for (id, b, _) in &bodies {
                let p = b.pos;
                lines += &format!("{k} {id} {:.4} {:.4} {:.4}\n", p.x, p.y, p.z);
            }
        }
    }
    lines
}

fn check(name: &str) {
    let got = path(name);
    let file = format!("{}/tests/paths/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("FB_BLESS").is_ok() {
        std::fs::write(&file, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&file).expect("no recorded path: run with FB_BLESS=1");
    let (got, want): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
    assert_eq!(got.len(), want.len(), "{name}: samples");
    let num = |s: &str| s.parse::<f64>().unwrap();
    for (g, w) in got.iter().zip(&want) {
        let (g, w): (Vec<&str>, Vec<&str>) = (g.split(' ').collect(), w.split(' ').collect());
        assert_eq!(g[..2], w[..2], "{name}: sample order");
        let d = m::hypot3(num(g[2]) - num(w[2]), num(g[3]) - num(w[3]), num(g[4]) - num(w[4]));
        assert!(
            d <= TOL,
            "{name}: bean {} at tick {} is {d:.3} m from its recorded path ({} {} {} instead of {} {} {})",
            g[1],
            g[0],
            g[2],
            g[3],
            g[4],
            w[2],
            w[3],
            w[4]
        );
    }
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

#[test]
fn portal() {
    check("portal");
}
