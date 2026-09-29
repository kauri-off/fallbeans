import * as THREE from 'three';
import { z } from 'zod';
import { shuffle } from '../../shared/rng';
import { pathBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import { armContactEta } from '../../sim/props';
import meta from './meta';

const DoorEvent = z.object({ i: z.number().int().min(0).max(63) });

export default defineMap(meta, (b, ctx) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 26, 18, 2, 38, PAL.blue);
  b.rails(7, 45, 9);

  interface Door {
    obj: THREE.Object3D;
    breakable: boolean;
    broken: boolean;
    t: number;
    col: Collider;
  }
  const doors: Door[] = [];
  const openX: number[] = [];
  for (const [z, nBreak] of [
    [14, 3],
    [24, 2],
    [34, 2],
  ] as const) {
    const idx = shuffle([0, 1, 2, 3, 4], b.rng).slice(0, nBreak);
    openX.push(-6.8 + (idx[0] ?? 0) * 3.4);
    for (let i = 0; i < 5; i++) {
      const x = -6.8 + i * 3.4;
      const obj = b.model('door');
      obj.position.set(x, 0, z);
      obj.scale.x = 3.4 / 3.1;
      obj.rotation.y = Math.PI;
      const id = doors.length;
      const d: Door = {
        obj,
        breakable: idx.includes(i),
        broken: false,
        t: 0,
        col: b.collider(b.anchor(x, 1.6, z), { type: 'box', hx: 1.7, hy: 1.6, hz: 0.3 }, { isStatic: true }),
      };
      if (d.breakable)
        d.col.onTouch = () => {
          // The server decides; the local player's prediction breaks it right away too.
          if (ctx.server) ctx.emit('door', { i: id });
          else breakDoor(id);
        };
      doors.push(d);
    }
    b.box(0, 3.6, z, 18.4, 0.8, 1.0, PAL.yellow);
    b.box(-9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
    b.box(9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
  }
  function breakDoor(id: number) {
    const d = doors[id];
    if (!d || d.broken || !d.breakable) return;
    d.broken = true;
    d.col.enabled = false;
    d.t = 0;
    ctx.sfx('break');
  }
  b.anim((_t, dt) => {
    for (const d of doors) {
      if (!d.broken || !d.obj.visible) continue;
      d.t += dt;
      d.obj.rotation.x = Math.min(Math.PI / 2, d.t * d.t * 7);
      if (d.t > 1.4) d.obj.visible = false;
    }
  });

  b.box(0, -1, 46.5, 3.6, 2, 5, PAL.yellow);
  const plats = [
    [54, 2, 1.5, 0],
    [70, 3, -1.7, 0],
    [86, 2, 1.9, 1],
  ] as const;
  const highAngle = (t: number) => -t * 1.1 + 1.5;
  plats.forEach(([z, n, sp, high], i) => {
    b.cyl(0, -1, z, 6, 2, i % 2 ? PAL.pink : PAL.purple);
    b.hub(0, 0, z, 1);
    b.rotor(0, 0.6, z, 5.7, n, (t) => t * sp + i, 0.75);
    if (high) b.rotor(0, 2.45, z, 5.7, 1, highAngle, 0.75);
    if (i < 2) b.box(0, -1, z + 8, 3.6, 2, 6, PAL.yellow);
  });
  b.box(0, -1, 93.5, 3.6, 2, 5, PAL.yellow);

  b.box(0, -1, 97, 10, 2, 6, PAL.purple);
  const movers = [
    [104, 0.9, 0],
    [110.5, 1.1, 2],
    [117, 0, 0],
    [123.5, 1.3, 4],
    [130, 1.0, 1],
  ] as const;
  const moverX = (sp: number, ph: number) => (t: number) => (sp === 0 ? 0 : Math.sin(t * sp + ph) * 4);
  movers.forEach(([z, sp, ph], i) => {
    const m = b.box(0, -0.5, z, 4.5, 1, 4.5, i % 2 ? PAL.orange : PAL.green, { dynamic: true });
    const fx = moverX(sp, ph);
    if (sp === 0)
      b.move((t) => {
        m.obj.rotation.y = t * 0.9;
      });
    else
      b.move((t) => {
        m.obj.position.x = fx(t);
      });
  });
  b.box(0, -1, 139, 12, 2, 10, PAL.purple);

  b.ramp(0, 144, 0, 164, 4, 12, PAL.blue);
  const ang = Math.atan2(4, 20);
  b.box(-6.3, 2.6, 154, 0.8, 1.2, 20.4, PAL.pink, { rot: [-ang, 0, 0] });
  b.box(6.3, 2.6, 154, 0.8, 1.2, 20.4, PAL.pink, { rot: [-ang, 0, 0] });
  for (const [x, z] of [
    [-3, 148],
    [3, 152],
    [-1, 157],
    [4, 160],
    [-4, 161],
  ] as const)
    b.bumper(x, ((z - 144) / 20) * 4 - 0.1, z, 0.9, 11);
  b.box(0, 3, 172, 18, 2, 16, PAL.yellow);
  b.finish(0, 4, 170);
  b.clouds(0, 90, 70, 40);

  const path: Waypoint[] = [{ x: 0, z: 9, w: 5 }];
  [14, 24, 34].forEach((z, i) => {
    path.push({ x: openX[i] ?? 0, z: z - 2, w: 0.4 }, { x: openX[i] ?? 0, z: z + 1.5, w: 0.8 });
  });
  path.push({ x: 0, z: 43.5, w: 0.3 });
  plats.forEach(([z, n, sp, high], i) => {
    // Jump the low arm as it comes (in reach of it only); never into the high one.
    const jumpWhen = (bot: BotView) => {
      const p = bot.body.pos;
      const r = Math.hypot(p.x, p.z - z);
      if (r > 7 || r < 1.2) return false;
      const eta = armContactEta(bot, bot.t * sp + i, sp, n, 0, z);
      return eta > 0.06 && eta < 0.2 && (!high || armContactEta(bot, highAngle(bot.t), -1.1, 1, 0, z) > 0.8);
    };
    path.push(
      { x: 0, z: z - 5.5, w: 0.3, jumpWhen },
      { x: 2.6, z: z - 3, w: 0.3, jumpWhen },
      { x: 2.6, z: z + 3, w: 0.3, jumpWhen },
      { x: 0, z: z + 6.5, w: 0.3, jumpWhen },
    );
  });
  path.push({ x: 0, z: 95, w: 0.4 }, { x: 0, z: 99, w: 0.2 });
  // Moving platforms: wait until the next one lines up, then jump across the gap.
  let edge = 100;
  for (const [z, sp, ph] of movers) {
    const fx = moverX(sp, ph);
    path.push({
      x: fx,
      z,
      wait: (bot) => Math.abs(fx(bot.t + 0.6) - bot.body.pos.x) < 1.2,
      jumpWhen: (
        (e) => (bot: BotView) =>
          bot.body.pos.z > e - 1.0 && bot.body.pos.z < e + 0.4
      )(edge),
    });
    edge = z + 2.25;
  }
  path.push({ x: 0, z: 136, w: 1, jumpWhen: (bot) => bot.body.pos.z > edge - 1.0 && bot.body.pos.z < edge + 0.4 });
  path.push({ x: 0, z: 150, w: 3 }, { x: 0, z: 172, w: 3 });

  return {
    spawns,
    killY: -14,
    finish: { z: 170, y: 3 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 45, p: new THREE.Vector3(0, 0.1, 46) },
      { z: 94, p: new THREE.Vector3(0, 0.1, 97) },
      { z: 134, p: new THREE.Vector3(0, 0.1, 138) },
    ],
    onEvent(name, data) {
      if (name !== 'door') return;
      const d = DoorEvent.safeParse(data);
      if (d.success) breakDoor(d.data.i);
    },
    bot: pathBrain(path),
  };
});
