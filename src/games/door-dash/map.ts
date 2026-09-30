import * as THREE from 'three';
import { z } from 'zod';
import { shuffle } from '../../shared/rng';
import { BOT_DT, humanize, initBot, navTo, pathBrain, steer, unstick, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotBrain, type BotInput, type BotView, defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import { armContactEta, glovePuncher } from '../../sim/props';
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
  const ROWS = [14, 24, 34, 42] as const;
  const doorX = (i: number) => -6.8 + i * 3.4;
  for (const [z, nBreak] of [
    [14, 2],
    [24, 2],
    [34, 2],
    [42, 2],
  ] as const) {
    const idx = shuffle([0, 1, 2, 3, 4], b.rng).slice(0, nBreak);
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
        col: b.collider(b.anchor(x, 1.6, z), { type: 'box', hx: 1.7, hy: 1.6, hz: 0.3 }, { isStatic: true, navSkip: true }),
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
    [54, 3, 1.5, 0],
    [70, 3, -1.8, 0],
    [86, 2, 2.0, 1],
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
    [104, 1.0, 0],
    [110.5, 1.25, 2],
    [117, 0, 0],
    [123.5, 1.45, 4],
    [130, 1.15, 1],
  ] as const;
  const moverX = (sp: number, ph: number) => (t: number) => (sp === 0 ? 0 : Math.sin(t * sp + ph) * 4);
  movers.forEach(([z, sp, ph], i) => {
    const m = b.box(0, -0.5, z, 4.5, 1, 4.5, i % 2 ? PAL.orange : PAL.green, { dynamic: true });
    const fx = moverX(sp, ph);
    if (sp === 0)
      b.move((t) => {
        m.obj.rotation.y = t * 1.15;
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
  // Gloves punching out of the ramp's rails: a knock back down the slope.
  for (const [z, side, w, ph] of [
    [150.5, -1, 1.2, 0],
    [155.5, 1, 1.05, 2.2],
  ] as const)
    glovePuncher(b, { x: side * 7.6, y: ((z - 144) / 20) * 4 + 0.95, z, side, w, ph, reach: 5.2, scale: 1.2, postTo: -2 });
  b.box(0, 3, 172, 18, 2, 16, PAL.yellow);
  b.finish(0, 4, 170);
  b.clouds(0, 90, 70, 40);

  let rest: BotBrain | undefined;
  const path: Waypoint[] = [];
  path.push({ x: 0, z: 43.5, w: 0.3 });
  plats.forEach(([z, n, sp, high], i) => {
    // Jump the low arm as it comes (in reach of it only); never into the high one.
    const jumpWhen = (bot: BotView) => {
      const p = bot.body.pos;
      const r = Math.hypot(p.x, p.z - z);
      if (r > 7 || r < 1.2) return false;
      const eta = armContactEta(bot, bot.t * sp + i, sp, n, 0, z);
      return eta > 0.1 && eta < 0.24 && (!high || armContactEta(bot, highAngle(bot.t), -1.1, 1, 0, z) > 0.8);
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
    bot: (bot, out) => {
      initBot(bot);
      rest ??= pathBrain(path);
      const p = bot.body.pos;
      const row = ROWS.findIndex((z) => p.z < z + 0.6);
      if (row < 0 || p.z < 7) return rest(bot, out);
      doorBrain(bot, out, row);
    },
  };

  /**
   * Doors like a player would: go through one somebody already broke, otherwise pick one (near
   * your line, or wherever the others are heading) and barge into it; if it holds, try the next.
   */
  function doorBrain(bot: BotView, out: BotInput, row: number) {
    const z = ROWS[row]!;
    const p = bot.body.pos;
    const key = `door${row}`;
    const tried = `tried${row}`;
    const rowDoors = doors.slice(row * 5, row * 5 + 5);
    let pick = bot.mem[key];
    const current = pick === undefined ? undefined : rowDoors[pick];
    // A door already open is the obvious choice (unless it is far off and ours may be fine).
    const open = rowDoors.map((d, i) => (d.broken ? i : -1)).filter((i) => i >= 0);
    if (current && !current.broken && open.length) {
      const nearest = open.reduce((a, i) => (Math.abs(doorX(i) - p.x) < Math.abs(doorX(a) - p.x) ? i : a));
      if (Math.abs(doorX(nearest) - p.x) < 7) pick = nearest;
    }
    if (pick === undefined) {
      const mask = bot.mem[tried] ?? 0;
      const options = [0, 1, 2, 3, 4].filter((i) => !(mask & (1 << i)));
      const pref = p.x + (bot.mem.off ?? 0) * 3;
      options.sort((a, c) => Math.abs(doorX(a) - pref) - Math.abs(doorX(c) - pref));
      pick = options[bot.rng() < 0.7 ? 0 : Math.min(options.length - 1, 1)] ?? Math.floor(bot.rng() * 5);
    }
    bot.mem[key] = pick;
    const x = doorX(pick);
    const door = rowDoors[pick]!;
    if (!door.broken && p.z > z - 1.05 && Math.abs(p.x - x) < 1.2) {
      // Pressed against it and it holds: a real door. Remember and pick another.
      bot.mem.push = (bot.mem.push ?? 0) + BOT_DT;
      if ((bot.mem.push ?? 0) > 0.25 + (bot.mem.react ?? 0.15)) {
        bot.mem[tried] = (bot.mem[tried] ?? 0) | (1 << pick);
        bot.mem[key] = undefined;
        bot.mem.push = 0;
      }
    } else bot.mem.push = 0;
    if (door.broken || p.z > z - 1.5) steer(bot, x, z + 2, out, 1);
    else navTo(bot, x, z - 1.2, out, bot.mem.spd ?? 1, 0.6);
    humanize(bot, out, { precise: p.z > z - 2.5 });
    unstick(bot, out);
  }
});
