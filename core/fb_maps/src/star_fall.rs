//! Stars fall on the arena, one about every second (bigger ones on the tower and the islands now and
//! then): run through them to collect. Knocked off, you lose them all to whoever knocked you; falling by
//! yourself, they are gone. A grab snatches one. Bars sweep round the tower in the middle; trampolines at
//! the rim throw you up to the islands. Where and when stars fall follows from the seed; who took which
//! one comes from the server.
use std::collections::BTreeMap;

use fb_shared::NEVER;
use fb_shared::cause::Hazard;
use fb_shared::{PlayerId, rgb, rgba};
use fb_sim::bots::{
    ArenaOpts, BOT_DT, BotBrain, BotInput, BotView, HumanOpts, Note, aim_landing, arena_brain, humanize, init_bot,
    nav_to, steer, unstick,
};
use fb_sim::builder::{Builder, PrimOpts, PropOpts};
use fb_sim::collider::{ColliderOpts, Shape};
use fb_sim::looks::LookId;
use fb_sim::m::{self, MinMax};
use fb_sim::map::{Cx, DecoChange, GameMeta, Genre, Hook, MapCtx, MapDef, MapEvent, MapId, MapLogic, MapSfx, MapSpec};
use fb_sim::math::{V3, dist_xz};
use fb_sim::nodes::ROOT;
use fb_sim::physics::BodyState;
use fb_sim::props::{SpinUp, arm_contact_eta};
use fb_sim::scene::Model;
use fb_sim::scene::Surface;
use fb_sim::scene::{Finish, Form, LookOut, Part, Piece, pal};
use fb_sim::world::World;

use crate::util::{deco, freq};

pub struct StarFall;

static META: GameMeta = GameMeta {
    grab: true,
    ..GameMeta::new(
        MapId::StarFall,
        "Звездопад",
        Genre::Points,
        "Звёзды сыплются на арену: собирайте! На башне и островах — крупные. Сбили вас — все звёзды достаются обидчику, упали сами — сгорают. Захват (Q / ПКМ) выхватывает звезду.",
        "Соберите больше всех звёзд",
        100.0,
    )
};

const ARENA_R: f64 = 15.0;
const TOWER_R: f64 = 2.5;
const TOWER_H: f64 = 4.0;
const ISLAND_X: f64 = 20.5;
const ISLAND_Y: f64 = 3.5;
const ISLAND_R: f64 = 3.2;
const TRAMP_X: f64 = 12.5;
/// The sweeping bars turn round the tower, from here to there (m from the middle).
const SWEEP_IN: f64 = 2.9;
const SWEEP_OUT: f64 = 8.2;
/// A star lies this long (s) if nobody takes it; a new one falls about this often.
const LIFE: f64 = 15.0;
const EVERY: f64 = 1.1;
const SPECIAL_EVERY: f64 = 11.0;
const REACH: f64 = 1.2;
const SNATCH_RANGE: f64 = 2.4;
/// After a star was snatched from somebody, nobody can snatch from them for this long (s).
const IMMUNE: f64 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Where {
    Ground,
    Tower,
    Island,
}

#[derive(Clone, Copy, Debug)]
struct Spot {
    pos: V3,
    value: i64,
    at: Where,
}

#[derive(Clone, Copy, Debug)]
struct Star {
    k: usize,
    spot: usize,
    at: f64,
    until: f64,
    value: i64,
}

/// Where and when stars fall (from the seed, the same everywhere).
struct Sky {
    spots: Vec<Spot>,
    stars: Vec<Star>,
    by_spot: Vec<Vec<usize>>,
    tower_spot: usize,
}

/// Who took which star (from the server's events).
struct Taken {
    taken: BTreeMap<usize, (PlayerId, f64)>,
    /// Stars before this one are gone (server).
    first: usize,
    immune: BTreeMap<PlayerId, f64>,
    /// What happened to me lately (client): a line on the HUD for a few seconds.
    flash: Option<(String, f64)>,
}

impl Taken {
    fn live(&self, s: &Star, t: f64) -> bool {
        t >= s.at && t < s.until && !self.taken.contains_key(&s.k)
    }
}

impl Sky {
    fn star_at(&self, taken: &Taken, spot: usize, t: f64) -> bool {
        self.by_spot[spot].iter().any(|&i| taken.live(&self.stars[i], t))
    }
}

/// The bars sweeping round the tower.
#[derive(Clone, Copy)]
struct Sweep {
    dir: f64,
    spin: SpinUp,
}

impl Sweep {
    fn angle(self, t: f64) -> f64 {
        self.dir * self.spin.angle(t)
    }

    fn omega(self, t: f64) -> f64 {
        self.dir * self.spin.omega(t)
    }

    /// A bot in the bars' way jumps them as they come.
    fn jump(self, bot: &BotView) -> bool {
        let p = bot.body.pos;
        let r = m::hypot(p.x, p.z);
        if bot.t <= 0.0 || r < SWEEP_IN - 0.8 || r > SWEEP_OUT + 0.8 || p.y > 1.0 {
            return false;
        }
        let eta = arm_contact_eta(p, self.angle(bot.t), self.omega(bot.t), 2, 0.0, 0.0, 0.36);
        eta > 0.1 && eta < 0.15 + bot.mem.traits().react * 0.3
    }
}

/// How the bot gets to a spot, and how much that costs (m, roughly).
fn cost_to(p: V3, sp: &Spot) -> f64 {
    match sp.at {
        Where::Ground => dist_xz(sp.pos, p),
        Where::Tower => m::hypot(p.x, p.z.abs() - TOWER_R - 0.8) + 6.0,
        Where::Island => m::hypot(m::sign(sp.pos.x) * TRAMP_X - p.x, p.z) + 9.0,
    }
}

fn go_to(bot: &mut BotView, sp: &Spot, out: &mut BotInput) {
    let p = bot.body.pos;
    match sp.at {
        Where::Ground => {
            nav_to(bot, sp.pos.x, sp.pos.z, out, 1.0, 0.4);
        }
        Where::Tower => {
            let side = if p.z >= 0.0 { 1.0 } else { -1.0 };
            let fz = side * (TOWER_R + 0.9);
            if m::hypot(p.x, p.z - fz) < 0.8 {
                steer(bot, 0.0, 0.0, out, 1.0);
            } else {
                nav_to(bot, 0.0, fz + side * 0.5, out, 1.0, 0.3);
            }
        }
        Where::Island => {
            let tx = m::sign(sp.pos.x) * TRAMP_X;
            if m::hypot(p.x - tx, p.z) < 2.0 {
                steer(bot, tx, 0.0, out, 1.0);
            } else {
                nav_to(bot, tx, 0.0, out, 1.0, 0.5);
            }
        }
    }
}

/// The round: the stars, who took them, and the bots after them.
struct Stars {
    sky: Sky,
    taken: Taken,
    participants: Vec<PlayerId>,
    /// Client: the stars' special.
    look: Option<Hook>,
    sweep: Sweep,
    wander: BotBrain,
    /// Bots' notes: which island a trampoline threw them to, the star they are after and until when.
    fly: Note<f64>,
    star: Note<usize>,
    star_until: Note<f64>,
}

impl Stars {
    /// What happened to me lately (client), on the HUD for a few seconds.
    fn say(&mut self, cx: &Cx, text: String) {
        self.taken.flash = Some((text, cx.t + 3.0));
    }
}

impl MapLogic for Stars {
    fn tick(&mut self, cx: &mut Cx, t: f64) {
        if t < 0.0 {
            return;
        }
        let n = self.sky.stars.len();
        while self.taken.first < n && self.sky.stars[self.taken.first].until <= t {
            self.taken.first += 1;
        }
        let mut i = self.taken.first;
        while i < n {
            let s = self.sky.stars[i];
            i += 1;
            if s.at > t {
                break;
            }
            if !self.taken.live(&s, t) {
                continue;
            }
            let sp = self.sky.spots[s.spot];
            // (Two beans at a star on the same tick: the one that joined first gets it.)
            for id in cx.bodies.ids() {
                let Some(body) = cx.bodies.get(id) else { continue };
                if body.in_portal() {
                    continue;
                }
                let dy = body.pos.y - sp.pos.y;
                if !(-0.8..=2.0).contains(&dy) {
                    continue;
                }
                if dist_xz(body.pos, sp.pos) > REACH + 0.3 * body.size {
                    continue;
                }
                self.emit(
                    cx,
                    MapEvent::Star {
                        k: u32::try_from(s.k).expect("fewer than 2³² stars"),
                        id,
                    },
                );
                let v = cx.score(id) + s.value;
                cx.set_score(id, v);
                break;
            }
        }
    }

    fn fall(&mut self, cx: &mut Cx, id: PlayerId, by: Option<PlayerId>) {
        let n = cx.score(id);
        if n <= 0 {
            return;
        }
        let to = by.filter(|&by| by != id && cx.bodies.get(by).is_some());
        cx.set_score(id, 0);
        if let Some(to) = to {
            let v = cx.score(to) + n;
            cx.set_score(to, v);
        }
        self.emit(cx, MapEvent::Drop { from: id, to, n });
    }

    fn grab(&mut self, cx: &mut Cx, actor: PlayerId, target: PlayerId) {
        let t = cx.t;
        if t < 0.0 || cx.score(target) <= 0 || self.taken.immune.get(&target).copied().unwrap_or(NEVER) > t {
            return;
        }
        let (Some(a), Some(v)) = (cx.bodies.get(actor), cx.bodies.get(target)) else {
            return;
        };
        if dist_xz(a.pos, v.pos) > SNATCH_RANGE || (a.pos.y - v.pos.y).abs() > 2.0 {
            return;
        }
        self.taken.immune.insert(target, t + IMMUNE);
        let (vt, va) = (cx.score(target) - 1, cx.score(actor) + 1);
        cx.set_score(target, vt);
        cx.set_score(actor, va);
        self.emit(
            cx,
            MapEvent::Snatch {
                from: target,
                to: actor,
            },
        );
    }

    fn event(&mut self, cx: &mut Cx, ev: &MapEvent) {
        let t = cx.t;
        let me = cx.me;
        let is_me = |id: PlayerId| me == Some(id);
        match *ev {
            MapEvent::Star { k, id } => {
                let k = k as usize;
                if k >= self.sky.stars.len() || self.taken.taken.contains_key(&k) {
                    return;
                }
                self.taken.taken.insert(k, (id, t));
                if is_me(id) {
                    cx.sfx(MapSfx::Pickup);
                }
            }
            MapEvent::Drop { from, to, n } => {
                if is_me(from) {
                    cx.sfx(MapSfx::Steal);
                    let text = if to.is_some() {
                        format!("Вас сбили: {n} ⭐ у соперника!")
                    } else {
                        format!("Вы упали: {n} ⭐ сгорели!")
                    };
                    self.say(cx, text);
                } else if to.is_some_and(is_me) {
                    cx.sfx(MapSfx::Steal);
                    self.say(cx, format!("+{n} ⭐ со сбитого соперника!"));
                }
            }
            MapEvent::Snatch { from, to } => {
                self.taken.immune.insert(from, t + IMMUNE);
                if is_me(from) {
                    cx.sfx(MapSfx::Steal);
                    self.say(cx, "У вас выхватили звезду!".to_string());
                } else if is_me(to) {
                    cx.sfx(MapSfx::Pickup);
                    self.say(cx, "+1 ⭐ — вы выхватили звезду!".to_string());
                }
            }
            _ => {}
        }
    }

    fn hud(&self, cx: &Cx) -> Option<String> {
        if let Some((text, until)) = &self.taken.flash
            && cx.t < *until
        {
            return Some(text.clone());
        }
        let me = cx.me.unwrap_or(PlayerId(0));
        Some(format!("Ваши звёзды: {} · Q / ПКМ — выхватить звезду", cx.score(me)))
    }

    fn start(&mut self, cx: &mut Cx) {
        for &id in &self.participants {
            let n = cx.score(id);
            cx.decorate(id, DecoChange::Badge((n > 0).then(|| format!("⭐{n}"))));
        }
    }

    fn look(&self, hook: Hook, _: &World, t: f64, out: &mut LookOut) {
        if self.look != Some(hook) {
            return;
        }
        let taken = &self.taken.taken;
        let sky = &self.sky;
        for (i, sp) in sky.spots.iter().enumerate() {
            let got_at = |k: usize| taken.get(&k).map(|&(_, at)| at);
            let Some(s) = sky.by_spot[i]
                .iter()
                .map(|&k| &sky.stars[k])
                .find(|x| t >= x.at - 0.5 && t < x.until && got_at(x.k).unwrap_or(1e9) + 0.3 > t)
            else {
                continue;
            };
            let got = got_at(s.k);
            let fall = (s.at - t).at_least(0.0) / 0.5;
            let pop = got.map_or(0.0, |at| ((t - at) / 0.3).at_most(1.0));
            let fade = ((s.until - t) / 1.5).at_most(1.0);
            let size = if sp.value > 1 {
                1.0 + sp.value as f64 * 0.25
            } else {
                1.1
            };
            let fi = i as f64;
            let y = sp.pos.y + 0.35 + fall * fall * 9.0 + m::sin(t * 2.6 + fi) * 0.12;
            let star = Piece::at(0, sp.pos.x, y, sp.pos.z).rot(0.0, t * 2.0 + fi, 0.0);
            out.pieces
                .push(star.scale(size * (fade * (1.0 + pop * 0.8) * (1.0 - pop)).at_least(0.01)));
            if fall <= 0.0 && got.is_none() {
                let ring = Piece::at(if sp.value > 1 { 2 } else { 1 }, sp.pos.x, sp.pos.y + 0.05, sp.pos.z);
                let ring = ring.rot(-m::PI / 2.0, 0.0, 0.0);
                out.pieces.push(ring.scale(1.0 + m::sin(t * 3.0 + fi) * 0.08));
            }
        }
    }

    fn bots(&self) -> bool {
        true
    }

    fn bot(&self, bot: &mut BotView, out: &mut BotInput) {
        init_bot(bot);
        let sky = &self.sky;
        let p = bot.body.pos;
        let t = bot.t;
        let mine = bot.score(bot.id);
        let taken = &self.taken;
        // On a ladder: keep climbing.
        if bot.body.state == BodyState::Ladder {
            steer(bot, 0.0, 0.0, out, 1.0);
            return;
        }
        // Thrown up by a trampoline: land on the island.
        let fly = bot.mem.get(self.fly);
        if !bot.body.grounded
            && let Some(fly) = fly
            && bot.body.state == BodyState::Normal
        {
            aim_landing(bot, fly * ISLAND_X, ISLAND_Y, 0.0, out);
            return;
        }
        if bot.body.grounded {
            bot.mem.remove(self.fly);
        } else if p.x.abs() > TRAMP_X - 2.0 && p.z.abs() < 2.0 && bot.body.vel.y > 12.0 {
            let fly = m::sign(p.x);
            bot.mem.set(self.fly, fly);
            aim_landing(bot, fly * ISLAND_X, ISLAND_Y, 0.0, out);
            return;
        }
        // Up on an island or the tower: its star, then back down.
        let on_island = p.y > ISLAND_Y - 0.6 && p.x.abs() > ARENA_R;
        let on_tower = p.y > TOWER_H - 0.6 && m::hypot(p.x, p.z) < TOWER_R + 0.3;
        if on_island || on_tower {
            let spot = if on_tower {
                sky.tower_spot
            } else {
                sky.tower_spot + if p.x < 0.0 { 1 } else { 2 }
            };
            let sp = sky.spots[spot];
            if sky.star_at(taken, spot, t) {
                steer(bot, sp.pos.x, sp.pos.z, out, 1.0);
            } else if on_island {
                steer(bot, m::sign(p.x) * 11.0, 0.0, out, 1.0);
                if p.x.abs() < ISLAND_X - ISLAND_R + 1.3 && bot.body.grounded {
                    out.jump = true;
                }
            } else {
                let a = bot.mem.traits().ph;
                steer(bot, m::cos(a) * 6.0, m::sin(a) * 6.0, out, 1.0);
            }
            let opts = HumanOpts {
                precise: true,
                ..Default::default()
            };
            humanize(bot, out, &opts);
            return;
        }
        if !bot.body.grounded {
            let opts = HumanOpts {
                fun: false,
                ..Default::default()
            };
            humanize(bot, out, &opts);
            return;
        }
        // Somebody with a pile of stars close by: go and take it off them (the pushy ones).
        let aggro = bot.mem.traits().aggro;
        let mut hunt = None;
        if aggro > 0.45 && mine < 4 && t > 4.0 {
            let mut best = 2;
            for o in bot.others {
                let n = bot.score(o.id);
                let d = dist_xz(o.pos, p);
                if o.down || n <= best || d > 9.0 || (o.pos.y - p.y).abs() > 1.0 {
                    continue;
                }
                best = n;
                hunt = Some(*o);
            }
        }
        if let Some(h) = hunt {
            let d = dist_xz(h.pos, p);
            nav_to(bot, h.pos.x + h.vel.x * 0.3, h.pos.z + h.vel.z * 0.3, out, 1.0, 1.0);
            out.grab = d < 1.8;
            if d > 2.0 && d < 3.4 && bot.rng.unit() < (0.3 + aggro) * BOT_DT * 3.0 {
                out.dive = true;
            }
            if self.sweep.jump(bot) {
                out.jump = true;
            }
            let opts = HumanOpts {
                rough: false,
                fun: false,
                avoid: false,
                ..Default::default()
            };
            humanize(bot, out, &opts);
            unstick(bot, out);
            return;
        }
        // The best star for the effort (a careful bot with a pile keeps off the rim and the islands).
        let mut target = bot.mem.get(self.star).and_then(|k| sky.stars.get(k));
        if target.is_none_or(|s| !taken.live(s, t) || t > bot.mem.get(self.star_until).unwrap_or(0.0)) {
            target = None;
            let mut best = 0.0;
            for s in &sky.stars[taken.first..] {
                if s.at > t + 0.3 {
                    break;
                }
                if !taken.live(s, t.at_least(s.at)) {
                    continue;
                }
                let sp = &sky.spots[s.spot];
                if sp.at == Where::Island && (mine > 2 || bot.mem.traits().skill < 0.55) {
                    continue;
                }
                if mine >= 4 && m::hypot(sp.pos.x, sp.pos.z) > 11.5 && sp.at == Where::Ground {
                    continue;
                }
                let mut score = s.value as f64 / (cost_to(p, sp) + 3.0);
                // Someone else is closer to it: less worth going for.
                for o in bot.others {
                    if dist_xz(o.pos, sp.pos) < dist_xz(p, sp.pos) - 1.0 {
                        score *= 0.6;
                    }
                }
                score *= 0.85 + bot.rng.unit() * 0.3;
                if score > best {
                    best = score;
                    target = Some(s);
                }
            }
            match target {
                Some(s) => bot.mem.set(self.star, s.k),
                None => bot.mem.remove(self.star),
            }
            let until = t + 2.0 + bot.rng.unit() * 1.5;
            bot.mem.set(self.star_until, until);
        }
        let Some(target) = target else {
            (self.wander)(bot, out);
            return;
        };
        go_to(bot, &sky.spots[target.spot], out);
        if self.sweep.jump(bot) {
            out.jump = true;
        }
        let opts = HumanOpts {
            fun: false,
            rough: mine < 3,
            avoid: true,
            ..Default::default()
        };
        humanize(bot, out, &opts);
        unstick(bot, out);
    }
}

impl MapDef for StarFall {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Starlight, LookId::Neon, LookId::Circus]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let fly: Note<f64> = b.note();
        let star: Note<usize> = b.note();
        let star_until: Note<f64> = b.note();
        b.cyl(0.0, -1.0, 0.0, ARENA_R, 2.0, pal::PURPLE, freq(0.3));
        b.cyl(0.0, 0.03, 0.0, ARENA_R + 0.05, 0.1, pal::YELLOW, deco());
        let ring = PrimOpts {
            freq: Some(0.35),
            ..deco()
        };
        b.cyl(0.0, 0.06, 0.0, SWEEP_OUT + 0.3, 0.1, pal::BLUE, ring);
        // The tower, with a ladder on either side, and the bars sweeping round it.
        let rock = PrimOpts {
            surface: Some(Surface::Rock),
            ..Default::default()
        };
        b.cyl(0.0, TOWER_H / 2.0, 0.0, TOWER_R, TOWER_H, pal::ORANGE, rock);
        b.ladder(0.0, 0.0, TOWER_R, TOWER_H, 0.0, rgb(0xffb347));
        b.ladder(0.0, 0.0, -TOWER_R, TOWER_H, m::PI, rgb(0xffb347));
        let dir = if b.rng.unit() < 0.5 { 1.0 } else { -1.0 };
        let sweep = Sweep {
            dir,
            spin: SpinUp::new(m::PI / 2.0 + 0.3, 0.55 + b.rng.unit() * 0.1, 0.0012),
        };
        let sweeper = b.anchor(0.0, 0.6, 0.0, ROOT);
        for k in 0..2 {
            let pivot = b.anchor(0.0, 0.0, 0.0, sweeper);
            b.world.nodes.get_mut(pivot).rot.y = f64::from(k) * m::PI;
            let holder = b.anchor(SWEEP_IN, 0.0, 0.0, pivot);
            let arm = b.model(Model::Arm, holder);
            b.world.nodes.get_mut(arm).scale = V3::new(SWEEP_OUT - SWEEP_IN, 1.0, 1.0);
            let at = b.anchor((SWEEP_OUT - SWEEP_IN) / 2.0, 0.0, 0.0, holder);
            b.collider(
                at,
                Shape::Box {
                    hx: (SWEEP_OUT - SWEEP_IN) / 2.0,
                    hy: 0.36,
                    hz: 0.36,
                },
                ColliderOpts {
                    hit: 0.45,
                    tag: Some(Hazard::Rotor),
                    sweep: true,
                    ..Default::default()
                },
            );
        }
        b.mover(move |t, ctx| ctx.node(sweeper).rot.y = sweep.angle(t));
        // Trampolines up to the islands beyond the rim (the big stars fall there).
        for sx in [-1.0, 1.0] {
            b.trampoline(sx * TRAMP_X, 0.0, 0.0, 1.5, 18.0);
            let grass = PrimOpts {
                surface: Some(Surface::Grass),
                ..Default::default()
            };
            b.cyl(sx * ISLAND_X, ISLAND_Y - 0.6, 0.0, ISLAND_R, 1.2, pal::GREEN, grass);
            let mush = PropOpts {
                scale: 0.7,
                ..Default::default()
            };
            b.prop(Model::Mushroom, sx * (ISLAND_X + 1.6), ISLAND_Y, 1.8, mush);
        }
        let bumpers: Vec<(f64, f64)> = [0.5, 7.0 / 6.0, 11.0 / 6.0]
            .iter()
            .map(|a| (m::cos(a * m::PI) * 9.9, m::sin(a * m::PI) * 9.9))
            .collect();
        for &(x, z) in &bumpers {
            b.bumper(x, 0.0, z, 0.8, 8.0);
        }
        for k in 0..4 {
            let a = f64::from(k) * 1.57 + 0.4;
            b.bonus(m::cos(a) * 11.8, 0.0, m::sin(a) * 11.8);
        }
        b.clouds(0.0, 0.0, 50.0);

        // Where stars may fall.
        let mut spots: Vec<Spot> = Vec::new();
        for (r, n, off) in [(5.4, 8, 0.2), (10.0, 12, 0.13), (12.8, 14, 0.3)] {
            for k in 0..n {
                let a = off + (f64::from(k) / f64::from(n)) * m::PI * 2.0;
                let x = m::cos(a) * r;
                let z = m::sin(a) * r;
                if x.abs() > TRAMP_X - 2.2 && z.abs() < 2.4 {
                    continue;
                }
                if bumpers.iter().any(|p| m::hypot(p.0 - x, p.1 - z) < 2.6) {
                    continue;
                }
                spots.push(Spot {
                    pos: V3::new(x, 0.0, z),
                    value: 1,
                    at: Where::Ground,
                });
            }
        }
        let tower_spot = spots.len();
        spots.push(Spot {
            pos: V3::new(0.0, TOWER_H, 0.0),
            value: 2,
            at: Where::Tower,
        });
        for sx in [-1.0, 1.0] {
            spots.push(Spot {
                pos: V3::new(sx * ISLAND_X, ISLAND_Y, 0.0),
                value: 3,
                at: Where::Island,
            });
        }

        // When and where they fall: from the seed, identical on the server and every client.
        let mut stars: Vec<Star> = Vec::new();
        let mut busy = vec![-1.0; spots.len()];
        let put = |stars: &mut Vec<Star>, busy: &mut Vec<f64>, spot: usize, at: f64, life: f64| {
            stars.push(Star {
                k: 0,
                spot,
                at,
                until: at + life,
                value: spots[spot].value,
            });
            busy[spot] = at + life;
        };
        let ground: Vec<usize> = (0..spots.len()).filter(|&i| spots[i].at == Where::Ground).collect();
        let special: Vec<usize> = (0..spots.len()).filter(|&i| spots[i].at != Where::Ground).collect();
        let duration = META.duration;
        let mut t = 0.4;
        while t < duration - 1.0 {
            let free: Vec<usize> = ground.iter().copied().filter(|&i| busy[i] <= t).collect();
            if !free.is_empty() {
                let i = free[b.rng.index(free.len())];
                put(&mut stars, &mut busy, i, t, LIFE);
            }
            t += if t < 3.0 {
                0.35
            } else {
                EVERY * (0.75 + b.rng.unit() * 0.5)
            };
        }
        let mut t = 7.0 + b.rng.unit() * 3.0;
        while t < duration - 4.0 {
            let free: Vec<usize> = special.iter().copied().filter(|&i| busy[i] <= t).collect();
            if !free.is_empty() {
                let i = free[b.rng.index(free.len())];
                put(&mut stars, &mut busy, i, t, LIFE + 5.0);
            }
            t += SPECIAL_EVERY * (0.85 + b.rng.unit() * 0.3);
        }
        stars.sort_by(|a, c| a.at.total_cmp(&c.at));
        for (i, s) in stars.iter_mut().enumerate() {
            s.k = i;
        }
        let by_spot = (0..spots.len())
            .map(|i| (0..stars.len()).filter(|&k| stars[k].spot == i).collect())
            .collect();
        let sky = Sky {
            spots,
            stars,
            by_spot,
            tower_spot,
        };
        // Stars on the course (client): one per spot, falling in, bobbing, taken with a pop.
        let look = (!b.server()).then(|| {
            let parts = [
                Part::new(Form::Model(Model::Star), rgb(0xffd23f), Finish::Glossy),
                Part::new(Form::Ring(0.55, 0.8), rgba(0xffd23f8c), Finish::Flat),
                Part::new(Form::Ring(0.55, 0.8), rgba(0xff9f4a8c), Finish::Flat),
            ];
            b.special_hook(ROOT, "stars", &parts)
        });

        let mut opts = ArenaOpts {
            radius: 11.0,
            ..Default::default()
        };
        opts.safe = Some(Box::new(|x, z, _| {
            let r = m::hypot(x, z);
            r > TOWER_R + 1.0 && r < ARENA_R - 2.5
        }));
        opts.jump_when = Some(Box::new(move |bot| sweep.jump(bot)));
        MapSpec {
            spawns: b.ring_spawns(8, 11.5, 0.1, m::PI / 8.0),
            kill_y: -10.0,
            face_center: true,
            view: Some(V3::new(0.0, 2.0, 0.0)),
            logic: Box::new(Stars {
                sky,
                taken: Taken {
                    taken: BTreeMap::new(),
                    first: 0,
                    immune: BTreeMap::new(),
                    flash: None,
                },
                participants: ctx.participants.to_vec(),
                look,
                sweep,
                wander: arena_brain(opts),
                fly,
                star,
                star_until,
            }),
            ..Default::default()
        }
    }
}
