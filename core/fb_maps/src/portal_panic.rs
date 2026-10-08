//! Gaps that only portals cross: one-way portals from island to island (under sweeping bars), portals
//! that open in turn, portals at the foot of a wall that throw you over it, portal cannons across the
//! void; in between, a few more challenges drawn from the seed.
use std::sync::Arc;

use fb_shared::{Rgb, rgb};
use fb_sim::bots::{BotView, SharedTest, Waypoint, aim_landing};
use fb_sim::builder::{Builder, OpenFn, PortalEnd, PortalOpts, portal_shut};
use fb_sim::course::{
    CourseOpts, SegOut, Segment, moving_platforms, pick_sections, pistons, portal_fork, race_course, rotor_decks,
    with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::props::arm_contact_eta;
use fb_sim::scene::{Piece, lamp_part, pal};

use crate::util::o;

pub struct PortalPanic;

static META: GameMeta = GameMeta::new(
    "portal-panic",
    "Портальный переполох",
    Genre::Race,
    "Пропасти, которые не перепрыгнуть: только порталы! Обычные ведут туда и обратно, односторонние — только вперёд (выход со стрелкой), одни открываются по очереди, другие швыряют через стену.",
    "Добегите до финиша",
    150.0,
);

const ONE_WAY: Rgb = rgb(0x39e0d0);
const CANNON: Rgb = rgb(0xff8a3d);
const BLINK: Rgb = rgb(0xa66bff);

fn end(x: f64, y: f64, z: f64) -> PortalEnd {
    PortalEnd { x, y, z, yaw: 0.0 }
}

fn one_way(open: Option<OpenFn>) -> PortalOpts {
    PortalOpts {
        one_way: true,
        closed: 0.3,
        open,
        ..Default::default()
    }
}

/// One-way, throwing beans out at `speed` and up at `lift` m/s.
fn thrown(speed: f64, lift: f64) -> PortalOpts {
    PortalOpts {
        speed,
        lift: Some(lift),
        ..one_way(None)
    }
}

/// A stretch of the route that walks into a portal at (x, z) and carries on.
fn through(x: f64, z: f64) -> [Waypoint; 2] {
    [Waypoint::exact(x, z - 2.4), Waypoint::exact(x, z + 0.6)]
}

/// Islands over gaps nobody can jump, each swept by a low bar: two one-way portals at the far end of each
/// lead on to the next island (left to left, right to right).
fn portal_islands(n: usize) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let w = 14.0;
        let gap = 13.0;
        let mut islands = vec![(s.z, 7.0)];
        for k in 1..=n {
            let (pz, pl) = islands[k - 1];
            islands.push((pz + pl + gap, 11.0));
        }
        let mut routes: Vec<Vec<Waypoint>> = vec![Vec::new(), Vec::new()];
        for (k, &(iz, il)) in islands.iter().enumerate() {
            let p = if k % 2 == 1 { pal::TEAL } else { pal::BLUE };
            s.b.box_(0.0, y - 1.0, iz + il / 2.0, w, 2.0, il, p, o());
            let mut jump_when: SharedTest = Arc::new(|_: &mut BotView| false);
            if k > 0 {
                let c = iz + il / 2.0 + 0.5;
                let sp = (0.9 + s.rng() * 0.5) * if s.rng() < 0.5 { -1.0 } else { 1.0 };
                let ph = s.rng() * 6.0;
                let ang = move |t: f64| if t <= 0.0 { ph } else { ph + t * sp };
                s.b.hub(0.0, y, c, 0.8);
                s.b.rotor(0.0, y + 0.6, c, 6.4, 2, ang, 0.45);
                jump_when = Arc::new(move |bot: &mut BotView| {
                    let p = bot.body.pos;
                    if bot.t <= 0.0 || (p.z - c).abs() > 7.0 || m::hypot(p.x, p.z - c) < 1.2 {
                        return false;
                    }
                    let eta = arm_contact_eta(p, ang(bot.t), sp, 2, 0.0, c, 0.36);
                    eta > 0.1 && eta < 0.24
                });
            }
            for (r, sx) in [-1.0, 1.0].into_iter().enumerate() {
                let route = &mut routes[r];
                let x = sx * 3.6;
                if k == 0 {
                    route.push(Waypoint::exact(x, iz + 2.0));
                } else {
                    route.push(Waypoint::exact(x, iz + il / 2.0).jump_shared(&jump_when));
                    route.push(Waypoint::exact(x, iz + il - 3.9).jump_shared(&jump_when));
                }
                if k == n {
                    continue;
                }
                let ez = iz + il - 1.5;
                let (nz, _) = islands[k + 1];
                s.b.portal(end(x, y, ez), end(x, y, nz + 0.8), ONE_WAY, one_way(None));
                route.push(Waypoint::exact(x, ez + 0.6).jump_shared(&jump_when));
            }
        }
        let (lz, ll) = islands[n];
        let end_z = lz + ll;
        s.b.bonus(0.0, y, islands[1].0 + 2.0);
        for r in &mut routes {
            r.push(Waypoint::spread(0.0, end_z + 0.5, 1.0));
        }
        SegOut {
            z: end_z,
            y,
            routes,
            ..Default::default()
        }
    })
}

/// A gap with three one-way portals in a row, each open for a moment in turn: go for the one about to open.
fn blinking_portals() -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let near = 9.0;
        let gap = 14.0;
        let far = s.z + near + gap;
        s.b.box_(0.0, y - 1.0, s.z + near / 2.0, 17.0, 2.0, near, pal::PURPLE, o());
        s.b.box_(0.0, y - 1.0, far + 4.0, 17.0, 2.0, 8.0, pal::PINK, o());
        let period = 3.4 + s.rng() * 1.2;
        let share = 0.42;
        let start = s.rng() * period;
        let mut routes = Vec::new();
        for (k, x) in [-5.2, 0.0, 5.2].into_iter().enumerate() {
            let ph = start + (k as f64 * period) / 3.0 + (s.rng() - 0.5) * 0.3;
            let open: OpenFn =
                Arc::new(move |t: f64| t > 0.0 && (((t + ph) % period) + period) % period / period < share);
            let ez = s.z + near - 1.6;
            let pair =
                s.b.portal(end(x, y, ez), end(x, y, far + 0.6), BLINK, one_way(Some(open.clone())));
            // A lamp over each: green while it is open.
            if !s.b.server() {
                let lamp = s.b.anchor(x, y + 3.35, ez, ROOT);
                let open = open.clone();
                s.b.special_look(lamp, "portal-lamp", &[lamp_part(0.26)], move |_, t, out| {
                    out.pieces
                        .push(Piece::at(0, 0.0, 0.0, 0.0).tone(if open(t) { 1.0 } else { 0.0 }));
                });
            }
            routes.push(vec![
                Waypoint::exact(x, ez - 3.2),
                Waypoint::exact(x, ez + 0.6).wait(move |bot| {
                    let p = bot.world.portals[pair];
                    !portal_shut(&p, Some(&open), bot.t + 0.15) && !portal_shut(&p, Some(&open), bot.t + 0.5)
                }),
                Waypoint::spread(0.0, far + 5.0, 1.0),
            ]);
        }
        s.b.bonus(0.0, y, s.z + 2.5);
        SegOut {
            z: far + 8.0,
            y,
            routes,
            checkpoint: Some((far + 0.5, V3::new(0.0, y + 0.1, far + 5.0))),
            ..Default::default()
        }
    })
}

/// A wall too high to climb: portals at its foot come out on top of it and throw whoever comes out over
/// it, down onto the landing beyond. Bumpers stand in the way to the portals.
fn portal_wall() -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let w = 16.0;
        let h = 7.0;
        let wz = s.z + 12.0;
        s.b.box_(0.0, y - 1.0, s.z + 6.0, w, 2.0, 12.0, pal::BLUE, o());
        s.b.box_(0.0, y + h / 2.0 - 1.0, wz, w, h + 2.0, 3.0, pal::PURPLE, o());
        s.b.box_(0.0, y - 1.0, wz + 1.5 + 7.0, w, 2.0, 14.0, pal::TEAL, o());
        let bumpers = [(-2.4 - s.rng(), s.z + 4.5), (2.4 + s.rng(), s.z + 4.5)];
        for (x, z) in bumpers {
            s.b.bumper(x, y, z, 0.8, 10.0);
        }
        let mut routes = Vec::new();
        for x in [-5.0, 0.0, 5.0] {
            let ez = wz - 3.0;
            s.b.portal(end(x, y, ez), end(x, y + h, wz - 1.1), CANNON, thrown(9.0, 6.0));
            let land = wz + 9.0;
            let mut route: Vec<Waypoint> = through(x, ez).into();
            route.push(
                Waypoint::exact(x, land).drive(move |bot, out| !bot.body.grounded && aim_landing(bot, x, y, land, out)),
            );
            route.push(Waypoint::spread(0.0, wz + 13.0, 1.0));
            routes.push(route);
        }
        SegOut {
            z: wz + 15.5,
            y,
            routes,
            checkpoint: Some((wz + 2.0, V3::new(0.0, y + 0.1, wz + 11.0))),
            ..Default::default()
        }
    })
}

/// Portal cannons: step into a portal in the middle of a ledge and be shot out of the one at its edge,
/// across the gap and down onto the next ledge (steer in the air to land).
fn portal_cannons(n: usize) -> Segment {
    Box::new(move |s| {
        let len = 10.0;
        let gap = 11.0;
        let drop = 2.5;
        let mut routes: Vec<Vec<Waypoint>> = vec![Vec::new(), Vec::new()];
        let mut z = s.z;
        let mut y = s.y;
        for k in 0..=n {
            let p = if k % 2 == 1 { pal::PINK } else { pal::BLUE };
            s.b.box_(0.0, y - 1.0, z + len / 2.0, 14.0, 2.0, len, p, o());
            if k == n {
                break;
            }
            let ny = y - drop;
            let nz = z + len + gap;
            for (r, x) in [-3.5, 3.5].into_iter().enumerate() {
                let ez = z + 3.5;
                s.b.portal(end(x, y, ez), end(x, y, z + len - 1.8), CANNON, thrown(13.0, 10.0));
                let land = nz + 3.5;
                let yy = ny;
                routes[r].extend(through(x, ez));
                routes[r].push(
                    Waypoint::exact(x, land)
                        .drive(move |bot, out| !bot.body.grounded && aim_landing(bot, x, yy, land, out)),
                );
            }
            z = nz;
            y = ny;
        }
        for r in &mut routes {
            r.push(Waypoint::spread(0.0, z + len - 0.5, 1.0));
        }
        SegOut {
            z: z + len,
            y,
            routes,
            checkpoint: Some((z + 0.5, V3::new(0.0, y + 0.1, z + 5.0))),
            ..Default::default()
        }
    })
}

/// Before the finish: a lone one-way portal over a last gap (a jump and a dive make it too, just).
fn last_hop() -> Segment {
    Box::new(move |s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 3.0, 10.0, 2.0, 6.0, pal::PURPLE, o());
        let far = s.z + 6.0 + 9.0;
        s.b.box_(0.0, y - 1.0, far + 3.0, 10.0, 2.0, 6.0, pal::PURPLE, o());
        s.b.portal(end(0.0, y, s.z + 4.0), end(0.0, y, far + 0.5), ONE_WAY, one_way(None));
        let mut route: Vec<Waypoint> = through(0.0, s.z + 4.0).into();
        route.push(Waypoint::spread(0.0, far + 5.0, 0.5));
        SegOut {
            z: far + 6.0,
            y,
            routes: vec![route],
            ..Default::default()
        }
    })
}

impl MapDef for PortalPanic {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Neon, LookId::Starlight, LookId::Candy]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![portal_fork(), rotor_decks(2), moving_platforms(4), pistons(3, 14.0)];
        let mut middle = pick_sections(&mut b.rng, pool, 2).into_iter();
        let sections = vec![
            portal_islands(2),
            blinking_portals(),
            middle.next().unwrap(),
            portal_wall(),
            middle.next().unwrap(),
            portal_cannons(2),
            last_hop(),
        ];
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
