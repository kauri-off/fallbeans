//! Bounce all the way: a mushroom forest, hovering trampolines, catapults over walls, one more thing from
//! the seed, a second (higher) forest and a giant trampoline up to the finish.
use fb_sim::bots::{Hop, Waypoint, WpX, aim_landing, hop_chain};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, bumper_ramp, pick_sections, race_course, seesaws, trampoline_gap, with_rests,
};
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::pal;

use crate::util::o;

pub struct BouncePark;

static META: GameMeta = GameMeta::new(
    "bounce-park",
    "Прыг-скок",
    Genre::Race,
    "Грибы, батуты и катапульты: скачите со шляпки на шляпку, ловите летающие батуты, взлетайте над стенами. Рулите в полёте, чтобы приземлиться куда надо!",
    "Допрыгайте до финиша",
    150.0,
);

const CAPS: [&str; 5] = ["#ff5f6d", "#ff9f4a", "#a66bff", "#39c0ff", "#ff5fa2"];

/// Giant mushrooms over the void, each cap a little higher than the last: bounce from cap to cap.
fn mushroom_forest(n: usize, power: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 3.0, 12.0, 2.0, 6.0, pal::PURPLE, o());
        let edge = s.z + 6.0;
        let mut hops = Vec::new();
        let mut x: f64 = 0.0;
        let mut z = edge + 3.2;
        let mut top = y - 0.4;
        for k in 0..n {
            let sc = 1.3 + s.rng() * 0.35;
            let base = top - 1.92 * sc;
            let cap = CAPS[(s.rng() * CAPS.len() as f64).floor() as usize];
            s.b.mushroom(x, base, z, sc, power, Some(cap));
            // A stalk down into the clouds (thinner than the stem: nothing to stand on).
            let stalk = PrimOpts {
                surface: Some("leaf"),
                seg: Some(16),
                ..Default::default()
            };
            s.b.cyl(x, base - 5.0, z, 0.3 * sc, 10.0, pal::GREEN, stalk);
            hops.push(Hop {
                x: WpX::At(x),
                y: top,
                z,
                r: 0.98 * sc,
                power,
            });
            if k == n - 1 {
                break;
            }
            z += 5.0 + s.rng() * 1.2;
            let dir = if s.rng() < 0.5 { -1.0 } else { 1.0 };
            x = (-4f64).max(4f64.min(x + dir * (1.5 + s.rng() * 1.8)));
            top += 0.7;
        }
        let land_y = top + 1.2;
        let z0 = z + 3.5;
        s.b.box_(0.0, land_y - 1.0, z0 + 3.5, 12.0, 2.0, 7.0, pal::PINK, o());
        let land = V3::new(0.0, land_y, z0 + 2.5);
        let key = format!("mf{}", m::round_js(s.z));
        let route = vec![
            Waypoint::w(0.0, s.z + 2.0, 1.0),
            Waypoint::w(0.0, land.z, 0.5).drive_boxed(hop_chain(key, hops, land, edge, None)),
            Waypoint::w(0.0, z0 + 6.0, 1.0),
        ];
        SegOut {
            z: z0 + 7.0,
            y: land_y,
            routes: vec![route],
            checkpoint: Some((z0 + 0.5, V3::new(0.0, land_y + 0.1, z0 + 3.5))),
            ..Default::default()
        }
    })
}

/// A trampoline hovering over the void on a little engine, sliding from side to side.
fn hover_trampoline(
    b: &mut Builder,
    y: f64,
    z: f64,
    r: f64,
    power: f64,
    fx: impl Fn(f64) -> f64 + Send + Sync + 'static,
) {
    let holder = b.anchor(0.0, y - 0.19, z, ROOT);
    let rim = PrimOpts {
        parent: Some(holder),
        dynamic: true,
        surface: Some("rubber"),
        ..Default::default()
    };
    b.cyl(0.0, -0.03, 0.0, r + 0.3, 0.3, pal::ORANGE, rim);
    let mat = PrimOpts {
        parent: Some(holder),
        dynamic: true,
        surface: Some("fabric"),
        pattern: Some("dots"),
        col: ColliderOpts {
            pad: power,
            ..Default::default()
        },
        ..Default::default()
    };
    b.cyl(0.0, 0.14, 0.0, r, 0.1, pal::BLUE, mat);
    let engine = PrimOpts {
        parent: Some(holder),
        no_collide: true,
        surface: Some("metal"),
        seg: Some(20),
        ..Default::default()
    };
    b.cyl(0.0, -0.55, 0.0, r * 0.45, 0.8, pal::hex("#39406b"), engine);
    let flame = PrimOpts {
        parent: Some(holder),
        no_collide: true,
        surface: Some("glossy"),
        seg: Some(16),
        ..Default::default()
    };
    b.cyl(0.0, -1.0, 0.0, r * 0.25, 0.25, pal::YELLOW, flame);
    b.mover(move |t, ctx| ctx.node(holder).pos.x = fx(t));
}

/// Trampolines hovering over a gap, sliding from side to side: jump on, bounce across.
fn hover_trampolines(n: usize, power: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 3.0, 12.0, 2.0, 6.0, pal::PURPLE, o());
        let edge = s.z + 6.0;
        let yy = y - 1.0;
        let mut hops = Vec::new();
        let mut first = None;
        let mut z = edge + 4.0;
        for k in 0..n {
            let amp = 2.0 + s.rng() * 0.8;
            let w = 0.7 + s.rng() * 0.5;
            let ph = s.rng() * 6.0;
            let fx = move |t: f64| m::sin(t * w + ph) * amp;
            hover_trampoline(s.b, yy, z, 1.6, power, fx);
            first.get_or_insert(fx);
            hops.push(Hop {
                x: WpX::Moving(Box::new(fx)),
                y: yy,
                z,
                r: 1.6,
                power,
            });
            if k < n - 1 {
                z += 5.5;
            }
        }
        let z0 = z + 3.5;
        let land_y = y + 1.0;
        s.b.box_(0.0, land_y - 1.0, z0 + 3.5, 12.0, 2.0, 7.0, pal::TEAL, o());
        let land = V3::new(0.0, land_y, z0 + 2.5);
        let fx0 = first.expect("a hovering trampoline");
        let ready = Box::new(move |bot: &mut fb_sim::bots::BotView| (fx0(bot.t + 0.55) - bot.body.pos.x).abs() < 1.0);
        let key = format!("ht{}", m::round_js(s.z));
        let route = vec![
            Waypoint::w(0.0, s.z + 2.0, 1.0),
            Waypoint::w(0.0, land.z, 0.5).drive_boxed(hop_chain(key, hops, land, edge, Some(ready))),
            Waypoint::w(0.0, z0 + 6.0, 1.0),
        ];
        SegOut {
            z: z0 + 7.0,
            y: land_y,
            routes: vec![route],
            checkpoint: Some((z0 + 0.5, V3::new(0.0, land_y + 0.1, z0 + 3.5))),
            ..Default::default()
        }
    })
}

/// Walls too high to climb, with catapult pads before each: over the wall and down the other side.
fn pad_catapults(n: usize) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let w = 12.0;
        let step = 13.0;
        let len = n as f64 * step + 4.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w, 2.0, len, pal::BLUE, o());
        s.b.rails(s.z, s.z + len, w / 2.0, y, pal::PINK);
        let mut routes: Vec<Vec<Waypoint>> = vec![Vec::new(), Vec::new()];
        for k in 0..n {
            let pz = s.z + 3.0 + k as f64 * step;
            let p = if k % 2 == 1 { pal::ORANGE } else { pal::PURPLE };
            s.b.box_(0.0, y + 2.0, pz + 4.0, w, 4.0, 1.0, p, o());
            let dz = if s.rng() < 0.5 { -0.5 } else { 0.5 };
            s.b.bumper(0.0, y, pz + dz, 0.7, 9.0);
            for (r, x) in [-3.0, 3.0].into_iter().enumerate() {
                s.b.pad(x, y, pz, 1.4, 16.0, Some((0.0, 8.0)));
                let land = pz + 9.3;
                routes[r].push(Waypoint::w(x, pz - 2.0, 0.0));
                routes[r].push(Waypoint::w(x, pz, 0.0));
                routes[r].push(
                    Waypoint::w(x, land, 0.0)
                        .drive(move |bot, out| !bot.body.grounded && aim_landing(bot, x, y, land, out)),
                );
            }
        }
        for r in &mut routes {
            r.push(Waypoint::w(0.0, s.z + len - 0.5, 1.0));
        }
        let z0 = s.z;
        SegOut {
            z: s.z + len,
            y,
            routes,
            forbidden: Some(Box::new(move |p| p.y > y + 3.5 && p.z > z0 && p.z < z0 + len)),
            ..Default::default()
        }
    })
}

/// A big trampoline down in a pit, and a high ledge beyond it: bounce up and steer onto it.
fn big_bounce(rise: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 2.5, 12.0, 2.0, 5.0, pal::PURPLE, o());
        let edge = s.z + 5.0;
        let gap = 8.0;
        let basin_y = y - 3.0;
        s.b.box_(0.0, basin_y - 1.0, edge + gap / 2.0, 12.0, 2.0, gap, pal::BLUE, o());
        for sx in [-1.0, 1.0] {
            s.b.box_(sx * 6.4, basin_y + 1.0, edge + gap / 2.0, 0.8, 4.0, gap, pal::PINK, o());
        }
        let tz = edge + gap * 0.45;
        let power = 23.5;
        for x in [-2.8, 2.8] {
            s.b.trampoline(x, basin_y, tz, 1.9, power);
        }
        let hops = |reverse: bool| {
            let mut xs = [-2.8, 2.8];
            if reverse {
                xs.reverse();
            }
            xs.into_iter()
                .map(|x| Hop {
                    x: WpX::At(x),
                    y: basin_y + 0.19,
                    z: tz,
                    r: 1.9,
                    power,
                })
                .collect::<Vec<_>>()
        };
        let top = y + rise;
        let far = edge + gap;
        s.b.box_(
            0.0,
            top - (rise + 3.5) / 2.0,
            far + 4.0,
            12.0,
            rise + 3.5,
            8.0,
            pal::PURPLE,
            o(),
        );
        let land = V3::new(0.0, top, far + 2.5);
        let routes = [-1.0, 1.0]
            .into_iter()
            .map(|k: f64| {
                vec![
                    Waypoint::w(k * 2.8, s.z + 1.5, 0.0),
                    Waypoint::w(0.0, land.z, 0.5).drive_boxed(hop_chain(
                        format!("bb{k}"),
                        hops(k > 0.0),
                        land,
                        edge,
                        None,
                    )),
                    Waypoint::w(0.0, far + 6.0, 1.0),
                ]
            })
            .collect();
        SegOut {
            z: far + 8.0,
            y: top,
            routes,
            checkpoint: Some((far + 0.5, V3::new(0.0, top + 0.1, far + 4.0))),
            forbidden: Some(Box::new(move |p| {
                p.z > edge && p.z < far && p.x.abs() > 6.0 && p.y > basin_y + 2.5
            })),
        }
    })
}

impl MapDef for BouncePark {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["jungle", "candy", "meadow"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![bumper_ramp(4.0, 22.0), trampoline_gap(), seesaws(3)];
        let extra = pick_sections(&mut b.rng, pool, 1);
        let mut sections = vec![mushroom_forest(5, 16.0), hover_trampolines(3, 15.0), pad_catapults(3)];
        sections.extend(extra);
        sections.push(mushroom_forest(6, 17.0));
        sections.push(big_bounce(6.0));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
