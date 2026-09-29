import * as THREE from 'three';
import { z } from 'zod';
import { shuffle } from '../../shared/rng';
import { initBot, steer, unstick } from '../../sim/bots';
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
    b.box(0, 0.35, 0, 4, 0.7, 4, PAL.pink, { parent: ramp, rot: [-0.34, 0, 0] });
  }
  for (const a of [0.4, 1.9, 3.3, 4.9]) b.bumper(Math.cos(a) * 10, 0, Math.sin(a) * 10, 1, 12);
  b.clouds(0, 0, 40);

  // Initial tails follow from the seed and the participant order, identical everywhere.
  const order = shuffle([...ctx.participants], b.rng);
  const n = Math.max(1, Math.min(order.length - 1, Math.ceil(order.length / 2)));
  let tails = new Set(order.slice(0, n));
  const immune = new Map<number, number>();
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
      return tails.has(me) ? `У вас хвост — убегайте! Очки: ${pts}` : `Хватайте хвост: Q / ПКМ. Очки: ${pts}`;
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
        steer(bot, Math.cos(bot.t * 0.3 + bot.id) * 8, Math.sin(bot.t * 0.3 + bot.id) * 8, out, 0.7);
        return;
      }
      if (mine) {
        // Run away, but stay inside the arena.
        let fx = p.x - near.pos.x;
        let fz = p.z - near.pos.z;
        const l = Math.hypot(fx, fz) || 1;
        fx /= l;
        fz /= l;
        const r = Math.hypot(p.x, p.z);
        if (r > ARENA_R - 4) {
          fx -= (p.x / r) * 1.5;
          fz -= (p.z / r) * 1.5;
        }
        steer(bot, p.x + fx * 4, p.z + fz * 4, out, nd < 8 ? (bot.mem.spd ?? 1) : 0.5);
      } else {
        steer(bot, near.pos.x, near.pos.z, out, bot.mem.spd ?? 1);
        out.grab = nd < 2;
        if (nd < 5 && nd > 3 && bot.body.grounded && bot.rng() < 0.05) out.dive = true;
      }
      unstick(bot, out);
    },
  };
});
