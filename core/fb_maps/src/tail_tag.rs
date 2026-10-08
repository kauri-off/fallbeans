//! Half of the players have tails, and a tail slows you down a little: grab somebody's tail and keep
//! yours. Whoever falls gives their tail to whoever knocked them off, or to the nearest bean.
use std::collections::BTreeMap;

use fb_shared::NEVER;
use fb_shared::rgb;
use fb_shared::rng::shuffle;
use fb_sim::bots::{
    ArenaOpts, BOT_DT, BotBrain, BotInput, BotView, HumanOpts, Note, arena_brain, humanize, init_bot, nav_to, unstick,
};
use fb_sim::builder::{Builder, PortalEnd, PortalOpts, PrimOpts, PropOpts};
use fb_sim::looks::LookId;
use fb_sim::m::{self, MinMax};
use fb_sim::map::{Cx, DecoChange, GameMeta, Genre, MapCtx, MapDef, MapEvent, MapLogic, MapSfx, MapSpec};
use fb_sim::math::{V3, dist_xz};
use fb_sim::nodes::ROOT;
use fb_sim::physics::Body;
use fb_sim::scene::Model;
use fb_sim::scene::Surface;
use fb_sim::scene::pal;

use crate::util::{deco, dynamic, freq, o};

pub struct TailTag;

static META: GameMeta = GameMeta {
    min_players: 2,
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
/// A hold lets go after this long (fb_arena).
const HOLD_MAX: f64 = 3.0;

struct Tails {
    /// Who has a tail, in the order they got it.
    tails: Vec<u32>,
    immune: BTreeMap<u32, f64>,
    /// Tails grabbed while immune (grabber, holder, since): the tail goes when the immunity ends, if the
    /// grabber still holds on.
    held: Vec<(u32, u32, f64)>,
    last_second: f64,
    participants: Vec<u32>,
    wander: BotBrain,
    /// Bots' notes: where a holder runs to, until when, and when it last turned away.
    flee_x: Note<f64>,
    flee_z: Note<f64>,
    flee_at: Note<f64>,
    fled_at: Note<f64>,
}

impl Tails {
    fn has(&self, id: u32) -> bool {
        self.tails.contains(&id)
    }

    /// The local player has a tail (client).
    fn mine(&self, me: Option<u32>) -> bool {
        me.is_some_and(|me| self.has(me))
    }

    /// The tail goes from `from` to `to`.
    fn pass(&mut self, cx: &mut Cx, from: u32, to: u32) {
        let mut next: Vec<u32> = self.tails.iter().copied().filter(|&id| id != from).collect();
        if !next.contains(&to) {
            next.push(to);
        }
        self.immune.insert(to, cx.t + IMMUNE);
        self.emit(cx, MapEvent::Tails { ids: next, by: to });
    }

    fn decorate_all(&self, cx: &mut Cx) {
        if cx.server {
            return;
        }
        for &id in &self.participants {
            cx.decorate(id, DecoChange::Tail(self.has(id)));
        }
    }
}

/// A grabber at `a` is close enough to take the tail of the bean at `v`.
fn in_reach(a: V3, v: V3) -> bool {
    dist_xz(a, v) <= STEAL_RANGE && (a.y - v.y).abs() <= 2.0
}

impl MapLogic for Tails {
    fn tick(&mut self, cx: &mut Cx, t: f64) {
        if t < 0.0 {
            return;
        }
        // A holder who left the room takes no tail with them: it goes to the bean without one with the fewest
        // points (else a round where every holder left has nobody scoring).
        if cx.server {
            let mut gone_ids = self.tails.clone();
            gone_ids.retain(|&id| cx.bodies.get(id).is_none());
            for gone in gone_ids {
                let to = cx
                    .bodies
                    .ids()
                    .into_iter()
                    .filter(|&oid| !self.has(oid))
                    .min_by_key(|&a| cx.score(a));
                if let Some(to) = to {
                    self.pass(cx, gone, to);
                }
            }
            // Tails held through their immunity change hands as it ends. A hold renews the grabber's slow-down
            // every tick (to 0.15 s ahead): one let go has it running out.
            let held = core::mem::take(&mut self.held);
            let mut keep = Vec::new();
            for (actor, target, since) in held {
                if t - since > HOLD_MAX || self.has(actor) || !self.has(target) {
                    continue;
                }
                let (Some(a), Some(v)) = (cx.bodies.get(actor), cx.bodies.get(target)) else {
                    continue;
                };
                if a.slow_until <= t + 0.1 || !in_reach(a.pos, v.pos) {
                    continue;
                }
                if self.immune.get(&target).copied().unwrap_or(NEVER) > t {
                    keep.push((actor, target, since));
                } else {
                    self.pass(cx, target, actor);
                }
            }
            self.held.extend(keep);
        }
        let s = t.floor();
        if s == self.last_second {
            return;
        }
        self.last_second = s;
        for &id in &self.tails {
            if cx.bodies.get(id).is_some() {
                let v = cx.score(id) + 1;
                cx.set_score(id, v);
            }
        }
    }

    /// Tails weigh you down a little: the chasers can catch up.
    fn bean(&self, id: u32, body: &mut Body, t: f64) {
        if t < 0.0 || !self.has(id) {
            return;
        }
        body.slow_k = if t < body.slow_until {
            body.slow_k.at_most(TAIL_SLOW)
        } else {
            TAIL_SLOW
        };
        body.slow_until = body.slow_until.at_least(t + 0.25);
    }

    fn grab(&mut self, cx: &mut Cx, actor: u32, target: u32) {
        let t = cx.t;
        if t < 0.0 || self.has(actor) || !self.has(target) {
            return;
        }
        let (Some(a), Some(v)) = (cx.bodies.get(actor), cx.bodies.get(target)) else {
            return;
        };
        if !in_reach(a.pos, v.pos) {
            return;
        }
        if self.immune.get(&target).copied().unwrap_or(NEVER) > t {
            // Just got it: the grabber has to hold on until the immunity is over (the tick).
            self.held.retain(|h| h.0 != actor);
            self.held.push((actor, target, t));
            return;
        }
        self.pass(cx, target, actor);
    }

    fn fall(&mut self, cx: &mut Cx, id: u32, by: Option<u32>) {
        if !self.has(id) {
            return;
        }
        let from = cx.bodies.get(id).map(|b| b.pos);
        let mut to = by.filter(|&by| by != id && !self.has(by) && cx.bodies.get(by).is_some());
        if to.is_none()
            && let Some(from) = from
        {
            // Nobody knocked them off: the tail goes to the nearest bean without one.
            let mut nd = f64::INFINITY;
            for oid in cx.bodies.ids() {
                if oid == id || self.has(oid) {
                    continue;
                }
                let Some(o) = cx.bodies.get(oid) else { continue };
                let d = dist_xz(o.pos, from);
                if d < nd {
                    nd = d;
                    to = Some(oid);
                }
            }
        }
        if let Some(to) = to {
            self.pass(cx, id, to);
        }
    }

    fn event(&mut self, cx: &mut Cx, ev: &MapEvent) {
        let MapEvent::Tails { ids, by } = ev else { return };
        let before = self.mine(cx.me);
        let mut tails: Vec<u32> = Vec::new();
        for &id in ids {
            if !tails.contains(&id) {
                tails.push(id);
            }
        }
        self.tails = tails;
        self.immune.insert(*by, cx.t + IMMUNE);
        // (The old tail holder's slow-down wears off by itself within a quarter of a second.)
        let after = self.mine(cx.me);
        self.decorate_all(cx);
        if before != after {
            cx.sfx(MapSfx::Steal);
        }
    }

    fn hud(&self, cx: &Cx) -> Option<String> {
        let pts = cx.me.map_or(0, |me| cx.score(me));
        Some(if self.mine(cx.me) {
            format!("У вас хвост — убегайте! Очки: {pts}")
        } else {
            format!("Отнимите чужой хвост (Q или ПКМ). Очки: {pts}")
        })
    }

    fn start(&mut self, cx: &mut Cx) {
        self.decorate_all(cx);
    }

    fn bots(&self) -> bool {
        true
    }

    fn bot(&self, bot: &mut BotView, out: &mut BotInput) {
        init_bot(bot);
        let p = bot.body.pos;
        let mine = self.has(bot.id);
        let mut near = None;
        let mut nd = 1e9;
        for o in bot.others {
            if self.has(o.id) == mine {
                continue;
            }
            let d = dist_xz(o.pos, p);
            if d < nd {
                nd = d;
                near = Some(*o);
            }
        }
        let Some(near) = near else {
            (self.wander)(bot, out);
            return;
        };
        if mine {
            // Run away (round the island and the bumpers), staying well inside the arena. A new way
            // out now and then, or when the chaser is close and the current one leads towards it.
            let fx = bot.mem.get(self.flee_x).unwrap_or(p.x) - p.x;
            let fz = bot.mem.get(self.flee_z).unwrap_or(p.z) - p.z;
            let towards = fx * (near.pos.x - p.x) + fz * (near.pos.z - p.z) > 0.0;
            let t = bot.t;
            if t > bot.mem.get(self.flee_at).unwrap_or(NEVER)
                || (nd < 3.0 && towards && t > bot.mem.get(self.fled_at).unwrap_or(NEVER) + 0.3)
            {
                bot.mem.set(self.fled_at, t);
                let mut best = f64::NEG_INFINITY;
                for _ in 0..8 {
                    let a = bot.rng.unit() * m::PI * 2.0;
                    let r = 3.0 + bot.rng.unit() * (ARENA_R - 6.0);
                    let x = m::cos(a) * r;
                    let z = m::sin(a) * r;
                    let mut score = m::hypot(x - near.pos.x, z - near.pos.z) - m::hypot(x - p.x, z - p.z) * 0.35;
                    if bot.nav.is_some_and(|n| !n.safe(x, z, p.y)) {
                        score -= 50.0;
                    }
                    if score > best {
                        best = score;
                        bot.mem.set(self.flee_x, x);
                        bot.mem.set(self.flee_z, z);
                    }
                }
                let flee = t + 0.8 + bot.rng.unit() * 0.8;
                bot.mem.set(self.flee_at, flee);
            }
            let (fx, fz) = (
                bot.mem.get(self.flee_x).unwrap_or(0.0),
                bot.mem.get(self.flee_z).unwrap_or(0.0),
            );
            nav_to(bot, fx, fz, out, if nd < 8.0 { 1.0 } else { 0.7 }, 0.8);
            // A chaser right behind: a hop or a dive to get away.
            if nd < 2.2 && bot.body.grounded && bot.rng.unit() < 0.25 {
                out.jump = true;
            }
        } else {
            // Chase, cutting the corner towards where the tail is going.
            let lead = 0.6f64.at_most(nd / 12.0);
            let tx = near.pos.x + near.vel.x * lead;
            let tz = near.pos.z + near.vel.z * lead;
            nav_to(bot, tx, tz, out, bot.mem.traits().spd, 1.0);
            out.grab = nd < 1.9;
            let ahead = m::hypot(tx - p.x, tz - p.z);
            if ahead < 4.5
                && ahead > 2.6
                && bot.body.grounded
                && bot.rng.unit() < (0.3 + bot.mem.traits().aggro) * BOT_DT * 2.0
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
    }
}

impl MapDef for TailTag {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Jungle, LookId::Meadow, LookId::Candy]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let flee_x: Note<f64> = b.note();
        let flee_z: Note<f64> = b.note();
        let flee_at: Note<f64> = b.note();
        let fled_at: Note<f64> = b.note();
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
                surface: Some(Surface::Grass),
                ..Default::default()
            };
            b.cyl(sx * 18.2, 3.2, 0.0, 3.0, 1.2, pal::GREEN, grass);
            let mush = PropOpts {
                scale: 0.8,
                ..Default::default()
            };
            b.prop(Model::Mushroom, sx * 19.0, 3.8, 1.2, mush);
        }
        let flip = if b.rng.unit() < 0.5 { 1.0 } else { -1.0 };
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
            rgb(0xff8a3d),
            PortalOpts::default(),
        );
        // Two platforms circling just outside the rim: hop on, ride round, hop off somewhere else.
        let orbit = 0.22 + b.rng.unit() * 0.08;
        for k in 0..2 {
            let p = if k == 1 { pal::ORANGE } else { pal::PINK };
            let node = b.box_(0.0, -0.5, 0.0, 3.2, 1.0, 3.2, p, dynamic()).node;
            let ph = k as f64 * m::PI + b.rng.unit();
            b.mover(move |t, ctx| {
                let a = ph + t.at_least(0.0) * orbit;
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
        let n = 1f64.at_least((len - 1.0).at_most((len / 2.0).ceil())) as usize;
        let tails = Tails {
            tails: order.into_iter().take(n).collect(),
            immune: BTreeMap::new(),
            held: Vec::new(),
            last_second: 0.0,
            participants: ctx.participants.to_vec(),
            wander: arena_brain(ArenaOpts {
                radius: ARENA_R - 4.0,
                ..Default::default()
            }),
            flee_x,
            flee_z,
            flee_at,
            fled_at,
        };
        MapSpec {
            spawns: b.ring_spawns(8, 9.0, 0.1, m::PI / 8.0),
            kill_y: -10.0,
            face_center: true,
            view: Some(V3::new(0.0, 1.0, 0.0)),
            logic: Box::new(tails),
            ..Default::default()
        }
    }
}
