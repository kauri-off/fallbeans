import * as THREE from 'three';
import { z } from 'zod';
import { shuffle } from '../../shared/rng';
import { arenaBrain, BOT_DT, humanize, initBot, navTo, unstick } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import meta from './meta';

const TailsEvent = z.object({ ids: z.array(z.number().int()).max(16), by: z.number().int().optional() });
const STEAL_RANGE = 2.4;
const IMMUNE = 1.5;
const ARENA_R = 15;

export default defineMap(meta, (b, ctx) => {
  b.cyl(0, -1, 0, ARENA_R, 2, PAL.teal, { freq: 0.3 });
  b.cyl(0, 0.03, 0, ARENA_R + 0.05, 0.1, PAL.yellow, { noCollide: true });
  // A raised island with ramps, and a few bumpers to dodge around
  b.box(0, 0.75, 0, 6, 1.5, 6, PAL.purple);
  for (const [x, zz, ry] of [
    [0, 5, 0],
    [0, -5, Math.PI],
    [5, 0, Math.PI / 2],
    [-5, 0, -Math.PI / 2],
  ] as const) {
    const ramp = b.anchor(x, 0, zz);
    ramp.rotation.y = ry;
    // From the island's top edge (1.5 m) down to the floor 4 m further out.
    b.box(0, 0.47, 0, 4, 0.6, 4.5, PAL.pink, { parent: ramp, rot: [Math.atan2(1.5, 4), 0, 0] });
  }
  // Between the spawn points (which sit at 22.5° + k·45°), never on top of one.
  for (const a of [0, 0.5, 1, 1.5]) b.bumper(Math.cos(a * Math.PI) * 10, 0, Math.sin(a * Math.PI) * 10, 1, 12);
  b.clouds(0, 0, 40);

  // Initial tails follow from the seed and the participant order, identical everywhere.
  const order = shuffle([...ctx.participants], b.rng);
  const n = Math.max(1, Math.min(order.length - 1, Math.ceil(order.length / 2)));
  let tails = new Set(order.slice(0, n));
  const immune = new Map<number, number>();
  const arenaWander = arenaBrain({ radius: ARENA_R - 4 });
  const decorateAll = () => {
    for (const id of ctx.participants) ctx.decorate(id, { tail: tails.has(id) });
  };
  decorateAll();
  let lastSecond = 0;

  return {
    spawns: b.ringSpawns(8, 9, 0.1, Math.PI / 8),
    killY: -10,
    faceCenter: true,
    view: new THREE.Vector3(0, 1, 0),
    tick(t) {
      if (t < 0) return;
      const s = Math.floor(t);
      if (s === lastSecond) return;
      lastSecond = s;
      for (const id of tails) if (ctx.bodies().has(id)) ctx.setScore(id, ctx.score(id) + 1);
    },
    onGrab(actor, target) {
      const t = ctx.now();
      if (t < 0 || tails.has(actor) || !tails.has(target)) return;
      if ((immune.get(target) ?? -1) > t) return;
      const a = ctx.bodies().get(actor);
      const v = ctx.bodies().get(target);
      if (!a || !v) return;
      if (Math.hypot(a.pos.x - v.pos.x, a.pos.z - v.pos.z) > STEAL_RANGE || Math.abs(a.pos.y - v.pos.y) > 2) return;
      const next = new Set(tails);
      next.delete(target);
      next.add(actor);
      immune.set(actor, t + IMMUNE);
      ctx.emit('tails', { ids: [...next], by: actor });
    },
    onEvent(name, data) {
      if (name !== 'tails') return;
      const d = TailsEvent.safeParse(data);
      if (!d.success) return;
      const before = tails.has(ctx.me());
      tails = new Set(d.data.ids);
      if (d.data.by !== undefined) immune.set(d.data.by, ctx.now() + IMMUNE);
      decorateAll();
      if (before !== tails.has(ctx.me())) ctx.sfx('steal');
    },
    hud() {
      const me = ctx.me();
      const pts = ctx.score(me);
      return tails.has(me) ? `У вас хвост — убегайте! Очки: ${pts}` : `Отнимите чужой хвост (Q или ПКМ). Очки: ${pts}`;
    },
    bot(bot, out) {
      initBot(bot);
      const p = bot.body.pos;
      const mine = tails.has(bot.id);
      let near: { id: number; pos: THREE.Vector3 } | undefined;
      let nd = 1e9;
      for (const o of bot.others) {
        if (tails.has(o.id) === mine) continue;
        const d = Math.hypot(o.pos.x - p.x, o.pos.z - p.z);
        if (d < nd) {
          nd = d;
          near = o;
        }
      }
      if (!near) {
        arenaWander(bot, out);
        return;
      }
      if (mine) {
        // Run away (round the island and the bumpers), staying well inside the arena.
        if (bot.t > (bot.mem.fleeAt ?? -1) || nd < 3) {
          let best = -Infinity;
          for (let k = 0; k < 8; k++) {
            const a = bot.rng() * Math.PI * 2;
            const r = 3 + bot.rng() * (ARENA_R - 6);
            const x = Math.cos(a) * r;
            const z = Math.sin(a) * r;
            let score = Math.hypot(x - near.pos.x, z - near.pos.z) - Math.hypot(x - p.x, z - p.z) * 0.35;
            if (bot.nav && !bot.nav.safe(x, z, p.y)) score -= 50;
            if (score > best) {
              best = score;
              bot.mem.fx = x;
              bot.mem.fz = z;
            }
          }
          bot.mem.fleeAt = bot.t + 0.8 + bot.rng() * 0.8;
        }
        navTo(bot, bot.mem.fx ?? 0, bot.mem.fz ?? 0, out, nd < 8 ? 1 : 0.7);
        // A chaser right behind: a hop or a dive to get away.
        if (nd < 2.2 && bot.body.grounded && bot.rng() < 0.25) out.jump = true;
      } else {
        // Chase, cutting the corner towards where the tail is going.
        const o = bot.others.find((x) => x.id === near.id);
        const lead = Math.min(0.6, nd / 12);
        const tx = near.pos.x + (o?.vel.x ?? 0) * lead;
        const tz = near.pos.z + (o?.vel.z ?? 0) * lead;
        navTo(bot, tx, tz, out, bot.mem.spd ?? 1, 1);
        out.grab = nd < 1.9;
        const ahead = Math.hypot(tx - p.x, tz - p.z);
        if (ahead < 4.5 && ahead > 2.6 && bot.body.grounded && bot.rng() < (0.3 + (bot.mem.aggro ?? 0.3)) * BOT_DT * 2)
          out.dive = true;
      }
      humanize(bot, out, { rough: false, fun: nd > 6 });
      unstick(bot, out);
    },
  };
});
