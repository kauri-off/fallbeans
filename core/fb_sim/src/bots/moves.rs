//! Steering, A* routes, getting unstuck, dives and landings.
use super::*;

/// A bot's personality, fixed for the round: how good, how pushy, how bouncy, how quick to react.
pub fn init_bot(bot: &mut BotView) {
    if bot.mem.traits.is_some() {
        return;
    }
    let r = &mut *bot.rng;
    let skill = 0.45 + r.unit() * 0.55;
    // Most players get stuck in: few bots are entirely peaceful.
    let aggro = 0.15 + 0.85 * m::pow(r.unit(), 1.1);
    let jumpy = r.unit();
    let react = 0.06 + (1.0 - skill) * 0.25 + r.unit() * 0.05;
    // People hold the stick all the way; only a few ease off.
    let spd = if r.unit() < 0.8 { 1.0 } else { 0.9 + r.unit() * 0.1 };
    let off = r.unit() * 2.0 - 1.0;
    let ph = r.unit() * 100.0;
    bot.mem.traits = Some(Traits {
        skill,
        aggro,
        jumpy,
        react,
        spd,
        off,
        ph,
    });
    bot.mem.stuck = 0.0;
}

pub fn steer(bot: &BotView, tx: f64, tz: f64, out: &mut BotInput, speed: f64) -> f64 {
    let dx = tx - bot.body.pos.x;
    let dz = tz - bot.body.pos.z;
    let d = m::hypot(dx, dz);
    if d < 0.3 {
        out.mx = 0.0;
        out.mz = 0.0;
        return d;
    }
    let k = (d / 1.5).at_most(1.0) * speed;
    out.mx = (dx / d) * k;
    out.mz = (dz / d) * k;
    d
}

/// Precise line following for narrow safe corridors, and holding a spot.
pub fn follow(bot: &BotView, tx: f64, tz: f64, out: &mut BotInput) -> f64 {
    let b = bot.body;
    let ex = tx - b.pos.x;
    let dz = tz - b.pos.z;
    let mx = (ex * 0.55).clamp(-1.0, 1.0);
    let mz = m::sign(dz) * (1.0 - mx.abs() * 0.5).at_most(dz.abs() * 0.6);
    let l = m::hypot(mx, mz);
    out.mx = if l > 1.0 { mx / l } else { mx };
    out.mz = if l > 1.0 { mz / l } else { mz };
    m::hypot(ex, dz)
}

/// Runs to (tx, tz) along an A* route over the course. Without a route (moving parts, no grid yet)
/// it heads straight there. Returns the remaining straight-line distance.
pub fn nav_to(bot: &mut BotView, tx: f64, tz: f64, out: &mut BotInput, speed: f64, radius: f64) -> f64 {
    let b = bot.body;
    let direct = m::hypot(tx - b.pos.x, tz - b.pos.z);
    let Some(nav) = bot.nav else {
        steer(bot, tx, tz, out, speed);
        return direct;
    };
    if direct < 1.2 {
        steer(bot, tx, tz, out, speed);
        return direct;
    }
    // In the air (or after a long time) an old route is worthless: head straight for it.
    if !b.grounded && (m::hypot(bot.plan.tx - tx, bot.plan.tz - tz) > 1.0 || bot.t - bot.plan.at > 2.0) {
        steer(bot, tx, tz, out, speed);
        return direct;
    }
    let off = match bot.plan.path.as_ref().and_then(|p| p.get(bot.plan.i)) {
        Some(cur) => dist_xz(cur.pos, b.pos) > 6.0 || cur.pos.y - b.pos.y > 2.5,
        None => true,
    };
    let stale = bot.t - bot.plan.at > 0.6 + (bot.id.0 % 5) as f64 * 0.07;
    if b.grounded && (off || stale || m::hypot(bot.plan.tx - tx, bot.plan.tz - tz) > 1.0) {
        // Extra route cost near other beans on the ground: bots run around a crowd instead of into it.
        let y = b.pos.y;
        let near: Vec<V3> = bot
            .others
            .iter()
            .filter(|o| (o.pos.y - y).abs() < 1.2 && dist_xz(o.pos, b.pos) < 12.0)
            .map(|o| o.pos)
            .collect();
        let crowd = |x: f64, z: f64| {
            let mut c = 0.0;
            for o in &near {
                let d = m::hypot(o.x - x, o.z - z);
                if d < 1.6 {
                    c += (1.6 - d) * 2.5;
                }
            }
            c
        };
        let opts = PathOpts {
            cost: if near.is_empty() { None } else { Some(&crowd) },
            radius,
            max_nodes: 2500,
        };
        bot.plan.path = nav.path(b.pos, tx, tz, None, &opts);
        bot.plan.i = 0;
        bot.plan.tx = tx;
        bot.plan.tz = tz;
        bot.plan.at = bot.t;
    }
    let path: &[NavPoint] = match bot.plan.path.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => {
            steer(bot, tx, tz, out, speed);
            return direct;
        }
    };
    // Move on past points we have reached or passed, and past points that would be a step back or
    // sideways (a fresh route starts at the nearest cell, which may lie behind).
    let mut i = bot.plan.i;
    while i < path.len() - 1 {
        let p = path[i];
        let q = path[i + 1];
        let d = dist_xz(p.pos, b.pos);
        let seg = dist_xz(q.pos, p.pos);
        let ahead = (q.pos.x - p.pos.x) * (b.pos.x - p.pos.x) + (q.pos.z - p.pos.z) * (b.pos.z - p.pos.z) > 0.0;
        let closer = dist_xz(q.pos, b.pos) <= seg + 0.2;
        let level = (p.pos.y - b.pos.y).abs() < 0.6 && (q.pos.y - b.pos.y).abs() < 0.6;
        if d < 0.7 || (!q.jump && !p.jump && d < 2.0 && (ahead || (closer && level))) {
            i += 1;
        } else {
            break;
        }
    }
    bot.plan.i = i;
    let p = path[i];
    let last = i == path.len() - 1;
    // Aim a little further along when the way there is plain ground: rounded turns, not zig-zags.
    let mut ax = p.pos.x;
    let mut az = p.pos.z;
    if let Some(&q) = path.get(i + 1)
        && !last
        && !p.jump
        && !q.jump
        && (q.pos.y - b.pos.y).abs() < 0.6
        && clear_to(nav, b.pos, q.pos.x, q.pos.z)
    {
        ax = (p.pos.x + q.pos.x) / 2.0;
        az = (p.pos.z + q.pos.z) / 2.0;
    }
    let dx = ax - b.pos.x;
    let dz = az - b.pos.z;
    let d = m::hypot(dx, dz);
    // Last leg: the exact target.
    if last {
        steer(bot, tx, tz, out, speed);
    } else if d > 1e-3 {
        out.mx = (dx / d) * speed;
        out.mz = (dz / d) * speed;
    }
    if p.jump && b.grounded {
        let gap = if i > 0 {
            let prev = path[i - 1];
            dist_xz(p.pos, prev.pos)
        } else {
            d
        };
        let climb = p.pos.y - b.pos.y > 0.5;
        let along = if d > 1e-3 {
            (b.vel.x * dx + b.vel.z * dz) / d
        } else {
            0.0
        };
        let when = if climb { 1.3 } else { (gap * 0.8 + 0.5).at_most(2.8) };
        if d < when && (along > 2.5 || climb) {
            out.jump = true;
        }
    }
    direct
}

/// Walkable all the way along a straight line from `from` to (x, z) (samples every half metre)?
fn clear_to(nav: Nav, from: V3, x: f64, z: f64) -> bool {
    let d = m::hypot(x - from.x, z - from.z);
    let n = (d / 0.5).ceil() as i64;
    for i in 1..=n {
        let f = i as f64 / n as f64;
        if !nav.safe(from.x + (x - from.x) * f, from.z + (z - from.z) * f, from.y) {
            return false;
        }
    }
    true
}

/// Stuck against something while trying to move: hop, then side-step.
pub fn unstick(bot: &mut BotView, out: &mut BotInput) {
    let b = bot.body;
    let p = b.pos;
    let moving = m::hypot(out.mx, out.mz) > 0.3;
    let mem = &mut *bot.mem;
    match mem.anchor {
        Some((ax, az)) if moving && m::hypot(p.x - ax, p.z - az) <= 0.6 => {
            if b.grounded && bot.t > 0.0 {
                mem.stuck += BOT_DT;
            }
        }
        _ => {
            mem.anchor = Some((p.x, p.z));
            mem.stuck = 0.0;
        }
    }
    let stuck = mem.stuck;
    if stuck > 0.45 && b.grounded {
        out.jump = true;
        bot.plan.at = NEVER;
    }
    if stuck > 1.4 {
        // Still stuck: side-step for a moment, to the own right of the line to a bean in the way, or
        // to one side of the heading; every other try the other way, and never off an edge.
        let (hx, hz) = heading(bot, out);
        let odd = if bot.id.0 % 2 == 1 { -1.0 } else { 1.0 };
        let mut sx = hz * odd;
        let mut sz = -hx * odd;
        let mut near = 1.6;
        for o in bot.others {
            let dx = o.pos.x - p.x;
            let dz = o.pos.z - p.z;
            let d = m::hypot(dx, dz);
            if d < near && d > 1e-3 && (o.pos.y - p.y).abs() < 1.2 {
                near = d;
                sx = dz / d;
                sz = -dx / d;
            }
        }
        let mem = &mut *bot.mem;
        mem.tries += 1;
        let flip = if mem.tries.is_multiple_of(2) { -1.0 } else { 1.0 };
        sx *= flip;
        sz *= flip;
        if bot.nav.is_some_and(|n| !n.safe(p.x + sx * 1.5, p.z + sz * 1.5, p.y)) {
            sx = -sx;
            sz = -sz;
        }
        mem.side = (sx, sz);
        mem.side_until = bot.t + 0.6;
        mem.stuck = 0.0;
    }
    if bot.mem.side_until > bot.t {
        let (hx, hz) = heading(bot, out);
        let x = bot.mem.side.0 + hx * 0.25;
        let z = bot.mem.side.1 + hz * 0.25;
        let l = m::hypot(x, z);
        out.mx = x / l;
        out.mz = z / l;
    }
}

/// Nearest other bean ahead of the bot within `range`, on about the same level.
pub(super) fn bean_ahead(bot: &BotView, range: f64, cone: f64) -> Option<(OtherView, f64)> {
    let b = bot.body;
    let fx = m::sin(b.yaw);
    let fz = m::cos(b.yaw);
    let mut best = None;
    let mut bd = range;
    for o in bot.others {
        if o.down || (o.pos.y - b.pos.y).abs() > 1.0 {
            continue;
        }
        let dx = o.pos.x - b.pos.x;
        let dz = o.pos.z - b.pos.z;
        let d = m::hypot(dx, dz);
        if d >= bd || d < 1e-3 || (dx * fx + dz * fz) / d < cone {
            continue;
        }
        bd = d;
        best = Some(*o);
    }
    best.map(|o| (o, bd))
}

/// Where a flying body comes down (and whether that is safe), and when.
#[derive(Clone, Copy, Debug)]
struct Landing {
    x: f64,
    z: f64,
    y: f64,
    t: f64,
    safe: bool,
}

/// Where a body flying with (vx, vz, vy) comes down on the navigation grid (checked every 50 ms).
fn landing(bot: &BotView, vx: f64, vz: f64, vy: f64) -> Option<Landing> {
    let nav = bot.nav?;
    let p = bot.body.pos;
    let mut py = p.y;
    let mut t = 0.05;
    while t <= 2.0 {
        let y = p.y + vy * t - (GRAVITY / 2.0) * t * t;
        let x = p.x + vx * t;
        let z = p.z + vz * t;
        if let Some((fy, safe)) = nav.floor_below(x, z, py)
            && y <= fy
        {
            return Some(Landing { x, z, y: fy, t, safe });
        }
        py = y;
        t += 0.05;
    }
    None
}

/// Ground the map knows better than the navigation grid (tiles that drop, floors that move): what
/// level the bean is going for, and whether (x, z) has ground there at time t.
pub struct LandCheck<'a> {
    pub y: f64,
    pub ok: &'a dyn Fn(f64, f64, f64) -> bool,
}

/// Where a body flying with (vx, vy, vz) gets back down to the level of `land`.
fn landing_on(bot: &BotView, vx: f64, vz: f64, vy: f64, land: &LandCheck) -> Option<Landing> {
    let p = bot.body.pos;
    if p.y < land.y {
        return None;
    }
    let mut t = 0.05;
    while t <= 2.0 {
        if p.y + vy * t - (GRAVITY / 2.0) * t * t > land.y {
            t += 0.05;
            continue;
        }
        let x = p.x + vx * t;
        let z = p.z + vz * t;
        return Some(Landing {
            x,
            z,
            y: land.y,
            t,
            safe: (land.ok)(x, z, bot.t + t),
        });
    }
    None
}

/// Heading of the stick (or the body, without one).
pub(super) fn heading(bot: &BotView, out: &BotInput) -> (f64, f64) {
    let l = m::hypot(out.mx, out.mz);
    if l > 0.1 {
        return (out.mx / l, out.mz / l);
    }
    (m::sin(bot.body.yaw), m::cos(bot.body.yaw))
}

/// Dives that are worth it: in the air, when the jump falls short and a dive still reaches ground;
/// held by someone, to break free when the way ahead is safe.
pub fn smart_dive(bot: &mut BotView, out: &mut BotInput, land: Option<&LandCheck>) {
    let b = bot.body;
    let nav = bot.nav;
    if (nav.is_none() && land.is_none()) || b.state != BodyState::Normal || out.dive {
        return;
    }
    let skill = bot.mem.traits().skill;
    let (fx, fz) = heading(bot, out);
    if !b.grounded && b.vel.y < 1.5 {
        let fly = |bot: &BotView, vx, vz, vy| match land {
            Some(l) => landing_on(bot, vx, vz, vy, l),
            None => landing(bot, vx, vz, vy),
        };
        let short = fly(bot, b.vel.x, b.vel.z, b.vel.y);
        if short.is_some_and(|s| s.safe) || (land.is_some() && short.is_none()) {
            return;
        }
        let along = (b.vel.x * fx + b.vel.z * fz).at_least(0.0);
        let sp = DIVE_SPEED.at_least(along.at_most(DIVE_SPEED * 1.25));
        let Some(far) = fly(bot, fx * sp, fz * sp, b.vel.y) else {
            return;
        };
        if !far.safe {
            return;
        }
        // Landing on safe ground, with room to slide on it.
        let room = match land {
            Some(l) => (l.ok)(far.x + fx * 1.2, far.z + fz * 1.2, bot.t + far.t + 0.1),
            None => nav.is_some_and(|n| n.safe(far.x + fx * 1.2, far.z + fz * 1.2, far.y)),
        };
        if room && bot.rng.unit() < 0.35 + skill * 0.6 {
            out.dive = true;
        }
        return;
    }
    let Some(nav) = nav else { return };
    let held = b.grounded && b.slow_until > bot.t && b.slow_k <= 0.5;
    if held && bot.rng.unit() < (0.25 + skill * 0.5) * BOT_DT * 6.0 {
        let y = b.pos.y;
        if [2.0, 4.0, 6.0]
            .iter()
            .all(|&d| nav.safe(b.pos.x + fx * d, b.pos.z + fz * d, y))
        {
            out.dive = true;
        } else {
            out.jump = true;
        }
    }
}

/// Whether a shove from the bot sends a bean at `o` off the course: the ground beyond it is unsafe,
/// while the bot's own run-up and landing are safe. Returns the direction and distance.
pub fn shove_off(bot: &BotView, o: V3, ok: &dyn Fn(f64, f64) -> bool) -> Option<(f64, f64, f64)> {
    let b = bot.body;
    let dx = o.x - b.pos.x;
    let dz = o.z - b.pos.z;
    let d = m::hypot(dx, dz);
    if d < 0.3 {
        return None;
    }
    let ux = dx / d;
    let uz = dz / d;
    let beyond = [2.0, 3.0, 4.0].iter().any(|&k| !ok(o.x + ux * k, o.z + uz * k));
    let own = [0.8, 1.6]
        .iter()
        .all(|&k: &f64| ok(b.pos.x + ux * k.at_most(d), b.pos.z + uz * k.at_most(d)));
    (beyond && own).then_some((ux, uz, d))
}

/// Direction from `o` towards the nearest unsafe ground (within 3 m), or None when it is well inside.
pub(super) fn edge_dir(o: V3, ok: &dyn Fn(f64, f64) -> bool) -> Option<(f64, f64)> {
    let mut sx = 0.0;
    let mut sz = 0.0;
    for k in 0..12 {
        let a = (k as f64 / 12.0) * m::PI * 2.0;
        let ax = m::sin(a);
        let az = m::cos(a);
        for r in [1.5, 2.5, 3.5] {
            if !ok(o.x + ax * r, o.z + az * r) {
                sx += ax / r;
                sz += az / r;
                break;
            }
        }
    }
    let l = m::hypot(sx, sz);
    (l > 1e-3).then(|| (sx / l, sz / l))
}
