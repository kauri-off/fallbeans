import { EMOTES } from '../shared/consts';
import type { BotBrain, BotInput, BotView } from './map';

/** Bot brains run at 20 Hz (BOT_EVERY ticks); timers below use this step. */
export const BOT_DT = 1 / 20;

export interface Waypoint {
  /** Target x, or a function of time for moving targets. */
  x: number | ((t: number) => number);
  z: number;
  /** Lateral spread between bots (static x only); 0 = an exact line (narrow or timed sections). */
  w?: number;
  /** Jump when within 2.6 m. */
  jump?: boolean;
  /** Jump whenever this says so (e.g. a rotor arm is about to sweep by). */
  jumpWhen?: (bot: BotView) => boolean;
  /** Stand still until this is true (e.g. the path ahead is clear). */
  wait?: (bot: BotView) => boolean;
  /** Stick deflection towards it (default: full). Careful sections go slower. */
  speed?: number;
  /**
   * Somewhere to go instead for now (e.g. stand on a button that opens a gate for the others);
   * null: carry on along the path.
   */
  detour?: (bot: BotView) => { x: number; z: number } | null;
  /** Full control for a special stretch (returns false to follow the waypoint as usual). */
  drive?: (bot: BotView, out: BotInput) => boolean;
}

/**
 * A bot's personality, fixed for the round: how good (timing, patience), how pushy (grabs,
 * tackles), how bouncy (jumping for fun) and how quick to react. Players differ; so do bots.
 */
export function initBot(bot: BotView) {
  if (bot.mem.init) return;
  const r = bot.rng;
  bot.mem.init = 1;
  bot.mem.skill = 0.45 + r() * 0.55;
  bot.mem.aggro = r() ** 1.4;
  bot.mem.jumpy = r();
  bot.mem.react = 0.06 + (1 - (bot.mem.skill ?? 0.7)) * 0.25 + r() * 0.05;
  // People hold the stick all the way; only a few ease off.
  bot.mem.spd = r() < 0.8 ? 1 : 0.9 + r() * 0.1;
  bot.mem.off = r() * 2 - 1;
  bot.mem.ph = r() * 100;
  bot.mem.stuck = 0;
}

export function steer(bot: BotView, tx: number, tz: number, out: BotInput, speed = 1) {
  const dx = tx - bot.body.pos.x;
  const dz = tz - bot.body.pos.z;
  const d = Math.hypot(dx, dz);
  if (d < 0.3) {
    out.mx = 0;
    out.mz = 0;
    return d;
  }
  const k = Math.min(1, d / 1.5) * speed;
  out.mx = (dx / d) * k;
  out.mz = (dz / d) * k;
  return d;
}

/** Precise line following for narrow safe corridors: corrects x hard while moving along z. */
export function follow(bot: BotView, tx: number, tz: number, out: BotInput) {
  const b = bot.body;
  const dz = tz - b.pos.z;
  const d = Math.hypot(tx - b.pos.x, dz);
  // Proportional with damping: no overshooting on moving or rolling ground.
  const mx = Math.max(-1, Math.min(1, (tx - b.pos.x) * 2.5 - b.vel.x * 0.18));
  const mz = Math.abs(dz) < 0.2 ? 0 : Math.sign(dz) * Math.max(0.2, 1 - Math.abs(mx));
  const l = Math.hypot(mx, mz) || 1;
  const k = Math.min(1, d / 0.8 + 0.2);
  out.mx = (mx / l) * k;
  out.mz = (mz / l) * k;
  return d;
}

/** Extra route cost near other beans on the ground: bots run around a crowd instead of into it. */
function crowdCost(bot: BotView) {
  const y = bot.body.pos.y;
  const near = bot.others.filter(
    (o) => Math.abs(o.pos.y - y) < 1.2 && Math.hypot(o.pos.x - bot.body.pos.x, o.pos.z - bot.body.pos.z) < 12,
  );
  if (!near.length) return undefined;
  return (x: number, z: number) => {
    let c = 0;
    for (const o of near) {
      const d = Math.hypot(o.pos.x - x, o.pos.z - z);
      if (d < 1.6) c += (1.6 - d) * 2.5;
    }
    return c;
  };
}

/**
 * Runs to (tx, tz) along an A* route over the course (around walls, up steps, over gaps, around
 * other beans). Without a route (moving parts, no grid yet) it heads straight there.
 * Returns the remaining straight-line distance.
 */
export function navTo(bot: BotView, tx: number, tz: number, out: BotInput, speed = 1, radius = 0.8): number {
  const b = bot.body;
  const direct = Math.hypot(tx - b.pos.x, tz - b.pos.z);
  const nav = bot.nav;
  if (!nav || direct < 1.2) {
    steer(bot, tx, tz, out, speed);
    return direct;
  }
  const plan = bot.plan;
  // In the air (or after a long time) an old route is worthless: head straight for it.
  if (!b.grounded && (Math.hypot(plan.tx - tx, plan.tz - tz) > 1 || bot.t - plan.at > 2)) {
    steer(bot, tx, tz, out, speed);
    return direct;
  }
  const cur = plan.path?.[plan.i];
  const off = cur ? Math.hypot(cur.x - b.pos.x, cur.z - b.pos.z) > 6 || cur.y - b.pos.y > 2.5 : true;
  const stale = bot.t - plan.at > 0.6 + (bot.id % 5) * 0.07;
  if (b.grounded && (off || stale || Math.hypot(plan.tx - tx, plan.tz - tz) > 1)) {
    plan.path = nav.path(b.pos, tx, tz, null, { radius, maxNodes: 2500, cost: crowdCost(bot) });
    plan.i = 0;
    plan.tx = tx;
    plan.tz = tz;
    plan.at = bot.t;
  }
  const path = plan.path;
  if (!path?.length) {
    steer(bot, tx, tz, out, speed);
    return direct;
  }
  // Move on past points we have reached (or passed).
  while (plan.i < path.length - 1) {
    const p = path[plan.i]!;
    const q = path[plan.i + 1]!;
    const d = Math.hypot(p.x - b.pos.x, p.z - b.pos.z);
    const ahead = (q.x - p.x) * (b.pos.x - p.x) + (q.z - p.z) * (b.pos.z - p.z) > 0;
    if (d < 0.7 || (ahead && d < 2 && !q.jump)) plan.i++;
    else break;
  }
  const p = path[plan.i]!;
  const last = plan.i === path.length - 1;
  const dx = p.x - b.pos.x;
  const dz = p.z - b.pos.z;
  const d = Math.hypot(dx, dz);
  // Last leg: the exact target (cells are coarse; and a route may end short of a target on
  // something the grid does not know, like a moving platform).
  if (last) steer(bot, tx, tz, out, speed);
  else if (d > 1e-3) {
    out.mx = (dx / d) * speed;
    out.mz = (dz / d) * speed;
  }
  if (p.jump && b.grounded) {
    const prev = path[plan.i - 1];
    const gap = prev ? Math.hypot(p.x - prev.x, p.z - prev.z) : d;
    const climb = p.y - b.pos.y > 0.5;
    const along = d > 1e-3 ? (b.vel.x * dx + b.vel.z * dz) / d : 0;
    const when = climb ? 1.3 : Math.min(2.8, gap * 0.8 + 0.5);
    if (d < when && (along > 2.5 || climb)) out.jump = true;
  }
  return direct;
}

/** Stuck against something while trying to move: hop, then side-step. */
export function unstick(bot: BotView, out: BotInput) {
  const moving = Math.hypot(out.mx, out.mz) > 0.3;
  // Real displacement, not velocity: being pushed back by another bean keeps velocity high.
  const p = bot.body.pos;
  const sp = bot.mem.lx === undefined ? 9 : Math.hypot(p.x - bot.mem.lx, p.z - (bot.mem.lz ?? 0)) / BOT_DT;
  bot.mem.lx = p.x;
  bot.mem.lz = p.z;
  bot.mem.stuck = moving && sp < 1.2 && bot.body.grounded ? (bot.mem.stuck ?? 0) + BOT_DT : 0;
  if ((bot.mem.stuck ?? 0) > 0.45) {
    out.jump = true;
    bot.plan.at = -1e9;
    if ((bot.mem.stuck ?? 0) > 1.4) {
      // Still stuck (another bean in the way): sidestep for a moment.
      bot.mem.side = bot.rng() < 0.5 ? -1 : 1;
      bot.mem.sideUntil = bot.t + 0.6;
      bot.mem.stuck = 0;
    }
  }
  if ((bot.mem.sideUntil ?? -1) > bot.t) {
    out.mx = bot.mem.side ?? 1;
    out.mz *= 0.3;
  }
}

/** Nearest other bean ahead of the bot within `range`, on about the same level. */
function beanAhead(bot: BotView, range: number, cone = 0.5) {
  const b = bot.body;
  const fx = Math.sin(b.yaw);
  const fz = Math.cos(b.yaw);
  let best: BotView['others'][number] | null = null;
  let bd = range;
  for (const o of bot.others) {
    if (o.down || Math.abs(o.pos.y - b.pos.y) > 1) continue;
    const dx = o.pos.x - b.pos.x;
    const dz = o.pos.z - b.pos.z;
    const d = Math.hypot(dx, dz);
    if (d >= bd || d < 1e-3 || (dx * fx + dz * fz) / d < cone) continue;
    bd = d;
    best = o;
  }
  return best ? { o: best, d: bd } : null;
}

export interface HumanOpts {
  /** Narrow or timed section: tiny wobble, no playing around. */
  precise?: boolean;
  /** Hops for fun on open ground (off where a mistimed jump is deadly). */
  fun?: boolean;
  /** Grab / tackle beans that get in the way. */
  rough?: boolean;
}

/**
 * The human touch on top of a brain's decision: hands are never perfectly steady, people bunny-hop
 * for no reason, sidestep others, shove and grab the ones in their way, and dive for the line.
 */
export function humanize(bot: BotView, out: BotInput, o: HumanOpts = {}) {
  const b = bot.body;
  const skill = bot.mem.skill ?? 0.7;
  const ph = bot.mem.ph ?? 0;
  const t = bot.t;
  // Wobble: a slow, uneven drift of the aim.
  const amount = o.precise ? 0.03 + (1 - skill) * 0.03 : 0.12 + (1 - skill) * 0.14;
  const a = (Math.sin(t * 0.9 + ph) * 0.6 + Math.sin(t * 2.3 + ph * 1.7) * 0.4) * amount;
  const c = Math.cos(a);
  const s = Math.sin(a);
  const mx = out.mx * c - out.mz * s;
  out.mz = out.mx * s + out.mz * c;
  out.mx = mx;
  if (o.precise) return;
  const moving = Math.hypot(out.mx, out.mz) > 0.5;
  // Someone right in front: step around them (or shove them, if that is the kind of player we are).
  const ahead = beanAhead(bot, 1.8, 0.55);
  if (ahead && moving) {
    const dx = ahead.o.pos.x - b.pos.x;
    const dz = ahead.o.pos.z - b.pos.z;
    const side = (bot.mem.off ?? 0) >= 0 ? 1 : -1;
    const k = (1.8 - ahead.d) * 0.6;
    out.mx += (-dz / ahead.d) * side * k;
    out.mz += (dx / ahead.d) * side * k;
  }
  if (o.rough !== false) {
    const aggro = bot.mem.aggro ?? 0.3;
    if ((bot.mem.grabUntil ?? -1) > t) out.grab = true;
    else if (ahead && ahead.d < 1.35 && b.grounded && bot.rng() < aggro * 1.2 * BOT_DT) {
      bot.mem.grabUntil = t + 0.4 + bot.rng() * 0.9;
      out.grab = true;
    } else if (
      ahead &&
      ahead.d > 1.8 &&
      ahead.d < 2.8 &&
      b.grounded &&
      bot.rng() < aggro * 0.35 * BOT_DT &&
      (!bot.nav || bot.nav.safe(b.pos.x + Math.sin(b.yaw) * 3.5, b.pos.z + Math.cos(b.yaw) * 3.5, b.pos.y))
    )
      out.dive = true;
  }
  // Bunny hops on open ground.
  if (o.fun !== false && b.grounded && moving && Math.hypot(b.vel.x, b.vel.z) > 5) {
    if (bot.rng() < (bot.mem.jumpy ?? 0.5) * 0.3 * BOT_DT) out.jump = true;
  }
  const l = Math.hypot(out.mx, out.mz);
  if (l > 1) {
    out.mx /= l;
    out.mz /= l;
  }
}

/** A bonus lying close by (ahead of the bot, on its level), if any: worth a small detour. */
export function bonusNear(bot: BotView, range = 6, ahead = true): { x: number; z: number } | null {
  const b = bot.body;
  let best: { x: number; z: number } | null = null;
  let bd = range;
  for (const x of bot.bonuses ?? []) {
    if (Math.abs(x.y - b.pos.y) > 1.2) continue;
    const d = Math.hypot(x.x - b.pos.x, x.z - b.pos.z);
    if (d < bd && (!ahead || x.z > b.pos.z - 1.5)) {
      bd = d;
      best = x;
    }
  }
  return best;
}

export function pathBrain(points: readonly Waypoint[], opts: { diveChance?: number } = {}): BotBrain {
  return (bot, out) => {
    initBot(bot);
    const b = bot.body;
    let i = bot.mem.wp ?? -1;
    const prev = points[i - 1];
    if (i < 0 || (prev && b.pos.z < prev.z - 4)) {
      i = points.findIndex((p) => p.z > b.pos.z - 0.5);
      if (i < 0) i = points.length - 1;
    }
    const targetX = (w: Waypoint) => (typeof w.x === 'function' ? w.x(bot.t) : w.x + (bot.mem.off ?? 0) * (w.w ?? 1.5));
    let wp = points[i];
    const reach = (w: Waypoint) => (w.w === 0 ? 0.5 : 1.2);
    while (
      wp &&
      (b.pos.z > wp.z + 0.3 || Math.hypot(targetX(wp) - b.pos.x, wp.z - b.pos.z) < reach(wp)) &&
      i < points.length - 1
    ) {
      i++;
      wp = points[i];
    }
    bot.mem.wp = i;
    if (!wp) return;
    if (wp.drive?.(bot, out)) return;
    const away = wp.detour?.(bot);
    if (away) {
      navTo(bot, away.x, away.z, out, 1, 0.4);
      humanize(bot, out, { precise: true });
      unstick(bot, out);
      return;
    }
    const precise = wp.w === 0 || !!wp.wait || typeof wp.x === 'function' || !!wp.jumpWhen;
    const skill = bot.mem.skill ?? 0.7;
    // Once a wait is over the bot commits to that waypoint (no turning back halfway). The less
    // patient ones sometimes just go for it.
    if (wp.wait && bot.mem.go !== i) {
      const ready = wp.wait(bot);
      const reckless = !ready && bot.rng() < (1 - skill) * 0.12 * BOT_DT;
      // Reaction time: seeing the gap is not the same as going. Once decided, the bot goes after its
      // reaction time even if the gap has closed by then (as people do); waiting for a gap that stays
      // open longer than the reaction time left slow bots stuck forever at fast hammers.
      if (ready || reckless) bot.mem.readyAt ??= bot.t + (reckless ? 0 : (bot.mem.react ?? 0.15) * 0.5);
      if (bot.mem.readyAt !== undefined && bot.t >= bot.mem.readyAt) {
        bot.mem.go = i;
        bot.mem.readyAt = undefined;
      }
    }
    if (wp.wait && bot.mem.go !== i) {
      // Hold at the previous waypoint (keeps a corridor position exact).
      const hold = points[i - 1];
      if (hold) follow(bot, targetX(hold), hold.z, out);
      else {
        out.mx = 0;
        out.mz = 0;
      }
      if (wp.jumpWhen?.(bot) && b.grounded) out.jump = true;
      // Impatient hops while waiting, where it is safe to.
      else if (b.grounded && bot.rng() < (bot.mem.jumpy ?? 0.5) * 0.25 * BOT_DT && bot.nav?.safe(b.pos.x, b.pos.z, b.pos.y))
        out.jump = true;
      humanize(bot, out, { precise: true });
      return;
    }
    let tx = targetX(wp);
    let tz = wp.z;
    // A bonus a few steps off the line (in open sections): go and take it.
    const bonus = precise ? null : bonusNear(bot);
    if (bonus && bonus.z < wp.z + 2) {
      tx = bonus.x;
      tz = bonus.z;
    }
    let d: number;
    const speed = (wp.speed ?? 1) * (typeof wp.x === 'function' ? 1 : (bot.mem.spd ?? 1));
    if (precise) d = wp.w === 0 ? follow(bot, tx, tz, out) : steer(bot, tx, tz, out, speed);
    else d = navTo(bot, tx, tz, out, speed, reach(wp));
    if (precise && wp.w === 0 && wp.speed) {
      out.mx *= wp.speed;
      out.mz *= wp.speed;
    }
    // Jumps work a moment after leaving the ground too (coyote time), as for players.
    const canJump = b.grounded || b.coyote > 0.02;
    if (wp.jump && d < 2.6 && canJump) out.jump = true;
    if (wp.jumpWhen?.(bot) && canJump) out.jump = true;
    if (opts.diveChance && b.grounded && bot.rng() < opts.diveChance * BOT_DT) out.dive = true;
    // The last stretch: dive over the line like everybody does.
    if (i === points.length - 1 && !precise && d < 6 && d > 3 && b.grounded && bot.rng() < 0.4 + skill * 0.4) out.dive = true;
    humanize(bot, out, { precise, fun: !precise });
    unstick(bot, out);
  };
}

/** Several routes (e.g. one per safe lane); each bot takes the one starting nearest to it. */
export function routesBrain(routes: readonly { x: number; points: readonly Waypoint[] }[]): BotBrain {
  const brains = routes.map((r) => pathBrain(r.points));
  return (bot, out) => {
    if (bot.mem.route === undefined) {
      const x = bot.body.pos.x;
      let best = 0;
      routes.forEach((r, i) => {
        if (Math.abs(r.x - x) < Math.abs(routes[best]!.x - x)) best = i;
      });
      bot.mem.route = best;
    }
    brains[bot.mem.route]!(bot, out);
  };
}

export interface ArenaOpts {
  x?: number;
  z?: number;
  radius: number;
  safe?: (x: number, z: number, t: number) => boolean;
  jumpWhen?: (bot: BotView) => boolean;
  retarget?: number;
  /** Places worth going to (steps, pads, bumpers…): the playground. */
  pois?: readonly { x: number; z: number }[];
  /** Emote now and then (lobby). */
  social?: boolean;
  /** The floor moves or drops (not in the navigation grid): only safe() decides where to stand. */
  ignoreNav?: boolean;
  /** Is there ground to stand on at (x, z) at time t? Bots steer round holes (with ignoreNav). */
  floor?: (x: number, z: number, t: number) => boolean;
}

/** Turns the stick away from holes just ahead (tries small turns first); stops at the edge if all else fails. */
function avoidHoles(bot: BotView, out: BotInput, floor: (x: number, z: number, t: number) => boolean) {
  const b = bot.body;
  const base = Math.atan2(out.mx, out.mz);
  const len = Math.hypot(out.mx, out.mz);
  const clear = (a: number) =>
    [0.9, 1.8, 2.7].every((d) => floor(b.pos.x + Math.sin(a) * d, b.pos.z + Math.cos(a) * d, bot.t + d / 8));
  for (const turn of [0, 0.4, -0.4, 0.8, -0.8, 1.3, -1.3, 1.9, -1.9, Math.PI]) {
    if (!clear(base + turn)) continue;
    out.mx = Math.sin(base + turn) * len;
    out.mz = Math.cos(base + turn) * len;
    return;
  }
  out.mx = 0;
  out.mz = 0;
}

/**
 * Moves around an arena like a player: picks spots that are safe and not crowded (or, for the
 * pushy ones, goes after somebody), runs there by A*, and times its jumps with a reaction delay.
 */
export function arenaBrain(opts: ArenaOpts): BotBrain {
  const cx = opts.x ?? 0;
  const cz = opts.z ?? 0;
  return (bot, out) => {
    initBot(bot);
    const b = bot.body;
    const t = bot.t;
    const aggro = bot.mem.aggro ?? 0.3;
    bot.mem.pref ??= opts.radius * (0.25 + bot.rng() * 0.45);
    const okAt = (x: number, z: number) =>
      (!opts.safe || opts.safe(x, z, t)) && (opts.ignoreNav || !bot.nav || bot.nav.safe(x, z, b.pos.y));
    // Hunting someone (grab them, knock them into the sweeper…)?
    let hunt = bot.mem.hunt !== undefined ? bot.others.find((o) => o.id === bot.mem.hunt) : undefined;
    if (hunt && (t > (bot.mem.huntUntil ?? 0) || hunt.down)) {
      hunt = undefined;
      bot.mem.hunt = undefined;
    }
    if (!hunt && t > (bot.mem.nextHunt ?? 0)) {
      bot.mem.nextHunt = t + 3 + bot.rng() * 6;
      if (bot.rng() < aggro * 0.6 && bot.others.length) {
        const o = bot.others[Math.floor(bot.rng() * bot.others.length)]!;
        if (Math.hypot(o.pos.x - b.pos.x, o.pos.z - b.pos.z) < 12 && okAt(o.pos.x, o.pos.z)) {
          hunt = o;
          bot.mem.hunt = o.id;
          bot.mem.huntUntil = t + 2.5 + bot.rng() * 2.5;
        }
      }
    }
    const expired = t > (bot.mem.until ?? -1e9);
    const unsafe = bot.mem.tx !== undefined && !okAt(bot.mem.tx, bot.mem.tz ?? 0);
    if (!hunt && (expired || unsafe || bot.mem.tx === undefined)) {
      let best = -Infinity;
      const pois = opts.pois ?? [];
      for (let k = 0; k < 8; k++) {
        let x: number;
        let z: number;
        if (pois.length && k < 2 && bot.rng() < 0.5) {
          const p = pois[Math.floor(bot.rng() * pois.length)]!;
          x = p.x;
          z = p.z;
        } else {
          const a = bot.rng() * Math.PI * 2;
          const r = Math.sqrt(bot.rng()) * opts.radius;
          x = cx + Math.cos(a) * r;
          z = cz + Math.sin(a) * r;
        }
        let score = okAt(x, z) ? 0 : -100;
        score -= Math.abs(Math.hypot(x - cx, z - cz) - (bot.mem.pref ?? 5)) * 0.25;
        let crowd = 9;
        for (const o of bot.others) crowd = Math.min(crowd, Math.hypot(o.pos.x - x, o.pos.z - z));
        score += Math.min(crowd, 4) * (0.6 - aggro * 0.5);
        score -= Math.hypot(x - b.pos.x, z - b.pos.z) * 0.08;
        score += bot.rng() * 1.5;
        if (score > best) {
          best = score;
          bot.mem.tx = x;
          bot.mem.tz = z;
        }
      }
      bot.mem.until = t + (opts.retarget ?? 2) + bot.rng() * 2.5;
    }
    const bonus = hunt ? null : bonusNear(bot, 8, false);
    const tx = hunt ? hunt.pos.x : bonus && okAt(bonus.x, bonus.z) ? bonus.x : (bot.mem.tx ?? cx);
    const tz = hunt ? hunt.pos.z : bonus && okAt(bonus.x, bonus.z) ? bonus.z : (bot.mem.tz ?? cz);
    const d = navTo(bot, tx, tz, out, hunt ? 1 : 0.85 + (bot.mem.spd ?? 1) * 0.15, hunt ? 1.1 : 0.9);
    if (hunt) {
      if (d < 1.4) {
        out.grab = true;
        bot.mem.grabUntil = t + 0.3;
      } else if (d < 2.6 && d > 1.8 && b.grounded && bot.rng() < aggro * 0.8 * BOT_DT) out.dive = true;
    } else if (d < 1) {
      // There: potter about instead of freezing.
      const w = t * 0.7 + (bot.mem.ph ?? 0);
      out.mx = Math.sin(w) * 0.25;
      out.mz = Math.cos(w * 1.3) * 0.25;
    }
    if (opts.floor && Math.hypot(out.mx, out.mz) > 0.1) avoidHoles(bot, out, opts.floor);
    if (opts.social && d < 1.5 && b.grounded && bot.rng() < 0.12 * BOT_DT) out.emote = 1 + Math.floor(bot.rng() * EMOTES);
    // jumpWhen models reaction time itself (a timing window that depends on bot.mem.react).
    if (opts.jumpWhen?.(bot) && b.grounded) out.jump = true;
    humanize(bot, out, { fun: !opts.jumpWhen, rough: !hunt });
    unstick(bot, out);
  };
}
