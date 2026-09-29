import type { BotBrain, BotInput, BotView } from './map';

/** Bot brains run at 20 Hz (BOT_EVERY ticks); timers below use this step. */
export const BOT_DT = 1 / 20;

export interface Waypoint {
  /** Target x, or a function of time for moving targets. */
  x: number | ((t: number) => number);
  z: number;
  /** Lateral spread between bots (static x only). */
  w?: number;
  /** Jump when within 2.6 m. */
  jump?: boolean;
  /** Jump whenever this says so (e.g. a rotor arm is about to sweep by). */
  jumpWhen?: (bot: BotView) => boolean;
  /** Stand still until this is true (e.g. the path ahead is clear). */
  wait?: (bot: BotView) => boolean;
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
  const mx = Math.max(-1, Math.min(1, (tx - b.pos.x) * 2.5));
  const mz = Math.abs(dz) < 0.2 ? 0 : Math.sign(dz) * Math.max(0.2, 1 - Math.abs(mx));
  const l = Math.hypot(mx, mz) || 1;
  const k = Math.min(1, d / 0.8 + 0.2);
  out.mx = (mx / l) * k;
  out.mz = (mz / l) * k;
  return d;
}

/** Per-bot personality: speed, lateral offset, reaction time. */
export function initBot(bot: BotView) {
  if (bot.mem.init) return;
  bot.mem.init = 1;
  bot.mem.spd = 0.82 + bot.rng() * 0.18;
  bot.mem.off = bot.rng() * 2 - 1;
  bot.mem.stuck = 0;
  bot.mem.react = 0.15 + bot.rng() * 0.25;
}

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
    // Once a wait is over the bot commits to that waypoint (no turning back halfway).
    if (wp.wait && bot.mem.go !== i && wp.wait(bot)) bot.mem.go = i;
    if (wp.wait && bot.mem.go !== i) {
      // Hold at the previous waypoint (keeps a corridor position exact).
      const hold = points[i - 1];
      if (hold) follow(bot, targetX(hold), hold.z, out);
      else {
        out.mx = 0;
        out.mz = 0;
      }
      if (wp.jumpWhen?.(bot) && b.grounded) out.jump = true;
      return;
    }
    const tx = targetX(wp);
    const d = wp.w === 0 ? follow(bot, tx, wp.z, out) : steer(bot, tx, wp.z, out, typeof wp.x === 'function' ? 1 : bot.mem.spd);
    if (wp.jump && d < 2.6 && b.grounded) out.jump = true;
    if (wp.jumpWhen?.(bot) && b.grounded) out.jump = true;
    if (opts.diveChance && b.grounded && bot.rng() < opts.diveChance * BOT_DT) out.dive = true;
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
}

export function arenaBrain(opts: ArenaOpts): BotBrain {
  const cx = opts.x ?? 0;
  const cz = opts.z ?? 0;
  return (bot, out) => {
    initBot(bot);
    const expired = bot.t > (bot.mem.until ?? -1e9);
    const unsafe = opts.safe && !opts.safe(bot.mem.tx ?? 0, bot.mem.tz ?? 0, bot.t);
    if (expired || unsafe || bot.mem.tx === undefined) {
      let tries = 0;
      do {
        const a = bot.rng() * Math.PI * 2;
        const r = Math.sqrt(bot.rng()) * opts.radius;
        bot.mem.tx = cx + Math.cos(a) * r;
        bot.mem.tz = cz + Math.sin(a) * r;
      } while (opts.safe && !opts.safe(bot.mem.tx, bot.mem.tz, bot.t) && ++tries < 12);
      bot.mem.until = bot.t + (opts.retarget ?? 2) + bot.rng() * 2;
    }
    steer(bot, bot.mem.tx ?? cx, bot.mem.tz ?? cz, out, (bot.mem.spd ?? 1) * 0.8);
    // jumpWhen models reaction time itself (a timing window that depends on bot.mem.react).
    if (opts.jumpWhen?.(bot) && bot.body.grounded) out.jump = true;
  };
}

/** Wanders the lobby playground. */
export const lobbyBrain = arenaBrain({ radius: 11, retarget: 3 });
