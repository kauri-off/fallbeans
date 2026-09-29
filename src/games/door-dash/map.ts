import * as THREE from 'three';
import { clone } from '../../client/engine/assets';
import { PAL } from '../../client/world/builder';
import { pathBrain, type Waypoint } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import { AtEvent } from '../../shared/game';
import meta from './meta';

export default defineMap(meta, (b, ctx) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 26, 18, 2, 38, PAL.blue);
  b.rails(7, 45, 9);

  interface Door {
    obj: THREE.Object3D;
    breakable: boolean;
    broken: boolean;
    t: number;
    col: ReturnType<typeof b.collider>;
  }
  const doors: Door[] = [];
  const openX: number[] = [];
  for (const [z, nBreak] of [
    [14, 3],
    [24, 2],
    [34, 2],
  ] as const) {
    const idx = [0, 1, 2, 3, 4].sort(() => b.rng() - 0.5).slice(0, nBreak);
    openX.push(-6.8 + (idx[0] ?? 0) * 3.4);
    for (let i = 0; i < 5; i++) {
      const x = -6.8 + i * 3.4;
      const obj = clone('door');
      obj.position.set(x, 0, z);
      obj.scale.x = 3.4 / 3.1;
      obj.rotation.y = Math.PI;
      b.group.add(obj);
      const id = doors.length;
      const d: Door = {
        obj,
        breakable: idx.includes(i),
        broken: false,
        t: 0,
        col: b.collider(b.anchor(x, 1.6, z), { type: 'box', hx: 1.7, hy: 1.6, hz: 0.3 }, { isStatic: true }),
      };
      if (d.breakable) d.col.onTouch = (_c, _n, body) => breakDoor(id, body.actor);
      doors.push(d);
    }
    b.box(0, 3.6, z, 18.4, 0.8, 1.0, PAL.yellow);
    b.box(-9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
    b.box(9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
  }
  function breakDoor(id: number, actor: number | undefined | null) {
    const d = doors[id];
    if (!d || d.broken) return;
    d.broken = true;
    d.col.enabled = false;
    d.t = 0;
    if (actor !== null) ctx.emit('door', { i: id }, actor);
    ctx.sfx('break');
  }
  b.update((_t, dt) => {
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
  plats.forEach(([z, n, sp, high], i) => {
    b.cyl(0, -1, z, 6, 2, i % 2 ? PAL.pink : PAL.purple);
    b.hub(0, 0, z, 1);
    b.rotor(0, 0.6, z, 5.7, n, (t) => t * sp + i, 0.75);
    if (high) b.rotor(0, 2.45, z, 5.7, 1, (t) => -t * 1.1 + 1.5, 0.75);
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
  const moverX: ((t: number) => number)[] = [];
  movers.forEach(([z, sp, ph], i) => {
    const m = b.box(0, -0.5, z, 4.5, 1, 4.5, i % 2 ? PAL.orange : PAL.green, { dynamic: true });
    const fx = (t: number) => (sp === 0 ? 0 : Math.sin(t * sp + ph) * 4);
    moverX.push(fx);
    if (sp === 0)
      b.update((t) => {
        m.mesh.rotation.y = t * 0.9;
      });
    else
      b.update((t) => {
        m.mesh.position.x = fx(t);
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
  path.push({ x: 0, z: 43.5, w: 0.3 }, { x: 0, z: 49, w: 0.3 });
  for (const z of [54, 70, 86]) path.push({ x: 2.6, z: z - 3, w: 0.5, jump: true }, { x: 2.6, z: z + 3, w: 0.5, jump: true }, { x: 0, z: z + 6.5, w: 0.3 });
  path.push({ x: 0, z: 95, w: 0.4 }, { x: 0, z: 100.5, w: 0.5 });
  for (const [z] of movers) path.push({ x: 0, z, w: 0.5 });
  path.push({ x: 0, z: 136, w: 2 }, { x: 0, z: 150, w: 3 }, { x: 0, z: 172, w: 3 });

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
      if (name !== 'at') return;
      const d = AtEvent.safeParse(data);
      if (d.success) breakDoor(d.data.i, null);
    },
    bot: pathBrain(path),
  };
});
