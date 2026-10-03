//! Half of the players have tails, and a tail slows you down a little: grab somebody's tail and keep
//! yours. Whoever falls gives their tail to whoever knocked them off, or to the nearest bean.
use std::collections::BTreeMap;

use fb_shared::rng::shuffle;
use fb_sim::bots::{ArenaOpts, BOT_DT, HumanOpts, arena_brain, humanize, init_bot, nav_to, unstick};
use fb_sim::builder::{Builder, PortalEnd, PortalOpts, PrimOpts, PropOpts};
use fb_sim::m;
use fb_sim::map::{BeanDeco, Cx, GameMeta, Genre, MapCtx, MapDef, MapSpec, json};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::pal;
use fb_sim::world::St;

use crate::util::{deco, dynamic, freq, o};

pub struct TailTag;

static META: GameMeta = GameMeta {
    min_players: Some(2),
    grab: true,
    ..GameMeta::new(
        "tail-tag",
        "Хвостики",
        Genre::Points,
        "У половины игроков есть хвосты, и с хвостом бежится медленнее. Хватайте (Q / ПКМ) чужой хвост и не отдавайте свой: упавший отдаёт хвост тому, кто столкнул, или ближайшему. Батуты, портал и платформы помогут уйти.",
        "Держите хвост как можно дольше",
        75.0,
    )
};

const STEAL_RANGE: f64 = 2.4;
const IMMUNE: f64 = 1.5;
const ARENA_R: f64 = 15.0;
/// Tail holders run at this share of full speed (the chasers are a little quicker).
const TAIL_SLOW: f64 = 0.86;

struct Tails {
    /// Who has a tail, in the order they got it (as a JS Set).
    tails: Vec<u32>,
    immune: BTreeMap<u32, f64>,
    last_second: f64,
}

impl Tails {
    fn has(&self, id: i64) -> bool {
        self.tails.iter().any(|&t| i64::from(t) == id)
    }
}

/// The tail goes from `from` to `to`.
fn pass(cx: &mut Cx, st: St<Tails>, from: u32, to: u32) {
    let t = cx.t;
    let s = cx.world.st_mut(st);
    let mut next: Vec<u32> = s.tails.iter().copied().filter(|&id| id != from).collect();
    if !next.contains(&to) {
        next.push(to);
    }
    s.immune.insert(to, t + IMMUNE);
    cx.emit("tails", json!({ "ids": next, "by": to }));
}

fn decorate_all(cx: &mut Cx, st: St<Tails>, participants: &[u32]) {
    if cx.server {
        return;
    }
    for &id in participants {
        let tail = cx.world.st(st).has(i64::from(id));
        cx.decorate(
            id,
            BeanDeco {
                tail: Some(tail),
                badge: None,
            },
        );
    }
}

impl MapDef for TailTag {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["jungle", "meadow", "candy"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        b.cyl(0.0, -1.0, 0.0, ARENA_R, 2.0, pal::TEAL, freq(0.3));
        b.cyl(0.0, 0.03, 0.0, ARENA_R + 0.05, 0.1, pal::YELLOW, deco());
        // A raised island with ramps, and a few bumpers to dodge around.
        b.box_(0.0, 0.75, 0.0, 6.0, 1.5, 6.0, pal::PURPLE, o());
        for (x, z, ry) in [
            (0.0, 5.0, 0.0),
            (0.0, -5.0, m::PI),
            (5.0, 0.0, m::PI / 2.0),
            (-5.0, 0.0, -m::PI / 2.0),
        ] {
            let ramp = b.anchor(x, 0.0, z, ROOT);
            b.world.nodes.get_mut(ramp).rot.y = ry;
            // From the island's top edge (1.5 m) down to the floor 4 m further out.
            let opts = PrimOpts {
                parent: Some(ramp),
                rot: Some(V3::new(m::atan2(1.5, 4.0), 0.0, 0.0)),
                ..Default::default()
            };
            b.box_(0.0, 0.47, 0.0, 4.0, 0.6, 4.5, pal::PINK, opts);
        }
        // Between the spawn points, never on top of one: two small sweepers, trampolines up to floating
        // islands, a pair of portals.
        for sz in [-1.0, 1.0] {
            b.hub(0.0, 0.0, sz * 11.6, 0.6);
            let angle = move |t: f64| if t <= 0.0 { sz * 0.8 } else { sz * (0.8 + t * 1.1) };
            b.rotor(0.0, 0.6, sz * 11.6, 2.9, 2, angle, 0.7);
        }
        for sx in [-1.0, 1.0] {
            b.trampoline(sx * 12.3, 0.0, 0.0, 1.5, 18.0);
            // A floating island beyond the rim (a refuge, until someone bounces after you).
            let grass = PrimOpts {
                surface: Some("grass"),
                ..Default::default()
            };
            b.cyl(sx * 18.2, 3.2, 0.0, 3.0, 1.2, pal::GREEN, grass);
            let mush = PropOpts {
                scale: Some(0.8),
                ..Default::default()
            };
            b.prop("mushroom", sx * 19.0, 3.8, 1.2, mush);
        }
        let flip = if b.rng.next() < 0.5 { 1.0 } else { -1.0 };
        b.portal(
            PortalEnd {
                x: 9.9 * flip,
                y: 0.0,
                z: 9.9,
                yaw: m::atan2(-9.9 * flip, -9.9),
            },
            PortalEnd {
                x: -9.9 * flip,
                y: 0.0,
                z: -9.9,
                yaw: m::atan2(9.9 * flip, 9.9),
            },
            "#ff8a3d",
            PortalOpts::default(),
        );
        // Two platforms circling just outside the rim: hop on, ride round, hop off somewhere else.
        let orbit = 0.22 + b.rng.next() * 0.08;
        for k in 0..2 {
            let p = if k == 1 { pal::ORANGE } else { pal::PINK };
            let node = b.box_(0.0, -0.5, 0.0, 3.2, 1.0, 3.2, p, dynamic()).node;
            let ph = k as f64 * m::PI + b.rng.next();
            b.mover(move |t, ctx| {
                let a = ph + t.max(0.0) * orbit;
                let n = ctx.node(node);
                n.pos = V3::new(m::cos(a) * 17.6, -0.5, m::sin(a) * 17.6);
                n.rot.y = -a;
            });
        }
        for (x, z) in [(0.0, 0.0), (7.0, -4.0), (-7.0, 4.0)] {
            b.bonus(x, if x != 0.0 { 0.0 } else { 1.5 }, z);
        }
        b.clouds(0.0, 0.0, 45.0);

        // Initial tails follow from the seed and the participant order, identical everywhere.
        let mut order = ctx.participants.to_vec();
        shuffle(&mut order, &mut b.rng);
        let len = order.len() as f64;
        let n = 1f64.max((len - 1.0).min((len / 2.0).ceil())) as usize;
        let st = b.state(Tails {
            tails: order.into_iter().take(n).collect(),
            immune: BTreeMap::new(),
            last_second: 0.0,
        });
        let arena_wander = arena_brain(ArenaOpts::new(ARENA_R - 4.0));
        let participants = ctx.participants.to_vec();
        let participants2 = participants.clone();

        MapSpec {
            spawns: b.ring_spawns(8, 9.0, 0.1, m::PI / 8.0),
            kill_y: -10.0,
            face_center: true,
            view: Some(V3::new(0.0, 1.0, 0.0)),
            tick: Some(Box::new(move |cx, t| {
                if t < 0.0 {
                    return;
                }
                // Tails weigh you down a little: the chasers can catch up.
                let tails = cx.world.st(st).tails.clone();
                for &id in &tails {
                    let Some(body) = cx.bodies.get_mut(id) else { continue };
                    body.slow_k = if t < body.slow_until {
                        body.slow_k.min(TAIL_SLOW)
                    } else {
                        TAIL_SLOW
                    };
                    body.slow_until = body.slow_until.max(t + 0.25);
                }
                let s = t.floor();
                if s == cx.world.st(st).last_second {
                    return;
                }
                cx.world.st_mut(st).last_second = s;
                for &id in &tails {
                    if cx.bodies.get(id).is_some() {
                        let v = cx.score(id) + 1.0;
                        cx.set_score(id, v);
                    }
                }
            })),
            on_grab: Some(Box::new(move |cx, actor, target| {
                let t = cx.t;
                let s = cx.world.st(st);
                if t < 0.0 || s.has(i64::from(actor)) || !s.has(i64::from(target)) {
                    return;
                }
                if s.immune.get(&target).copied().unwrap_or(-1.0) > t {
                    return;
                }
                let (Some(a), Some(v)) = (cx.bodies.get(actor), cx.bodies.get(target)) else {
                    return;
                };
                if m::hypot(a.pos.x - v.pos.x, a.pos.z - v.pos.z) > STEAL_RANGE || (a.pos.y - v.pos.y).abs() > 2.0 {
                    return;
                }
                pass(cx, st, target, actor);
            })),
            on_fall: Some(Box::new(move |cx, id, by| {
                let s = cx.world.st(st);
                if !s.has(i64::from(id)) {
                    return;
                }
                let from = cx.bodies.get(id).map(|b| b.pos);
                let mut to = by.filter(|&by| by != id && !s.has(i64::from(by)) && cx.bodies.get(by).is_some());
                if to.is_none()
                    && let Some(from) = from
                {
                    // Nobody knocked them off: the tail goes to the nearest bean without one.
                    let mut nd = f64::INFINITY;
                    for oid in cx.bodies.ids() {
                        if oid == id || s.has(i64::from(oid)) {
                            continue;
                        }
                        let Some(o) = cx.bodies.get(oid) else { continue };
                        let d = m::hypot(o.pos.x - from.x, o.pos.z - from.z);
                        if d < nd {
                            nd = d;
                            to = Some(oid);
                        }
                    }
                }
                if let Some(to) = to {
                    pass(cx, st, id, to);
                }
            })),
            on_event: Some(Box::new(move |cx, name, data| {
                if name != "tails" {
                    return;
                }
                let Some(ids) = data["ids"].as_array().filter(|a| a.len() <= 16) else {
                    return;
                };
                let Some(ids) = ids
                    .iter()
                    .map(|v| v.as_i64().map(|i| i as u32))
                    .collect::<Option<Vec<_>>>()
                else {
                    return;
                };
                let me = cx.me();
                let t = cx.t;
                let s = cx.world.st_mut(st);
                let before = s.has(me);
                let mut tails: Vec<u32> = Vec::new();
                for id in ids {
                    if !tails.contains(&id) {
                        tails.push(id);
                    }
                }
                s.tails = tails;
                if let Some(by) = data["by"].as_i64() {
                    s.immune.insert(by as u32, t + IMMUNE);
                }
                // (The old tail holder's slow-down wears off by itself within a quarter of a second.)
                let after = s.has(me);
                decorate_all(cx, st, &participants);
                if before != after {
                    cx.sfx("steal");
                }
            })),
            hud: Some(Box::new(move |cx| {
                let me = cx.me();
                let pts = cx.score(me as u32);
                Some(if cx.world.st(st).has(me) {
                    format!("У вас хвост — убегайте! Очки: {pts}")
                } else {
                    format!("Отнимите чужой хвост (Q или ПКМ). Очки: {pts}")
                })
            })),
            bot: Some(Box::new(move |bot, out| {
                init_bot(bot);
                let p = bot.body.pos;
                let s = bot.world.st(st);
                let mine = s.has(i64::from(bot.id));
                let mut near = None;
                let mut nd = 1e9;
                for o in bot.others {
                    if s.has(i64::from(o.id)) == mine {
                        continue;
                    }
                    let d = m::hypot(o.pos.x - p.x, o.pos.z - p.z);
                    if d < nd {
                        nd = d;
                        near = Some(*o);
                    }
                }
                let Some(near) = near else {
                    arena_wander(bot, out);
                    return;
                };
                if mine {
                    // Run away (round the island and the bumpers), staying well inside the arena. A new way
                    // out now and then, or when the chaser is close and the current one leads towards it.
                    let fx = bot.mem.get("fx").unwrap_or(p.x) - p.x;
                    let fz = bot.mem.get("fz").unwrap_or(p.z) - p.z;
                    let towards = fx * (near.pos.x - p.x) + fz * (near.pos.z - p.z) > 0.0;
                    let t = bot.t;
                    if t > bot.mem.get("fleeAt").unwrap_or(-1.0)
                        || (nd < 3.0 && towards && t > bot.mem.get("fledAt").unwrap_or(-1.0) + 0.3)
                    {
                        bot.mem.set("fledAt", t);
                        let mut best = f64::NEG_INFINITY;
                        for _ in 0..8 {
                            let a = bot.rng.next() * m::PI * 2.0;
                            let r = 3.0 + bot.rng.next() * (ARENA_R - 6.0);
                            let x = m::cos(a) * r;
                            let z = m::sin(a) * r;
                            let mut score =
                                m::hypot(x - near.pos.x, z - near.pos.z) - m::hypot(x - p.x, z - p.z) * 0.35;
                            if bot.nav.is_some_and(|n| !n.safe(x, z, p.y)) {
                                score -= 50.0;
                            }
                            if score > best {
                                best = score;
                                bot.mem.set("fx", x);
                                bot.mem.set("fz", z);
                            }
                        }
                        let flee = t + 0.8 + bot.rng.next() * 0.8;
                        bot.mem.set("fleeAt", flee);
                    }
                    let (fx, fz) = (bot.mem.get("fx").unwrap_or(0.0), bot.mem.get("fz").unwrap_or(0.0));
                    nav_to(bot, fx, fz, out, if nd < 8.0 { 1.0 } else { 0.7 }, 0.8);
                    // A chaser right behind: a hop or a dive to get away.
                    if nd < 2.2 && bot.body.grounded && bot.rng.next() < 0.25 {
                        out.jump = true;
                    }
                } else {
                    // Chase, cutting the corner towards where the tail is going.
                    let lead = 0.6f64.min(nd / 12.0);
                    let tx = near.pos.x + near.vel.x * lead;
                    let tz = near.pos.z + near.vel.z * lead;
                    nav_to(bot, tx, tz, out, bot.mem.spd.unwrap_or(1.0), 1.0);
                    out.grab = nd < 1.9;
                    let ahead = m::hypot(tx - p.x, tz - p.z);
                    if ahead < 4.5
                        && ahead > 2.6
                        && bot.body.grounded
                        && bot.rng.next() < (0.3 + bot.mem.aggro.unwrap_or(0.3)) * BOT_DT * 2.0
                    {
                        out.dive = true;
                    }
                }
                let opts = HumanOpts {
                    rough: false,
                    fun: nd > 6.0,
                    avoid: mine,
                    ..Default::default()
                };
                humanize(bot, out, &opts);
                unstick(bot, out);
            })),
            on_start: Some(Box::new(move |cx| decorate_all(cx, st, &participants2))),
            ..Default::default()
        }
    }
}
