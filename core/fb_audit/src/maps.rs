//! Audits of one map: meta and spec rules, spawns and respawns, clipping, navigation, determinism,
//! budgets and bot balance (port of `audit/maps.ts`).
use std::collections::{BTreeMap, BTreeSet};

use fb_arena::{Arena, ArenaKind, PawnStatus, Stepper, tick_bodies, touch_hook};
use fb_shared::game::Genre;
use fb_shared::m::MinMaxJs;
use fb_shared::{DT, MAX_PLAYERS, m};
use fb_sim::collider::{ColId, Collider, Contact, Shape};
use fb_sim::map::MapDef;
use fb_sim::math::{V3, len};
use fb_sim::nav::{Nav, NavGrid, PathOpts};
use fb_sim::nodes::{NodeId, ROOT};
use fb_sim::physics::{Body, BodyInput, R, SPHERES, StepEvents};
use rayon::prelude::*;
use serde_json::json;

use crate::clock::Clock;
use crate::harness::{Harness, INTRO, Opts, median, quantile};
use crate::{Audit, Ctx, Out, Run, r1, r3};

/// Beans never get closer than this (physics BEAN_GAP); spawns must be further apart.
const MIN_SPAWN_GAP: f64 = 1.1;

pub const AUDITS: [Audit; 9] = [
    audit("meta", meta),
    audit("spec", spec),
    audit("spawn", spawn),
    audit("respawn", respawn),
    audit("clip", clip),
    audit("nav", nav),
    audit("determinism", determinism),
    Audit {
        name: "limits",
        run: Run::Map(limits),
        timed: true,
    },
    audit("balance", balance),
];

const fn audit(name: &'static str, f: fn(&'static dyn MapDef, &Ctx, &mut Out)) -> Audit {
    Audit {
        name,
        run: Run::Map(f),
        timed: false,
    }
}

/// A map built on its own (no beans), for geometry checks.
fn built(map: &'static dyn MapDef, seed: u32) -> Arena {
    Arena::new(map, ArenaKind::Round, seed, -INTRO, &[], false).0
}

/// Steps a lone bean as the arena does (map touch handlers included), from sim time t0 for `seconds`;
/// `each` after every tick, true to stop.
fn simulate(
    arena: &mut Arena,
    body: &mut Body,
    t0: f64,
    seconds: f64,
    input: BodyInput,
    mut each: impl FnMut(f64, &Body, &StepEvents) -> bool,
) {
    let Arena { world, spec, .. } = arena;
    let mut scores = BTreeMap::new();
    let mut out = Vec::new();
    let mut ev = StepEvents::default();
    let n = m::round_js(seconds / DT) as i64;
    for i in 1..=n {
        let t = t0 + i as f64 * DT;
        {
            let mut touch = touch_hook(&mut spec.touches, true, t, None, &mut scores, &mut out);
            let mut one = [Stepper {
                id: 99,
                body: &mut *body,
                ev: &mut ev,
                input,
            }];
            tick_bodies(world, t, &mut one, &[], &mut touch);
        }
        out.clear();
        if each(t, body, &ev) {
            return;
        }
    }
}

/// Deepest overlap of the bean's spheres with solid colliders, ignoring the ground it stands on.
fn overlap(body: &Body, arena: &Arena) -> (f64, Option<ColId>) {
    let world = &arena.world;
    let mut near = Vec::new();
    let mut c = Contact::default();
    let mut best = (0.0, None);
    world.query(body.pos.x, body.pos.z, 1.5, &mut near);
    for &i in &near {
        let col = world.col(i);
        if !col.enabled || col.trigger {
            continue;
        }
        for h in SPHERES {
            let center = V3::new(body.pos.x, body.pos.y + h, body.pos.z);
            if !col.contact(center, R, &mut c) {
                continue;
            }
            if c.normal.y > 0.7 && h == SPHERES[0] {
                continue;
            }
            if c.depth > best.0 {
                best = (c.depth, Some(i));
            }
        }
    }
    best
}

fn label(arena: &Arena, c: Option<ColId>) -> String {
    let Some(i) = c else { return "?".into() };
    let c = arena.world.col(i);
    let shape = match c.shape {
        Shape::Box { .. } => "box",
        Shape::Cyl { .. } => "cyl",
        Shape::Sphere { .. } => "sphere",
    };
    let tag = c.tag.map_or(String::new(), |t| format!(" ({t})"));
    format!("#{} {shape}{tag}{}", c.index, if c.is_static { "" } else { " moving" })
}

// ------------------------------------------------------------------ meta & spec rules

fn meta(map: &'static dyn MapDef, _: &Ctx, out: &mut Out) {
    let meta = map.meta();
    for p in meta.problems() {
        out.error(p);
    }
    if fb_maps::GAMES.iter().filter(|g| g.meta().title == meta.title).count() > 1 {
        out.error(format!("title \"{}\" is used twice", meta.title));
    }
    if fb_maps::MAPS.iter().filter(|g| g.meta().id == meta.id).count() != 1 {
        out.error(format!("id \"{}\" is not in fb_maps::MAPS exactly once", meta.id));
    }
    out.metric("duration", meta.duration);
    out.metric("genre", meta.genre.id());
}

fn spec(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let a = built(map, ctx.seed);
    let (spec, meta, world) = (&a.spec, map.meta(), &a.world);
    let sp = &spec.spawns;
    let forbidden = |p: V3| spec.forbidden.as_ref().is_some_and(|f| f(p));
    out.metric("spawns", sp.len());
    if sp.len() < MAX_PLAYERS {
        out.error(format!(
            "{} spawns for up to {MAX_PLAYERS} players (beans would share a spawn)",
            sp.len()
        ));
    }
    for (i, &p) in sp.iter().enumerate() {
        if !p.is_finite() {
            out.error(format!("spawn {i} is not a finite position"));
        }
        if forbidden(p) {
            out.error(format!("spawn {i} is inside a forbidden (shortcut) zone"))
                .at(p);
        }
        for (j, &q) in sp[..i].iter().enumerate() {
            let d = len(p - q);
            if d < MIN_SPAWN_GAP {
                out.error(format!("spawns {j} and {i} are {} m apart", r3(d))).at(p);
            }
        }
    }
    let lowest = sp.iter().map(|p| p.y).fold(f64::INFINITY, m::min);
    if spec.kill_y > lowest - 2.0 {
        out.error(format!(
            "killY {} is within 2 m of the lowest spawn (y {})",
            spec.kill_y,
            r3(lowest)
        ));
    }
    let cps = &spec.checkpoints;
    out.metric("checkpoints", cps.len());
    for (i, c) in cps.iter().enumerate() {
        if i > 0 && c.z < cps[i - 1].z {
            out.error(format!(
                "checkpoint {i} threshold ({}) is below checkpoint {} ({})",
                c.z,
                i - 1,
                cps[i - 1].z
            ));
        }
        if forbidden(c.p) {
            out.error(format!("checkpoint {i} respawn point is in a forbidden zone"))
                .at(c.p);
        }
        if c.p.y < spec.kill_y + 2.0 {
            out.error(format!("checkpoint {i} respawn point is below the kill height"))
                .at(c.p);
        }
    }
    if meta.genre == Genre::Race {
        match spec.finish {
            None => {
                out.error("a race without a finish");
            }
            Some(fin) => {
                let start = sp
                    .iter()
                    .map(|&p| spec.progress.as_ref().map_or(p.z, |f| f(p)))
                    .fold(f64::NEG_INFINITY, m::max);
                if fin.z <= start {
                    out.error("the finish is not ahead of the spawns");
                }
                out.metric("length", r1(fin.z - start));
                if cps.is_empty() {
                    out.warn("a race without checkpoints: every fall goes back to the start");
                }
            }
        }
    } else {
        if spec.finish.is_some() {
            out.warn(format!("a {} game with a finish line", meta.genre.id()));
        }
        if spec.view.is_none() {
            out.warn("arena without a view point: the camera looks at the origin");
        }
    }
    if spec.bot.is_none() {
        out.error("no bot brain: bots will stand still");
    }
    out.metric("colliders", world.colliders.len());
    out.metric("moving", world.dynamic.len());
    out.metric("movers", world.movers.len());
    out.metric("hash", world.hash(false));
}

// ------------------------------------------------------------------ spawns and respawns

struct Stand {
    start: (f64, Option<ColId>),
    hit: Option<&'static str>,
    hit_at: f64,
    fell: bool,
    drift: f64,
    grounded: bool,
    ground: Option<ColId>,
    dy: f64,
    end: V3,
}

/// Does a bean put at `p` at time t0 stand safely for `seconds`?
fn stand_test(arena: &mut Arena, p: V3, t0: f64, seconds: f64) -> Stand {
    let kill_y = arena.spec.kill_y;
    let mut b = Body::new(99);
    b.reset(p, 0.0);
    arena.world.goto(t0);
    let start = overlap(&b, arena);
    let mut hit = None;
    let mut hit_at = 0.0;
    let mut fell = false;
    simulate(arena, &mut b, t0, seconds, BodyInput::default(), |t, b, ev| {
        if hit.is_none() && (ev.hazard.is_some() || ev.knocked || ev.stunned) {
            hit = Some(ev.hazard.unwrap_or(if ev.knocked { "knocked" } else { "stunned" }));
            hit_at = t - t0;
        }
        if b.pos.y < kill_y {
            fell = true;
            return true;
        }
        false
    });
    Stand {
        start,
        hit,
        hit_at,
        fell,
        drift: m::hypot(b.pos.x - p.x, b.pos.z - p.z),
        grounded: b.grounded,
        ground: (b.ground_col >= 0).then_some(b.ground_col as ColId),
        dy: b.pos.y - p.y,
        end: b.pos,
    }
}

fn spawn(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let mut a = built(map, ctx.seed);
    let mut worst: f64 = 0.0;
    for (i, p) in a.spec.spawns.clone().into_iter().enumerate() {
        let r = stand_test(&mut a, p, 0.0, 2.0);
        worst = worst.max_js(r.start.0);
        if r.start.0 > 0.05 {
            let what = label(&a, r.start.1);
            out.error(format!("spawn {i} starts {} m inside {what}", r3(r.start.0)))
                .at(p);
        }
        let static_ground = r.ground.is_some_and(|g| a.world.col(g).is_static);
        if r.fell {
            out.error(format!("a bean on spawn {i} falls off without moving")).at(p);
        } else if r.hit.is_none() && !r.grounded {
            out.warn(format!("a bean on spawn {i} is not standing after the start"))
                .at(r.end);
        } else if r.hit.is_none() && static_ground && r.dy < -0.6 {
            // (On moving floors, a drum that rolls, the bean is carried away: nothing to measure.)
            out.warn(format!("spawn {i} is {} m above the ground", r3(-r.dy))).at(p);
        }
        // A hazard reaching a bean that stands still at the start: under a second leaves no time to react.
        if let Some(h) = r.hit {
            let msg = format!(
                "a bean standing on spawn {i} is hit ({h}) {} s after the start",
                r1(r.hit_at)
            );
            let f = if r.hit_at < 1.0 { out.warn(msg) } else { out.info(msg) };
            f.at(p).t(r3(r.hit_at));
        }
        if r.drift > 1.0 && !r.fell {
            out.info(format!("a bean on spawn {i} drifts {} m by itself", r1(r.drift)))
                .at(p);
        }
    }
    out.metric("worstStartOverlap", r3(worst));
}

fn respawn(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let mut a = built(map, ctx.seed);
    let times = if ctx.quick {
        vec![10.0]
    } else {
        vec![5.0, 20.0, map.meta().duration * 0.6]
    };
    let mut tests = 0;
    let cps: Vec<V3> = a.spec.checkpoints.iter().map(|c| c.p).collect();
    for (i, cp) in cps.into_iter().enumerate() {
        for dx in [-1.0, 0.0, 1.0] {
            for &t0 in &times {
                // Same place the arena respawns at (± the per-player jitter).
                let p = V3::new(cp.x + dx, cp.y + 0.5, cp.z);
                let r = stand_test(&mut a, p, t0, 1.5);
                tests += 1;
                let side = format!("x{}{dx}", if dx >= 0.0 { "+" } else { "" });
                if r.start.0 > 0.1 {
                    let what = label(&a, r.start.1);
                    out.error(format!("checkpoint {i} respawn ({side}) starts inside {what}"))
                        .at(p)
                        .t(t0);
                }
                if r.fell {
                    out.error(format!(
                        "a bean respawned at checkpoint {i} ({side}) falls off right away"
                    ))
                    .at(p)
                    .t(t0);
                } else if let Some(h) = r.hit {
                    out.warn(format!("a bean respawned at checkpoint {i} is hit ({h}) within 1.5 s"))
                        .at(p)
                        .t(t0);
                }
            }
        }
    }
    out.metric("tests", tests);
}

// ------------------------------------------------------------------ clipping (moving parts through others)

fn sample_points(c: &Collider) -> Vec<V3> {
    let mut pts = Vec::new();
    match c.shape {
        Shape::Box { hx, hy, hz } => {
            for x in [-1.0, 0.0, 1.0] {
                for y in [-1.0, 0.0, 1.0] {
                    for z in [-1.0, 0.0, 1.0] {
                        if x != 0.0 || y != 0.0 || z != 0.0 {
                            pts.push(V3::new(x * hx, y * hy, z * hz));
                        }
                    }
                }
            }
        }
        Shape::Cyl { r, hh } => {
            for i in 0..12 {
                let a = (i as f64 / 12.0) * m::PI * 2.0;
                for y in [-1.0, 0.0, 1.0] {
                    pts.push(V3::new(m::cos(a) * r, y * hh, m::sin(a) * r));
                }
            }
            pts.push(V3::new(0.0, hh, 0.0));
            pts.push(V3::new(0.0, -hh, 0.0));
        }
        Shape::Sphere { r } => {
            for d in [
                [1.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0],
            ] {
                pts.push(V3::new(d[0] * r, d[1] * r, d[2] * r));
            }
        }
    }
    pts
}

/// Parts of one prop (hammer head and arm, rotor and hub) share an ancestor below the map root.
fn related(arena: &Arena, a: &Collider, b: &Collider) -> bool {
    let nodes = &arena.world.nodes;
    let chain = |from: NodeId| {
        let mut up = BTreeSet::new();
        let mut n = Some(from);
        while let Some(id) = n.filter(|&id| id != ROOT) {
            up.insert(id);
            n = nodes.get(id).parent;
        }
        up
    };
    !chain(a.node).is_disjoint(&chain(b.node))
}

struct Clip {
    a: ColId,
    b: ColId,
    depth: f64,
    t: f64,
    at: V3,
    hits: u32,
    times: BTreeSet<usize>,
}

fn clip(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let mut a = built(map, ctx.seed);
    let meta = map.meta();
    let dynamic: Vec<ColId> = a
        .world
        .dynamic
        .iter()
        .copied()
        .filter(|&i| a.world.col(i).enabled)
        .collect();
    let local: Vec<Vec<V3>> = a.world.colliders.iter().map(sample_points).collect();
    let mut pairs: BTreeMap<(ColId, ColId), Clip> = BTreeMap::new();
    let step = if ctx.quick { 0.25 } else { 0.1 };
    let end = meta.duration.min_js(if ctx.quick { 20.0 } else { 60.0 });
    let samples = (end / step).floor() as usize + 1;
    let mut near = Vec::new();
    let mut c = Contact::default();
    let mut checks: u64 = 0;
    // (Time summed step by step, as TS does: the last sample depends on it.)
    let (mut s, mut t) = (0, 0.0);
    while t <= end {
        a.world.set_time(t);
        let world = &a.world;
        let mut test = |ia: ColId, ib: ColId| {
            let (ca, cb) = (world.col(ia), world.col(ib));
            // Points of a inside b.
            for &p in &local[ia as usize] {
                let w = ca.cur.apply_point(p);
                checks += 1;
                if !cb.contact(w, 0.001, &mut c) || c.depth < 0.06 {
                    continue;
                }
                let e = pairs.entry((ia.min(ib), ia.max(ib))).or_insert_with(|| Clip {
                    a: ia,
                    b: ib,
                    depth: 0.0,
                    t,
                    at: w,
                    hits: 0,
                    times: BTreeSet::new(),
                });
                e.hits += 1;
                e.times.insert(s);
                if c.depth > e.depth {
                    (e.depth, e.t, e.at) = (c.depth, t, w);
                }
            }
        };
        for &ia in &dynamic {
            let ca = world.col(ia);
            world.query(ca.center.x, ca.center.z, ca.radius, &mut near);
            for &ib in &near {
                let cb = world.col(ib);
                if ib == ia
                    || !cb.enabled
                    || ca.sinks
                    || cb.sinks
                    || ca.trigger
                    || cb.trigger
                    || related(&a, ca, cb)
                    || (!cb.is_static && ib < ia)
                {
                    continue;
                }
                test(ia, ib);
                test(ib, ia);
            }
        }
        s += 1;
        t += step;
    }
    // Overlapping nearly all the time: an axle or a hinge (a rotor arm in its hub), not clipping.
    let attached = pairs
        .values()
        .filter(|p| p.times.len() as f64 >= samples as f64 * 0.9)
        .count();
    let mut list: Vec<&Clip> = pairs
        .values()
        .filter(|p| (p.times.len() as f64) < samples as f64 * 0.9)
        .collect();
    list.sort_by(|x, y| y.depth.total_cmp(&x.depth));
    if attached > 0 {
        out.metric("attached", attached);
    }
    for p in list.iter().take(12) {
        let (la, lb) = (label(&a, Some(p.a)), label(&a, Some(p.b)));
        out.warn(format!("{la} and {lb} pass through each other by {} m", r3(p.depth)))
            .at(p.at)
            .t(r3(p.t))
            .data(json!({ "samples": p.hits, "share": r3(p.times.len() as f64 / samples as f64) }));
    }
    if list.len() > 12 {
        out.info(format!("{} more clipping pairs", list.len() - 12));
    }
    out.metric("pairs", list.len());
    out.metric("worstDepth", r3(list.first().map_or(0.0, |p| p.depth)));
    out.metric("checks", checks);
}

// ------------------------------------------------------------------ navigation

fn nav(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let mut a = built(map, ctx.seed);
    a.world.goto(0.0);
    let clock = Clock::start();
    let forbidden = a.spec.forbidden.as_deref().map(|f| f as &dyn Fn(V3) -> bool);
    let grid = NavGrid::build(&a.world, forbidden);
    out.metric("buildMs", r1(clock.ms()));
    let mut off_grid = Vec::new();
    for (i, p) in a.spec.spawns.clone().into_iter().enumerate() {
        // Moving floors (platforms, tiles that drop) are not in the grid by design.
        let r = stand_test(&mut a, p, 0.0, 0.3);
        if r.ground.is_some_and(|g| !a.world.col(g).is_static) {
            continue;
        }
        off_grid.push((i, p));
    }
    let nav = Nav::new(&grid, &a.world);
    for (i, p) in off_grid {
        if nav.ground_at(p.x, p.z, p.y, 1.2).is_none() {
            out.warn(format!("spawn {i} is not on walkable ground for bots")).at(p);
        }
    }
    for (i, c) in a.spec.checkpoints.iter().enumerate() {
        if c.z > -50.0 && nav.ground_at(c.p.x, c.p.z, c.p.y, 1.2).is_none() {
            out.warn(format!("checkpoint {i} respawn point is not on walkable ground"))
                .at(c.p);
        }
    }
    if map.meta().genre == Genre::Race
        && let Some(fin) = a.spec.finish
    {
        let path = nav.path(a.spec.spawns[0], 0.0, fin.z + 1.0, Some(fin.y), &PathOpts::default());
        out.metric("staticPathToFinish", path.is_some());
        if path.is_none() {
            out.info("no path to the finish over static ground alone (jumps or moving parts are needed)");
        }
    }
}

// ------------------------------------------------------------------ determinism

/// Two runs with the same seed build the same world and stay in the same state. (Wall clocks and
/// unseeded randomness are kept out of the core at compile time: `core/clippy.toml`.)
fn determinism(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let seconds = if ctx.quick { 8 } else { 25 };
    let runs: Vec<(String, Vec<String>)> = [0, 1]
        .par_iter()
        .map(|_| {
            let mut h = Harness::new(
                map,
                &Opts {
                    seed: ctx.seed,
                    ..Default::default()
                },
            );
            let world = h.arena.world.hash(false);
            let hashes = (1..=seconds)
                .map(|t| {
                    h.run_to(t as f64);
                    h.arena.state_hash()
                })
                .collect();
            (world, hashes)
        })
        .collect();
    if runs[0].0 != runs[1].0 {
        out.error("building the map twice with the same seed gives different geometry");
    }
    if let Some(i) = runs[0].1.iter().zip(&runs[1].1).position(|(x, y)| x != y) {
        out.error(format!("two runs with the same seed diverge after {} s", i + 1))
            .t((i + 1) as f64);
    }
    out.metric("seconds", seconds);
}

// ------------------------------------------------------------------ budgets

fn limits(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let a = built(map, ctx.seed);
    let n = a.world.colliders.len();
    if n > 2000 {
        out.warn(format!("{n} colliders (budget 2000)"));
    }
    if a.world.dynamic.len() > 200 {
        out.warn(format!("{} moving colliders (budget 200)", a.world.dynamic.len()));
    }
    // Simulation cost with 8 bots, measured after a warm-up (bot navigation grid) of 3 s.
    let mut h = Harness::new(
        map,
        &Opts {
            seed: ctx.seed,
            ..Default::default()
        },
    );
    h.run_to(3.0);
    let tick0 = h.arena.tick;
    let clock = Clock::start();
    h.run_to(3.0 + if ctx.quick { 4.0 } else { 12.0 });
    let per_tick = clock.ms() / (h.arena.tick - tick0).max(1) as f64;
    out.metric("msPerTick", r3(per_tick));
    // 120 ticks/s on one core shared with the other rooms: keep a round well under 10% of a core.
    if per_tick > 0.8 {
        out.warn(format!("{} ms per tick with 8 bots (budget 0.8 ms)", r3(per_tick)));
    }
}

// ------------------------------------------------------------------ bots: balance, stuck spots, cost

#[derive(Default)]
struct SeedRun {
    stuck: Vec<(u32, V3, f64)>,
    finish_times: Vec<f64>,
    out_times: Vec<f64>,
    survivors: usize,
    top_score: f64,
    /// Falls that were not eliminations: (progress, cause).
    falls: Vec<(f64, &'static str)>,
    bot_seconds: f64,
    sim_ms: f64,
    ticks: i64,
}

fn play_seed(map: &'static dyn MapDef, seed: u32) -> SeedRun {
    let mut h = Harness::new(
        map,
        &Opts {
            seed,
            ..Default::default()
        },
    );
    let mut r = SeedRun::default();
    let mut last: BTreeMap<u32, (V3, f64)> = BTreeMap::new();
    let end = map.meta().duration as i64;
    let tick0 = h.arena.tick;
    let clock = Clock::start();
    for t in 0..=end {
        let t = t as f64;
        h.run_to(t);
        // Stuck: in play, but has not moved half a metre in 12 s.
        for p in &h.arena.pawns {
            if p.status != PawnStatus::Play {
                continue;
            }
            match last.get(&p.id) {
                Some(&(at, since)) if len(at - p.body.pos) <= 0.5 => {
                    if t - since >= 12.0 {
                        r.stuck.push((p.id, p.body.pos, t));
                        last.insert(p.id, (p.body.pos, t));
                    }
                }
                _ => {
                    last.insert(p.id, (p.body.pos, t));
                }
            }
        }
        if h.alive() == 0 {
            break;
        }
    }
    r.sim_ms = clock.ms();
    r.ticks = h.arena.tick - tick0;
    r.bot_seconds = h.ids.len() as f64 * h.arena.time().max_js(1.0);
    r.finish_times = h.finishes.iter().map(|f| f.1).collect();
    r.out_times = h.falls.iter().filter(|f| f.out).map(|f| f.t).collect();
    r.survivors = h.alive();
    r.top_score = h.arena.scores.values().copied().fold(0.0, m::max);
    r.falls = h
        .falls
        .iter()
        .filter(|f| !f.out)
        .map(|f| (f.progress, f.cause))
        .collect();
    r
}

fn balance(map: &'static dyn MapDef, ctx: &Ctx, out: &mut Out) {
    let meta = map.meta();
    let seeds = if ctx.quick {
        vec![ctx.seed]
    } else {
        vec![ctx.seed, 23, 37, 51, 77]
    };
    let runs: Vec<SeedRun> = seeds.par_iter().map(|&s| play_seed(map, s)).collect();
    // In the order first seen: ties among the hottest keep it (as a JS Map).
    let mut spots: Vec<(String, u32)> = Vec::new();
    let mut falls = 0;
    let mut stuck = 0;
    for (r, seed) in runs.iter().zip(&seeds) {
        for &(id, p, t) in &r.stuck {
            stuck += 1;
            out.warn(format!("bot {id} (seed {seed}) has not moved for 12 s"))
                .at(p)
                .t(t);
        }
        for &(progress, cause) in &r.falls {
            falls += 1;
            let key = format!("z≈{} {cause}", m::round_js(progress / 5.0) * 5.0);
            match spots.iter_mut().find(|(k, _)| *k == key) {
                Some(s) => s.1 += 1,
                None => spots.push((key, 1)),
            }
        }
    }
    let n = runs.len() as f64;
    let beans = 8.0 * n;
    let finish_times: Vec<f64> = runs.iter().flat_map(|r| r.finish_times.iter().copied()).collect();
    let out_times: Vec<f64> = runs.iter().flat_map(|r| r.out_times.iter().copied()).collect();
    let first_outs: Vec<f64> = runs
        .iter()
        .filter(|r| !r.out_times.is_empty())
        .map(|r| r.out_times.iter().copied().fold(f64::INFINITY, m::min))
        .collect();
    let bot_seconds: f64 = runs.iter().map(|r| r.bot_seconds).sum();
    let sim_ms: f64 = runs.iter().map(|r| r.sim_ms).sum();
    let ticks: i64 = runs.iter().map(|r| r.ticks).sum();
    out.metric("seeds", runs.len());
    out.metric("fallsPerBotMin", r1(falls as f64 / bot_seconds * 60.0));
    out.metric("msPerTick8Bots", r3(sim_ms / ticks.max(1) as f64));
    if stuck > 0 {
        out.metric("stuck", stuck as u32);
    }
    let mut hot = spots;
    hot.sort_by_key(|s| std::cmp::Reverse(s.1));
    if !hot.is_empty() {
        let s: Vec<String> = hot.iter().take(3).map(|(k, v)| format!("{k} ×{v}")).collect();
        out.metric("fallHotspots", s.join("; "));
    }
    match meta.genre {
        Genre::Race => {
            let rate = finish_times.len() as f64 / beans;
            let p50 = median(&finish_times);
            out.metric("finishRate", r3(rate));
            out.metric("finishP50", r1(p50));
            out.metric("finishP90", r1(quantile(&finish_times, 0.9)));
            let pct = m::round_js(rate * 100.0);
            if rate < 0.3 {
                out.error(format!("only {pct}% of bots finish"));
            } else if rate < 0.6 {
                out.warn(format!("only {pct}% of bots finish"));
            }
            if p50 > meta.duration * 0.85 {
                out.warn(format!(
                    "median finish {} s is close to the {} s limit",
                    r1(p50),
                    meta.duration
                ));
            }
            if p50 < 20.0 {
                out.info(format!("median finish in {} s: a short course", r1(p50)));
            }
        }
        Genre::Survival => {
            let first = first_outs.iter().copied().fold(meta.duration, m::min);
            out.metric("firstOut", r1(first));
            out.metric("outP50", r1(median(&out_times)));
            out.metric(
                "survivorsAtEnd",
                r1(runs.iter().map(|r| r.survivors as f64).sum::<f64>() / n),
            );
            if first_outs.iter().any(|&t| t < 3.0) {
                out.error("a bot is eliminated in the first 3 seconds");
            }
            if out_times.is_empty() {
                out.warn("no bot is ever eliminated: the round always runs to the time limit");
            }
        }
        Genre::Points => {
            out.metric("topScoreAvg", r1(runs.iter().map(|r| r.top_score).sum::<f64>() / n));
            if runs.iter().all(|r| r.top_score <= 0.0) {
                out.warn("bots never score in this points game");
            }
        }
    }
}
