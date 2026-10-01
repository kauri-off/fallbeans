import type * as THREE from 'three';
import { EMOTES } from '../shared/consts';
import type { BotBrain, BotInput, BotView } from './map';
import { DIVE_SPEED, GRAVITY, RUN_SPEED } from './physics';

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
  // Most players get stuck in: few bots are entirely peaceful.
  bot.mem.aggro = 0.15 + 0.85 * r() ** 1.1;
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

/**
 * Precise line following for narrow safe corridors, and holding a spot: the stick asks for a speed
 * in proportion to the distance left (in x across the corridor, in z along it), so the bean eases in
 * instead of overshooting and correcting back and forth at every decision.
 */
export function follow(bot: BotView, tx: number, tz: number, out: BotInput) {
  const b = bot.body;
  const ex = tx - b.pos.x;
  const dz = tz - b.pos.z;
  const mx = Math.max(-1, Math.min(1, ex * 0.55));
  const mz = Math.sign(dz) * Math.min(1 - Math.abs(mx) * 0.5, Math.abs(dz) * 0.6);
  const l = Math.hypot(mx, mz);
  out.mx = l > 1 ? mx / l : mx;
  out.mz = l > 1 ? mz / l : mz;
  return Math.hypot(ex, dz);
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
  // Move on past points we have reached or passed, and past points that would be a step back or
  // sideways (a fresh route starts at the nearest cell, which may lie behind).
  while (plan.i < path.length - 1) {
    const p = path[plan.i]!;
    const q = path[plan.i + 1]!;
    const d = Math.hypot(p.x - b.pos.x, p.z - b.pos.z);
    const seg = Math.hypot(q.x - p.x, q.z - p.z);
    const ahead = (q.x - p.x) * (b.pos.x - p.x) + (q.z - p.z) * (b.pos.z - p.z) > 0;
    const closer = Math.hypot(q.x - b.pos.x, q.z - b.pos.z) <= seg + 0.2;
    const level = Math.abs(p.y - b.pos.y) < 0.6 && Math.abs(q.y - b.pos.y) < 0.6;
    if (d < 0.7 || (!q.jump && !p.jump && d < 2 && (ahead || (closer && level)))) plan.i++;
    else break;
  }
  const p = path[plan.i]!;
  const last = plan.i === path.length - 1;
  // Aim a little further along when the way there is plain ground: rounded turns, not zig-zags.
  let ax = p.x;
  let az = p.z;
  const q = path[plan.i + 1];
  if (q && !last && !p.jump && !q.jump && Math.abs(q.y - b.pos.y) < 0.6 && clearTo(nav, b.pos, q.x, q.z)) {
    ax = (p.x + q.x) / 2;
    az = (p.z + q.z) / 2;
  }
  const dx = ax - b.pos.x;
  const dz = az - b.pos.z;
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

/** Walkable all the way along a straight line from `from` to (x, z) (samples every half metre)? */
function clearTo(nav: NonNullable<BotView['nav']>, from: THREE.Vector3, x: number, z: number) {
  const d = Math.hypot(x - from.x, z - from.z);
  const n = Math.ceil(d / 0.5);
  for (let i = 1; i <= n; i++) {
    const f = i / n;
    if (!nav.safe(from.x + (x - from.x) * f, from.z + (z - from.z) * f, from.y)) return false;
  }
  return true;
}

/**
 * Stuck against something while trying to move: hop, then side-step. Stuck means no real progress
 * (0.6 m) from where the bot last got somewhere: velocity lies (a bean pushed back by another keeps
 * it high), and the hop itself must not count as progress, or the side-step never comes.
 */
export function unstick(bot: BotView, out: BotInput) {
  const m = bot.mem;
  const b = bot.body;
  const p = b.pos;
  const moving = Math.hypot(out.mx, out.mz) > 0.3;
  if (!moving || m.ax === undefined || Math.hypot(p.x - m.ax, p.z - (m.az ?? 0)) > 0.6) {
    m.ax = p.x;
    m.az = p.z;
    m.stuck = 0;
    // (Not before the start: nobody can move yet.)
  } else if (b.grounded && bot.t > 0) m.stuck = (m.stuck ?? 0) + BOT_DT;
  const stuck = m.stuck ?? 0;
  if (stuck > 0.45 && b.grounded) {
    out.jump = true;
    bot.plan.at = -1e9;
  }
  if (stuck > 1.4) {
    // Still stuck: side-step for a moment. Against another bean, both step to their own right of
    // the line between them (so the two go opposite ways, like people in a corridor); against a
    // wall, to one side of the heading. Every other try goes the other way, and never off an edge.
    const [hx, hz] = heading(bot, out);
    const odd = bot.id % 2 === 1 ? -1 : 1;
    let sx = hz * odd;
    let sz = -hx * odd;
    let near = 1.6;
    for (const o of bot.others) {
      const dx = o.pos.x - p.x;
      const dz = o.pos.z - p.z;
      const d = Math.hypot(dx, dz);
      if (d < near && d > 1e-3 && Math.abs(o.pos.y - p.y) < 1.2) {
        near = d;
        sx = dz / d;
        sz = -dx / d;
      }
    }
    m.tries = (m.tries ?? 0) + 1;
    const flip = m.tries % 2 === 0 ? -1 : 1;
    sx *= flip;
    sz *= flip;
    if (bot.nav && !bot.nav.safe(p.x + sx * 1.5, p.z + sz * 1.5, p.y)) {
      sx = -sx;
      sz = -sz;
    }
    m.sdx = sx;
    m.sdz = sz;
    m.sideUntil = bot.t + 0.6;
    m.stuck = 0;
  }
  if ((m.sideUntil ?? -1) > bot.t) {
    const [hx, hz] = heading(bot, out);
    const x = (m.sdx ?? 1) + hx * 0.25;
    const z = (m.sdz ?? 0) + hz * 0.25;
    const l = Math.hypot(x, z);
    out.mx = x / l;
    out.mz = z / l;
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

/**
 * Where a body flying from its position with horizontal velocity (vx, vz) and vertical vy comes
 * down (checked every 50 ms of flight), or null if there is nothing under it within two seconds.
 */
function landing(bot: BotView, vx: number, vz: number, vy: number) {
  const nav = bot.nav;
  const p = bot.body.pos;
  if (!nav) return null;
  let py = p.y;
  for (let t = 0.05; t <= 2; t += 0.05) {
    const y = p.y + vy * t - (GRAVITY / 2) * t * t;
    const x = p.x + vx * t;
    const z = p.z + vz * t;
    const f = nav.floorBelow(x, z, py);
    if (f && y <= f.y) return { x, z, t, ...f };
    py = y;
  }
  return null;
}

/**
 * Ground the map knows better than the navigation grid (tiles that drop, floors that move): what
 * level the bean is going for, and whether (x, z) has ground there at time t.
 */
export interface LandCheck {
  y: number;
  ok: (x: number, z: number, t: number) => boolean;
}

/** Where a body flying with (vx, vy, vz) gets back down to the level of `land`, or null if it never does. */
function landingOn(bot: BotView, vx: number, vz: number, vy: number, land: LandCheck) {
  const p = bot.body.pos;
  if (p.y < land.y) return null;
  for (let t = 0.05; t <= 2; t += 0.05) {
    if (p.y + vy * t - (GRAVITY / 2) * t * t > land.y) continue;
    const x = p.x + vx * t;
    const z = p.z + vz * t;
    return { x, z, y: land.y, safe: land.ok(x, z, bot.t + t), t };
  }
  return null;
}

/** Heading of the stick (or the body, without one). */
function heading(bot: BotView, out: BotInput): [number, number] {
  const l = Math.hypot(out.mx, out.mz);
  if (l > 0.1) return [out.mx / l, out.mz / l];
  return [Math.sin(bot.body.yaw), Math.cos(bot.body.yaw)];
}

/**
 * Dives that are worth it: in the air, when the jump falls short and a dive still reaches ground;
 * held by someone, to break free (a dive shakes off any grip) when the way ahead is safe.
 */
export function smartDive(bot: BotView, out: BotInput, land?: LandCheck) {
  const b = bot.body;
  const nav = bot.nav;
  if ((!nav && !land) || b.state !== 'normal' || out.dive) return;
  const skill = bot.mem.skill ?? 0.7;
  const [fx, fz] = heading(bot, out);
  if (!b.grounded && b.vel.y < 1.5) {
    const fly = (vx: number, vz: number, vy: number) => (land ? landingOn(bot, vx, vz, vy, land) : landing(bot, vx, vz, vy));
    const short = fly(b.vel.x, b.vel.z, b.vel.y);
    if (short?.safe || (land && !short)) return;
    const along = Math.max(0, b.vel.x * fx + b.vel.z * fz);
    const sp = Math.max(DIVE_SPEED, Math.min(along, DIVE_SPEED * 1.25));
    const far = fly(fx * sp, fz * sp, Math.max(b.vel.y, 3));
    if (!far?.safe) return;
    // Landing on safe ground, with room to slide on it.
    const room = land
      ? land.ok(far.x + fx * 1.2, far.z + fz * 1.2, bot.t + far.t + 0.1)
      : !!nav?.safe(far.x + fx * 1.2, far.z + fz * 1.2, far.y);
    if (room && bot.rng() < 0.35 + skill * 0.6) out.dive = true;
    return;
  }
  if (!nav) return;
  const held = b.grounded && b.slowUntil > bot.t && b.slowK <= 0.5;
  if (held && bot.rng() < (0.25 + skill * 0.5) * BOT_DT * 6) {
    const y = b.pos.y;
    if ([2, 4, 6].every((d) => nav.safe(b.pos.x + fx * d, b.pos.z + fz * d, y))) out.dive = true;
    else out.jump = true;
  }
}

/**
 * Whether a shove from the bot sends `o` off the course: the ground beyond it (away from the bot) is
 * unsafe, while the bot's own run-up and landing are safe. Returns the direction of the shove.
 */
export function shoveOff(bot: BotView, o: { pos: THREE.Vector3 }, ok: (x: number, z: number) => boolean) {
  const b = bot.body;
  const dx = o.pos.x - b.pos.x;
  const dz = o.pos.z - b.pos.z;
  const d = Math.hypot(dx, dz);
  if (d < 0.3) return null;
  const ux = dx / d;
  const uz = dz / d;
  const beyond = [2, 3, 4].some((k) => !ok(o.pos.x + ux * k, o.pos.z + uz * k));
  const self = [0.8, 1.6].every((k) => ok(b.pos.x + ux * Math.min(k, d), b.pos.z + uz * Math.min(k, d)));
  return beyond && self ? { ux, uz, d } : null;
}

/** Direction from `o` towards the nearest unsafe ground (within 3 m), or null when it is well inside. */
function edgeDir(o: { pos: THREE.Vector3 }, ok: (x: number, z: number) => boolean): [number, number] | null {
  let sx = 0;
  let sz = 0;
  for (let k = 0; k < 12; k++) {
    const a = (k / 12) * Math.PI * 2;
    const ax = Math.sin(a);
    const az = Math.cos(a);
    for (const r of [1.5, 2.5, 3.5])
      if (!ok(o.pos.x + ax * r, o.pos.z + az * r)) {
        sx += ax / r;
        sz += az / r;
        break;
      }
  }
  const l = Math.hypot(sx, sz);
  return l > 1e-3 ? [sx / l, sz / l] : null;
}

export interface HumanOpts {
  /** Narrow or timed section: tiny wobble, no playing around. */
  precise?: boolean;
  /** Hops for fun on open ground (off where a mistimed jump is deadly). */
  fun?: boolean;
  /** Grab / tackle beans that get in the way. */
  rough?: boolean;
  /** Step round beans in the way (off when going for one). */
  avoid?: boolean;
  /** Real ground for rescue dives, where the navigation grid does not know it (see LandCheck). */
  land?: LandCheck;
  /** How keen on tackling others with a dive (0: never; races less than arenas). Default 1. */
  attack?: number;
  /** Where the bot may step aside to dodge (default: the navigation grid; none: jump only). */
  safe?: (x: number, z: number) => boolean;
}

/** Tackle reach (centre to centre) and the height a tackle still connects within (see ServerArena.interact). */
const TACKLE_REACH = 1.6;
const TACKLE_HEIGHT = 1.3;

/**
 * The most urgent attack coming at the bot: a dive (or fast slide) whose line passes within tackle
 * reach soon, or somebody reaching out to grab it from close by. `t`: seconds until it lands;
 * (ax, az): which way to get out of it (across the attacker's line, or straight away from a grab).
 */
function incoming(bot: BotView): { t: number; ax: number; az: number; dive: boolean; id: number } | null {
  const b = bot.body;
  let best: { t: number; ax: number; az: number; dive: boolean; id: number } | null = null;
  for (const o of bot.others) {
    if (o.down || Math.abs(o.pos.y - b.pos.y) > TACKLE_HEIGHT + 0.4) continue;
    const rx = b.pos.x - o.pos.x;
    const rz = b.pos.z - o.pos.z;
    const d = Math.hypot(rx, rz);
    if (d > 7 || d < 1e-3) continue;
    if (o.dive) {
      const vx = o.vel.x - b.vel.x;
      const vz = o.vel.z - b.vel.z;
      const v2 = vx * vx + vz * vz;
      if (v2 < 16) continue;
      // Closest approach of the attacker's line.
      const tca = (rx * vx + rz * vz) / v2;
      if (tca < 0 || tca > 0.6) continue;
      const mx = rx - vx * tca;
      const mz = rz - vz * tca;
      const miss = Math.hypot(mx, mz);
      if (miss > TACKLE_REACH + 0.3) continue;
      // Out across its line, on the side the bot is already on (a head-on dive: either side).
      const v = Math.sqrt(v2);
      let sx = -vz / v;
      let sz = vx / v;
      if (sx * mx + sz * mz < 0 || (miss < 0.2 && (bot.mem.off ?? 0) < 0)) {
        sx = -sx;
        sz = -sz;
      }
      if (!best || tca < best.t) best = { t: tca, ax: sx, az: sz, dive: true, id: o.id };
    } else if (o.reach && d < 2.6) {
      // Somebody about to grab: back off out of reach (a grab takes whatever is in front of them).
      const closing = (o.vel.x * rx + o.vel.z * rz) / d;
      const t = Math.max(0, d - 2.1) / Math.max(1, closing);
      if (!best || t < best.t) best = { t, ax: rx / d, az: rz / d, dive: false, id: o.id };
    }
  }
  return best;
}

/**
 * Getting out of the way of a tackle or a grab, as people try to (the better ones, more often and
 * sooner): a step aside where there is ground to step to, else a jump (a dive is low: a jump in
 * time can clear it). Needs the bot's reaction time before it lands. Returns whether it dodges.
 */
function dodge(bot: BotView, out: BotInput, safe: ((x: number, z: number) => boolean) | null): boolean {
  const b = bot.body;
  const t = bot.t;
  const m = bot.mem;
  if ((m.dodgeUntil ?? -1) > t) {
    out.mx = m.dodgeX ?? 0;
    out.mz = m.dodgeZ ?? 0;
    return true;
  }
  if (b.state !== 'normal' || (m.dodgeSeen ?? -1) > t) return false;
  const threat = incoming(bot);
  if (!threat) return false;
  // Whoever went for the bot is remembered for a counterattack (see retaliate), dodged or not.
  m.foe = threat.id;
  m.foeUntil = t + 3;
  const skill = m.skill ?? 0.7;
  const react = m.react ?? 0.15;
  // One look per attack: whether this bot notices it in time is decided once.
  m.dodgeSeen = t + Math.max(0.3, threat.t + 0.1);
  if (threat.t < react * 0.6 || bot.rng() > 0.25 + skill * 0.6) return false;
  const step = [0.8, 1.6].every((k) => !safe || safe(b.pos.x + threat.ax * k, b.pos.z + threat.az * k));
  // (Counted for audits and tuning.)
  m.dodges = (m.dodges ?? 0) + 1;
  if (safe && step) {
    m.dodgeUntil = t + Math.min(0.45, threat.t + 0.2);
    m.dodgeX = threat.ax;
    m.dodgeZ = threat.az;
    out.mx = threat.ax;
    out.mz = threat.az;
    return true;
  }
  // No room to step aside: hop over a dive (timed: just before it arrives).
  if (threat.dive && b.grounded && threat.t < 0.35) {
    out.jump = true;
    return true;
  }
  return false;
}

/**
 * A tackle the bot has a reason for, and where to aim it: back at whoever just went for it (once
 * their own dive is spent: they are sliding or getting up, open to a hit), or at somebody who
 * comes too close (to shove them off, as players do). `kind` says which.
 */
function retaliate(
  bot: BotView,
  safe: ((x: number, z: number) => boolean) | null,
): { ux: number; uz: number; kind: 'counter' | 'space' } | null {
  const b = bot.body;
  const m = bot.mem;
  const t = bot.t;
  const landsSafe = (ux: number, uz: number) => !safe || [2, 3.5].every((k) => safe(b.pos.x + ux * k, b.pos.z + uz * k));
  // Counterattack.
  const foe = m.foe !== undefined && (m.foeUntil ?? -1) > t ? bot.others.find((o) => o.id === m.foe) : undefined;
  if (foe && !foe.dive && Math.abs(foe.pos.y - b.pos.y) < 0.8) {
    const dx = foe.pos.x + foe.vel.x * 0.2 - b.pos.x;
    const dz = foe.pos.z + foe.vel.z * 0.2 - b.pos.z;
    const d = Math.hypot(dx, dz);
    if (d > 0.5 && d < 3.4 && landsSafe(dx / d, dz / d)) return { ux: dx / d, uz: dz / d, kind: 'counter' };
  }
  // Personal space: the nearest bean closing in (or already right on top of the bot).
  let near: { ux: number; uz: number; d: number } | null = null;
  for (const o of bot.others) {
    if (o.down || o.dive || Math.abs(o.pos.y - b.pos.y) > 0.8) continue;
    const dx = o.pos.x - b.pos.x;
    const dz = o.pos.z - b.pos.z;
    const d = Math.hypot(dx, dz);
    if (d > 1.8 || d < 1e-3 || (near && d > near.d)) continue;
    const closing = -((o.vel.x - b.vel.x) * dx + (o.vel.z - b.vel.z) * dz) / d;
    if (closing > 0.3 || d < 1.2) near = { ux: dx / d, uz: dz / d, d };
  }
  if (near && landsSafe(near.ux, near.uz)) return { ux: near.ux, uz: near.uz, kind: 'space' };
  return null;
}

/**
 * Somebody worth tackling with a dive: within a dive's range where they will be in a moment, near
 * the way the bot is going, on the same level, with safe ground to land on. Returns the direction.
 */
function tackleAim(bot: BotView, out: BotInput, safe: ((x: number, z: number) => boolean) | null) {
  const b = bot.body;
  const [hx, hz] = heading(bot, out);
  let best: { ux: number; uz: number; d: number } | null = null;
  for (const o of bot.others) {
    if (o.down || o.dive || Math.abs(o.pos.y - b.pos.y) > 0.8) continue;
    // Where they will be when the dive gets there (it covers ~3 m in a quarter of a second).
    const px = o.pos.x + o.vel.x * 0.22;
    const pz = o.pos.z + o.vel.z * 0.22;
    const dx = px - b.pos.x;
    const dz = pz - b.pos.z;
    const d = Math.hypot(dx, dz);
    if (d < 1.4 || d > 3.4) continue;
    const ux = dx / d;
    const uz = dz / d;
    // Roughly the way the bot is going: no turning round for it.
    if (ux * hx + uz * hz < 0.6) continue;
    if (safe && ![2, 3.5, 5].every((k) => safe(b.pos.x + ux * k, b.pos.z + uz * k))) continue;
    if (!best || d < best.d) best = { ux, uz, d };
  }
  return best;
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
  smartDive(bot, out, o.land);
  const nav = bot.nav;
  const gy = b.pos.y;
  const safe = o.safe ?? (nav ? (x: number, z: number) => nav.safe(x, z, gy) : null);
  // Out of the way of a tackle or a grab (on narrow ground: only by jumping).
  if (!out.dive && dodge(bot, out, o.precise ? null : safe)) return;
  // Held by somebody: that is who gets it once the bot is free.
  if (b.grounded && b.slowUntil > t && b.slowK <= 0.5) {
    let d = 2.6;
    for (const x of bot.others) {
      const dx = Math.hypot(x.pos.x - b.pos.x, x.pos.z - b.pos.z);
      if (dx < d) {
        d = dx;
        bot.mem.foe = x.id;
        bot.mem.foeUntil = t + 3;
      }
    }
  }
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
  if (ahead && moving && o.avoid !== false) {
    const dx = ahead.o.pos.x - b.pos.x;
    const dz = ahead.o.pos.z - b.pos.z;
    const side = (bot.mem.off ?? 0) >= 0 ? 1 : -1;
    const k = (1.8 - ahead.d) * 0.6;
    out.mx += (-dz / ahead.d) * side * k;
    out.mz += (dx / ahead.d) * side * k;
  }
  if (o.rough !== false) {
    const aggro = bot.mem.aggro ?? 0.3;
    const shove =
      ahead && nav && b.grounded && t > 5 && ahead.d > 1.3 && ahead.d < 2.8
        ? shoveOff(bot, ahead.o, (x, z) => nav.safe(x, z, ahead.o.pos.y))
        : null;
    // Somebody at the edge right ahead: tackle them over it (the pushy ones, mostly).
    if (shove && bot.rng() < (0.1 + aggro * 0.5) * BOT_DT * 3) out.dive = true;
    else if ((bot.mem.grabUntil ?? -1) > t) out.grab = true;
    else if (ahead && ahead.d < 1.35 && b.grounded && bot.rng() < aggro * 1.2 * BOT_DT) {
      bot.mem.grabUntil = t + 0.4 + bot.rng() * 0.9;
      out.grab = true;
    } else if (b.grounded && b.state === 'normal' && t > 2) {
      const scale = o.attack ?? 1;
      // Paying back an attack, or shoving off whoever crowds the bot (now and then grabbing instead).
      const back = retaliate(bot, safe);
      const rate = back ? (back.kind === 'counter' ? 0.8 + aggro * 2.2 : 0.6 + aggro * 2.4) * scale : 0;
      if (back && bot.rng() < rate * BOT_DT) {
        if (back.kind === 'space' && bot.rng() < 0.3) {
          bot.mem.grabUntil = t + 0.4 + bot.rng() * 0.6;
          out.grab = true;
          return;
        }
        out.mx = back.ux;
        out.mz = back.uz;
        out.dive = true;
        bot.mem.aimed = 1;
        // (Counted for audits and tuning.)
        if (back.kind === 'counter') {
          bot.mem.foe = undefined;
          bot.mem.counters = (bot.mem.counters ?? 0) + 1;
        } else bot.mem.shoves = (bot.mem.shoves ?? 0) + 1;
        return;
      }
      // A tackle: dive at where somebody will be (the pushy ones, much more often).
      const aim = tackleAim(bot, out, safe);
      const keen = (0.2 + aggro * 1.5) * scale;
      if (aim && bot.rng() < keen * BOT_DT) {
        out.mx = aim.ux;
        out.mz = aim.uz;
        out.dive = true;
        bot.mem.aimed = 1;
        return;
      }
    }
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

/**
 * The stick as a hand moves it: eased towards what the brain wants instead of snapping to it each
 * decision, so small corrections back and forth do not show as twitching. Letting go is quicker.
 */
export function smoothStick(bot: BotView, out: BotInput) {
  const m = bot.mem;
  // A tackle goes where it is aimed, not where the eased stick happens to point.
  const aimed = out.dive && m.aimed === 1;
  m.aimed = 0;
  if (aimed) {
    m.smx = out.mx;
    m.smz = out.mz;
    return;
  }
  const px = m.smx ?? out.mx;
  const pz = m.smz ?? out.mz;
  const k = Math.hypot(out.mx, out.mz) < 0.15 ? 0.7 : 0.5;
  out.mx = px + (out.mx - px) * k;
  out.mz = pz + (out.mz - pz) * k;
  m.smx = out.mx;
  m.smz = out.mz;
}

/**
 * In the air: steers so that the body comes down on (tx, tz) when it falls to height ty (a pad, a
 * platform), asking for the horizontal speed that gets it there in time (air control does the rest).
 * Returns false when it cannot get down there any more (already below it): it heads straight there.
 */
export function aimLanding(bot: BotView, tx: number, ty: number, tz: number, out: BotInput): boolean {
  const b = bot.body;
  const dx = tx - b.pos.x;
  const dz = tz - b.pos.z;
  const disc = b.vel.y * b.vel.y + 2 * GRAVITY * (b.pos.y - ty);
  if (disc < 0) {
    steer(bot, tx, tz, out);
    return false;
  }
  const t = Math.max(0.08, (b.vel.y + Math.sqrt(disc)) / GRAVITY);
  // (Air control: full stick asks for a run, or for the speed the body already flies at.)
  const top = Math.max(RUN_SPEED, Math.hypot(b.vel.x, b.vel.z));
  let mx = dx / t / top;
  let mz = dz / t / top;
  const l = Math.hypot(mx, mz);
  if (l > 1) {
    mx /= l;
    mz /= l;
  }
  out.mx = mx;
  out.mz = mz;
  return true;
}

/** A pad in a chain of bounces (x may move with time). */
export interface Hop {
  x: number | ((t: number) => number);
  y: number;
  z: number;
  /** Radius of the pad, and how hard it throws up (m/s). */
  r: number;
  power: number;
}

/**
 * Bouncing across pads (mushrooms, trampolines) to a landing spot: a waypoint `drive`. On the ground
 * before the first pad it runs at it (jumping at `edge`); in the air it aims for the next pad, and
 * after each bounce for the one after, then for the landing. Returns false once on the ground past `edge`.
 */
export function hopChain(
  key: string,
  hops: readonly Hop[],
  land: { x: number; y: number; z: number },
  edge: number,
  /** On the ground at the edge: is now a good time to go (e.g. the first pad is coming)? */
  ready?: (bot: BotView) => boolean,
) {
  const hx = (h: Hop, t: number) => (typeof h.x === 'function' ? h.x(t) : h.x);
  return (bot: BotView, out: BotInput): boolean => {
    const b = bot.body;
    const p = b.pos;
    if (b.grounded) {
      if (p.z > edge + 0.5) {
        // Missed, and down in a basin with the pads: back onto the nearest one.
        if (p.y > land.y - 1) return false;
        let near = hops[0]!;
        for (const h of hops)
          if (Math.hypot(hx(h, bot.t) - p.x, h.z - p.z) < Math.hypot(hx(near, bot.t) - p.x, near.z - p.z)) near = h;
        if (Math.abs(near.y - p.y) > 1) return false;
        bot.mem[key] = hops.indexOf(near);
        steer(bot, hx(near, bot.t), near.z, out);
        return true;
      }
      bot.mem[key] = 0;
      const h = hops[0]!;
      if (ready && !ready(bot)) {
        // Wait where the first pad passes: knocked off to the side, a bot waited for a pad that
        // never comes that far.
        let lo = Number.POSITIVE_INFINITY;
        let hi = Number.NEGATIVE_INFINITY;
        for (let k = 0; k < 40; k++) {
          const x = hx(h, bot.t + k * 0.25);
          lo = Math.min(lo, x);
          hi = Math.max(hi, x);
        }
        const wx = hi - lo > 1 ? Math.min(hi - 0.5, Math.max(lo + 0.5, p.x)) : lo;
        follow(bot, wx, edge - 1.3, out);
        return true;
      }
      const x = hx(h, bot.t + 0.5);
      steer(bot, x, h.z, out);
      if (p.z > edge - 1.4) out.jump = true;
      return true;
    }
    if (b.state !== 'normal') return false;
    let i = bot.mem[key] ?? 0;
    // Just thrown up by a pad: the next one is the target.
    hops.forEach((h, j) => {
      if (b.vel.y > h.power * 0.7 && Math.hypot(p.x - hx(h, bot.t), p.z - h.z) < h.r + 1.2 && Math.abs(p.y - h.y) < 2.5)
        i = Math.max(i, j + 1);
    });
    bot.mem[key] = i;
    const h = hops[i];
    if (!h) {
      aimLanding(bot, land.x, land.y, land.z, out);
      return true;
    }
    // Where a moving pad will be by the time the body comes down to it.
    const disc = b.vel.y * b.vel.y + 2 * GRAVITY * (p.y - h.y);
    const t = disc > 0 ? (b.vel.y + Math.sqrt(disc)) / GRAVITY : 0;
    aimLanding(bot, hx(h, bot.t + t), h.y, h.z, out);
    return true;
  };
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
    if (wp.drive?.(bot, out)) {
      // Special stretches jam too (bots backing off for another run into the ones behind them).
      unstick(bot, out);
      return;
    }
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
    humanize(bot, out, { precise, fun: !precise, attack: 0.6 });
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
    // Judged from the ground the bot stands (or last stood) on: in the air everything looks unsafe.
    if (b.grounded) bot.mem.gy = b.pos.y;
    const gy = bot.mem.gy ?? b.pos.y;
    const okAt = (x: number, z: number) =>
      (!opts.safe || opts.safe(x, z, t)) && (opts.ignoreNav || !bot.nav || bot.nav.safe(x, z, gy));
    // Hunting someone: best a bean near the edge, to shove it off (or knock it into the sweeper…).
    let hunt = bot.mem.hunt !== undefined ? bot.others.find((o) => o.id === bot.mem.hunt) : undefined;
    if (hunt && (t > (bot.mem.huntUntil ?? 0) || hunt.down)) {
      hunt = undefined;
      bot.mem.hunt = undefined;
    }
    // (Everyone starts near the edge: nobody goes after anybody in the first few seconds.)
    bot.mem.nextHunt ??= 5 + bot.rng() * 6;
    if (!hunt && b.grounded && t > bot.mem.nextHunt) {
      bot.mem.nextHunt = t + 3 + bot.rng() * 6;
      if (bot.rng() < aggro * 0.7) {
        let best = -Infinity;
        for (const o of bot.others) {
          const d = Math.hypot(o.pos.x - b.pos.x, o.pos.z - b.pos.z);
          if (o.down || d > 12 || Math.abs(o.pos.y - b.pos.y) > 1) continue;
          const score = (edgeDir(o, okAt) ? 5 : 0) - d * 0.35 + bot.rng() * 2;
          if (score > best) {
            best = score;
            hunt = o;
          }
        }
        if (hunt) {
          bot.mem.hunt = hunt.id;
          bot.mem.huntUntil = t + 3 + bot.rng() * 3;
        }
      }
    }
    const expired = t > (bot.mem.until ?? -1e9);
    const unsafe = bot.mem.tx !== undefined && !okAt(bot.mem.tx, bot.mem.tz ?? 0);
    if (!hunt && (bot.mem.tx === undefined || (b.grounded && (expired || unsafe)))) {
      bot.mem.arrived = 0;
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
    let tx = bonus && okAt(bonus.x, bonus.z) ? bonus.x : (bot.mem.tx ?? cx);
    let tz = bonus && okAt(bonus.x, bonus.z) ? bonus.z : (bot.mem.tz ?? cz);
    // Shoving: from the inside, towards the edge. Not lined up yet: get round to the inside first.
    const shove = hunt ? shoveOff(bot, hunt, okAt) : null;
    if (hunt) {
      tx = hunt.pos.x;
      tz = hunt.pos.z;
      const e = shove ? null : edgeDir(hunt, okAt);
      if (e && okAt(hunt.pos.x - e[0] * 1.9, hunt.pos.z - e[1] * 1.9)) {
        tx = hunt.pos.x - e[0] * 1.9;
        tz = hunt.pos.z - e[1] * 1.9;
      }
    }
    const d = navTo(bot, tx, tz, out, hunt ? 1 : 0.85 + (bot.mem.spd ?? 1) * 0.15, hunt ? 1.1 : 0.9);
    if (hunt && shove) {
      // Lined up: charge straight at them, and tackle from close by (or just run them over).
      out.mx = shove.ux;
      out.mz = shove.uz;
      if (shove.d > 1.3 && shove.d < 2.7 && b.grounded && bot.rng() < (0.2 + aggro * 0.6) * BOT_DT * 6) out.dive = true;
    } else if (hunt) {
      const hd = Math.hypot(hunt.pos.x - b.pos.x, hunt.pos.z - b.pos.z);
      // Where the prey will be when a dive gets there.
      const px = hunt.pos.x + hunt.vel.x * 0.22 - b.pos.x;
      const pz = hunt.pos.z + hunt.vel.z * 0.22 - b.pos.z;
      const pd = Math.hypot(px, pz);
      if (hd < 1.4) {
        out.grab = true;
        bot.mem.grabUntil = t + 0.3;
      } else if (
        pd > 1.6 &&
        pd < 3.2 &&
        b.grounded &&
        b.state === 'normal' &&
        [2, 3.5].every((k) => okAt(b.pos.x + (px / pd) * k, b.pos.z + (pz / pd) * k)) &&
        bot.rng() < (0.3 + aggro * 1.5) * BOT_DT
      ) {
        out.mx = px / pd;
        out.mz = pz / pd;
        out.dive = true;
        bot.mem.aimed = 1;
      }
    } else if (d < (bot.mem.arrived ? 1.8 : 1)) {
      // There: potter about round the spot instead of freezing (and without leaving it and coming back).
      bot.mem.arrived = 1;
      const w = t * 0.5 + (bot.mem.ph ?? 0);
      const px = tx + Math.sin(w) * 0.6;
      const pz = tz + Math.cos(w * 1.3) * 0.6;
      if (okAt(px, pz)) steer(bot, px, pz, out, 0.3);
      else steer(bot, tx, tz, out, 0.3);
    }
    if (opts.floor && Math.hypot(out.mx, out.mz) > 0.1) avoidHoles(bot, out, opts.floor);
    if (opts.social && d < 1.5 && b.grounded && bot.rng() < 0.12 * BOT_DT) out.emote = 1 + Math.floor(bot.rng() * EMOTES);
    // jumpWhen models reaction time itself (a timing window that depends on bot.mem.react). Just
    // before landing counts too: the jump is kept for a moment and goes off on touchdown.
    const landing = !b.grounded && b.vel.y < 0 && b.pos.y - gy < 0.35;
    if (opts.jumpWhen?.(bot) && (b.grounded || landing)) out.jump = true;
    humanize(bot, out, { fun: !opts.jumpWhen, rough: !hunt, avoid: !hunt, safe: okAt });
    unstick(bot, out);
  };
}
