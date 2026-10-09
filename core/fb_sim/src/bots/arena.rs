//! The arena brain: hold ground, hunt beans, keep off the edges.
use super::*;

pub struct ArenaOpts {
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub safe: Option<Box<dyn Fn(f64, f64, f64) -> bool + Send + Sync>>,
    pub jump_when: Option<BotTest>,
    /// Seconds before a bot picks another spot (plus up to 2.5 at random).
    pub retarget: f64,
    /// Places worth going to (steps, pads, bumpers…): the playground.
    pub pois: Vec<(f64, f64)>,
    /// Emote now and then (lobby).
    pub social: bool,
    /// The floor moves or drops (not in the navigation grid): only `safe` decides where to stand.
    pub ignore_nav: bool,
    /// Is there ground to stand on at (x, z) at time t? Bots steer round holes (with `ignore_nav`).
    pub floor: Option<Box<dyn Fn(f64, f64, f64) -> bool + Send + Sync>>,
}

impl Default for ArenaOpts {
    fn default() -> Self {
        Self {
            x: 0.0,
            z: 0.0,
            radius: 0.0,
            safe: None,
            jump_when: None,
            retarget: 2.0,
            pois: Vec::new(),
            social: false,
            ignore_nav: false,
            floor: None,
        }
    }
}

/// Turns the stick away from holes just ahead (small turns first); stops at the edge if all else fails.
fn avoid_holes(bot: &BotView, out: &mut BotInput, floor: &dyn Fn(f64, f64, f64) -> bool) {
    let b = bot.body;
    let base = m::atan2(out.mx, out.mz);
    let len = m::hypot(out.mx, out.mz);
    let clear = |a: f64| {
        [0.9, 1.8, 2.7]
            .iter()
            .all(|&d| floor(b.pos.x + m::sin(a) * d, b.pos.z + m::cos(a) * d, bot.t + d / 8.0))
    };
    for turn in [0.0, 0.4, -0.4, 0.8, -0.8, 1.3, -1.3, 1.9, -1.9, m::PI] {
        if !clear(base + turn) {
            continue;
        }
        out.mx = m::sin(base + turn) * len;
        out.mz = m::cos(base + turn) * len;
        return;
    }
    out.mx = 0.0;
    out.mz = 0.0;
}

/// Moves around an arena like a player: picks spots that are safe and not crowded (or, for the pushy
/// ones, goes after somebody), runs there by A*, and times its jumps with a reaction delay.
pub fn arena_brain(opts: ArenaOpts) -> BotBrain {
    Box::new(move |bot, out| arena_step(&opts, bot, out))
}

fn arena_step(opts: &ArenaOpts, bot: &mut BotView, out: &mut BotInput) {
    let cx = opts.x;
    let cz = opts.z;
    init_bot(bot);
    let b = bot.body;
    let t = bot.t;
    let aggro = bot.mem.traits().aggro;
    if bot.mem.arena.pref.is_none() {
        bot.mem.arena.pref = Some(opts.radius * (0.25 + bot.rng.unit() * 0.45));
    }
    // Judged from the ground the bot stands (or last stood) on: in the air everything looks unsafe.
    if b.grounded {
        bot.mem.arena.gy = Some(b.pos.y);
    }
    let gy = bot.mem.arena.gy.unwrap_or(b.pos.y);
    let nav = bot.nav;
    let ok_at = |x: f64, z: f64| {
        opts.safe.as_ref().is_none_or(|s| s(x, z, t)) && (opts.ignore_nav || nav.is_none_or(|n| n.safe(x, z, gy)))
    };
    // Hunting someone: best a bean near the edge, to shove it off.
    let mut hunt = bot
        .mem
        .arena
        .hunt
        .and_then(|id| bot.others.iter().find(|o| o.id == id).copied());
    if let Some(h) = hunt
        && (t > bot.mem.arena.hunt_until || h.down)
    {
        hunt = None;
        bot.mem.arena.hunt = None;
    }
    // (Everyone starts near the edge: nobody goes after anybody in the first few seconds.)
    if bot.mem.arena.next_hunt.is_none() {
        bot.mem.arena.next_hunt = Some(5.0 + bot.rng.unit() * 6.0);
    }
    if hunt.is_none() && b.grounded && t > bot.mem.arena.next_hunt.unwrap_or(0.0) {
        bot.mem.arena.next_hunt = Some(t + 3.0 + bot.rng.unit() * 6.0);
        if bot.rng.unit() < aggro * 0.7 {
            let mut best = f64::NEG_INFINITY;
            for o in bot.others {
                let d = dist_xz(o.pos, b.pos);
                if o.down || d > 12.0 || (o.pos.y - b.pos.y).abs() > 1.0 {
                    continue;
                }
                let score =
                    (if edge_dir(o.pos, &ok_at).is_some() { 5.0 } else { 0.0 }) - d * 0.35 + bot.rng.unit() * 2.0;
                if score > best {
                    best = score;
                    hunt = Some(*o);
                }
            }
            if let Some(h) = hunt {
                bot.mem.arena.hunt = Some(h.id);
                bot.mem.arena.hunt_until = t + 3.0 + bot.rng.unit() * 3.0;
            }
        }
    }
    let expired = t > bot.mem.arena.until;
    let unsafe_spot = bot.mem.arena.target.is_some_and(|(tx, tz)| !ok_at(tx, tz));
    if hunt.is_none() && (bot.mem.arena.target.is_none() || (b.grounded && (expired || unsafe_spot))) {
        bot.mem.arena.arrived = false;
        let mut best = f64::NEG_INFINITY;
        let pois = &opts.pois;
        for k in 0..8 {
            let (x, z);
            if !pois.is_empty() && k < 2 && bot.rng.unit() < 0.5 {
                let p = pois[bot.rng.index(pois.len())];
                x = p.0;
                z = p.1;
            } else {
                let a = bot.rng.unit() * m::PI * 2.0;
                let r = m::sqrt(bot.rng.unit()) * opts.radius;
                x = cx + m::cos(a) * r;
                z = cz + m::sin(a) * r;
            }
            let mut score = if ok_at(x, z) { 0.0 } else { -100.0 };
            score -= (m::hypot(x - cx, z - cz) - bot.mem.arena.pref.unwrap_or(5.0)).abs() * 0.25;
            let mut crowd: f64 = 9.0;
            for o in bot.others {
                crowd = crowd.at_most(m::hypot(o.pos.x - x, o.pos.z - z));
            }
            score += crowd.at_most(4.0) * (0.6 - aggro * 0.5);
            score -= m::hypot(x - b.pos.x, z - b.pos.z) * 0.08;
            score += bot.rng.unit() * 1.5;
            if score > best {
                best = score;
                bot.mem.arena.target = Some((x, z));
            }
        }
        bot.mem.arena.until = t + opts.retarget + bot.rng.unit() * 2.5;
    }
    let bonus = if hunt.is_some() {
        None
    } else {
        bonus_near(bot, 8.0, false)
    };
    let mut tx = match bonus {
        Some((bx, bz)) if ok_at(bx, bz) => bx,
        _ => bot.mem.arena.target.map_or(cx, |t| t.0),
    };
    let mut tz = match bonus {
        Some((bx, bz)) if ok_at(bx, bz) => bz,
        _ => bot.mem.arena.target.map_or(cz, |t| t.1),
    };
    // Shoving: from the inside, towards the edge. Not lined up yet: get round to the inside first.
    let shove = hunt.and_then(|h| shove_off(bot, h.pos, &ok_at));
    if let Some(h) = hunt {
        tx = h.pos.x;
        tz = h.pos.z;
        let e = if shove.is_some() { None } else { edge_dir(h.pos, &ok_at) };
        if let Some((ex, ez)) = e
            && ok_at(h.pos.x - ex * 1.9, h.pos.z - ez * 1.9)
        {
            tx = h.pos.x - ex * 1.9;
            tz = h.pos.z - ez * 1.9;
        }
    }
    let speed = if hunt.is_some() {
        1.0
    } else {
        0.85 + bot.mem.traits().spd * 0.15
    };
    let d = nav_to(bot, tx, tz, out, speed, if hunt.is_some() { 1.1 } else { 0.9 });
    if let (Some(_), Some((ux, uz, sd))) = (hunt, shove) {
        // Lined up: charge straight at them, and tackle from close by (or just run them over).
        out.mx = ux;
        out.mz = uz;
        if sd > 1.3 && sd < 2.7 && b.grounded && bot.rng.unit() < (0.2 + aggro * 0.6) * BOT_DT * 6.0 {
            out.dive = true;
        }
    } else if let Some(h) = hunt {
        let hd = dist_xz(h.pos, b.pos);
        // Where the prey will be when a dive gets there.
        let px = h.pos.x + h.vel.x * 0.22 - b.pos.x;
        let pz = h.pos.z + h.vel.z * 0.22 - b.pos.z;
        let pd = m::hypot(px, pz);
        if hd < 1.4 {
            out.grab = true;
            bot.mem.grab_until = t + 0.3;
        } else if pd > 1.6
            && pd < 3.2
            && b.grounded
            && b.state == BodyState::Normal
            && [2.0, 3.5]
                .iter()
                .all(|&k| ok_at(b.pos.x + (px / pd) * k, b.pos.z + (pz / pd) * k))
            && bot.rng.unit() < (0.3 + aggro * 1.5) * BOT_DT
        {
            out.mx = px / pd;
            out.mz = pz / pd;
            out.dive = true;
            bot.mem.aimed = true;
        }
    } else if d < (if bot.mem.arena.arrived { 1.8 } else { 1.0 }) {
        // There: potter about round the spot instead of freezing.
        bot.mem.arena.arrived = true;
        let w = t * 0.5 + bot.mem.traits().ph;
        let px = tx + m::sin(w) * 0.6;
        let pz = tz + m::cos(w * 1.3) * 0.6;
        if ok_at(px, pz) {
            steer(bot, px, pz, out, 0.3);
        } else {
            steer(bot, tx, tz, out, 0.3);
        }
    }
    if let Some(floor) = &opts.floor
        && m::hypot(out.mx, out.mz) > 0.1
    {
        avoid_holes(bot, out, floor.as_ref());
    }
    if opts.social && d < 1.5 && b.grounded && bot.rng.unit() < 0.12 * BOT_DT {
        out.emote = 1 + bot.rng.index(EMOTES as usize) as u32;
    }
    // `jump_when` models reaction time itself. Just before landing counts too: the jump is kept for a
    // moment and goes off on touchdown.
    let landing = !b.grounded && b.vel.y < 0.0 && b.pos.y - gy < 0.35;
    if opts.jump_when.as_ref().is_some_and(|f| f(bot)) && (b.grounded || landing) {
        out.jump = true;
    }
    humanize(
        bot,
        out,
        &HumanOpts {
            fun: opts.jump_when.is_none(),
            rough: hunt.is_none(),
            avoid: hunt.is_none(),
            safe: Some(&ok_at),
            ..Default::default()
        },
    );
    unstick(bot, out);
}
