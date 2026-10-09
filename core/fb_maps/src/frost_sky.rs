//! Down an icy slope, over gaps only a dive makes, sliding under low bars, a section from the seed,
//! another slope, then flying shuttles over the clouds to the finish.
use std::sync::Arc;

use fb_shared::NEVER;
use fb_shared::rgb;
use fb_sim::bots::{BotInput, BotView, Note, SharedTest, Waypoint, follow, steer};
use fb_sim::builder::{Builder, PrimOpts, Ramp};
use fb_sim::collider::ColliderOpts;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, edge_jump, moving_platforms, pick_sections, race_course, seesaws, with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapId, MapSpec};
use fb_sim::math::{V3, v3};
use fb_sim::physics::BodyState;
use fb_sim::props::arm_contact_eta;
use fb_sim::scene::Surface;
use fb_sim::scene::{Palette, pal};

use crate::util::{deco, dynamic, o, rot};

pub struct FrostSky;

static META: GameMeta = GameMeta::new(
    MapId::FrostSky,
    "Ледяные небеса",
    Genre::Race,
    "Скользкие ледяные склоны, пропасти, которые берутся только прыжком с нырком (E или ЛКМ в полёте), низкие перекладины — под ними только проскользить, и летающие платформы над облаками.",
    "Доскользите до финиша",
    150.0,
);

const ICE_PAL: Palette = [rgb(0xbfe9ff), rgb(0xe4f6ff)];

fn ice(slip: f64) -> PrimOpts {
    PrimOpts {
        col: ColliderOpts {
            slip,
            ..Default::default()
        },
        surface: Some(Surface::Ice),
        ..Default::default()
    }
}

/// A slope of ice down between rails, bumpers on the way, and a run-out of ice at the bottom.
fn ice_slope(drop: f64, len: f64) -> Segment {
    Box::new(move |s| {
        let w = 12.0;
        let y0 = s.y;
        s.b.box_(v3(0.0, y0 - 1.0, s.z + 2.0), v3(w, 2.0, 4.0), pal::PURPLE, o());
        let z0 = s.z + 4.0;
        let z1 = z0 + len;
        let y1 = y0 - drop;
        s.b.ramp(
            Ramp {
                x: 0.0,
                z0,
                y0,
                z1,
                y1,
                width: w,
                thick: 1.0,
            },
            ICE_PAL,
            ice(0.9),
        );
        let ang = m::atan2(drop, len);
        for sx in [-1.0, 1.0] {
            s.b.box_(
                v3(sx * (w / 2.0 + 0.4), (y0 + y1) / 2.0 + 0.6, (z0 + z1) / 2.0),
                v3(0.8, 1.2, m::hypot(len, drop)),
                pal::PINK,
                rot(ang, 0.0, 0.0),
            );
        }
        let y_at = |z: f64| y0 + ((z - z0) / len) * (y1 - y0);
        for k in 0..4 {
            let bz = z0 + 5.0 + f64::from(k) * 5.5;
            let x = (if k % 2 == 1 { 1.0 } else { -1.0 }) * (1.2 + s.rng() * 2.8);
            s.b.bumper(v3(x, y_at(bz) - 0.1, bz), 0.8, 9.0);
        }
        s.b.box_(v3(0.0, y1 - 1.0, z1 + 3.0), v3(w, 2.0, 6.0), ICE_PAL, ice(0.9));
        s.b.rails(z1, z1 + 6.0, w / 2.0, y1, pal::PINK);
        s.b.bonus(v3(0.0, y1, z1 + 3.0));
        SegOut {
            z: z1 + 6.0,
            y: y1,
            routes: vec![vec![
                Waypoint::spread(0.0, z0 + len * 0.5, 3.0),
                Waypoint::spread(0.0, z1 + 5.5, 2.0),
            ]],
            forbidden: Some(Box::new(move |p| {
                p.z > z0 && p.z < z1 + 6.0 && p.x.abs() > w / 2.0 + 0.05
            })),
            ..Default::default()
        }
    })
}

/// Gaps a jump alone falls short of: jump from the edge, dive in the air to reach the other side.
fn dive_gaps(n: u32, gap: f64) -> Segment {
    Box::new(move |s| {
        let len = 6.0;
        let y = s.y;
        let mut route = Vec::new();
        let mut z = s.z;
        for k in 0..=n {
            let p = if k % 2 == 1 { pal::TEAL } else { pal::BLUE };
            s.b.box_(v3(0.0, y - 1.0, z + len / 2.0), v3(9.0, 2.0, len), p, o());
            // Take-off line near the edge.
            s.b.box_(
                v3(0.0, y + 0.01, z + len - 0.6),
                v3(9.0, 0.02, 0.35),
                pal::YELLOW,
                deco(),
            );
            if k == n {
                break;
            }
            let edge = z + len;
            let next = edge + gap;
            route.push(Waypoint::spread(0.0, edge - 2.0, 0.5));
            route.push(
                Waypoint::exact(0.0, next + 2.0)
                    .jump_when(edge_jump(edge, 0.9))
                    // In the air past the edge, on the way down: dive for the far side.
                    .drive(move |bot, out| {
                        let body = bot.body;
                        if body.grounded || body.state != BodyState::Normal || body.pos.z < edge || body.vel.y > 2.5 {
                            return false;
                        }
                        steer(bot, 0.0, next + 2.0, out, 1.0);
                        if body.pos.z < next - 0.5 {
                            out.dive = true;
                        }
                        true
                    }),
            );
            z = next;
        }
        route.push(Waypoint::spread(0.0, z + len - 1.0, 0.5));
        SegOut {
            z: z + len,
            y,
            routes: vec![route],
            checkpoint: Some((z + 0.5, V3::new(0.0, y + 0.1, z + 3.0))),
            ..Default::default()
        }
    })
}

/// Low bars across an icy run: nobody gets under them standing up; a dive from the yellow stripes slides
/// under.
fn dive_bars(n: u32) -> Segment {
    Box::new(move |s| {
        let w = 12.0;
        let gap_z = 11.0;
        let y = s.y;
        let z0 = s.z;
        let len = f64::from(n) * gap_z + 4.0;
        s.b.box_(v3(0.0, y - 1.0, z0 + len / 2.0), v3(w, 2.0, len), ICE_PAL, ice(0.5));
        s.b.rails(z0, z0 + len, w / 2.0, y, pal::PINK);
        let mut route = Vec::new();
        for k in 0..n {
            let bz = z0 + 9.0 + f64::from(k) * gap_z;
            let p = if k % 2 == 1 { pal::ORANGE } else { pal::PURPLE };
            s.b.box_(v3(0.0, y + 1.2 + 1.5, bz), v3(w + 1.6, 3.0, 0.8), p, o());
            // Where to dive from.
            for dz in [7.0, 5.5] {
                s.b.box_(v3(0.0, y + 0.01, bz - dz), v3(w, 0.02, 0.4), pal::YELLOW, deco());
            }
            let key: Note<f64> = s.b.note();
            route.push(Waypoint::exact(0.0, bz + 2.5).drive(move |bot, out| {
                let body = bot.body;
                let p = body.pos;
                if p.z > bz + 0.6 {
                    return false;
                }
                if body.state != BodyState::Normal {
                    out.mx = 0.0;
                    out.mz = 1.0;
                    return true;
                }
                // Stopped at the bar: back off for another run.
                if bot.mem.get(key).unwrap_or(NEVER) > bot.t {
                    follow(bot, 0.0, bz - 9.0, out);
                    return true;
                }
                if body.grounded && p.z > bz - 2.2 && m::hypot(body.vel.x, body.vel.z) < 2.0 {
                    let until = bot.t + 1.0;
                    bot.mem.set(key, until);
                    return true;
                }
                follow(bot, 0.0, bz + 3.0, out);
                out.mz = 1.0;
                if body.grounded && p.z > bz - 7.2 && p.z < bz - 5.3 {
                    out.dive = true;
                }
                true
            }));
        }
        route.push(Waypoint::spread(0.0, z0 + len - 0.5, 0.5));
        SegOut {
            z: z0 + len,
            y,
            routes: vec![route],
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z0 + len && p.y > y + 3.5)),
            ..Default::default()
        }
    })
}

/// Round decks of ice with sweeping bars: jump them without sliding off.
fn ice_rotors(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let mut zz = s.z;
        let mut route = Vec::new();
        for _ in 0..n {
            let r = 5.8 + s.rng();
            let c = zz + 4.0 + r;
            s.b.box_(
                v3(0.0, y - 1.0, (zz + c - r + 0.3) / 2.0),
                v3(3.6, 2.0, c - r + 0.3 - zz),
                pal::YELLOW,
                o(),
            );
            s.b.cyl(v3(0.0, y - 1.0, c), r + 0.3, 2.0, ICE_PAL, ice(0.35));
            s.b.hub(v3(0.0, y, c), 1.0);
            let sp = (0.9 + s.rng() * 0.4) * if s.rng() < 0.5 { -1.0 } else { 1.0 };
            let ph = s.rng() * 6.0;
            let ang = move |t: f64| if t <= 0.0 { ph } else { ph + t * sp };
            s.b.rotor(v3(0.0, y + 0.6, c), r, 2, ang, 0.45);
            let jump_when: SharedTest = Arc::new(move |bot: &mut BotView| {
                let p = bot.body.pos;
                if bot.t <= 0.0 || m::hypot(p.x, p.z - c) < 1.2 || m::hypot(p.x, p.z - c) > r + 1.0 {
                    return false;
                }
                let eta = arm_contact_eta(p, ang(bot.t), sp, 2, 0.0, c, 0.36);
                eta > 0.1 && eta < 0.24
            });
            let side = if s.rng() < 0.5 { -1.0 } else { 1.0 };
            route.extend([
                Waypoint::spread(0.0, c - r - 1.5, 0.3).jump_shared(&jump_when),
                Waypoint::spread(side * 2.6, c - 2.5, 0.3).jump_shared(&jump_when),
                Waypoint::spread(side * 2.6, c + 2.5, 0.3).jump_shared(&jump_when),
                Waypoint::spread(0.0, c + r + 1.0, 0.3).jump_shared(&jump_when),
            ]);
            zz = c + r - 0.3;
        }
        s.b.box_(v3(0.0, y - 1.0, zz + 2.0), v3(3.6, 2.0, 4.0), pal::YELLOW, o());
        SegOut {
            z: zz + 4.0,
            y,
            routes: vec![route],
            ..Default::default()
        }
    })
}

/// A platform shuttling between two docks (and up), resting at each end for a moment.
#[derive(Clone, Copy)]
struct Shuttle {
    x: f64,
    za: f64,
    zb: f64,
    ya: f64,
    yb: f64,
    dwell: f64,
    travel: f64,
    phase: f64,
}

impl Shuttle {
    fn u(&self, t: f64) -> f64 {
        let ease = |u: f64| u * u * (3.0 - 2.0 * u);
        let period = 2.0 * (self.dwell + self.travel);
        let f = (((t + self.phase) % period) + period) % period;
        if f < self.dwell {
            0.0
        } else if f < self.dwell + self.travel {
            ease((f - self.dwell) / self.travel)
        } else if f < 2.0 * self.dwell + self.travel {
            1.0
        } else {
            1.0 - ease((f - 2.0 * self.dwell - self.travel) / self.travel)
        }
    }

    fn pos(&self, t: f64) -> V3 {
        let k = self.u(t);
        V3::new(
            self.x,
            self.ya + (self.yb - self.ya) * k,
            self.za + (self.zb - self.za) * k,
        )
    }

    /// −1 resting at the near dock, 1 at the far one, 0 on the way.
    fn at(&self, t: f64) -> i32 {
        let k = self.u(t);
        if k <= 0.0 {
            -1
        } else if k >= 1.0 {
            1
        } else {
            0
        }
    }

    /// Where a bean stands on it.
    fn top(&self, t: f64) -> V3 {
        let q = self.pos(t);
        V3::new(q.x, q.y + 0.5, q.z)
    }
}

/// Bots across a gap on shuttles: wait at the near dock for one to rest there, step on, keep to its
/// middle, step off when it rests at the far dock.
fn ride_drive(
    ferries: Vec<Shuttle>,
    near_edge: f64,
    far_edge: f64,
    far_y: f64,
) -> impl Fn(&mut BotView, &mut BotInput) -> bool + Send + Sync + 'static {
    move |bot, out| {
        let body = bot.body;
        let p = body.pos;
        let t = bot.t;
        if body.grounded && p.z > far_edge + 0.3 && (p.y - far_y).abs() < 0.5 {
            return false;
        }
        let mut ride = ferries[0];
        for f in &ferries {
            if (f.top(t).x - p.x).abs() < (ride.top(t).x - p.x).abs() {
                ride = *f;
            }
        }
        let fp = ride.top(t);
        let aboard = p.z > near_edge + 0.1 && p.z < far_edge - 0.1 && (p.y - fp.y).abs() < 0.8;
        if aboard {
            if ride.at(t) == 1 && ride.at(t + 0.5) == 1 {
                steer(bot, fp.x, far_edge + 2.5, out, 1.0);
            } else {
                follow(bot, fp.x, fp.z, out);
            }
            return true;
        }
        if !body.grounded {
            steer(bot, fp.x, fp.z, out, 1.0);
            return true;
        }
        // On the near dock: the shuttle that rests here (and stays a moment), or the one that comes next.
        let mut next = ferries[0];
        let mut soonest = f64::INFINITY;
        for f in &ferries {
            let mut dt = 0.0;
            while dt < 8.0 {
                if f.at(t + dt) == -1 && f.at(t + dt + 0.9) == -1 {
                    if dt < soonest {
                        soonest = dt;
                        next = *f;
                    }
                    break;
                }
                dt += 0.25;
            }
        }
        let g = next.top(t);
        // Lined up with its lane on the dock first, then straight on board.
        if soonest > 0.0 || (p.x - g.x).abs() > 0.8 {
            follow(bot, g.x, near_edge - 1.2, out);
        } else {
            steer(bot, g.x, g.z, out, 1.0);
        }
        true
    }
}

/// Flying platforms: shuttles over the clouds to an island higher up, and others on to a higher dock.
fn sky_shuttles() -> Segment {
    Box::new(|s| {
        let w = 14.0;
        let size = 4.4;
        let gap = 16.0;
        let y = s.y;
        s.b.box_(v3(0.0, y - 1.0, s.z + 3.0), v3(w, 2.0, 6.0), pal::PURPLE, o());
        let legs = [(s.z + 6.0, y, y + 2.0), (s.z + 6.0 + gap + 6.0, y + 2.0, y + 4.0)];
        let mut route = vec![Waypoint::spread(0.0, s.z + 2.5, 1.0)];
        let dwell = 1.8;
        let travel = 3.4;
        for (k, &(near, y0, y1)) in legs.iter().enumerate() {
            let far = near + gap;
            let za = near + 0.35 + size / 2.0;
            let zb = far - 0.35 - size / 2.0;
            let ph = s.rng() * 10.0;
            let ferries: Vec<Shuttle> = [-3.0, 3.0]
                .into_iter()
                .enumerate()
                .map(|(i, x)| {
                    let sh = Shuttle {
                        x,
                        za,
                        zb,
                        ya: y0 - 0.5,
                        yb: y1 - 0.5,
                        dwell,
                        travel,
                        phase: ph + i as f64 * (dwell + travel),
                    };
                    let p = if i == 1 { pal::ORANGE } else { pal::GREEN };
                    let node = s.b.box_(v3(x, y0 - 0.5, za), v3(size, 1.0, size), p, dynamic()).node;
                    let pod = PrimOpts {
                        parent: Some(node),
                        no_collide: true,
                        surface: Some(Surface::Metal),
                        seg: 20,
                        ..Default::default()
                    };
                    s.b.cyl(v3(0.0, -0.8, 0.0), 1.1, 0.7, pal::solid(rgb(0x39406b)), pod);
                    s.b.mover(move |t, ctx| ctx.node(node).pos = sh.pos(t));
                    sh
                })
                .collect();
            let len = if k == legs.len() - 1 { 7.0 } else { 6.0 };
            let p = if k == 1 { pal::PINK } else { pal::TEAL };
            s.b.box_(v3(0.0, y1 - 1.0, far + len / 2.0), v3(w, 2.0, len), p, o());
            route.push(Waypoint::spread(0.0, far + 2.5, 0.5).drive(ride_drive(ferries, near, far, y1)));
        }
        let end = legs[1].0 + gap + 7.0;
        let top_y = y + 4.0;
        route.push(Waypoint::spread(0.0, end - 1.0, 1.0));
        SegOut {
            z: end,
            y: top_y,
            routes: vec![route],
            checkpoint: Some((end - 6.5, V3::new(0.0, top_y + 0.1, end - 3.5))),
            ..Default::default()
        }
    })
}

impl MapDef for FrostSky {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Snow, LookId::Starlight, LookId::Ocean]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![ice_rotors(2), moving_platforms(4), seesaws(3)];
        let extra = pick_sections(&mut b.rng, pool, 1);
        let mut sections = vec![ice_slope(5.0, 26.0), dive_gaps(3, 7.5), dive_bars(3)];
        sections.extend(extra);
        sections.push(ice_slope(4.0, 22.0));
        sections.push(sky_shuttles());
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
