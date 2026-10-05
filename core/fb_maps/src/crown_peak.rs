//! The climb: a fork up to the first plateau (a ramp with balls, or sliding steps), hammer bridges, a
//! glove-swept plateau with launch pads up to a rotor deck, then more climbing drawn from the seed, and the
//! summit: sliding steps up to the crown, guarded by a sweeper. First to touch it wins.
use std::sync::Arc;

use fb_sim::bots::Waypoint;
use fb_sim::builder::Builder;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, edge_jump, hammer_bridges, moving_platforms, pick_sections, portal_fork, race_course,
    sliding_gates, timed_doors, tipping_bridge, trampoline_gap, with_rests,
};
use fb_sim::m::{self, MinMax};
use fb_sim::map::{Finish, GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::props::{BallLaneOpts, GloveOpts, arm_contact_eta, glove_puncher, rolling_balls, sweep_eta, y_on_ramp};
use fb_sim::scene::pal;

use crate::util::{dynamic, freq, o, rot};

pub struct CrownPeak;

static META: GameMeta = GameMeta {
    finale: true,
    ..GameMeta::new(
        "crown-peak",
        "Гора короны",
        Genre::Race,
        "Долгий подъём: склон с шарами или скользящие ступени, мосты под молотами, перчатки и батуты, дальше — испытания в случайном порядке. Кто первым коснётся короны на вершине, тот и победил!",
        "Доберитесь до короны",
        170.0,
    )
};

/// Up to a plateau: a ramp with balls in two lanes (blocks on the strip between), or sliding steps.
fn climb_fork(rise: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let z0 = s.z;
        let top = z0 + 29.0;
        let y1 = y + rise;
        let yr = move |zz: f64| y_on_ramp(zz, z0, y, top, y1);
        // Left: the ramp.
        s.b.ramp(-5.0, z0, y, top, y1, 7.0, pal::BLUE, 1.0, o());
        let ang = m::atan2(rise, top - z0);
        let l = m::hypot(top - z0, rise);
        s.b.box_(
            -8.9,
            (y + y1) / 2.0 + 0.6,
            (z0 + top) / 2.0,
            0.8,
            1.2,
            l,
            pal::PINK,
            rot(-ang, 0.0, 0.0),
        );
        let period = 3.3 + s.rng() * 0.6;
        let balls = rolling_balls(
            s.b,
            BallLaneOpts {
                lanes: vec![-7.0, -3.0],
                z_top: top - 1.0,
                y_top: y1,
                z_bottom: z0 + 1.0,
                y_bottom: y,
                radius: 1.0,
                speed: Arc::new(|t| 8.5 + t * 0.02),
                period,
                per_lane: 1,
                pal: None,
            },
        );
        let blocks = [z0 + 7.0, z0 + 15.0, z0 + 23.0];
        for (i, &zb) in blocks.iter().enumerate() {
            let p = if i % 2 == 1 { pal::ORANGE } else { pal::YELLOW };
            s.b.box_(-5.0, yr(zb) + 0.6, zb, 1.8, 1.8, 1.4, p, rot(-ang, 0.0, 0.0));
        }
        // Right: sliding steps.
        let steps: Vec<(f64, f64, f64, f64)> = (0..5)
            .map(|k| {
                let w = 0.8 + s.rng() * 0.6;
                let ph = s.rng() * 6.0;
                (
                    z0 + 3.0 + k as f64 * 5.4,
                    y + 1.3 + k as f64 * ((rise - 1.3) / 4.0),
                    w,
                    ph,
                )
            })
            .collect();
        let step_x = |w: f64, ph: f64, t: f64| 5.0 + m::sin(t * w + ph) * 2.2;
        for (k, &(z, sy, w, ph)) in steps.iter().enumerate() {
            let p = if k % 2 == 1 { pal::ORANGE } else { pal::GREEN };
            let node = s.b.box_(5.0, sy - 0.5, z, 3.2, 1.0, 3.0, p, dynamic()).node;
            s.b.mover(move |t, ctx| ctx.node(node).pos.x = step_x(w, ph, t));
        }
        s.b.box_(0.0, y1 - 1.0, top + 5.0, 20.0, 2.0, 10.0, pal::PURPLE, o());

        let ramp = |lane: f64| {
            let mut pts = vec![Waypoint::spread(-5.0, z0 + 0.5, 0.0)];
            for (i, &zb) in blocks.iter().enumerate() {
                let lx = -5.0 + (if i % 2 == 1 { 1.0 } else { -1.0 }) * 1.55;
                pts.push(Waypoint::spread(-5.0, zb - 2.3, 0.0));
                let balls = balls.clone();
                pts.push(Waypoint::spread(lx, zb - 0.8, 0.0).wait(move |bot| {
                    !balls.danger(
                        (-5f64).at_most(lx) - 0.3,
                        (-5f64).at_least(lx) + 0.3,
                        zb - 2.5,
                        zb + 2.5,
                        bot.t,
                        0.9,
                    )
                }));
                pts.push(Waypoint::spread(lx, zb + 0.9, 0.0));
                pts.push(Waypoint::spread(-5.0, zb + 2.3, 0.0));
            }
            pts.push(Waypoint::spread(lane, top + 2.0, 0.3));
            pts.push(Waypoint::spread(0.0, top + 5.0, 1.0));
            pts
        };
        let mut step_route = vec![Waypoint::spread(5.0, z0 - 0.8, 0.0)];
        let mut edge = z0;
        for &(z, _, w, ph) in &steps {
            step_route.push(
                Waypoint::moving(move |t| step_x(w, ph, t), z)
                    .wait(move |bot| (step_x(w, ph, bot.t + 0.55) - bot.body.pos.x).abs() < 1.0)
                    .jump_when(edge_jump(edge, 1.1)),
            );
            edge = z + 1.5;
        }
        step_route.push(Waypoint::spread(3.0, top + 2.0, 0.3).jump_when(edge_jump(edge, 1.1)));
        step_route.push(Waypoint::spread(0.0, top + 5.0, 1.0));
        SegOut {
            z: top + 10.0,
            y: y1,
            routes: vec![ramp(-3.0), ramp(3.0), step_route],
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < top && p.x < -8.4)),
            checkpoint: Some((top + 0.5, V3::new(0.0, y1 + 0.1, top + 5.0))),
        }
    })
}

/// A plateau swept by gloves, then launch pads up to a deck with a sweeper.
fn glove_launch(rise: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let z0 = s.z;
        s.b.box_(0.0, y - 1.0, z0 + 7.0, 14.0, 2.0, 14.0, pal::PURPLE, o());
        let gloves: Vec<_> = [(z0 + 5.3, -1.0), (z0 + 8.2, 1.0)]
            .iter()
            .map(|&(z, side)| {
                let w = 1.1 + s.rng() * 0.4;
                let ph = s.rng() * 6.0;
                glove_puncher(
                    s.b,
                    GloveOpts {
                        x: side * 9.0,
                        y: y + 0.95,
                        z,
                        side,
                        w,
                        ph,
                        reach: 8.5,
                        scale: 1.3,
                        post_to: Some(y - 6.0),
                    },
                )
            })
            .collect();
        let pad_z = z0 + 10.2;
        s.b.pad(-3.0, y, pad_z, 1.3, 19.0, None);
        s.b.pad(3.0, y, pad_z, 1.3, 19.0, None);
        let deck_y = y + rise;
        let deck_z = z0 + 20.0;
        s.b.box_(0.0, deck_y - 1.0, deck_z, 14.0, 2.0, 10.0, pal::PINK, freq(0.3));
        s.b.hub(0.0, deck_y, deck_z, 0.9);
        let sp = 1.2 + s.rng() * 0.5;
        let ph = s.rng() * 6.0;
        let ang = move |t: f64| if t <= 0.0 { ph } else { ph + t * sp };
        s.b.rotor(0.0, deck_y + 0.6, deck_z, 5.8, 2, ang, 0.45);
        s.b.box_(0.0, deck_y - 1.0, deck_z + 7.0, 8.0, 2.0, 4.0, pal::PURPLE, o());
        let route = |x: f64| {
            let gloves = gloves.clone();
            vec![
                Waypoint::spread(x, z0 + 2.0, 0.5),
                Waypoint::spread(x, pad_z, 0.0).wait(move |bot| {
                    [0.0, 0.25, 0.5, 0.75, 1.0]
                        .iter()
                        .all(|dt| gloves.iter().all(|g| g.x_at(bot.t + dt).abs() > 9.0 - 8.5 * 0.3))
                }),
                Waypoint::spread(0.0, deck_z + 4.5, 0.3).jump_when(move |bot| {
                    let p = bot.body.pos;
                    p.y > deck_y - 0.5 && sweep_eta(p.x, p.z, ang(bot.t), sp, 2, 0.0, deck_z) < 0.15
                }),
                Waypoint::spread(0.0, deck_z + 7.0, 0.5),
            ]
        };
        SegOut {
            z: deck_z + 9.0,
            y: deck_y,
            routes: vec![route(-3.0), route(3.0)],
            // Falling off the deck: back before the pads (the deck is all within the rotor's reach).
            checkpoint: Some((z0 + 0.5, V3::new(0.0, y + 0.1, z0 + 2.5))),
            ..Default::default()
        }
    })
}

/// The summit: three sliding steps, then the crown over a platform guarded by a slow sweeper.
fn summit(b: &mut Builder, z0: f64, y0: f64) -> (Finish, Vec<Waypoint>) {
    let slide_x = |k: f64, t: f64| m::sin(t * (0.8 + k * 0.25) + k * 2.0) * 1.6;
    b.box_(0.0, y0 - 1.0, z0 + 1.5, 8.0, 2.0, 3.0, pal::PURPLE, o());
    for k in 0..3 {
        let kf = k as f64;
        let p = if k % 2 == 1 { pal::ORANGE } else { pal::GREEN };
        let node = b
            .box_(
                0.0,
                y0 + 0.5 + kf * 1.2,
                z0 + 5.0 + kf * 3.5,
                4.0,
                1.0,
                3.0,
                p,
                dynamic(),
            )
            .node;
        b.mover(move |t, ctx| ctx.node(node).pos.x = slide_x(kf, t));
    }
    let top_y = y0 + 4.0;
    let cz = z0 + 20.0;
    b.box_(0.0, top_y - 1.0, cz, 14.0, 2.0, 10.0, pal::YELLOW, o());
    let hub_z = cz + 1.5;
    let cw = 1.1;
    let crown_ang = move |t: f64| if t <= 0.0 { 0.0 } else { t * cw };
    b.hub(0.0, top_y, hub_z, 0.6);
    b.rotor(0.0, top_y + 0.6, hub_z, 5.0, 2, crown_ang, 0.45);
    let crown = b.model("crown", ROOT);
    let n = b.world.nodes.get_mut(crown);
    n.pos = V3::new(0.0, top_y + 2.6, cz - 1.0);
    n.scale = V3::splat(1.6);
    let summit_jump = edge_jump(z0 + 13.5, 1.1);
    let route = vec![
        Waypoint::spread(0.0, z0 + 1.5, 0.3),
        Waypoint::moving(move |t| slide_x(0.0, t), z0 + 5.0).jump_when(edge_jump(z0 + 3.0, 1.1)),
        Waypoint::moving(move |t| slide_x(1.0, t), z0 + 8.5).jump_when(edge_jump(z0 + 6.5, 1.1)),
        Waypoint::moving(move |t| slide_x(2.0, t), z0 + 12.0).jump_when(edge_jump(z0 + 10.0, 1.1)),
        Waypoint::spread(0.0, cz - 1.0, 0.2).jump_when(move |bot| {
            if bot.body.pos.z < z0 + 14.0 {
                return summit_jump(bot);
            }
            let eta = arm_contact_eta(bot.body.pos, crown_ang(bot.t), cw, 2, 0.0, hub_z, 0.36);
            eta > 0.1 && eta < 0.24
        }),
    ];
    let finish = Finish {
        z: cz - 2.4,
        y: top_y - 1.0,
        half_width: Some(2.2),
    };
    (finish, route)
}

impl MapDef for CrownPeak {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["royal", "snow", "castle"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            sliding_gates(4, 5.0),
            moving_platforms(5),
            tipping_bridge(6),
            trampoline_gap(),
            timed_doors(2, 16.0),
            portal_fork(),
        ];
        let more = pick_sections(&mut b.rng, pool, 3);
        let mut sections = vec![climb_fork(8.0), hammer_bridges(3), glove_launch(5.0)];
        sections.extend(more);
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            finish_with: Some(Box::new(summit)),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
