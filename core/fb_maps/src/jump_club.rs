//! Jump the low bar, stay under the high one. Direction, the number of bars and how fast they speed up
//! come from the seed.
use fb_sim::bots::{ArenaOpts, BOT_DT, BotInput, BotView, Note, arena_brain};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::m::{self, MinMax};
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::physics::BodyState;
use fb_sim::props::{SpinUp, arm_contact_eta};
use fb_sim::scene::pal;

pub struct JumpClub;

static META: GameMeta = GameMeta::new(
    "jump-club",
    "Прыг-клуб",
    Genre::Survival,
    "Перепрыгивайте нижнюю балку и не попадайтесь под верхнюю. Со временем обе крутятся всё быстрее!",
    "Не упадите",
    75.0,
);

impl MapDef for JumpClub {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["neon", "starlight", "circus"]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let seen_at: Note<f64> = b.note();
        let seen_eta: Note<f64> = b.note();
        let rng = &mut b.rng;
        let dir = if rng.next() < 0.5 { 1.0 } else { -1.0 };
        let low_arms = 2;
        let high_arms = if rng.next() < 0.35 { 1 } else { 2 };
        let acc = 0.009 + rng.next() * 0.005;
        // The low bar starts 10° past a spawn pair and eases in: the first bean it reaches has over a second.
        let low = SpinUp::new(-0.26, 1.1 + rng.next() * 0.15, acc);
        let hk = 0.006 + rng.next() * 0.004;
        let low_ang = move |t: f64| dir * low.angle(t);
        let low_omega = move |t: f64| dir * low.omega(t);
        let high_ang = move |t: f64| dir * (m::PI / 2.0 - if t <= 0.0 { 0.0 } else { 0.7 * t + hk * t * t });
        let high_omega = move |t: f64| -dir * (0.7 + 2.0 * hk * t);

        let freq = |f: f64| PrimOpts {
            freq: Some(f),
            ..Default::default()
        };
        let deco = |f: Option<f64>| PrimOpts {
            freq: f,
            no_collide: true,
            ..Default::default()
        };
        b.cyl(0.0, -1.0, 0.0, 13.0, 2.0, pal::BLUE, freq(0.35));
        b.cyl(0.0, 0.03, 0.0, 13.05, 0.1, pal::YELLOW, deco(None));
        b.cyl(0.0, 0.06, 0.0, 11.5, 0.1, pal::BLUE, deco(Some(0.35)));
        b.hub(0.0, 0.0, 0.0, 1.2);
        b.rotor(0.0, 0.6, 0.0, 12.6, low_arms, low_ang, 0.6);
        b.rotor(0.0, 2.45, 0.0, 12.6, high_arms, high_ang, 0.6);
        for a in [0.0, 1.0, 2.0, 3.0] {
            b.bonus(m::cos(a * 1.57 + 0.8) * 8.0, 0.0, m::sin(a * 1.57 + 0.8) * 8.0);
        }
        b.clouds(0.0, 0.0, 40.0);
        let spawns = [25.0, 65.0, 115.0, 155.0, 205.0, 245.0, 295.0, 335.0]
            .map(|d: f64| {
                let a = (d * m::PI) / 180.0;
                V3::new(m::cos(a) * 6.0, 0.1, m::sin(a) * 6.0)
            })
            .to_vec();
        // Seconds until the low and the high bar reach a bean at p.
        let etas = move |p: V3, t: f64| {
            let eta = arm_contact_eta(p, low_ang(t), low_omega(t), low_arms, 0.0, 0.0, 0.36);
            let high = arm_contact_eta(p, high_ang(t), high_omega(t), high_arms, 0.0, 0.0, 0.36);
            (eta, high)
        };
        let mut opts = ArenaOpts::new(8.0);
        opts.safe = Some(Box::new(|x, z, _| m::hypot(x, z) > 3.5));
        opts.jump_when = Some(Box::new(move |bot: &mut BotView| {
            let t = bot.t.at_least(0.0);
            if t <= 0.0 {
                return false;
            }
            let (eta, high) = etas(bot.body.pos, t);
            // How fast the bar really closes in, seen between two looks: a quick one would slip through
            // the reaction window, so jump now if it will be too late at the next.
            let mem = &mut *bot.mem;
            let (jc_t, jc_eta) = (mem.get(seen_at), mem.get(seen_eta));
            let seen = jc_t.is_some_and(|jt| t - jt < 0.2) && jc_eta.unwrap_or(0.0) > eta;
            let rate = if seen {
                0.2f64.at_least((jc_eta.unwrap_or(eta) - eta) / (t - jc_t.unwrap_or(t)))
            } else {
                1.0
            };
            mem.set(seen_eta, eta);
            mem.set(seen_at, t);
            let when = eta / rate;
            let next = when - BOT_DT;
            // Worse bots react late (and sometimes too late).
            let late = 0.15 + mem.traits.react * 0.3;
            when > 0.1 && (when < late || (next < 0.1 && when < 0.32)) && high > 0.7
        }));
        let brain = arena_brain(opts);
        // Both bars coming by at about the same time: run along the circle towards the one that comes
        // first (under the high one standing, over the low one).
        let dodge = move |bot: &mut BotView, out: &mut BotInput| {
            let t = bot.t.at_least(0.0);
            let p = bot.body.pos;
            let r = m::hypot(p.x, p.z);
            if t <= 0.0 || r < 2.0 || bot.body.state != BodyState::Normal {
                return;
            }
            let (eta, high) = etas(p, t);
            let low_first = eta < high;
            let clash = if low_first { high - eta < 0.8 } else { eta - high < 0.35 };
            // Better players spot it sooner.
            let sees = 0.3 + bot.mem.traits.skill * 0.9;
            if !clash || eta.at_most(high) > sees || eta.at_most(high) < 0.08 {
                return;
            }
            let w = if low_first { low_omega(t) } else { high_omega(t) };
            let s = -m::sign(w);
            out.mx = (s * p.z) / r;
            out.mz = (-s * p.x) / r;
        };
        MapSpec {
            spawns,
            kill_y: -6.0,
            face_center: true,
            view: Some(V3::new(0.0, 3.0, 0.0)),
            bot: Some(Box::new(move |bot, out| {
                brain(bot, out);
                dodge(bot, out);
            })),
            ..Default::default()
        }
    }
}
