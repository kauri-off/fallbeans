import * as THREE from 'three';
import type { Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import {
  gloveAlley,
  pickSections,
  pistons,
  raceCourse,
  type Segment,
  slidingGates,
  tippingBridge,
  withRests,
} from '../../sim/course';
import { defineMap } from '../../sim/map';
import { rollingBalls, yOnRamp } from '../../sim/props';
import meta from './meta';

/**
 * An icy slope you cannot run up: only the zig-zag carpet strips give grip, balls roll down across
 * them and blocks slide across your way. Slip off and you slide back down.
 */
function iceSlope(): Segment {
  return (s) => {
    const { b, rng } = s;
    const A0 = { z: s.z + 2, y: s.y };
    const A1 = { z: s.z + 42, y: s.y + 10 };
    const W = 20;
    const yA = (zz: number) => yOnRamp(zz, A0.z, A0.y, A1.z, A1.y);
    b.box(0, s.y - 1, s.z + 1, 18, 2, 2, PAL.purple);
    const ang = Math.atan2(A1.y - A0.y, A1.z - A0.z);
    const cosA = Math.cos(ang);
    const ice = b.view?.plain('#d6f2ff', { roughness: 0.08, metalness: 0.05 }, 'ice');
    b.ramp(0, A0.z, A0.y, A1.z, A1.y, W, PAL.blue, 1, { material: ice, slip: 1 });
    const lenA = Math.hypot(A1.z - A0.z, A1.y - A0.y);
    for (const sx of [-1, 1])
      b.box(sx * (W / 2 + 0.4), (A0.y + A1.y) / 2 + 0.6, (A0.z + A1.z) / 2, 0.8, 1.2, lenA, PAL.pink, { rot: [-ang, 0, 0] });
    // Carpet corners: a zig-zag, mirrored or not by the seed.
    const flip = rng() < 0.5 ? -1 : 1;
    const carpet: [number, number][] = [
      [0, A0.z + 0.5],
      [-7 * flip, A0.z + 6],
      [7 * flip, A0.z + 16],
      [-7 * flip, A0.z + 26],
      [6 * flip, A0.z + 35],
      [0, A1.z],
    ];
    const CW = 3.2;
    for (let i = 0; i < carpet.length - 1; i++) {
      const [x0, z0] = carpet[i]!;
      const [x1, z1] = carpet[i + 1]!;
      const dzSlope = (z1 - z0) / cosA;
      const len = Math.hypot(x1 - x0, dzSlope) + CW * 0.7;
      const yaw = Math.atan2(x1 - x0, dzSlope);
      const cz = (z0 + z1) / 2;
      b.box((x0 + x1) / 2, yA(cz) + 0.1 / cosA, cz, CW, 0.2, len, i % 2 ? PAL.green : PAL.yellow, {
        rot: [-ang, yaw, 0],
        freq: 0.8,
        surface: 'carpet',
      });
    }
    const lanes = [-5.5, 0, 5.5];
    const balls = rollingBalls(b, {
      lanes,
      zTop: A1.z - 1,
      yTop: A1.y,
      zBottom: A0.z + 1,
      yBottom: A0.y,
      radius: 1.1,
      speed: (t) => 9 + t * 0.03,
      period: 4 + rng() * 1,
      perLane: 1,
    });
    // Blocks sliding across the slope over the carpets: wait for one to pass.
    const blocks = [A0.z + 11, A0.z + 21, A0.z + 31].map((bz) => ({ z: bz, w: 0.7 + rng() * 0.5, ph: rng() * 6 }));
    const blockX = (k: (typeof blocks)[number], t: number) => Math.sin(t * k.w + k.ph) * (W / 2 - 1.6);
    for (const k of blocks) {
      const anchor = b.anchor(0, yA(k.z), k.z);
      anchor.rotation.x = -ang;
      const blk = b.box(0, 0.25 + 0.2 + 0.8, 0, 1.8, 1.6, 1.8, PAL.orange, {
        parent: anchor,
        dynamic: true,
        hit: 0.6,
        tag: 'block',
      });
      b.move((t) => {
        blk.obj.position.x = blockX(k, t);
      });
    }
    b.bonus(0, yA(A0.z + 20), A0.z + 20);
    b.box(0, A1.y - 1, A1.z + 4, W, 2, 8, PAL.purple);

    const path: Waypoint[] = [{ x: 0, z: s.z + 1, w: 1 }];
    for (let i = 1; i < carpet.length; i++) {
      const [x0, z0] = carpet[i - 1]!;
      const [x1, z1] = carpet[i]!;
      const segLen = Math.hypot(x1 - x0, z1 - z0);
      // Where the carpet crosses a ball lane or a block's track: stop before it, go when clear.
      const stops: { f: number; clear: (t: number) => boolean }[] = [];
      for (const lane of lanes) {
        if ((lane - x0) * (lane - x1) >= 0) continue;
        const f = (lane - x0) / (x1 - x0);
        const zz = z0 + (z1 - z0) * f;
        stops.push({ f, clear: (t) => !balls.danger(lane - 1.5, lane + 1.5, zz - 2.5, zz + 2.5, t, 1.1) });
      }
      for (const k of blocks) {
        if ((k.z - z0) * (k.z - z1) >= 0) continue;
        const f = (k.z - z0) / (z1 - z0);
        const xx = x0 + (x1 - x0) * f;
        stops.push({ f, clear: (t) => [0, 0.3, 0.6, 0.9].every((dt) => Math.abs(blockX(k, t + dt) - xx) > 3) });
      }
      stops.sort((a, c) => a.f - c.f);
      for (const st of stops) {
        const before = Math.max(0, st.f - 2.4 / segLen);
        path.push({ x: x0 + (x1 - x0) * before, z: z0 + (z1 - z0) * before, w: 0.2 });
        path.push({ x: x0 + (x1 - x0) * st.f, z: z0 + (z1 - z0) * st.f, w: 0.2, wait: (bot) => st.clear(bot.t) });
      }
      path.push({ x: x1, z: z1, w: 0.2 });
    }
    path.push({ x: 0, z: A1.z + 4, w: 1 });
    return {
      z: A1.z + 8,
      y: A1.y,
      routes: [path],
      forbidden: (p) => p.z > A0.z && p.z < A1.z && (Math.abs(p.x) > W / 2 + 0.05 || p.y > yA(p.z) + 2.6),
      checkpoint: { from: A1.z + 0.5, p: new THREE.Vector3(0, A1.y + 0.1, A1.z + 4) },
    };
  };
}

/** A grippy ramp with balls in two lanes and blocks on the strip between them: step aside when no ball comes. */
function ballRamp(rise = 6): Segment {
  return (s) => {
    const { b, rng } = s;
    const z0 = s.z;
    const len = 28;
    const z1 = z0 + len;
    const yR = (zz: number) => yOnRamp(zz, z0, s.y, z1, s.y + rise);
    b.ramp(0, z0, s.y, z1, s.y + rise, 9, PAL.blue);
    const ang = Math.atan2(rise, len);
    for (const sx of [-1, 1])
      b.box(sx * 4.9, s.y + rise / 2 + 0.6, (z0 + z1) / 2, 0.8, 1.2, Math.hypot(len, rise), PAL.pink, { rot: [-ang, 0, 0] });
    const balls = rollingBalls(b, {
      lanes: [-2.7, 2.7],
      zTop: z1 - 1,
      yTop: s.y + rise,
      zBottom: z0 + 1,
      yBottom: s.y,
      radius: 1,
      speed: (t) => 8.5 + t * 0.02,
      period: 3.2 + rng() * 0.8,
      perLane: 1,
    });
    const blocks = [z0 + 7, z0 + 14, z0 + 21];
    for (const [i, zb] of blocks.entries())
      b.box(0, yR(zb) + 0.6, zb, 1.8, 1.8, 1.4, i % 2 ? PAL.orange : PAL.yellow, { rot: [-ang, 0, 0] });
    b.box(0, s.y + rise - 1, z1 + 3, 16, 2, 6, PAL.purple);
    const pts: Waypoint[] = [{ x: 0, z: z0 + 0.5, w: 0 }];
    blocks.forEach((zb, i) => {
      const lx = (i % 2 ? 1 : -1) * 1.55;
      pts.push({ x: 0, z: zb - 2.3, w: 0 });
      pts.push({
        x: lx,
        z: zb - 0.8,
        w: 0,
        wait: (bot) => !balls.danger(Math.min(0, lx) - 0.3, Math.max(0, lx) + 0.3, zb - 2.5, zb + 2.5, bot.t, 0.9),
      });
      pts.push({ x: lx, z: zb + 0.9, w: 0 }, { x: 0, z: zb + 2.3, w: 0 });
    });
    pts.push({ x: 0, z: z1 + 3, w: 1 });
    return {
      z: z1 + 6,
      y: s.y + rise,
      routes: [pts],
      forbidden: (p) => p.z > z0 && p.z < z1 && Math.abs(p.x) > 4.4,
      checkpoint: { from: z1 + 0.5, p: new THREE.Vector3(0, s.y + rise + 0.1, z1 + 3) },
    };
  };
}

export default defineMap(meta, (b, ctx) => {
  b.style.pattern = 'waves';
  const middle = pickSections(b.rng, [ballRamp(6), pistons(3), tippingBridge(5), gloveAlley(3)], 2);
  return raceCourse(b, ctx, { sections: withRests([iceSlope(), ...middle, slidingGates(4, 6)]) });
});
