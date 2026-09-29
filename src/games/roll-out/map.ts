import * as THREE from 'three';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import meta from './meta';

const R = 7;
const N = 20;
const CY = -R;
const STEP = (2 * Math.PI) / N;

export default defineMap(meta, (b) => {
  const rings = [
    { z: -9, dir: 1, pal: PAL.pink },
    { z: 0, dir: -1, pal: PAL.blue },
    { z: 9, dir: 1, pal: PAL.yellow },
  ].map((r) => {
    const group = b.anchor(0, CY, r.z);
    const missing = new Set<number>();
    while (missing.size < 4) {
      const k = Math.floor(b.rng() * N);
      if (k > 2 && k < N - 2) missing.add(k);
    }
    const width = ((2 * Math.PI * R) / N) * 0.96;
    for (let k = 0; k < N; k++) {
      if (missing.has(k)) continue;
      const a = (k / N) * Math.PI * 2;
      b.box(Math.sin(a) * (R - 0.25), Math.cos(a) * (R - 0.25), 0, width, 0.5, 8, k % 2 ? r.pal : PAL.white, {
        parent: group,
        rot: [0, 0, -a],
        dynamic: true,
      });
    }
    if (b.view) {
      const torus = b.view.own(new THREE.TorusGeometry(R, 0.25, 8, 48));
      for (const dz of [-4.1, 4.1]) {
        const rim = new THREE.Mesh(torus, b.view.plain('#5a3fb8'));
        rim.position.z = dz;
        group.add(rim);
      }
    }
    const angle = (t: number) => (t <= 0 ? 0 : r.dir * (0.35 * t + 0.002 * t * t));
    b.move((t) => {
      group.rotation.z = angle(t);
    });
    // Runs of missing slats as angular intervals [from, to] in slat units.
    const sorted = [...missing].sort((x, y) => x - y);
    const holes: { from: number; to: number }[] = [];
    for (const k of sorted) {
      const last = holes.at(-1);
      if (last && k - 0.5 === last.to) last.to = k + 0.5;
      else holes.push({ from: k - 0.5, to: k + 0.5 });
    }
    return { ...r, missing, angle, holes };
  });
  b.cyl(0, CY, 0, 1.2, 30, '#5a3fb8', { rot: [Math.PI / 2, 0, 0], noCollide: true });
  b.clouds(0, 0, 40, 30, -40, -10);

  const spawns = [-9, 0, 9].flatMap((z) => [-1.2, 1.2].map((x) => new THREE.Vector3(x, 0.1, z)));
  spawns.push(new THREE.Vector3(0, 0.1, -11), new THREE.Vector3(0, 0.1, 11));
  const ringAt = (z: number) => rings.reduce((best, r) => (Math.abs(r.z - z) < Math.abs(best.z - z) ? r : best));

  return {
    spawns,
    killY: -16,
    isOut: (p) => Math.hypot(p.x, p.y - CY) < R - 1.4,
    view: new THREE.Vector3(0, 2, 0),
    bot(bot, out) {
      const p = bot.body.pos;
      const ring = ringAt(p.z);
      bot.mem.spd ??= 0.85 + bot.rng() * 0.15;
      const t = Math.max(0, bot.t);
      const omega = t > 0 ? ring.dir * (0.35 + 0.004 * t) : 0;
      // The top of the drum carries us sideways at −ω·R; run against it to stay on top.
      const carry = -omega * R;
      const up = carry === 0 ? 0 : -Math.sign(carry);
      const lane = ring.z + ((bot.id % 3) - 1) * 1.5;
      let mx = Math.max(-1, Math.min(1, -p.x * 0.8 - carry / 8.5));
      const mz = Math.max(-1, Math.min(1, (lane - p.z) * 0.6));
      // Holes (runs of missing slats) come from upstream. Hop when the near edge is at our feet,
      // with just enough upstream speed to clear the hole while the surface moves under us.
      const theta = ring.angle(t);
      const phi = Math.atan2(p.x, p.y - CY);
      let hop: { near: number; width: number } | null = null;
      for (const h of ring.holes) {
        let a0 = h.from * STEP - theta - phi;
        a0 = Math.atan2(Math.sin(a0), Math.cos(a0));
        const a1 = a0 + (h.to - h.from) * STEP;
        const near = up > 0 ? a0 : -a1;
        const width = (h.to - h.from) * STEP * R;
        const dist = near * R;
        if (up !== 0 && dist > -0.3 && dist < 1.0 && (!hop || dist < hop.near)) hop = { near: dist, width };
      }
      if (hop && bot.body.grounded) {
        out.jump = true;
        mx = up * Math.min(1, Math.max(0.15, (hop.width + 1.2 - Math.abs(carry) * 0.75) / 0.75 / 8.5));
        bot.mem.hopMx = mx;
        bot.mem.hopUntil = bot.t + 0.7;
      } else if (!bot.body.grounded && (bot.mem.hopUntil ?? -1) > bot.t) mx = bot.mem.hopMx ?? mx;
      const l = Math.hypot(mx, mz) || 1;
      const k = Math.min(1, l) * (bot.mem.spd ?? 1);
      out.mx = (mx / l) * k;
      out.mz = (mz / l) * k;
    },
  };
});
