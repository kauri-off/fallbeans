//! What makes a bot look human: wobble, idle hops, dodges, tackles and grabs.
use super::*;

pub struct HumanOpts<'a> {
    /// Narrow or timed section: tiny wobble, no playing around.
    pub precise: bool,
    /// Hops for fun on open ground.
    pub fun: bool,
    /// Grab / tackle beans that get in the way.
    pub rough: bool,
    /// Step round beans in the way.
    pub avoid: bool,
    /// Real ground for rescue dives, where the navigation grid does not know it.
    pub land: Option<&'a LandCheck<'a>>,
    /// How keen on tackling others with a dive (0: never). Default 1.
    pub attack: f64,
    /// A race: crowders are shoved only ahead and to the sides.
    pub forward: bool,
    /// Where the bot may step aside to dodge (default: the navigation grid).
    pub safe: Option<&'a dyn Fn(f64, f64) -> bool>,
}

impl Default for HumanOpts<'_> {
    fn default() -> Self {
        Self {
            precise: false,
            fun: true,
            rough: true,
            avoid: true,
            land: None,
            attack: 1.0,
            forward: false,
            safe: None,
        }
    }
}

/// How close another bean may come before the bot shoves it off with a dive (m).
const SPACE: f64 = 2.6;
/// Tackle reach (centre to centre) and the height a tackle still connects within.
const TACKLE_REACH: f64 = 1.6;
const TACKLE_HEIGHT: f64 = 1.3;

struct Threat {
    t: f64,
    ax: f64,
    az: f64,
    dive: bool,
    id: u32,
}

/// The most urgent attack coming at the bot: a dive whose line passes within tackle reach soon, or
/// somebody reaching out to grab it from close by.
fn incoming(bot: &BotView) -> Option<Threat> {
    let b = bot.body;
    let mut best: Option<Threat> = None;
    for o in bot.others {
        if o.down || (o.pos.y - b.pos.y).abs() > TACKLE_HEIGHT + 0.4 {
            continue;
        }
        let rx = b.pos.x - o.pos.x;
        let rz = b.pos.z - o.pos.z;
        let d = m::hypot(rx, rz);
        if d > 7.0 || d < 1e-3 {
            continue;
        }
        if o.dive {
            let vx = o.vel.x - b.vel.x;
            let vz = o.vel.z - b.vel.z;
            let v2 = vx * vx + vz * vz;
            if v2 < 16.0 {
                continue;
            }
            // Closest approach of the attacker's line.
            let tca = (rx * vx + rz * vz) / v2;
            if !(0.0..=0.6).contains(&tca) {
                continue;
            }
            let mx = rx - vx * tca;
            let mz = rz - vz * tca;
            let miss = m::hypot(mx, mz);
            if miss > TACKLE_REACH + 0.3 {
                continue;
            }
            // Out across its line, on the side the bot is already on.
            let v = m::sqrt(v2);
            let mut sx = -vz / v;
            let mut sz = vx / v;
            if sx * mx + sz * mz < 0.0 || (miss < 0.2 && bot.mem.traits().off < 0.0) {
                sx = -sx;
                sz = -sz;
            }
            if best.as_ref().is_none_or(|b| tca < b.t) {
                best = Some(Threat {
                    t: tca,
                    ax: sx,
                    az: sz,
                    dive: true,
                    id: o.id,
                });
            }
        } else if o.reach && d < 2.6 {
            // Somebody about to grab: back off out of reach.
            let closing = (o.vel.x * rx + o.vel.z * rz) / d;
            let t = (d - 2.1).at_least(0.0) / closing.at_least(1.0);
            if best.as_ref().is_none_or(|b| t < b.t) {
                best = Some(Threat {
                    t,
                    ax: rx / d,
                    az: rz / d,
                    dive: false,
                    id: o.id,
                });
            }
        }
    }
    best
}

/// Getting out of the way of a tackle or a grab: a step aside where there is ground, else a jump.
fn dodge(bot: &mut BotView, out: &mut BotInput, safe: Option<&dyn Fn(f64, f64) -> bool>) -> bool {
    let b = bot.body;
    let t = bot.t;
    if bot.mem.dodge_until > t {
        (out.mx, out.mz) = bot.mem.dodge;
        return true;
    }
    if b.state != BodyState::Normal || bot.mem.dodge_seen > t {
        return false;
    }
    let Some(threat) = incoming(bot) else {
        return false;
    };
    let mem = &mut *bot.mem;
    // Whoever went for the bot is remembered for a counterattack, dodged or not.
    mem.foe = Some(threat.id);
    mem.foe_until = t + 3.0;
    let skill = mem.traits().skill;
    let react = mem.traits().react;
    // One look per attack: whether this bot notices it in time is decided once.
    mem.dodge_seen = t + 0.3f64.at_least(threat.t + 0.1);
    if threat.t < react * 0.6 || bot.rng.unit() > 0.25 + skill * 0.6 {
        return false;
    }
    let step = [0.8, 1.6]
        .iter()
        .all(|&k| safe.is_none_or(|s| s(b.pos.x + threat.ax * k, b.pos.z + threat.az * k)));
    let mem = &mut *bot.mem;
    if safe.is_some() && step {
        mem.dodge_until = t + 0.45f64.at_most(threat.t + 0.2);
        mem.dodge = (threat.ax, threat.az);
        out.mx = threat.ax;
        out.mz = threat.az;
        return true;
    }
    // No room to step aside: hop over a dive (timed: just before it arrives).
    if threat.dive && b.grounded && threat.t < 0.35 {
        out.jump = true;
        return true;
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum Payback {
    Counter,
    Space,
}

/// A tackle the bot has a reason for, and where to aim it.
fn retaliate(
    bot: &BotView,
    safe: Option<&dyn Fn(f64, f64) -> bool>,
    forward: Option<(f64, f64)>,
) -> Option<(f64, f64, Payback)> {
    let b = bot.body;
    let mem = &*bot.mem;
    let t = bot.t;
    let lands_safe =
        |ux: f64, uz: f64| safe.is_none_or(|s| [2.0, 3.5].iter().all(|&k| s(b.pos.x + ux * k, b.pos.z + uz * k)));
    // Counterattack.
    let foe = match mem.foe {
        Some(id) if mem.foe_until > t => bot.others.iter().find(|o| o.id == id),
        _ => None,
    };
    if let Some(foe) = foe
        && !foe.dive
        && (foe.pos.y - b.pos.y).abs() < 1.0
    {
        let dx = foe.pos.x + foe.vel.x * 0.2 - b.pos.x;
        let dz = foe.pos.z + foe.vel.z * 0.2 - b.pos.z;
        let d = m::hypot(dx, dz);
        if d > 0.5 && d < 3.4 && lands_safe(dx / d, dz / d) {
            return Some((dx / d, dz / d, Payback::Counter));
        }
    }
    // Personal space: the nearest bean closing in (or already right on top of the bot).
    let mut near: Option<(f64, f64, f64)> = None;
    for o in bot.others {
        if o.down || o.dive || (o.pos.y - b.pos.y).abs() > 0.8 {
            continue;
        }
        let dx = o.pos.x - b.pos.x;
        let dz = o.pos.z - b.pos.z;
        let d = m::hypot(dx, dz);
        if d > SPACE || d < 1e-3 || near.is_some_and(|n| d > n.2) {
            continue;
        }
        if let Some((fx, fz)) = forward
            && (dx * fx + dz * fz) / d < -0.3
        {
            continue;
        }
        let closing = -((o.vel.x - b.vel.x) * dx + (o.vel.z - b.vel.z) * dz) / d;
        if closing > 0.2 || d < 1.6 {
            near = Some((dx / d, dz / d, d));
        }
    }
    match near {
        Some((ux, uz, _)) if lands_safe(ux, uz) => Some((ux, uz, Payback::Space)),
        _ => None,
    }
}

/// Somebody worth tackling with a dive: within range where they will be in a moment, near the way
/// the bot is going, on the same level, with safe ground to land on. Returns the direction.
fn tackle_aim(bot: &BotView, out: &BotInput, safe: Option<&dyn Fn(f64, f64) -> bool>) -> Option<(f64, f64)> {
    let b = bot.body;
    let (hx, hz) = heading(bot, out);
    let mut best: Option<(f64, f64, f64)> = None;
    for o in bot.others {
        if o.dive || (o.pos.y - b.pos.y).abs() > 0.8 {
            continue;
        }
        // Where they will be when the dive gets there (it covers ~3 m in a quarter of a second).
        let px = o.pos.x + o.vel.x * 0.22;
        let pz = o.pos.z + o.vel.z * 0.22;
        let dx = px - b.pos.x;
        let dz = pz - b.pos.z;
        let d = m::hypot(dx, dz);
        if !(1.2..=3.8).contains(&d) {
            continue;
        }
        let ux = dx / d;
        let uz = dz / d;
        // Roughly the way the bot is going: no turning round for it.
        if ux * hx + uz * hz < 0.6 {
            continue;
        }
        if let Some(s) = safe
            && ![2.0, 3.5, 5.0].iter().all(|&k| s(b.pos.x + ux * k, b.pos.z + uz * k))
        {
            continue;
        }
        if best.is_none_or(|b| d < b.2) {
            best = Some((ux, uz, d));
        }
    }
    best.map(|b| (b.0, b.1))
}

/// The human touch on top of a brain's decision: unsteady hands, hops for no reason, sidesteps,
/// shoves and grabs, dives at others.
pub fn humanize(bot: &mut BotView, out: &mut BotInput, o: &HumanOpts) {
    let b = bot.body;
    let skill = bot.mem.traits().skill;
    let ph = bot.mem.traits().ph;
    let t = bot.t;
    smart_dive(bot, out, o.land);
    let nav = bot.nav;
    let gy = b.pos.y;
    let nav_safe = move |x: f64, z: f64| nav.is_some_and(|n| n.safe(x, z, gy));
    let safe: Option<&dyn Fn(f64, f64) -> bool> = match (o.safe, nav) {
        (Some(s), _) => Some(s),
        (None, Some(_)) => Some(&nav_safe),
        (None, None) => None,
    };
    // Out of the way of a tackle or a grab (on narrow ground: only by jumping).
    if !out.dive && dodge(bot, out, if o.precise { None } else { safe }) {
        return;
    }
    // Held by somebody: that is who gets it once the bot is free.
    if b.grounded && b.slow_until > t && b.slow_k <= 0.5 {
        let mut d = 2.6;
        for x in bot.others {
            let dx = dist_xz(x.pos, b.pos);
            if dx < d {
                d = dx;
                bot.mem.foe = Some(x.id);
                bot.mem.foe_until = t + 3.0;
            }
        }
    }
    // Wobble: a slow, uneven drift of the aim.
    let amount = if o.precise {
        0.03 + (1.0 - skill) * 0.03
    } else {
        0.12 + (1.0 - skill) * 0.14
    };
    let a = (m::sin(t * 0.9 + ph) * 0.6 + m::sin(t * 2.3 + ph * 1.7) * 0.4) * amount;
    let c = m::cos(a);
    let s = m::sin(a);
    let mx = out.mx * c - out.mz * s;
    out.mz = out.mx * s + out.mz * c;
    out.mx = mx;
    if o.precise {
        return;
    }
    let moving = m::hypot(out.mx, out.mz) > 0.5;
    // Someone right in front: step around them (or shove them, if that is the kind of player we are).
    let ahead = bean_ahead(bot, 1.8, 0.55);
    if let Some((ao, ad)) = ahead
        && moving
        && o.avoid
    {
        let dx = ao.pos.x - b.pos.x;
        let dz = ao.pos.z - b.pos.z;
        let side = if bot.mem.traits().off >= 0.0 { 1.0 } else { -1.0 };
        let k = (1.8 - ad) * 0.6;
        out.mx += (-dz / ad) * side * k;
        out.mz += (dx / ad) * side * k;
    }
    if o.rough {
        let aggro = bot.mem.traits().aggro;
        let shove = match (ahead, nav) {
            (Some((ao, ad)), Some(n)) if b.grounded && t > 5.0 && ad > 1.3 && ad < 2.8 => {
                let oy = ao.pos.y;
                shove_off(bot, ao.pos, &|x, z| n.safe(x, z, oy))
            }
            _ => None,
        };
        // Somebody at the edge right ahead: tackle them over it (the pushy ones, mostly).
        if shove.is_some() && bot.rng.unit() < (0.1 + aggro * 0.5) * BOT_DT * 3.0 {
            out.dive = true;
        } else if bot.mem.grab_until > t {
            out.grab = true;
        } else if ahead.is_some_and(|(_, d)| d < 1.35) && b.grounded && bot.rng.unit() < aggro * 1.2 * BOT_DT {
            bot.mem.grab_until = t + 0.4 + bot.rng.unit() * 0.9;
            out.grab = true;
        } else if b.grounded && b.state == BodyState::Normal && t > 2.0 {
            let scale = o.attack;
            // Paying back an attack, or shoving off whoever crowds the bot (now and then grabbing instead).
            let back = retaliate(bot, safe, if o.forward { Some(heading(bot, out)) } else { None });
            let rate = match back {
                Some((_, _, Payback::Counter)) => (1.0 + aggro * 2.5) * scale,
                Some((_, _, Payback::Space)) => (1.0 + aggro * 3.0) * scale,
                None => 0.0,
            };
            if let Some((ux, uz, kind)) = back
                && bot.rng.unit() < rate * BOT_DT
            {
                if kind == Payback::Space && bot.rng.unit() < 0.3 {
                    bot.mem.grab_until = t + 0.4 + bot.rng.unit() * 0.6;
                    out.grab = true;
                    return;
                }
                out.mx = ux;
                out.mz = uz;
                out.dive = true;
                bot.mem.aimed = true;
                if kind == Payback::Counter {
                    bot.mem.foe = None;
                }
                return;
            }
            // A tackle: dive at where somebody will be (the pushy ones, much more often).
            let aim = tackle_aim(bot, out, safe);
            let keen = (0.35 + aggro * 2.0) * scale;
            if let Some((ux, uz)) = aim
                && bot.rng.unit() < keen * BOT_DT
            {
                out.mx = ux;
                out.mz = uz;
                out.dive = true;
                bot.mem.aimed = true;
                return;
            }
        }
    }
    // Bunny hops on open ground.
    if o.fun
        && b.grounded
        && moving
        && m::hypot(b.vel.x, b.vel.z) > 5.0
        && bot.rng.unit() < bot.mem.traits().jumpy * 0.3 * BOT_DT
    {
        out.jump = true;
    }
    let l = m::hypot(out.mx, out.mz);
    if l > 1.0 {
        out.mx /= l;
        out.mz /= l;
    }
}

/// The stick as a hand moves it: eased towards what the brain wants. Letting go is quicker.
pub fn smooth_stick(mem: &mut BotMem, out: &mut BotInput) {
    // A tackle goes where it is aimed, not where the eased stick happens to point.
    let aimed = out.dive && mem.aimed;
    mem.aimed = false;
    if aimed {
        mem.stick = Some((out.mx, out.mz));
        return;
    }
    let (px, pz) = mem.stick.unwrap_or((out.mx, out.mz));
    let k = if m::hypot(out.mx, out.mz) < 0.15 { 0.7 } else { 0.5 };
    out.mx = px + (out.mx - px) * k;
    out.mz = pz + (out.mz - pz) * k;
    mem.stick = Some((out.mx, out.mz));
}
