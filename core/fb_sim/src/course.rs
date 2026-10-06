//! Race courses built from sections. A map lists its sections (fixed ones, and
//! pools the seed draws from and shuffles); each builds itself from where the last one ended, with its
//! own timings drawn from the seed, and tells the bots how to get through (one or more routes). Rest
//! platforms between them are checkpoints. Server and clients build the same course.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::bots::{BOT_DT, BotBrain, BotView, Note, SharedTest, Waypoint, init_bot, path_step};
use crate::builder::{Builder, PortalEnd, PortalOpts, PrimOpts};
use crate::collider::{ColId, ColliderOpts, Shape};
use crate::m::{self, MinMax};
use crate::map::{Checkpoint, Cx, Finish, MapCtx, MapEvent, MapSpec, OnTick, PosTest, SegEvent};
use crate::math::V3;
use crate::nodes::{NodeId, ROOT};
use crate::props::{GloveOpts, arm_contact_eta, glove_puncher};
use crate::scene::{Palette, Piece, lamp_part, pal};
use fb_shared::rng::{Rng, shuffle};

pub type SegHandler = Box<dyn FnMut(&mut Cx, &SegEvent) + Send + Sync>;

pub struct SegCtx<'a> {
    pub b: &'a mut Builder,
    pub ctx: &'a MapCtx<'a>,
    /// Where the section starts (the end of the previous one) and the floor height there.
    pub z: f64,
    pub y: f64,
    /// The section's place in the course: its events carry it.
    seg: u32,
    handlers: &'a mut BTreeMap<u32, Vec<SegHandler>>,
    ticks: &'a mut Vec<OnTick>,
    /// Notes the course's bots share between sections.
    pub notes: CourseNotes,
}

/// Bots' notes that hold across the sections of a course.
#[derive(Clone, Copy)]
pub struct CourseNotes {
    /// Helping at a gate: until when, on which side, and the gate (its z) last helped at.
    pub help: Note<f64>,
    pub help_side: Note<f64>,
    pub helped: Note<f64>,
    /// Seconds spent pushing a door that holds.
    pub push: Note<f64>,
}

impl SegCtx<'_> {
    pub fn rng(&mut self) -> f64 {
        self.b.rng.next()
    }

    /// Handles this section's events.
    pub fn on(&mut self, f: impl FnMut(&mut Cx, &SegEvent) + Send + Sync + 'static) {
        self.handlers.entry(self.seg).or_default().push(Box::new(f));
    }

    /// The section's number, for `seg_emit`.
    pub fn seg(&self) -> u32 {
        self.seg
    }

    /// Server: per-tick logic of this section.
    pub fn tick(&mut self, f: impl FnMut(&mut Cx, f64) + Send + Sync + 'static) {
        self.ticks.push(Box::new(f));
    }
}

/// Emits an event of section `seg` (from `SegCtx::seg`).
pub fn seg_emit(cx: &mut Cx, seg: u32, ev: SegEvent) {
    cx.emit(MapEvent::Seg { seg, ev });
}

#[derive(Default)]
pub struct SegOut {
    /// Where the next section starts, and its floor height.
    pub z: f64,
    pub y: f64,
    /// Ways through for bots (each from z to the end); a bot picks one per section.
    pub routes: Vec<Vec<Waypoint>>,
    /// Standing here counts as a shortcut (tops of walls, frames).
    pub forbidden: Option<PosTest>,
    /// This section ends on a platform worth a checkpoint: respawn at `p`, active past `from`.
    pub checkpoint: Option<(f64, V3)>,
}

pub type Segment = Box<dyn FnOnce(&mut SegCtx) -> SegOut>;

/// A special end instead of the finish platform (a summit with a crown): built from (z, y).
pub type FinishWith = Box<dyn FnOnce(&mut Builder, f64, f64) -> (Finish, Vec<Waypoint>)>;

#[derive(Default)]
pub struct CourseOpts {
    /// Sections in order (`pick_sections` for random ones).
    pub sections: Vec<Segment>,
    /// Length of the finish platform.
    pub finish_len: Option<f64>,
    pub clouds: Option<u32>,
    pub finish_with: Option<FinishWith>,
}

/// A few sections drawn from a pool (without repeats) in a seeded order.
pub fn pick_sections(rng: &mut Rng, mut pool: Vec<Segment>, n: usize) -> Vec<Segment> {
    shuffle(&mut pool, rng);
    pool.truncate(n);
    pool
}

/// Alternates rest platforms between sections.
pub fn with_rests(list: Vec<Segment>, len: f64) -> Vec<Segment> {
    let mut out = Vec::new();
    for (i, s) in list.into_iter().enumerate() {
        if i > 0 {
            out.push(rest(len, 16.0, pal::PURPLE));
        }
        out.push(s);
    }
    out
}

/// Jump just before an edge (z) when running at it.
pub fn edge_jump(edge: f64, before: f64) -> impl Fn(&mut BotView) -> bool + Send + Sync + 'static {
    move |bot| bot.body.pos.z > edge - before && bot.body.pos.z < edge + 0.3
}

fn d() -> PrimOpts {
    PrimOpts::default()
}

fn dynamic() -> PrimOpts {
    PrimOpts {
        dynamic: true,
        ..Default::default()
    }
}

/// Builds a race course and returns its spec (spawns, finish, checkpoints, events, bots).
pub fn race_course(b: &mut Builder, ctx: &MapCtx, o: CourseOpts) -> MapSpec {
    let spawns = b.start_area(0.0);
    let mut handlers: BTreeMap<u32, Vec<SegHandler>> = BTreeMap::new();
    let notes = CourseNotes {
        help: b.note(),
        help_side: b.note(),
        helped: b.note(),
        push: b.note(),
    };
    let mut ticks: Vec<OnTick> = Vec::new();
    let mut routes: Vec<Vec<Vec<Waypoint>>> = Vec::new();
    let mut forbidden: Vec<PosTest> = Vec::new();
    let mut checkpoints = vec![Checkpoint {
        z: -100.0,
        p: V3::new(0.0, 0.1, 10.0),
    }];
    // Out of the start pen onto a platform (the first respawn point is on it).
    b.box_(0.0, -1.0, 10.0, 18.0, 2.0, 6.0, pal::PURPLE, d());
    let mut zz = 13.0;
    let mut y = 0.0;
    let mut min_y: f64 = 0.0;
    let mut max_y: f64 = 0.0;
    for (i, seg) in o.sections.into_iter().enumerate() {
        let mut s = SegCtx {
            b,
            ctx,
            z: zz,
            y,
            seg: i as u32,
            handlers: &mut handlers,
            ticks: &mut ticks,
            notes,
        };
        let out = seg(&mut s);
        routes.push(out.routes);
        if let Some(f) = out.forbidden {
            forbidden.push(f);
        }
        if let Some((from, p)) = out.checkpoint {
            checkpoints.push(Checkpoint { z: from, p });
        }
        zz = out.z;
        y = out.y;
        min_y = min_y.at_most(y);
        max_y = max_y.at_least(y);
    }
    let finish = if let Some(fw) = o.finish_with {
        let (finish, route) = fw(b, zz, y);
        routes.push(vec![route]);
        max_y = max_y.at_least(finish.y + 1.0);
        finish
    } else {
        let len = o.finish_len.unwrap_or(16.0);
        b.box_(0.0, y - 1.0, zz + len / 2.0, 18.0, 2.0, len, pal::YELLOW, d());
        let finish_z = zz + 3.0;
        b.finish(0.0, y, finish_z);
        routes.push(vec![vec![
            Waypoint::spread(0.0, finish_z - 1.0, 2.0),
            Waypoint::spread(0.0, finish_z + 5.0, 3.0),
        ]]);
        Finish {
            z: finish_z,
            y: y - 1.0,
            half_width: None,
        }
    };
    b.clouds_with(
        0.0,
        zz / 2.0,
        60f64.at_least(zz * 0.45),
        o.clouds.unwrap_or(40),
        min_y - 30.0,
        max_y + 6.0,
    );
    let picks: Vec<Note<usize>> = routes.iter().map(|_| b.note()).collect();
    MapSpec {
        spawns,
        kill_y: min_y - 14.0,
        finish: Some(finish),
        checkpoints,
        forbidden: Some(Box::new(move |p| forbidden.iter().any(|f| f(p)))),
        on_event: Some(Box::new(move |cx, ev| {
            let MapEvent::Seg { seg, ev } = ev else { return };
            for h in handlers.get_mut(seg).into_iter().flatten() {
                h(cx, ev);
            }
        })),
        tick: Some(Box::new(move |cx, t| {
            for f in &mut ticks {
                f(cx, t);
            }
        })),
        bot: Some(course_brain(routes, picks)),
        ..Default::default()
    }
}

/// Each bot takes one route per section (chosen when it starts, kept in `picks`) and follows the joined path.
pub fn course_brain(sections: Vec<Vec<Vec<Waypoint>>>, picks: Vec<Note<usize>>) -> BotBrain {
    Box::new(move |bot, out| {
        init_bot(bot);
        let mut pts: Vec<&Waypoint> = Vec::new();
        for (r, &k) in sections.iter().zip(&picks) {
            let pick = match bot.mem.get(k) {
                Some(v) => v,
                None => {
                    let v = if r.len() > 1 {
                        (bot.rng.next() * r.len() as f64).floor() as usize
                    } else {
                        0
                    };
                    bot.mem.set(k, v);
                    v
                }
            };
            if let Some(route) = r.get(pick).or(r.first()) {
                pts.extend(route);
            }
        }
        path_step(&pts, 0.0, bot, out);
    })
}

// ------------------------------------------------------------------ sections

/// A plain platform (a checkpoint).
pub fn rest(len: f64, w: f64, p: Palette) -> Segment {
    Box::new(move |s| {
        s.b.box_(0.0, s.y - 1.0, s.z + len / 2.0, w, 2.0, len, p, d());
        SegOut {
            z: s.z + len,
            y: s.y,
            routes: vec![vec![Waypoint::spread(0.0, s.z + len / 2.0, 1.5)]],
            checkpoint: Some((s.z + 0.5, V3::new(0.0, s.y + 0.1, s.z + len / 2.0))),
            ..Default::default()
        }
    })
}

/// A narrow bridge (connector).
fn bridge(b: &mut Builder, z0: f64, z1: f64, y: f64) {
    b.box_(0.0, y - 1.0, (z0 + z1) / 2.0, 3.6, 2.0, z1 - z0, pal::YELLOW, d());
}

fn shared(f: impl Fn(&mut BotView) -> bool + Send + Sync + 'static) -> SharedTest {
    Arc::new(f)
}

/// Round decks with a hub and sweeping arms (a low one to jump, sometimes a high one to stay under),
/// joined by bridges. Speeds and directions from the seed.
pub fn rotor_decks(n: u32) -> Segment {
    Box::new(move |s| {
        let mut zz = s.z;
        let y = s.y;
        let mut route = Vec::new();
        for i in 0..n {
            let r = 5.6 + s.rng() * 1.0;
            let c = zz + 4.0 + r;
            bridge(s.b, zz, c - r + 0.3, y);
            let deck_pal = if i % 2 == 1 { pal::PINK } else { pal::PURPLE };
            let o = PrimOpts {
                freq: Some(0.35),
                ..Default::default()
            };
            s.b.cyl(0.0, y - 1.0, c, r + 0.3, 2.0, deck_pal, o);
            s.b.hub(0.0, y, c, 1.0);
            let arms = if s.rng() < 0.5 { 2 } else { 3 };
            let sp = (1.1 + s.rng() * 0.8) * if s.rng() < 0.5 { -1.0 } else { 1.0 };
            let ph = s.rng() * 6.0;
            let low = move |t: f64| if t <= 0.0 { ph } else { ph + t * sp };
            s.b.rotor(0.0, y + 0.6, c, r, arms, low, 0.45);
            let high = s.rng() < 0.55;
            let hsp = -m::sign(sp) * (0.8 + s.rng() * 0.5);
            let high_ang = move |t: f64| if t <= 0.0 { ph + 1.3 } else { ph + 1.3 + t * hsp };
            if high {
                s.b.rotor(0.0, y + 2.45, c, r, 1, high_ang, 0.45);
            }
            if i == 0 {
                s.b.bonus(-r * 0.55, y, c);
            }
            let jump_when = shared(move |bot| {
                let p = bot.body.pos;
                let d = m::hypot(p.x, p.z - c);
                if d < 1.2 || bot.t <= 0.0 {
                    return false;
                }
                let eta = arm_contact_eta(p, low(bot.t), sp, arms, 0.0, c, 0.36);
                // Where the bean is when the arm comes round: running in, it is in reach by then.
                let v = bot.body.vel;
                let vin = if d > 1e-3 {
                    -(v.x * p.x + v.z * (p.z - c)) / d
                } else {
                    0.0
                };
                if d - vin.at_least(0.0) * eta.at_least(0.0) > r + 1.2 {
                    return false;
                }
                eta > 0.1 && eta < 0.24 && (!high || arm_contact_eta(p, high_ang(bot.t), hsp, 1, 0.0, c, 0.36) > 0.8)
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
        bridge(s.b, zz, zz + 4.0, y);
        SegOut {
            z: zz + 4.0,
            y,
            routes: vec![route],
            ..Default::default()
        }
    })
}

/// Platforms sliding (or swinging) from side to side over a gap: jump across when the next one comes.
pub fn moving_platforms(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 3.0, 10.0, 2.0, 6.0, pal::PURPLE, d());
        let mut zz = s.z + 6.0;
        let mut route = vec![Waypoint::spread(0.0, s.z + 4.0, 0.2)];
        let mut edge = zz;
        for i in 0..n {
            let c = zz + 2.0 + 2.25;
            let sp = 0.9 + s.rng() * 0.7;
            let ph = s.rng() * 6.0;
            let amp = 3.0 + s.rng() * 1.5;
            let swing = s.rng() < 0.35;
            let fx = move |t: f64| m::sin(t * sp + ph) * amp;
            let fy = move |t: f64| {
                if swing {
                    -(1.0 - m::cos(m::sin(t * sp + ph) * 0.6)) * 3.0
                } else {
                    0.0
                }
            };
            let p = if i % 2 == 1 { pal::ORANGE } else { pal::GREEN };
            let node = s.b.box_(0.0, y - 0.5, c, 4.5, 1.0, 4.5, p, dynamic()).node;
            s.b.mover(move |t, ctx| {
                let n = ctx.node(node);
                n.pos.x = fx(t);
                n.pos.y = y - 0.5 + fy(t);
            });
            if i == n / 2 {
                s.b.bonus(0.0, y, c);
            }
            route.push(
                Waypoint::moving(fx, c)
                    .wait(move |bot| (fx(bot.t + 0.6) - bot.body.pos.x).abs() < 1.2 && fy(bot.t + 0.6).abs() < 0.5)
                    .jump_when(edge_jump(edge, 1.0)),
            );
            edge = c + 2.25;
            zz = c + 2.25;
        }
        zz += 2.0;
        s.b.box_(0.0, y - 1.0, zz + 3.0, 10.0, 2.0, 6.0, pal::PURPLE, d());
        route.push(Waypoint::spread(0.0, zz + 3.0, 0.5).jump_when(edge_jump(edge, 1.0)));
        SegOut {
            z: zz + 6.0,
            y,
            routes: vec![route],
            checkpoint: Some((zz + 0.5, V3::new(0.0, y + 0.1, zz + 3.0))),
            ..Default::default()
        }
    })
}

/// Narrow bridges under swinging hammers (two of them side by side: pick one), timings from the seed.
pub fn hammer_bridges(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let len = n as f64 * 7.0 + 6.5;
        let mut routes = Vec::new();
        for bx in [-4.5, 4.5] {
            let p = if bx < 0.0 { pal::BLUE } else { pal::TEAL };
            s.b.box_(bx, y - 1.0, s.z + len / 2.0, 3.2, 2.0, len, p, d());
            let mut route = vec![Waypoint::spread(bx, s.z + 0.8, 0.0)];
            for k in 0..n {
                // The two bridges' hammers are staggered (their heads swing over the other bridge).
                let hz = s.z + 3.5 + k as f64 * 7.0 + if bx > 0.0 { 3.5 } else { 0.0 };
                let w = 1.7 + s.rng() * 0.9;
                let ph = s.rng() * m::TAU;
                s.b.hammer(bx, y + 7.4, hz, w, ph, 1.12, true);
                let head_x = move |t: f64| bx + 6.0 * m::sin(m::sin(t * w + ph) * 1.12);
                route.push(Waypoint::spread(bx, hz - 2.6, 0.0));
                route.push(Waypoint::spread(bx, hz + 2.0, 0.0).wait(move |bot| {
                    [0.0, 0.2, 0.4, 0.6, 0.8]
                        .iter()
                        .all(|dt| (head_x(bot.t + dt) - bx).abs() > 2.6)
                }));
            }
            route.push(Waypoint::spread(bx, s.z + len + 1.0, 0.0));
            routes.push(route);
        }
        let (z0, z_end) = (s.z, s.z + len);
        s.b.box_(0.0, y - 1.0, z_end + 3.0, 16.0, 2.0, 6.0, pal::PURPLE, d());
        SegOut {
            z: z_end + 6.0,
            y,
            routes,
            // On top of the hammer frames.
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z_end && p.y > y + 2.0)),
            checkpoint: Some((z_end + 0.5, V3::new(0.0, y + 0.1, z_end + 3.0))),
        }
    })
}

/// Open fraction (0 shut … 1 open) of a door that opens `share` of every `period`, smoothly.
pub fn cycle_open(t: f64, period: f64, phase: f64, share: f64) -> f64 {
    let f = (((t + phase) % period) + period) % period / period;
    let ramp = 0.08;
    if f < share {
        return 1f64.at_most(f / ramp).at_most((share - f) / ramp);
    }
    0.0
}

/// Walls across the course with sliding doors that open and shut on their own rhythms.
pub fn timed_doors(rows: u32, w: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let gap_z = 7.0;
        let len = rows as f64 * gap_z + 2.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w, 2.0, len, pal::BLUE, d());
        s.b.rails(s.z, s.z + len, w / 2.0, y, pal::PINK);
        let mut routes: Vec<Vec<Waypoint>> = vec![Vec::new(), Vec::new()];
        let door_w = 3.2;
        for r in 0..rows {
            let wz = s.z + 4.0 + r as f64 * gap_z;
            let doors: Vec<[f64; 4]> = [-4.2, 4.2]
                .iter()
                .map(|dx| {
                    let x = dx + (s.rng() - 0.5) * 1.5;
                    let period = 3.2 + s.rng() * 1.8;
                    let phase = s.rng() * 5.0;
                    let share = 0.38 + s.rng() * 0.12;
                    [x, period, phase, share]
                })
                .collect();
            // Wall pieces around the two doorways.
            let edges = [
                -w / 2.0,
                doors[0][0] - door_w / 2.0,
                doors[0][0] + door_w / 2.0,
                doors[1][0] - door_w / 2.0,
                doors[1][0] + door_w / 2.0,
                w / 2.0,
            ];
            for k in (0..edges.len()).step_by(2) {
                let (a, c) = (edges[k], edges[k + 1]);
                let p = if r % 2 == 1 { pal::ORANGE } else { pal::PURPLE };
                s.b.box_((a + c) / 2.0, y + 1.6, wz, c - a, 3.2, 0.8, p, d());
            }
            s.b.box_(0.0, y + 3.5, wz, w, 0.6, 0.9, pal::YELLOW, d());
            for (di, &[x, period, phase, share]) in doors.iter().enumerate() {
                for side in [-1.0, 1.0] {
                    let o = PrimOpts {
                        dynamic: true,
                        col: ColliderOpts {
                            sinks: true,
                            nav_skip: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    let leaf = s.b.box_(
                        x + side * door_w / 4.0,
                        y + 1.6,
                        wz,
                        door_w / 2.0,
                        3.2,
                        0.4,
                        pal::YELLOW,
                        o,
                    );
                    let node = leaf.node;
                    s.b.mover(move |t, ctx| {
                        ctx.node(node).pos.x =
                            x + side * (door_w / 4.0 + cycle_open(t, period, phase, share) * (door_w / 2.0 - 0.05));
                    });
                }
                let open = move |t: f64| cycle_open(t, period, phase, share);
                routes[di].push(Waypoint::spread(x, wz - 2.2, 0.0));
                routes[di].push(
                    Waypoint::spread(x, wz + 1.6, 0.0)
                        .wait(move |bot| open(bot.t + 0.15) > 0.7 && open(bot.t + 0.55) > 0.7),
                );
            }
        }
        s.b.bonus(0.0, y, s.z + 4.0 + gap_z / 2.0);
        for r in &mut routes {
            r.push(Waypoint::spread(0.0, s.z + len + 0.5, 1.0));
        }
        let z0 = s.z;
        SegOut {
            z: s.z + len,
            y,
            routes,
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z0 + len && p.y > y + 2.5)),
            ..Default::default()
        }
    })
}

/// The heavy gate's button state: transitions from the server; the gate's lift follows from them.
struct Gate {
    pressed: bool,
    level0: f64,
    at: f64,
    period: f64,
    phase: f64,
}

const GATE_RATE: f64 = 2.2;

impl Gate {
    fn held(&self, t: f64) -> f64 {
        if self.pressed {
            1f64.at_most(self.level0 + 0f64.at_least(t - self.at) * GATE_RATE)
        } else {
            0f64.at_least(self.level0 - 0f64.at_least(t - self.at) * GATE_RATE)
        }
    }

    fn open(&self, t: f64) -> f64 {
        cycle_open(t, self.period, self.phase, 0.22).at_least(self.held(t))
    }

    fn set_pressed(&mut self, on: bool, t: f64) {
        self.level0 = self.held(t);
        self.at = t;
        self.pressed = on;
    }
}

/// A heavy gate that only opens now and then by itself, or while somebody stands on one of the
/// buttons beside it: hold it for the others (and lose time), or wait for your turn.
pub fn coop_gate(w: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let len = 16.0;
        let wz = s.z + 10.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w, 2.0, len, pal::TEAL, d());
        s.b.rails(s.z, s.z + len, w / 2.0, y, pal::PINK);
        let gw = 4.4;
        for side in [-1.0, 1.0] {
            let x = side * (gw / 2.0 + (w / 2.0 - gw / 2.0) / 2.0);
            s.b.box_(x, y + 1.8, wz, w / 2.0 - gw / 2.0, 3.6, 1.0, pal::PURPLE, d());
        }
        s.b.box_(0.0, y + 3.9, wz, w, 0.6, 1.1, pal::YELLOW, d());
        let period = 8.0 + s.rng() * 3.0;
        let phase = s.rng() * period;
        let gate = s.b.state(Gate {
            pressed: false,
            level0: 0.0,
            at: -1e9,
            period,
            phase,
        });
        s.on(move |cx, ev| {
            if let SegEvent::Button { on, at } = *ev {
                cx.world.st_mut(gate).set_pressed(on, at);
            }
        });
        for side in [-1.0, 1.0] {
            let o = PrimOpts {
                dynamic: true,
                col: ColliderOpts {
                    sinks: true,
                    nav_skip: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            let node =
                s.b.box_(side * gw / 4.0, y + 1.8, wz, gw / 2.0, 3.6, 0.5, pal::ORANGE, o)
                    .node;
            s.b.mover(move |t, ctx| {
                let open = ctx.st(gate).open(t);
                ctx.node(node).pos.x = side * (gw / 4.0 + open * (gw / 2.0 - 0.05));
            });
        }
        // Buttons: round plates at both sides of the approach.
        let mut buttons: Vec<ColId> = Vec::new();
        let bx = w / 2.0 - 2.0;
        let bz = s.z + 4.0;
        for side in [-1.0, 1.0] {
            let rim = PrimOpts {
                no_collide: true,
                surface: Some("rubber"),
                ..Default::default()
            };
            s.b.cyl(side * bx, y + 0.02, bz, 1.25, 0.1, pal::hex("#5a3fb8"), rim);
            let top = PrimOpts {
                surface: Some("rubber"),
                ..Default::default()
            };
            buttons.push(s.b.cyl(side * bx, y + 0.1, bz, 1.0, 0.2, pal::RED, top).col());
        }
        if !s.b.server() {
            let lamp = s.b.anchor(0.0, y + 4.6, wz, ROOT);
            s.b.special_look(lamp, "gate-lamp", &[lamp_part(0.35)], move |w, t, out| {
                let on = w.st(gate).open(t) > 0.5;
                out.pieces
                    .push(Piece::at(0, 0.0, 0.0, 0.0).tone(if on { 1.0 } else { 0.0 }));
            });
        }
        // Server: somebody standing on a button holds the gate open.
        let (seg, notes) = (s.seg(), s.notes);
        s.tick(move |cx, t| {
            let mut any = false;
            for id in cx.bodies.ids() {
                if let Some(body) = cx.bodies.get(id)
                    && body.grounded
                    && buttons.iter().any(|&c| body.ground_col == c as i32)
                {
                    any = true;
                }
            }
            if any != cx.world.st(gate).pressed && t >= 0.0 {
                cx.world.st_mut(gate).set_pressed(any, t);
                seg_emit(cx, seg, SegEvent::Button { on: any, at: t });
            }
        });
        let route = vec![
            Waypoint::spread(0.0, s.z + 2.0, 1.0),
            // Helpful bots go and stand on a button for a while when the gate is shut and nobody helps.
            Waypoint::spread(0.0, wz - 2.0, 0.8).detour(move |bot| {
                if bot.mem.get(notes.help).unwrap_or(-1e9) > bot.t {
                    let side = bot.mem.get(notes.help_side).unwrap_or(1.0);
                    return Some((side * bx, bz));
                }
                if bot.mem.get(notes.helped) == Some(wz) || bot.body.pos.z > wz - 3.5 || bot.t <= 0.0 {
                    return None;
                }
                let g = bot.world.st(gate);
                if g.open(bot.t) < 0.3
                    && !g.pressed
                    && bot.mem.traits.aggro < 0.35
                    && bot.rng.next() < 0.4 * BOT_DT * 10.0
                {
                    bot.mem.set(notes.helped, wz);
                    let side = if bot.body.pos.x < 0.0 { -1.0 } else { 1.0 };
                    bot.mem.set(notes.help_side, side);
                    let until = bot.t + 3.0 + bot.rng.next() * 3.0;
                    bot.mem.set(notes.help, until);
                    return Some((side * bx, bz));
                }
                None
            }),
            Waypoint::spread(0.0, wz + 2.0, 0.3).wait(move |bot| {
                let g = bot.world.st(gate);
                g.open(bot.t) > 0.75 && g.open(bot.t + 0.5) > 0.7
            }),
            Waypoint::spread(0.0, s.z + len - 1.0, 1.0),
        ];
        SegOut {
            z: s.z + len,
            y,
            routes: vec![route],
            forbidden: Some(Box::new(move |p| (p.z - wz).abs() < 1.0 && p.y > y + 2.0)),
            ..Default::default()
        }
    })
}

struct Door {
    breakable: bool,
    broken: bool,
    /// Sim time it broke: it tips over backwards and is gone 1.4 s later.
    broken_at: f64,
    obj: NodeId,
    col: ColId,
    x: f64,
    row: u32,
}

/// Rows of doors: most are solid, some burst open when you run into them (the first one through finds out).
pub fn door_rows(rows: u32, w: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let gap_z = 9.0;
        let len = rows as f64 * gap_z + 3.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w + 1.0, 2.0, len, pal::BLUE, d());
        s.b.rails(s.z, s.z + len, (w + 1.0) / 2.0, y, pal::PINK);
        let n = 5;
        let dw = w / n as f64;
        let mut doors: Vec<Door> = Vec::new();
        let mut breakable_cols: Vec<(ColId, usize)> = Vec::new();
        let mut row_z = Vec::new();
        for r in 0..rows {
            let wz = s.z + 5.0 + r as f64 * gap_z;
            row_z.push(wz);
            let n_break = if s.rng() < 0.5 { 1 } else { 2 };
            let mut idx = [0usize, 1, 2, 3, 4];
            shuffle(&mut idx, &mut s.b.rng);
            let idx = &idx[..n_break];
            for i in 0..n {
                let x = -w / 2.0 + dw / 2.0 + i as f64 * dw;
                let obj = s.b.model("door", ROOT);
                let nd = s.b.world.nodes.get_mut(obj);
                nd.pos = V3::new(x, y, wz);
                nd.scale.x = dw / 3.1;
                nd.rot.y = m::PI;
                let id = doors.len();
                let at = s.b.anchor(x, y + 1.6, wz, ROOT);
                let col = s.b.collider(
                    at,
                    Shape::Box {
                        hx: dw / 2.0,
                        hy: 1.6,
                        hz: 0.3,
                    },
                    ColliderOpts {
                        is_static: true,
                        nav_skip: true,
                        ..Default::default()
                    },
                );
                let breakable = idx.contains(&i);
                if breakable {
                    breakable_cols.push((col, id));
                }
                doors.push(Door {
                    breakable,
                    broken: false,
                    broken_at: 0.0,
                    obj,
                    col,
                    x,
                    row: r,
                });
            }
            s.b.box_(0.0, y + 3.6, wz, w + 0.4, 0.8, 1.0, pal::YELLOW, d());
        }
        let st = s.b.state(doors);
        let break_door = move |cx: &mut Cx, id: usize| {
            let Some(d) = cx.world.st_mut(st).get_mut(id) else {
                return;
            };
            if d.broken || !d.breakable {
                return;
            }
            d.broken = true;
            d.broken_at = cx.t;
            let col = d.col;
            cx.world.colliders[col as usize].enabled = false;
            cx.sfx("break");
        };
        s.b.mover(move |t, ctx| {
            for i in 0..ctx.st(st).len() {
                let d = &ctx.st(st)[i];
                if !d.broken {
                    continue;
                }
                let (obj, dt) = (d.obj, (t - d.broken_at).at_least(0.0));
                let n = ctx.node(obj);
                n.rot.x = (dt * dt * 7.0).at_most(m::PI / 2.0);
                n.visible = dt <= 1.4;
            }
        });
        let seg = s.seg();
        for (col, id) in breakable_cols {
            s.b.on_touch(col, move |cx, _, _, _| {
                if cx.server {
                    seg_emit(cx, seg, SegEvent::Door(id as u32));
                } else {
                    break_door(cx, id);
                }
            });
        }
        s.on(move |cx, ev| {
            if let SegEvent::Door(id) = *ev {
                break_door(cx, id as usize);
            }
        });
        let notes = s.notes;
        let mut route = Vec::new();
        for (r, &wz) in row_z.iter().enumerate() {
            let (pick_key, tried_key): (Note<usize>, Note<u32>) = (s.b.note(), s.b.note());
            let r = r as u32;
            // Like a player: take a door someone broke, or barge into one; if it holds, try the next.
            route.push(Waypoint::spread(0.0, wz + 1.5, 0.5).detour(move |bot| {
                let p = bot.body.pos;
                if p.z > wz + 0.6 {
                    return None;
                }
                let row: Vec<(bool, f64)> = bot
                    .world
                    .st(st)
                    .iter()
                    .filter(|d| d.row == r)
                    .map(|d| (d.broken, d.x))
                    .collect();
                let mut pick = bot.mem.get(pick_key);
                let open: Vec<usize> = (0..row.len()).filter(|&i| row[i].0).collect();
                if !open.is_empty() && pick.is_none_or(|k| !row.get(k).is_some_and(|d| d.0)) {
                    let mut nearest = open[0];
                    for &i in &open[1..] {
                        if (row[i].1 - p.x).abs() < (row[nearest].1 - p.x).abs() {
                            nearest = i;
                        }
                    }
                    if (row[nearest].1 - p.x).abs() < 7.0 {
                        pick = Some(nearest);
                    }
                }
                let pick = match pick {
                    Some(k) => k,
                    None => {
                        let mask = bot.mem.get(tried_key).unwrap_or(0);
                        let mut options: Vec<usize> = (0..5).filter(|i| mask & (1 << i) == 0).collect();
                        let pref = p.x + bot.mem.traits.off * 3.0;
                        options.sort_by(|&a, &c| {
                            let (da, dc) = ((row[a].1 - pref).abs(), (row[c].1 - pref).abs());
                            da.total_cmp(&dc)
                        });
                        let first = bot.rng.next() < 0.7;
                        let at = if first {
                            0
                        } else {
                            1.min(options.len().saturating_sub(1))
                        };
                        match options.get(at) {
                            Some(&k) => k,
                            None => (bot.rng.next() * 5.0).floor() as usize,
                        }
                    }
                };
                bot.mem.set(pick_key, pick);
                let (broken, dx) = row[pick];
                if !broken && p.z > wz - 1.05 && (p.x - dx).abs() < 1.2 {
                    let push = bot.mem.get(notes.push).unwrap_or(0.0) + BOT_DT;
                    bot.mem.set(notes.push, push);
                    if push > 0.25 + bot.mem.traits.react {
                        let tried = bot.mem.get(tried_key).unwrap_or(0) | (1 << pick);
                        bot.mem.set(tried_key, tried);
                        bot.mem.remove(pick_key);
                        bot.mem.set(notes.push, 0.0);
                    }
                } else {
                    bot.mem.set(notes.push, 0.0);
                }
                let aligned = (p.x - dx).abs() < 0.5;
                Some(if broken || aligned || p.z > wz - 1.5 {
                    (dx, wz + 2.0)
                } else {
                    (dx, wz - 1.4)
                })
            }));
        }
        route.push(Waypoint::spread(0.0, s.z + len - 0.5, 1.0));
        let z0 = s.z;
        SegOut {
            z: s.z + len,
            y,
            routes: vec![route],
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z0 + len && p.y > y + 3.0)),
            ..Default::default()
        }
    })
}

/// A climb up a ramp through bumpers, with gloves punching out of the rails.
pub fn bumper_ramp(rise: f64, len: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let z0 = s.z;
        let z1 = s.z + len;
        let w = 12.0;
        s.b.ramp(0.0, z0, y, z1, y + rise, w, pal::BLUE, 1.0, d());
        let ang = m::atan2(rise, len);
        let y_at = |zz: f64| y + ((zz - z0) / len) * rise;
        for sx in [-1.0, 1.0] {
            let o = PrimOpts {
                rot: Some(V3::new(-ang, 0.0, 0.0)),
                ..Default::default()
            };
            s.b.box_(
                sx * (w / 2.0 + 0.4),
                (y * 2.0 + rise) / 2.0 + 0.6,
                (z0 + z1) / 2.0,
                0.8,
                1.2,
                len + 0.4,
                pal::PINK,
                o,
            );
        }
        for k in 0..5 {
            let bz = z0 + 4.0 + k as f64 * ((len - 7.0) / 4.0);
            let bx = (if k % 2 == 1 { 1.0 } else { -1.0 }) * (1.5 + s.rng() * 2.5);
            s.b.bumper(bx, y_at(bz) - 0.1, bz, 0.9, 11.0);
        }
        for (f, side) in [(0.3, -1.0), (0.62, 1.0)] {
            let gz = z0 + len * f;
            let w = 1.0 + s.rng() * 0.4;
            let ph = s.rng() * 6.0;
            glove_puncher(
                s.b,
                GloveOpts {
                    x: side * 7.6,
                    y: y_at(gz) + 0.95,
                    z: gz,
                    side,
                    w,
                    ph,
                    reach: 5.2,
                    scale: 1.2,
                    post_to: Some(y - 2.0),
                },
            );
        }
        s.b.box_(0.0, y + rise - 1.0, z1 + 2.0, 14.0, 2.0, 4.0, pal::PURPLE, d());
        SegOut {
            z: z1 + 4.0,
            y: y + rise,
            routes: vec![vec![
                Waypoint::spread(0.0, z0 + len * 0.45, 3.0),
                Waypoint::spread(0.0, z1 + 2.0, 2.0),
            ]],
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z1 && p.x.abs() > w / 2.0 + 0.05)),
            ..Default::default()
        }
    })
}

/// Tilting seesaw platforms in a zig-zag: jump across as they level out.
pub fn seesaws(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let mut zz = s.z + 3.0;
        let mut route = vec![Waypoint::spread(0.0, s.z + 1.0, 1.0)];
        s.b.box_(0.0, y - 1.0, s.z + 1.5, 12.0, 2.0, 3.0, pal::PURPLE, d());
        let mut edge = s.z + 3.0;
        for i in 0..n {
            let x = (if i % 2 == 1 { 1.0 } else { -1.0 }) * 2.5;
            let c = zz + 2.0 + 3.75;
            let w1 = 1.0 + s.rng() * 0.5;
            let ph = s.rng() * 6.0;
            let p = if i % 2 == 1 { pal::PINK } else { pal::TEAL };
            let node = s.b.box_(x, y - 0.5, c, 7.5, 1.0, 7.5, p, dynamic()).node;
            s.b.mover(move |t, ctx| {
                let n = ctx.node(node);
                n.rot.z = m::sin(t * w1 + ph) * 0.36;
                n.rot.x = m::sin(t * 0.8 + ph) * 0.12;
            });
            route.push(Waypoint::spread(x, c, 0.3).jump_when(edge_jump(edge, 1.1)));
            edge = c + 3.75;
            zz = c + 3.75;
        }
        zz += 2.0;
        s.b.box_(0.0, y - 1.0, zz + 3.0, 12.0, 2.0, 6.0, pal::PURPLE, d());
        route.push(Waypoint::spread(0.0, zz + 3.0, 0.5).jump_when(edge_jump(edge, 1.1)));
        SegOut {
            z: zz + 6.0,
            y,
            routes: vec![route],
            checkpoint: Some((zz + 0.5, V3::new(0.0, y + 0.1, zz + 3.0))),
            ..Default::default()
        }
    })
}

/// A conveyor belt running back at you, with punching walls and bumpers.
pub fn conveyor(len: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let speed = 3.6 + s.rng() * 1.2;
        let cz = s.z + len / 2.0;
        let belt = PrimOpts {
            col: ColliderOpts {
                conveyor: Some(V3::new(0.0, 0.0, -speed)),
                ..Default::default()
            },
            surface: Some("rubber"),
            ..Default::default()
        };
        s.b.box_(0.0, y - 1.0, cz, 9.0, 2.0, len, pal::WHITE, belt);
        for sx in [-1.0, 1.0] {
            s.b.box_(sx * 4.9, y + 0.6, cz, 0.8, 1.2, len, pal::YELLOW, d());
        }
        let mut route = Vec::new();
        let punches = (len / 9.0).floor() as u32;
        for k in 0..punches {
            let pz = s.z + 5.0 + k as f64 * 8.5;
            let side = if k % 2 == 1 { 1.0 } else { -1.0 };
            let w = 1.4 + s.rng() * 0.6;
            let ph = s.rng() * 6.0;
            let px = move |t: f64| side * (5.6 - 3.2 * 0f64.at_least(m::sin(t * w + ph)));
            let o = PrimOpts {
                dynamic: true,
                col: ColliderOpts {
                    hit: 0.9,
                    tag: Some("pusher"),
                    sinks: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            let node = s.b.box_(0.0, y + 0.9, pz, 3.4, 1.8, 1.2, pal::ORANGE, o).node;
            s.b.mover(move |t, ctx| ctx.node(node).pos.x = px(t));
            s.b.bumper(-side * 2.9, y, pz + 4.0, 0.75, 10.0);
            let lane = -side * 1.9;
            route.push(Waypoint::spread(lane, pz - 2.0, 0.0));
            route.push(
                Waypoint::spread(lane, pz + 1.5, 0.0).wait(move |bot| px(bot.t + 0.4).abs() > 3.4 || side * lane < 0.0),
            );
        }
        s.b.bonus(0.0, y, s.z + len * 0.5);
        route.push(Waypoint::spread(0.0, s.z + len + 0.5, 0.5));
        let z0 = s.z;
        SegOut {
            z: s.z + len,
            y,
            routes: vec![route],
            forbidden: Some(Box::new(move |p| {
                p.z > z0 + 1.0 && p.z < z0 + len - 1.0 && p.x.abs() > 4.3 && p.y > y + 0.6
            })),
            ..Default::default()
        }
    })
}

/// A gap with a trampoline down in it: bounce up to the (raised) platform on the other side.
pub fn trampoline_gap() -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let rise = 2.0;
        let basin_y = y - 3.0;
        let gap = 9.0;
        s.b.box_(0.0, y - 1.0, s.z + 2.0, 12.0, 2.0, 4.0, pal::PURPLE, d());
        // A basin under the gap (with a rim), and the trampoline in it.
        s.b.box_(
            0.0,
            basin_y - 1.0,
            s.z + 4.0 + gap / 2.0,
            12.0,
            2.0,
            gap,
            pal::BLUE,
            d(),
        );
        for sx in [-1.0, 1.0] {
            s.b.box_(
                sx * 6.4,
                basin_y + 1.0,
                s.z + 4.0 + gap / 2.0,
                0.8,
                4.0,
                gap,
                pal::PINK,
                d(),
            );
        }
        let tz = s.z + 4.0 + gap * 0.42;
        for tx in [-2.8, 2.8] {
            s.b.trampoline(tx, basin_y, tz, 1.9, 19.5);
        }
        let far = s.z + 4.0 + gap;
        s.b.box_(0.0, y + rise - 2.5, far + 4.0, 12.0, 5.0, 8.0, pal::PURPLE, d());
        let routes = [-2.8, 2.8]
            .iter()
            .map(|&tx| {
                vec![
                    Waypoint::spread(tx, tz, 0.0),
                    // Missed the trampoline (down in the basin, past it): back to the nearest one.
                    Waypoint::spread(tx * 0.5, far + 2.5, 0.3).detour(move |bot| {
                        (bot.body.pos.y < basin_y + 1.0).then_some((if bot.body.pos.x < 0.0 { -2.8 } else { 2.8 }, tz))
                    }),
                    Waypoint::spread(0.0, far + 5.0, 1.0),
                ]
            })
            .collect();
        let z0 = s.z;
        SegOut {
            z: far + 8.0,
            y: y + rise,
            routes,
            checkpoint: Some((far + 0.5, V3::new(0.0, y + rise + 0.1, far + 4.0))),
            forbidden: Some(Box::new(move |p| {
                p.z > z0 + 4.0 && p.z < far && p.x.abs() > 6.0 && p.y > basin_y + 2.5
            })),
        }
    })
}

/// A fork: the long way round (a zig-zag between walls), or a leap over a gap to a portal that puts
/// you at the far end.
pub fn portal_fork() -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let len = 26.0;
        // Right: the long zig-zag.
        s.b.box_(4.5, y - 1.0, s.z + len / 2.0, 7.0, 2.0, len, pal::GREEN, d());
        for sx in [1.0, 8.0] {
            s.b.box_(sx, y + 1.2, s.z + len / 2.0, 0.8, 2.4, len, pal::PINK, d());
        }
        let mut zig = vec![Waypoint::spread(4.5, s.z + 1.0, 0.0)];
        for k in 0..4 {
            let wz = s.z + 4.0 + k as f64 * 5.5;
            let left = k % 2 == 0;
            s.b.box_(
                if left { 3.5 } else { 5.5 },
                y + 1.2,
                wz,
                4.2,
                2.4,
                0.8,
                pal::PURPLE,
                d(),
            );
            let gx = if left { 6.3 } else { 2.7 };
            zig.push(Waypoint::spread(gx, wz - 1.4, 0.0));
            zig.push(Waypoint::spread(gx, wz + 1.4, 0.0));
        }
        zig.push(Waypoint::spread(4.5, s.z + len + 1.0, 0.0));
        // Left: a short run, a gap, the portal.
        s.b.box_(-4.5, y - 1.0, s.z + 4.0, 6.0, 2.0, 8.0, pal::BLUE, d());
        let gap_end = s.z + 8.0 + 3.0 + s.rng() * 0.6;
        s.b.box_(-4.5, y - 1.0, gap_end + 2.0, 5.0, 2.0, 4.0, pal::BLUE, d());
        let pz = gap_end + 2.4;
        s.b.box_(0.0, y - 1.0, s.z + len + 3.0, 16.0, 2.0, 6.0, pal::PURPLE, d());
        s.b.portal(
            PortalEnd {
                x: -4.5,
                y,
                z: pz,
                yaw: m::PI,
            },
            PortalEnd {
                x: -4.5,
                y,
                z: s.z + len + 3.2,
                yaw: 0.0,
            },
            "#39e0d0",
            PortalOpts::default(),
        );
        s.b.bonus(-4.5, y, s.z + 5.0);
        let hop = vec![
            Waypoint::spread(-4.5, s.z + 5.0, 0.0),
            Waypoint::spread(-4.5, gap_end + 1.2, 0.0).jump_when(edge_jump(s.z + 8.0, 1.1)),
            Waypoint::spread(-4.5, pz + 0.5, 0.0),
            Waypoint::spread(0.0, s.z + len + 5.0, 1.0),
        ];
        zig.push(Waypoint::spread(0.0, s.z + len + 5.0, 1.0));
        let z0 = s.z;
        SegOut {
            z: s.z + len + 6.0,
            y,
            routes: vec![zig, hop],
            forbidden: Some(Box::new(move |p| {
                p.z > z0 && p.z < z0 + len && p.x > 0.0 && p.y > y + 2.0
            })),
            checkpoint: Some((s.z + len + 0.5, V3::new(2.0, y + 0.1, s.z + len + 4.0))),
        }
    })
}

/// A walkway with gloves punching across it from both sides, out of rhythm with each other.
pub fn glove_alley(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let len = n as f64 * 5.0 + 4.0;
        let w = 7.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w, 2.0, len, pal::TEAL, d());
        let mut route = vec![Waypoint::spread(0.0, s.z + 1.0, 0.0)];
        for k in 0..n {
            let gz = s.z + 3.0 + k as f64 * 5.0;
            let side = if k % 2 == 1 { 1.0 } else { -1.0 };
            let gw = 1.1 + s.rng() * 0.5;
            let ph = s.rng() * 6.0;
            let gx = glove_puncher(
                s.b,
                GloveOpts {
                    x: side * (w / 2.0 + 2.6),
                    y: y + 0.95,
                    z: gz,
                    side,
                    w: gw,
                    ph,
                    reach: w * 0.8,
                    scale: 1.3,
                    post_to: Some(y - 4.0),
                },
            );
            let rest = side * (w / 2.0 + 2.6);
            route.push(Waypoint::spread(0.0, gz - 2.2, 0.0));
            route.push(Waypoint::spread(0.0, gz + 1.8, 0.0).wait(move |bot| {
                [0.0, 0.25, 0.5]
                    .iter()
                    .all(|dt| (gx.x_at(bot.t + dt) - rest).abs() < 1.2)
            }));
        }
        route.push(Waypoint::spread(0.0, s.z + len + 0.5, 0.5));
        SegOut {
            z: s.z + len,
            y,
            routes: vec![route],
            ..Default::default()
        }
    })
}

/// A ramp (or flat run) through walls with a gap sliding from side to side.
pub fn sliding_gates(n: u32, rise: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let len = n as f64 * 8.0 + 4.0;
        let w = 16.0;
        let z0 = s.z;
        let z1 = s.z + len;
        let y_at = move |zz: f64| y + ((zz - z0) / len) * rise;
        s.b.ramp(0.0, z0, y, z1, y + rise, w, pal::TEAL, 1.0, d());
        let ang = m::atan2(rise, len);
        for sx in [-1.0, 1.0] {
            let o = PrimOpts {
                rot: Some(V3::new(-ang, 0.0, 0.0)),
                ..Default::default()
            };
            let l = m::hypot(len, rise);
            s.b.box_(
                sx * (w / 2.0 + 0.4),
                (2.0 * y + rise) / 2.0 + 0.6,
                (z0 + z1) / 2.0,
                0.8,
                1.2,
                l,
                pal::PINK,
                o,
            );
        }
        let mut route = Vec::new();
        for k in 0..n {
            let gz = z0 + 5.0 + k as f64 * 8.0;
            let gw = 0.8 + s.rng() * 0.6;
            let ph = s.rng() * 6.0;
            let gap = 3.6 - k as f64 * 0.15;
            let gx = move |t: f64| m::sin(t * gw + ph) * (w / 2.0 - gap / 2.0 - 0.4);
            let gate = s.b.anchor(0.0, y_at(gz), gz, ROOT);
            for side in [-1.0, 1.0] {
                let o = PrimOpts {
                    parent: Some(gate),
                    dynamic: true,
                    col: ColliderOpts {
                        tag: Some("gate"),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let p = if k % 2 == 1 { pal::ORANGE } else { pal::PURPLE };
                s.b.box_(side * (gap / 2.0 + w / 2.0), 1.4, 0.0, w, 3.4, 0.8, p, o);
            }
            s.b.mover(move |t, ctx| ctx.node(gate).pos.x = gx(t));
            route.push(Waypoint::spread(0.0, gz - 3.0, 0.5));
            route.push(Waypoint::moving(gx, gz + 1.2).wait(move |bot| (gx(bot.t + 0.45) - bot.body.pos.x).abs() < 1.2));
            route.push(Waypoint::moving(gx, gz + 2.5));
        }
        s.b.box_(0.0, y + rise - 1.0, z1 + 3.0, 16.0, 2.0, 6.0, pal::PURPLE, d());
        route.push(Waypoint::spread(0.0, z1 + 3.0, 1.0));
        SegOut {
            z: z1 + 6.0,
            y: y + rise,
            routes: vec![route],
            forbidden: Some(Box::new(move |p| {
                p.z > z0 && p.z < z1 && (p.x.abs() > w / 2.0 + 0.05 || p.y > y_at(p.z) + 2.8)
            })),
            checkpoint: Some((z1 + 0.5, V3::new(0.0, y + rise + 0.1, z1 + 3.0))),
        }
    })
}

/// A narrow bridge of flaps that tip over now and then (dropping whoever is on them).
pub fn tipping_bridge(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let flap = 3.0;
        s.b.box_(0.0, y - 1.0, s.z + 1.0, 6.0, 2.0, 2.0, pal::PURPLE, d());
        let mut route = vec![Waypoint::spread(0.0, s.z + 1.0, 0.0)];
        let mut zz = s.z + 2.0;
        for k in 0..n {
            let c = zz + flap / 2.0 + 0.15;
            let period = 3.0 + s.rng() * 2.0;
            let ph = s.rng() * period;
            // Tipped for 0.9 s of every period.
            let tip = move |t: f64| {
                if t <= 0.0 {
                    return 0.0;
                }
                let f = (((t + ph) % period) + period) % period;
                if f < 0.9 { m::sin((f / 0.9) * m::PI) } else { 0.0 }
            };
            let pivot = s.b.anchor(0.0, y - 0.25, c, ROOT);
            let o = PrimOpts {
                parent: Some(pivot),
                dynamic: true,
                ..Default::default()
            };
            let p = if k % 2 == 1 { pal::ORANGE } else { pal::YELLOW };
            s.b.box_(0.0, 0.0, 0.0, 3.4, 0.5, flap, p, o);
            let dir = if s.rng() < 0.5 { -1.0 } else { 1.0 };
            s.b.mover(move |t, ctx| ctx.node(pivot).rot.z = dir * tip(t) * 1.35);
            route.push(
                Waypoint::spread(0.0, c + flap / 2.0 - 0.3, 0.0)
                    .wait(move |bot| [0.1, 0.4, 0.7].iter().all(|dt| tip(bot.t + dt) < 0.05)),
            );
            zz += flap + 0.3;
        }
        s.b.box_(0.0, y - 1.0, zz + 2.0, 12.0, 2.0, 4.0, pal::PURPLE, d());
        route.push(Waypoint::spread(0.0, zz + 2.0, 0.5));
        SegOut {
            z: zz + 4.0,
            y,
            routes: vec![route],
            checkpoint: Some((zz + 0.5, V3::new(0.0, y + 0.1, zz + 2.0))),
            ..Default::default()
        }
    })
}

/// Blocks shooting out of both side walls across the floor, row after row: dash between them.
pub fn pistons(rows: u32, w: f64) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let gap_z = 5.0;
        let len = rows as f64 * gap_z + 4.0;
        s.b.box_(0.0, y - 1.0, s.z + len / 2.0, w, 2.0, len, pal::BLUE, d());
        let mut route = vec![Waypoint::spread(0.0, s.z + 1.0, 0.5)];
        for k in 0..rows {
            let pz = s.z + 3.0 + k as f64 * gap_z;
            let period = 2.2 + s.rng() * 1.2;
            let ph = s.rng() * period;
            // Out for 40% of the period (a fast shove, a slower pull back).
            let out = move |t: f64| {
                if t <= 0.0 {
                    return 0.0;
                }
                let f = ((((t + ph) % period) + period) % period) / period;
                if f < 0.1 {
                    f / 0.1
                } else if f < 0.4 {
                    1.0
                } else if f < 0.55 {
                    1.0 - (f - 0.4) / 0.15
                } else {
                    0.0
                }
            };
            for side in [-1.0, 1.0] {
                let deco = PrimOpts {
                    no_collide: true,
                    ..Default::default()
                };
                s.b.box_(side * (w / 2.0 + 1.5), y + 1.0, pz, 3.0, 2.4, 2.2, pal::PURPLE, deco);
                let o = PrimOpts {
                    dynamic: true,
                    col: ColliderOpts {
                        hit: 0.9,
                        tag: Some("pusher"),
                        sinks: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let node =
                    s.b.box_(
                        side * (w / 2.0 + 1.5),
                        y + 0.8,
                        pz,
                        w / 2.0 + 0.5,
                        1.6,
                        1.8,
                        pal::ORANGE,
                        o,
                    )
                    .node;
                s.b.mover(move |t, ctx| {
                    ctx.node(node).pos.x = side * (w / 2.0 + (w / 4.0 + 0.25) - out(t) * (w / 2.0 - 0.1));
                });
            }
            route.push(Waypoint::spread(0.0, pz - 2.0, 0.5));
            route.push(
                Waypoint::spread(0.0, pz + 1.6, 0.5)
                    .wait(move |bot| out(bot.t + 0.1) < 0.02 && out(bot.t + 0.45) < 0.02),
            );
        }
        s.b.bonus(w / 2.0 - 2.0, y, s.z + 3.0 + gap_z * 1.5);
        route.push(Waypoint::spread(0.0, s.z + len + 0.5, 0.5));
        SegOut {
            z: s.z + len,
            y,
            routes: vec![route],
            ..Default::default()
        }
    })
}
