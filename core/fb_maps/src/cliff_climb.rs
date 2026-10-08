//! Up and up: rock steps to catch the edges of, a cliff with ladders under a pendulum, something from the
//! seed, shelves zig-zagging up over the void, and a tower of ladders to the finish at the top.
use std::sync::Arc;

use fb_shared::rgb;
use fb_sim::bots::{BotView, SharedTest, Waypoint};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::course::{
    CourseOpts, SegOut, Segment, hammer_bridges, pick_sections, race_course, sliding_gates, tipping_bridge, with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::props::arm_contact_eta;
use fb_sim::scene::Surface;
use fb_sim::scene::{Palette, pal};

use crate::util::o;

pub struct CliffClimb;

static META: GameMeta = GameMeta::new(
    "cliff-climb",
    "Скалолазы",
    Genre::Race,
    "Всё выше и выше! Запрыгивайте на уступы и цепляйтесь за край, лезьте по лестницам (прыжок — соскочить) и не попадитесь под маятник. Наверху ждёт финиш.",
    "Заберитесь на вершину",
    160.0,
);

const STEP_PALS: [Palette; 6] = [pal::ORANGE, pal::YELLOW, pal::GREEN, pal::TEAL, pal::BLUE, pal::PURPLE];

fn rock() -> PrimOpts {
    PrimOpts {
        surface: Some(Surface::Rock),
        ..Default::default()
    }
}

/// Rock steps a little under the reach of a jump: jump, catch the edge, pull up. Boulders to hop over.
fn ledge_steps(n: u32, rise: f64) -> Segment {
    Box::new(move |s| {
        let w = 14.0;
        let depth = 3.5;
        s.b.box_(0.0, s.y - 1.0, s.z + 2.0, w, 2.0, 4.0, pal::PURPLE, o());
        let mut z = s.z + 4.0;
        let mut y = s.y;
        let mut route = Vec::new();
        for k in 0..n {
            y += rise;
            let d = if k == n - 1 { 7.0 } else { depth };
            let h = y - s.y + 3.0;
            let p = STEP_PALS[k as usize % STEP_PALS.len()];
            s.b.box_(0.0, y - h / 2.0, z + d / 2.0, w, h, d, p, rock());
            if k % 2 == 1 && k < n - 1 {
                let x = -4.0 + s.rng() * 8.0;
                s.b.box_(x, y + 0.4, z + d / 2.0, 2.4, 0.8, 1.2, pal::WHITE, rock());
            }
            route.push(Waypoint::spread(0.0, z + d / 2.0, 2.0));
            z += d;
        }
        s.b.bonus(3.0, s.y + rise, s.z + 5.7);
        SegOut {
            z,
            y,
            routes: vec![route],
            checkpoint: Some((z - 6.5, V3::new(0.0, y + 0.1, z - 3.5))),
            ..Default::default()
        }
    })
}

/// A cliff face with ladders up it, and a pendulum swinging along the face at mid height: wait for it to
/// pass (or hang on and let it go by), then up.
fn ladder_wall(h: f64) -> Segment {
    Box::new(move |s| {
        let w = 14.0;
        let wf = s.z + 10.0;
        s.b.box_(0.0, s.y - 1.0, s.z + 5.0, w, 2.0, 10.0, pal::BLUE, o());
        s.b.box_(
            0.0,
            s.y + (h - 2.0) / 2.0,
            wf + 4.0,
            w,
            h + 2.0,
            8.0,
            pal::PURPLE,
            rock(),
        );
        let lxs = [-5.25, -1.75, 1.75, 5.25];
        for lx in lxs {
            s.b.ladder(lx, s.y, wf, s.y + h, m::PI, rgb(0xffb347));
        }
        // The pendulum: its head sweeps along the face over every ladder, at the height of a climber.
        let py = s.y + h + 3.2;
        let pz = wf - 1.2;
        for sx in [-7.4, 7.4] {
            s.b.box_(sx, (s.y + py) / 2.0, pz, 0.8, py - s.y, 0.8, pal::PINK, o());
        }
        s.b.box_(0.0, py + 0.4, pz, 15.6, 0.8, 1.2, pal::PINK, o());
        let sp = 1.1 + s.rng() * 0.4;
        let ph = s.rng() * m::TAU;
        s.b.hammer(0.0, py, pz, sp, ph, 1.0, false);
        let head_x = move |t: f64| 6.0 * m::sin(m::sin(t * sp + ph));
        let top = s.y + h;
        SegOut {
            z: wf + 8.0,
            y: top,
            routes: lxs
                .iter()
                .map(|&lx| {
                    vec![
                        Waypoint::exact(lx, wf - 4.0),
                        Waypoint::exact(lx, wf + 1.2).wait(move |bot| {
                            let mut dt = 0.1;
                            while dt <= 1.5 {
                                if (head_x(bot.t + dt) - lx).abs() < 2.4 {
                                    return false;
                                }
                                dt += 0.1;
                            }
                            true
                        }),
                        Waypoint::spread(0.0, wf + 5.0, 1.0),
                    ]
                })
                .collect(),
            checkpoint: Some((wf + 0.5, V3::new(0.0, top + 0.1, wf + 5.0))),
            ..Default::default()
        }
    })
}

/// Narrow rock shelves stepping up from side to side over the void: jump across and up to the next one
/// (catch its edge if the jump is short).
fn zigzag_ledges(n: u32, rise: f64) -> Segment {
    Box::new(move |s| {
        s.b.box_(0.0, s.y - 1.0, s.z + 2.5, 10.0, 2.0, 5.0, pal::PURPLE, o());
        let mut shelves = Vec::new();
        let mut y = s.y;
        let mut z = s.z + 5.0 + 1.5;
        let mut side = if s.rng() < 0.5 { -1.0 } else { 1.0 };
        for k in 0..n {
            y += rise;
            let x = side * 2.6;
            let p = STEP_PALS[k as usize % STEP_PALS.len()];
            s.b.box_(x, y - 1.5, z, 4.0, 3.0, 3.0, p, rock());
            shelves.push((x, y, z));
            side = -side;
            z += 2.4;
        }
        let z0 = z - 0.4;
        s.b.box_(0.0, y + rise - 1.0, z0 + 3.5, 12.0, 2.0, 7.0, pal::PINK, o());
        let top_y = y + rise;
        let mut route = vec![Waypoint::spread(0.0, s.z + 3.0, 0.5)];
        let mut hops = shelves;
        hops.push((0.0, top_y, z0 + 1.5));
        // Jump from the edge of the shelf the bot stands on (it runs at the middle of the next one).
        let mut from = (0.0, s.y, 5.0, s.z + 5.0);
        let last = hops.len() - 1;
        for (k, &(hx, hy, hz)) in hops.iter().enumerate() {
            let (fx, fy, fhx, fz1) = from;
            route.push(Waypoint::exact(hx, hz).jump_when(move |bot| {
                let p = bot.body.pos;
                if !bot.body.grounded || (p.y - fy).abs() > 0.4 {
                    return false;
                }
                (p.x - fx).abs() > fhx - 0.7 || p.z > fz1 - 0.7 || m::hypot(p.x - hx, p.z - hz) < 3.0
            }));
            from = (hx, hy, 2.0, hz + 1.5);
            if k == last {
                route.push(Waypoint::spread(0.0, z0 + 5.0, 1.0));
            }
        }
        SegOut {
            z: z0 + 7.0,
            y: top_y,
            routes: vec![route],
            checkpoint: Some((z0 + 0.5, V3::new(0.0, top_y + 0.1, z0 + 4.0))),
            ..Default::default()
        }
    })
}

/// A tower of shelves, each a ladder's climb above the last; sweeping bars on the shelves on the way
/// (jump them, even at the foot of a ladder).
fn ladder_tower(levels: u32, rise: f64) -> Segment {
    Box::new(move |s| {
        let w = 12.0;
        let depth = 8.0;
        s.b.box_(0.0, s.y - 1.0, s.z + 3.5, w, 2.0, 7.0, pal::BLUE, o());
        let mut routes: [Vec<Waypoint>; 2] = [Vec::new(), Vec::new()];
        let never: SharedTest = Arc::new(|_: &mut BotView| false);
        let mut jump_when = never.clone();
        let mut end = s.z;
        let mut top = s.y;
        for k in 1..=levels {
            let face = s.z + 7.0 + (k - 1) as f64 * depth;
            let y = s.y + k as f64 * rise;
            let d = if k == levels { 9.0 } else { depth };
            let h = y - s.y + 3.0;
            let p = STEP_PALS[(k as usize + 2) % STEP_PALS.len()];
            s.b.box_(0.0, y - h / 2.0, face + d / 2.0, w, h, d, p, rock());
            let spread = 2.0 + s.rng() * 2.5;
            for (r, sx) in [-1.0, 1.0].into_iter().enumerate() {
                let lx = sx * spread;
                let color = if k % 2 == 1 { rgb(0xffb347) } else { rgb(0xf4f1ff) };
                s.b.ladder(lx, y - rise, face, y, m::PI, color);
                routes[r].push(Waypoint::exact(lx, face - 2.6).jump_shared(&jump_when));
                routes[r].push(Waypoint::exact(lx, face + 1.2));
            }
            jump_when = never.clone();
            if k < levels {
                let c = face + depth / 2.0;
                let sp = (0.9 + s.rng() * 0.4) * if s.rng() < 0.5 { -1.0 } else { 1.0 };
                let ph = s.rng() * 6.0;
                let ang = move |t: f64| if t <= 0.0 { ph } else { ph + t * sp };
                s.b.hub(0.0, y, c, 0.7);
                s.b.rotor(0.0, y + 0.6, c, 3.5, 1, ang, 0.45);
                jump_when = Arc::new(move |bot: &mut BotView| {
                    let p = bot.body.pos;
                    if bot.t <= 0.0 || (p.y - y).abs() > 0.5 || m::hypot(p.x, p.z - c) < 1.2 {
                        return false;
                    }
                    let eta = arm_contact_eta(p, ang(bot.t), sp, 1, 0.0, c, 0.36);
                    eta > 0.1 && eta < 0.24
                });
            }
            end = face + d;
            top = y;
        }
        for r in &mut routes {
            r.push(Waypoint::spread(0.0, end - 2.0, 1.0));
        }
        SegOut {
            z: end,
            y: top,
            routes: routes.into(),
            checkpoint: Some((end - 7.5, V3::new(0.0, top + 0.1, end - 4.0))),
            ..Default::default()
        }
    })
}

impl MapDef for CliffClimb {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Snow, LookId::Desert, LookId::Castle]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![tipping_bridge(5), hammer_bridges(3), sliding_gates(3, 3.0)];
        let extra = pick_sections(&mut b.rng, pool, 1);
        let mut sections = vec![ledge_steps(4, 1.5), ladder_wall(6.0)];
        sections.extend(extra);
        sections.push(zigzag_ledges(6, 1.6));
        sections.push(ladder_tower(3, 4.0));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
