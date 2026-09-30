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
/** Tail holders run at this share of full speed (the chasers are a little quicker). */
const TAIL_SLOW = 0.86;

export default defineMap(meta, (b, ctx) => {
  b.style.pattern = 'dots';
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
  // Between the spawn points (which sit at 22.5° + k·45°), never on top of one: two small sweepers
  // (their arms stop short of the spawns), trampolines up to floating islands, a pair of portals.
  for (const sz of [-1, 1]) {
    b.hub(0, 0, sz * 11.6, 0.6);
    b.rotor(0, 0.6, sz * 11.6, 2.9, 2, (t) => (t <= 0 ? sz * 0.8 : sz * (0.8 + t * 1.1)), 0.7);
  }
  for (const sx of [-1, 1]) {
    b.trampoline(sx * 12.3, 0, 0, 1.5, 18);
    // A floating island beyond the rim (a refuge, until someone bounces after you).
    b.cyl(sx * 18.2, 3.2, 0, 3, 1.2, PAL.green, { surface: 'grass' });
    b.prop('mushroom', sx * 19, 3.8, 1.2, { scale: 0.8 });
  }
  const rng = b.rng;
  const flipP = rng() < 0.5 ? 1 : -1;
  b.portal(
    { x: 9.9 * flipP, y: 0, z: 9.9, yaw: Math.atan2(-9.9 * flipP, -9.9) },
    { x: -9.9 * flipP, y: 0, z: -9.9, yaw: Math.atan2(9.9 * flipP, 9.9) },
    '#ff8a3d',
  );
  // Two platforms circling just outside the rim: hop on, ride round, hop off somewhere else.
  const orbit = 0.22 + rng() * 0.08;
  for (const k of [0, 1]) {
    const pl = b.box(0, -0.5, 0, 3.2, 1, 3.2, k ? PAL.orange : PAL.pink, { dynamic: true });
    const ph = k * Math.PI + rng();
    b.move((t) => {
      const a = ph + Math.max(0, t) * orbit;
      pl.obj.position.set(Math.cos(a) * 17.6, -0.5, Math.sin(a) * 17.6);
      pl.obj.rotation.y = -a;
    });
  }
  for (const [x, z] of [
    [0, 0],
    [7, -4],
    [-7, 4],
  ] as const)
    b.bonus(x, x ? 0 : 1.5, z);
  b.clouds(0, 0, 45);

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
      // Tails weigh you down a little: the chasers can catch up.
      for (const id of tails) {
        const body = ctx.bodies().get(id);
        if (!body) continue;
        body.slowK = t < body.slowUntil ? Math.min(body.slowK, TAIL_SLOW) : TAIL_SLOW;
        body.slowUntil = Math.max(body.slowUntil, t + 0.25);
      }
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
      // (The old tail holder's slow-down wears off by itself within a quarter of a second.)
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
        // A new way out now and then, or when the chaser is close and the current one leads towards it.
        const fx = (bot.mem.fx ?? p.x) - p.x;
        const fz = (bot.mem.fz ?? p.z) - p.z;
        const towards = fx * (near.pos.x - p.x) + fz * (near.pos.z - p.z) > 0;
        if (bot.t > (bot.mem.fleeAt ?? -1) || (nd < 3 && towards && bot.t > (bot.mem.fledAt ?? -1) + 0.3)) {
          bot.mem.fledAt = bot.t;
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
      humanize(bot, out, { rough: false, fun: nd > 6, avoid: mine });
      unstick(bot, out);
    },
  };
});
