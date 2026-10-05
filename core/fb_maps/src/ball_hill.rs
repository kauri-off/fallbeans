//! An icy slope you cannot run up: only the zig-zag carpet strips give grip, balls roll down across them
//! and blocks slide across your way; then challenges drawn from the seed, and gates with sliding gaps.
use std::sync::Arc;

use fb_sim::bots::Waypoint;
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, glove_alley, pick_sections, pistons, race_course, sliding_gates, tipping_bridge,
    with_rests,
};
use fb_sim::m::{self, MinMax};
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::props::{BallLaneOpts, rolling_balls, y_on_ramp};
use fb_sim::scene::pal;

use crate::util::{o, rot};

pub struct BallHill;

static META: GameMeta = GameMeta::new(
    "ball-hill",
    "Скользкий склон",
    Genre::Race,
    "Ледяной склон: держаться можно только на ковровых дорожках, сверху катятся шары, поперёк ездят блоки. Дальше — случайные испытания и ворота со сдвигающимися проёмами.",
    "Доберитесь до финиша",
    150.0,
);

/// An icy slope you cannot run up: only the carpet strips give grip.
fn ice_slope() -> Segment {
    Box::new(|s| {
        let y = s.y;
        let (a0z, a0y) = (s.z + 2.0, y);
        let (a1z, a1y) = (s.z + 42.0, y + 10.0);
        let w = 20.0;
        let ya = move |zz: f64| y_on_ramp(zz, a0z, a0y, a1z, a1y);
        s.b.box_(0.0, y - 1.0, s.z + 1.0, 18.0, 2.0, 2.0, pal::PURPLE, o());
        let ang = m::atan2(a1y - a0y, a1z - a0z);
        let cos_a = m::cos(ang);
        let ice = PrimOpts {
            col: ColliderOpts {
                slip: 1.0,
                ..Default::default()
            },
            surface: Some("ice"),
            ..Default::default()
        };
        s.b.ramp(0.0, a0z, a0y, a1z, a1y, w, pal::BLUE, 1.0, ice);
        let len_a = m::hypot(a1z - a0z, a1y - a0y);
        for sx in [-1.0, 1.0] {
            let x = sx * (w / 2.0 + 0.4);
            s.b.box_(
                x,
                (a0y + a1y) / 2.0 + 0.6,
                (a0z + a1z) / 2.0,
                0.8,
                1.2,
                len_a,
                pal::PINK,
                rot(-ang, 0.0, 0.0),
            );
        }
        // Carpet corners: a zig-zag, mirrored or not by the seed.
        let flip = if s.rng() < 0.5 { -1.0 } else { 1.0 };
        let carpet = [
            (0.0, a0z + 0.5),
            (-7.0 * flip, a0z + 6.0),
            (7.0 * flip, a0z + 16.0),
            (-7.0 * flip, a0z + 26.0),
            (6.0 * flip, a0z + 35.0),
            (0.0, a1z),
        ];
        let cw = 3.2;
        for i in 0..carpet.len() - 1 {
            let (x0, z0) = carpet[i];
            let (x1, z1) = carpet[i + 1];
            let dz_slope = (z1 - z0) / cos_a;
            let len = m::hypot(x1 - x0, dz_slope) + cw * 0.7;
            let yaw = m::atan2(x1 - x0, dz_slope);
            let cz = (z0 + z1) / 2.0;
            let p = if i % 2 == 1 { pal::GREEN } else { pal::YELLOW };
            let opts = PrimOpts {
                rot: Some(V3::new(-ang, yaw, 0.0)),
                freq: Some(0.8),
                surface: Some("carpet"),
                ..Default::default()
            };
            s.b.box_((x0 + x1) / 2.0, ya(cz) + 0.1 / cos_a, cz, cw, 0.2, len, p, opts);
        }
        let lanes = [-5.5, 0.0, 5.5];
        let period = 4.0 + s.rng() * 1.0;
        let balls = rolling_balls(
            s.b,
            BallLaneOpts {
                lanes: lanes.to_vec(),
                z_top: a1z - 1.0,
                y_top: a1y,
                z_bottom: a0z + 1.0,
                y_bottom: a0y,
                radius: 1.1,
                speed: Arc::new(|t| 9.0 + t * 0.03),
                period,
                per_lane: 1,
                pal: None,
            },
        );
        // Blocks sliding across the slope over the carpets: wait for one to pass.
        let blocks: Vec<(f64, f64, f64)> = [a0z + 11.0, a0z + 21.0, a0z + 31.0]
            .iter()
            .map(|&bz| {
                let bw = 0.7 + s.rng() * 0.5;
                let ph = s.rng() * 6.0;
                (bz, bw, ph)
            })
            .collect();
        let block_x = move |bw: f64, ph: f64, t: f64| m::sin(t * bw + ph) * (w / 2.0 - 1.6);
        for &(bz, bw, ph) in &blocks {
            let anchor = s.b.anchor(0.0, ya(bz), bz, ROOT);
            s.b.world.nodes.get_mut(anchor).rot.x = -ang;
            let opts = PrimOpts {
                parent: Some(anchor),
                dynamic: true,
                col: ColliderOpts {
                    hit: 0.6,
                    tag: Some("block"),
                    ..Default::default()
                },
                ..Default::default()
            };
            let node =
                s.b.box_(0.0, 0.25 + 0.2 + 0.8, 0.0, 1.8, 1.6, 1.8, pal::ORANGE, opts)
                    .node;
            s.b.mover(move |t, ctx| ctx.node(node).pos.x = block_x(bw, ph, t));
        }
        s.b.bonus(0.0, ya(a0z + 20.0), a0z + 20.0);
        s.b.box_(0.0, a1y - 1.0, a1z + 4.0, w, 2.0, 8.0, pal::PURPLE, o());

        let mut path = vec![Waypoint::spread(0.0, s.z + 1.0, 1.0)];
        type Clear = Box<dyn Fn(f64) -> bool + Send + Sync>;
        for i in 1..carpet.len() {
            let (x0, z0) = carpet[i - 1];
            let (x1, z1) = carpet[i];
            let seg_len = m::hypot(x1 - x0, z1 - z0);
            // Where the carpet crosses a ball lane or a block's track: stop before it, go when clear.
            let mut stops: Vec<(f64, Clear)> = Vec::new();
            for lane in lanes {
                if (lane - x0) * (lane - x1) >= 0.0 {
                    continue;
                }
                let f = (lane - x0) / (x1 - x0);
                let zz = z0 + (z1 - z0) * f;
                let balls = balls.clone();
                stops.push((
                    f,
                    Box::new(move |t| !balls.danger(lane - 1.5, lane + 1.5, zz - 2.5, zz + 2.5, t, 1.1)),
                ));
            }
            for &(bz, bw, ph) in &blocks {
                if (bz - z0) * (bz - z1) >= 0.0 {
                    continue;
                }
                let f = (bz - z0) / (z1 - z0);
                let xx = x0 + (x1 - x0) * f;
                stops.push((
                    f,
                    Box::new(move |t| {
                        [0.0, 0.3, 0.6, 0.9]
                            .iter()
                            .all(|dt| (block_x(bw, ph, t + dt) - xx).abs() > 3.0)
                    }),
                ));
            }
            stops.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap_or(core::cmp::Ordering::Equal));
            for (f, clear) in stops {
                let before = 0f64.at_least(f - 2.4 / seg_len);
                path.push(Waypoint::spread(x0 + (x1 - x0) * before, z0 + (z1 - z0) * before, 0.2));
                path.push(Waypoint::spread(x0 + (x1 - x0) * f, z0 + (z1 - z0) * f, 0.2).wait(move |bot| clear(bot.t)));
            }
            path.push(Waypoint::spread(x1, z1, 0.2));
        }
        path.push(Waypoint::spread(0.0, a1z + 4.0, 1.0));
        SegOut {
            z: a1z + 8.0,
            y: a1y,
            routes: vec![path],
            forbidden: Some(Box::new(move |p| {
                p.z > a0z && p.z < a1z && (p.x.abs() > w / 2.0 + 0.05 || p.y > ya(p.z) + 2.6)
            })),
            checkpoint: Some((a1z + 0.5, V3::new(0.0, a1y + 0.1, a1z + 4.0))),
        }
    })
}

/// A grippy ramp with balls in two lanes and blocks on the strip between them: step aside when no ball comes.
fn ball_ramp(rise: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let z0 = s.z;
        let len = 28.0;
        let z1 = z0 + len;
        let yr = move |zz: f64| y_on_ramp(zz, z0, y, z1, y + rise);
        s.b.ramp(0.0, z0, y, z1, y + rise, 9.0, pal::BLUE, 1.0, o());
        let ang = m::atan2(rise, len);
        for sx in [-1.0, 1.0] {
            let l = m::hypot(len, rise);
            s.b.box_(
                sx * 4.9,
                y + rise / 2.0 + 0.6,
                (z0 + z1) / 2.0,
                0.8,
                1.2,
                l,
                pal::PINK,
                rot(-ang, 0.0, 0.0),
            );
        }
        let period = 3.2 + s.rng() * 0.8;
        let balls = rolling_balls(
            s.b,
            BallLaneOpts {
                lanes: vec![-2.7, 2.7],
                z_top: z1 - 1.0,
                y_top: y + rise,
                z_bottom: z0 + 1.0,
                y_bottom: y,
                radius: 1.0,
                speed: Arc::new(|t| 8.5 + t * 0.02),
                period,
                per_lane: 1,
                pal: None,
            },
        );
        let blocks = [z0 + 7.0, z0 + 14.0, z0 + 21.0];
        for (i, &zb) in blocks.iter().enumerate() {
            let p = if i % 2 == 1 { pal::ORANGE } else { pal::YELLOW };
            s.b.box_(0.0, yr(zb) + 0.6, zb, 1.8, 1.8, 1.4, p, rot(-ang, 0.0, 0.0));
        }
        s.b.box_(0.0, y + rise - 1.0, z1 + 3.0, 16.0, 2.0, 6.0, pal::PURPLE, o());
        let mut pts = vec![Waypoint::spread(0.0, z0 + 0.5, 0.0)];
        for (i, &zb) in blocks.iter().enumerate() {
            let lx = (if i % 2 == 1 { 1.0 } else { -1.0 }) * 1.55;
            pts.push(Waypoint::spread(0.0, zb - 2.3, 0.0));
            let balls = balls.clone();
            pts.push(Waypoint::spread(lx, zb - 0.8, 0.0).wait(move |bot| {
                !balls.danger(
                    lx.at_most(0.0) - 0.3,
                    lx.at_least(0.0) + 0.3,
                    zb - 2.5,
                    zb + 2.5,
                    bot.t,
                    0.9,
                )
            }));
            pts.push(Waypoint::spread(lx, zb + 0.9, 0.0));
            pts.push(Waypoint::spread(0.0, zb + 2.3, 0.0));
        }
        pts.push(Waypoint::spread(0.0, z1 + 3.0, 1.0));
        SegOut {
            z: z1 + 6.0,
            y: y + rise,
            routes: vec![pts],
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z1 && p.x.abs() > 4.4)),
            checkpoint: Some((z1 + 0.5, V3::new(0.0, y + rise + 0.1, z1 + 3.0))),
        }
    })
}

impl MapDef for BallHill {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["snow", "meadow", "candy"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![ball_ramp(6.0), pistons(3, 14.0), tipping_bridge(5), glove_alley(3)];
        let middle = pick_sections(&mut b.rng, pool, 2);
        let mut sections = vec![ice_slope()];
        sections.extend(middle);
        sections.push(sliding_gates(4, 6.0));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
