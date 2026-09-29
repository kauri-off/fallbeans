import type { BotBrain, BotInput, BotView } from './map';

export interface Waypoint {
  x: number;
  z: number;
  w?: number;
  jump?: boolean;
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

function init(bot: BotView) {
  if (bot.mem.init) return;
  bot.mem.init = 1;
  bot.mem.spd = 0.82 + bot.rng() * 0.18;
  bot.mem.off = bot.rng() * 2 - 1;
  bot.mem.stuck = 0;
  bot.mem.react = 0.15 + bot.rng() * 0.25;
}

export function unstick(bot: BotView, out: BotInput, dt: number) {
  const moving = Math.hypot(out.mx, out.mz) > 0.3;
  const sp = Math.hypot(bot.body.vel.x, bot.body.vel.z);
  bot.mem.stuck = moving && sp < 1.2 && bot.body.grounded ? (bot.mem.stuck ?? 0) + dt : 0;
  if ((bot.mem.stuck ?? 0) > 0.45) {
    out.jump = true;
    if ((bot.mem.stuck ?? 0) > 1.4) {
      out.mx += bot.rng() - 0.5;
      bot.mem.stuck = 0;
    }
  }
}

export function pathBrain(points: readonly Waypoint[], opts: { dt?: number; diveChance?: number } = {}): BotBrain {
  const dt = opts.dt ?? 1 / 20;
  return (bot, out) => {
    init(bot);
    const b = bot.body;
    let i = bot.mem.wp ?? -1;
    const prev = points[i - 1];
    if (i < 0 || (prev && b.pos.z < prev.z - 4)) {
      i = points.findIndex((p) => p.z > b.pos.z - 0.5);
      if (i < 0) i = points.length - 1;
    }
    let wp = points[i];
    while (wp && (b.pos.z > wp.z + 0.3 || Math.hypot(wp.x - b.pos.x, wp.z - b.pos.z) < 1.2) && i < points.length - 1) {
      i++;
      wp = points[i];
    }
    bot.mem.wp = i;
    if (!wp) return;
    const tx = wp.x + (bot.mem.off ?? 0) * (wp.w ?? 1.5);
    const d = steer(bot, tx, wp.z, out, bot.mem.spd);
    if (wp.jump && d < 2.6 && b.grounded) out.jump = true;
    if (opts.diveChance && b.grounded && bot.rng() < opts.diveChance * dt) out.dive = true;
    unstick(bot, out, dt);
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

export function arenaBrain(opts: ArenaOpts, dt = 1 / 20): BotBrain {
  const cx = opts.x ?? 0;
  const cz = opts.z ?? 0;
  return (bot, out) => {
    init(bot);
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
    if (opts.jumpWhen?.(bot) && bot.body.grounded) {
      if ((bot.mem.jumpDelay ?? -1) < 0) bot.mem.jumpDelay = bot.mem.react ?? 0.2;
    }
    if ((bot.mem.jumpDelay ?? -1) >= 0) {
      bot.mem.jumpDelay = (bot.mem.jumpDelay ?? 0) - dt;
      if ((bot.mem.jumpDelay ?? 0) < 0) {
        out.jump = true;
        bot.mem.jumpDelay = -1;
      }
    }
  };
}
