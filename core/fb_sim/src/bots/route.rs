//! The waypoint brain: hop chains and the walk along a map's path.
use super::*;

/// In the air: steers so that the body comes down on (tx, tz) when it falls to height ty. Returns
/// false when it cannot get down there any more (already below it): it heads straight there.
pub fn aim_landing(bot: &BotView, tx: f64, ty: f64, tz: f64, out: &mut BotInput) -> bool {
    let b = bot.body;
    let dx = tx - b.pos.x;
    let dz = tz - b.pos.z;
    let disc = b.vel.y * b.vel.y + 2.0 * GRAVITY * (b.pos.y - ty);
    if disc < 0.0 {
        steer(bot, tx, tz, out, 1.0);
        return false;
    }
    let t = 0.08f64.at_least((b.vel.y + m::sqrt(disc)) / GRAVITY);
    // (Air control: full stick asks for a run, or for the speed the body already flies at.)
    let top = RUN_SPEED.at_least(m::hypot(b.vel.x, b.vel.z));
    let mut mx = dx / t / top;
    let mut mz = dz / t / top;
    let l = m::hypot(mx, mz);
    if l > 1.0 {
        mx /= l;
        mz /= l;
    }
    out.mx = mx;
    out.mz = mz;
    true
}

/// A pad in a chain of bounces (x may move with time).
pub struct Hop {
    pub x: WpX,
    pub y: f64,
    pub z: f64,
    /// Radius of the pad, and how hard it throws up (m/s).
    pub r: f64,
    pub power: f64,
}

fn hop_x(h: &Hop, t: f64) -> f64 {
    match &h.x {
        WpX::At(x) => *x,
        WpX::Moving(f) => f(t),
    }
}

/// Bouncing across pads to a landing spot: a waypoint `drive`. Returns false once on the ground past `edge`.
pub fn hop_chain(key: Note<usize>, hops: Vec<Hop>, land: V3, edge: f64, ready: Option<BotTest>) -> Drive {
    Box::new(move |bot, out| {
        let b = bot.body;
        let p = b.pos;
        if b.grounded {
            if p.z > edge + 0.5 {
                // Missed, and down in a basin with the pads: back onto the nearest one.
                if p.y > land.y - 1.0 {
                    return false;
                }
                let mut near = 0;
                for (j, h) in hops.iter().enumerate() {
                    let n = &hops[near];
                    if m::hypot(hop_x(h, bot.t) - p.x, h.z - p.z) < m::hypot(hop_x(n, bot.t) - p.x, n.z - p.z) {
                        near = j;
                    }
                }
                let n = &hops[near];
                if (n.y - p.y).abs() > 1.0 {
                    return false;
                }
                bot.mem.set(key, near);
                steer(bot, hop_x(n, bot.t), n.z, out, 1.0);
                return true;
            }
            bot.mem.set(key, 0);
            let h = &hops[0];
            if let Some(ready) = &ready
                && !ready(bot)
            {
                // Wait where the first pad passes.
                let mut lo = f64::INFINITY;
                let mut hi = f64::NEG_INFINITY;
                for k in 0..40 {
                    let x = hop_x(h, bot.t + f64::from(k) * 0.25);
                    lo = lo.at_most(x);
                    hi = hi.at_least(x);
                }
                let wx = if hi - lo > 1.0 {
                    (hi - 0.5).at_most((lo + 0.5).at_least(p.x))
                } else {
                    lo
                };
                follow(bot, wx, edge - 1.3, out);
                return true;
            }
            let x = hop_x(h, bot.t + 0.5);
            steer(bot, x, h.z, out, 1.0);
            if p.z > edge - 1.4 {
                out.jump = true;
            }
            return true;
        }
        if b.state != BodyState::Normal {
            return false;
        }
        let mut i = bot.mem.get(key).unwrap_or(0);
        // Just thrown up by a pad: the next one is the target.
        for (j, h) in hops.iter().enumerate() {
            if b.vel.y > h.power * 0.7
                && m::hypot(p.x - hop_x(h, bot.t), p.z - h.z) < h.r + 1.2
                && (p.y - h.y).abs() < 2.5
            {
                i = i.max(j + 1);
            }
        }
        bot.mem.set(key, i);
        let Some(h) = hops.get(i) else {
            aim_landing(bot, land.x, land.y, land.z, out);
            return true;
        };
        // Where a moving pad will be by the time the body comes down to it.
        let disc = b.vel.y * b.vel.y + 2.0 * GRAVITY * (p.y - h.y);
        let t = if disc > 0.0 {
            (b.vel.y + m::sqrt(disc)) / GRAVITY
        } else {
            0.0
        };
        aim_landing(bot, hop_x(h, bot.t + t), h.y, h.z, out);
        true
    })
}

/// A bonus lying close by (ahead of the bot, on its level), if any: worth a small detour.
pub fn bonus_near(bot: &BotView, range: f64, ahead: bool) -> Option<(f64, f64)> {
    let b = bot.body;
    let mut best = None;
    let mut bd = range;
    for x in bot.bonuses {
        if (x.y - b.pos.y).abs() > 1.2 {
            continue;
        }
        let d = dist_xz(*x, b.pos);
        if d < bd && (!ahead || x.z > b.pos.z - 1.5) {
            bd = d;
            best = Some((x.x, x.z));
        }
    }
    best
}

fn target_x(bot: &BotView, w: &Waypoint) -> f64 {
    match &w.x {
        WpX::Moving(f) => f(bot.t),
        WpX::At(x) => x + bot.mem.traits().off * w.lane.width(),
    }
}

/// Was the bot put somewhere else since its last decision (a respawn, a portal)? A jump in position
/// that its speed does not explain. Keeps this decision's position for the next one.
fn teleported(bot: &mut BotView) -> bool {
    let b = bot.body;
    let speed = m::hypot3(b.vel.x, b.vel.y, b.vel.z);
    let seen = bot.mem.way.seen.replace((b.pos, bot.t, speed));
    seen.is_some_and(|(p, t, v)| {
        let dt = bot.t - t;
        let moved = m::hypot3(b.pos.x - p.x, b.pos.y - p.y, b.pos.z - p.z);
        // (After a gap in the decisions there is nothing to compare with.)
        dt <= 2.5 * BOT_DT && moved > 3.0 + v.at_least(speed) * dt * 2.0
    })
}

/// Follows waypoints along a course (by z), with waits, timed jumps, detours and special stretches.
pub fn path_step<W: Borrow<Waypoint>>(
    points: &[W],
    dive_chance: f64,
    logic: &dyn MapLogic,
    bot: &mut BotView,
    out: &mut BotInput,
) {
    init_bot(bot);
    let jumped = teleported(bot);
    let b = bot.body;
    let n = points.len();
    let was = bot.mem.way.wp;
    let reach = |w: &Waypoint| if w.lane == Lane::Exact { 0.5 } else { 1.2 };
    let behind = |i: usize| i >= 1 && b.pos.z < points[i - 1].borrow().z - 4.0;
    let mut i = match was {
        Some(i) if !behind(i) => i,
        _ => points
            .iter()
            .position(|p| p.borrow().z > b.pos.z - 0.5)
            .unwrap_or(n.saturating_sub(1)),
    };
    while i < n
        && (b.pos.z > points[i].borrow().z + 0.3
            || m::hypot(
                target_x(bot, points[i].borrow()) - b.pos.x,
                points[i].borrow().z - b.pos.z,
            ) < reach(points[i].borrow()))
        && i < n - 1
    {
        i += 1;
    }
    // A wait belongs to one approach of one waypoint: a new waypoint, or the same one again after a fall
    // (from the checkpoint before it), waits again.
    if was != Some(i) || jumped {
        bot.mem.way.go = None;
        bot.mem.way.ready_at = None;
    }
    bot.mem.way.wp = Some(i);
    let Some(wp) = points.get(i).map(Borrow::borrow) else {
        return;
    };
    let hooked = match wp.hook {
        Some(h) => logic.steer(h, bot, out),
        None => Steer::Follow,
    };
    if hooked == Steer::Drove || wp.drive.as_ref().is_some_and(|drive| drive(bot, out)) {
        // Special stretches jam too (bots backing off for another run into the ones behind them).
        unstick(bot, out);
        return;
    }
    let detour = match hooked {
        Steer::Detour(ax, az) => Some((ax, az)),
        _ => wp.detour.as_ref().and_then(|detour| detour(bot)),
    };
    if let Some((ax, az)) = detour {
        nav_to(bot, ax, az, out, 1.0, 0.4);
        humanize(
            bot,
            out,
            &HumanOpts {
                precise: true,
                ..Default::default()
            },
        );
        unstick(bot, out);
        return;
    }
    let moving_x = matches!(wp.x, WpX::Moving(_));
    let precise = wp.lane == Lane::Exact || wp.wait.is_some() || moving_x || wp.jump_when.is_some();
    let skill = bot.mem.traits().skill;
    // Once a wait is over the bot commits to that waypoint. The less patient ones sometimes just go for it.
    if let Some(wait) = &wp.wait
        && bot.mem.way.go != Some(i)
    {
        let ready = wait(bot);
        let reckless = !ready && bot.rng.unit() < (1.0 - skill) * 0.12 * BOT_DT;
        if (ready || reckless) && bot.mem.way.ready_at.is_none() {
            bot.mem.way.ready_at = Some(bot.t + if reckless { 0.0 } else { bot.mem.traits().react * 0.5 });
        }
        if let Some(at) = bot.mem.way.ready_at
            && bot.t >= at
        {
            bot.mem.way.go = Some(i);
            bot.mem.way.ready_at = None;
        }
    }
    if wp.wait.is_some() && bot.mem.way.go != Some(i) {
        // Hold at the previous waypoint (keeps a corridor position exact).
        if i > 0 {
            let hold = points[i - 1].borrow();
            let hx = target_x(bot, hold);
            follow(bot, hx, hold.z, out);
        } else {
            out.mx = 0.0;
            out.mz = 0.0;
        }
        let when = wp.jump_when.as_ref().is_some_and(|f| f(bot));
        if when && b.grounded {
            out.jump = true;
        } else if b.grounded
            && bot.rng.unit() < bot.mem.traits().jumpy * 0.25 * BOT_DT
            && bot.nav.is_some_and(|nav| nav.safe(b.pos.x, b.pos.z, b.pos.y))
        {
            // Impatient hops while waiting, where it is safe to.
            out.jump = true;
        }
        humanize(
            bot,
            out,
            &HumanOpts {
                precise: true,
                ..Default::default()
            },
        );
        return;
    }
    let mut tx = target_x(bot, wp);
    let mut tz = wp.z;
    // A bonus a few steps off the line (in open sections): go and take it.
    if !precise
        && let Some((bx, bz)) = bonus_near(bot, 6.0, true)
        && bz < wp.z + 2.0
    {
        tx = bx;
        tz = bz;
    }
    let speed = wp.speed.unwrap_or(1.0) * if moving_x { 1.0 } else { bot.mem.traits().spd };
    let d = if precise {
        if wp.lane == Lane::Exact {
            follow(bot, tx, tz, out)
        } else {
            steer(bot, tx, tz, out, speed)
        }
    } else {
        nav_to(bot, tx, tz, out, speed, reach(wp))
    };
    if precise
        && wp.lane == Lane::Exact
        && let Some(s) = wp.speed
        && s != 0.0
    {
        out.mx *= s;
        out.mz *= s;
    }
    // Jumps work a moment after leaving the ground too (coyote time), as for players.
    let can_jump = b.grounded || b.coyote > 0.02;
    if wp.jump && d < 2.6 && can_jump {
        out.jump = true;
    }
    if wp.jump_when.as_ref().is_some_and(|f| f(bot)) && can_jump {
        out.jump = true;
    }
    if dive_chance != 0.0 && b.grounded && bot.rng.unit() < dive_chance * BOT_DT {
        out.dive = true;
    }
    // The last stretch: dive over the line like everybody does.
    if i == n - 1 && !precise && d < 6.0 && d > 3.0 && b.grounded && bot.rng.unit() < 0.4 + skill * 0.4 {
        out.dive = true;
    }
    humanize(
        bot,
        out,
        &HumanOpts {
            precise,
            fun: !precise,
            attack: 0.9,
            forward: true,
            ..Default::default()
        },
    );
    unstick(bot, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bot {
        rng: Rng,
        mem: BotMem,
        plan: BotPlan,
    }

    impl Bot {
        /// One decision of a waypoint brain, standing at `pos` at time t.
        fn decide(&mut self, points: &[Waypoint], pos: V3, t: f64) {
            let world = World::default();
            let mut body = Body::new(0);
            body.pos = pos;
            body.grounded = true;
            let scores = BTreeMap::new();
            let mut view = BotView {
                id: PlayerId(1),
                body: &body,
                t,
                rng: &mut self.rng,
                mem: &mut self.mem,
                plan: &mut self.plan,
                others: &[],
                nav: None,
                bonuses: &[],
                world: &world,
                scores: &scores,
            };
            path_step(points, 0.0, &crate::map::NoLogic, &mut view, &mut BotInput::default());
        }
    }

    #[test]
    fn waits_again_at_a_hazard_after_a_respawn() {
        let points = [
            Waypoint::at(0.0, 0.0),
            Waypoint::at(0.0, 10.0).wait(|_| true),
            Waypoint::at(0.0, 20.0),
        ];
        let mut bot = Bot {
            rng: Rng::new(7),
            mem: BotMem::default(),
            plan: BotPlan::default(),
        };
        let mut t = 0.0;
        while bot.mem.way.go != Some(1) {
            assert!(t < 2.0, "never got ready to go");
            bot.decide(&points, V3::new(0.0, 0.0, 5.0), t);
            t += BOT_DT;
        }
        // On its way to the hazard it stays committed…
        for z in [5.5, 6.0, 6.5] {
            bot.decide(&points, V3::new(0.0, 0.0, z), t);
            t += BOT_DT;
            assert_eq!(bot.mem.way.go, Some(1));
        }
        // …until it is knocked off and back at the checkpoint before it: the same waypoint, waited for again.
        bot.decide(&points, V3::new(0.0, 0.5, 1.0), t);
        assert_eq!(bot.mem.way.wp, Some(1));
        assert_eq!(bot.mem.way.go, None);
    }
}
