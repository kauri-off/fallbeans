import * as THREE from 'three';
import { humanize, initBot } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import meta from './meta';

const R = 7;
const N = 20;
const CY = -R;
const STEP = (2 * Math.PI) / N;

/**
 * Three huge drums with missing slats turn under you: run against the turn and hop the holes. Fall in
 * through a hole and you land inside the drum: keep running on the inside (the holes come round down
 * there too). Only falling out of a drum ends your round.
 */
export default defineMap(
  meta,
  (b) => {
    const rings = [
      { z: -9, dir: 1, pal: PAL.pink },
      { z: 0, dir: -1, pal: PAL.blue },
      { z: 9, dir: 1, pal: PAL.yellow },
    ].map((r) => {
      const group = b.anchor(0, CY, r.z);
      const missing = new Set<number>();
      // The outer drums lose one slat more than the middle one.
      while (missing.size < (r.z === 0 ? 4 : 5)) {
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
          const rim = new THREE.Mesh(torus, b.view.plain('#5a3fb8', {}, 'rubber'));
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
      return { ...r, missing, angle, holes, group };
    });
    // Spokes at the rims instead of an axle through the middle (the inside is part of the course).
    if (b.view)
      for (const r of rings)
        for (const dz of [-4.1, 4.1])
          for (let k = 0; k < 3; k++)
            b.box(0, 0, dz * 1.06, 0.3, R * 2 - 0.6, 0.3, '#5a3fb8', {
              noCollide: true,
              parent: r.group,
              rot: [0, 0, (k / 3) * Math.PI],
            });
    for (const r of rings) b.bonus(0, 0.05, r.z + 2.5);
    b.clouds(0, 0, 40, 30, -40, -10);

    const spawns = [-9, 0, 9].flatMap((z) => [-1.2, 1.2].map((x) => new THREE.Vector3(x, 0.1, z)));
    spawns.push(new THREE.Vector3(0, 0.1, -11), new THREE.Vector3(0, 0.1, 11));
    const ringAt = (z: number) => rings.reduce((best, r) => (Math.abs(r.z - z) < Math.abs(best.z - z) ? r : best));

    return {
      spawns,
      killY: -16,
      view: new THREE.Vector3(0, 2, 0),
      bot(bot, out) {
        initBot(bot);
        const p = bot.body.pos;
        const ring = ringAt(p.z);
        // Inside the drum (fallen in): the same game on the inner surface, at the bottom.
        const inside = p.y < CY;
        const RS = inside ? R - 0.5 : R;
        const mirror = inside ? -1 : 1;
        const t = Math.max(0, bot.t);
        const omega = t > 0 ? ring.dir * (0.35 + 0.004 * t) : 0;
        // The top of the drum carries us sideways at −ω·R; "up" is against it.
        const carry = -omega * RS * mirror;
        const up = carry === 0 ? 0 : -Math.sign(carry);
        const lane = ring.z + ((bot.id % 3) - 1) * 1.5 + (bot.mem.off ?? 0) * 0.4;
        // Holes as intervals along the surface, in metres towards "up" from the bot.
        const theta = ring.angle(t);
        const phi = Math.atan2(p.x, p.y - CY);
        let ahead: { near: number; far: number } | null = null;
        let behind = Infinity;
        for (const h of ring.holes) {
          let a0 = h.from * STEP - theta - phi;
          a0 = Math.atan2(Math.sin(a0), Math.cos(a0));
          const a1 = a0 + (h.to - h.from) * STEP;
          const upA = up * mirror;
          const near = upA >= 0 ? a0 * RS : -a1 * RS;
          const far = upA >= 0 ? a1 * RS : -a0 * RS;
          if (far > -0.2 && near > -0.4) {
            if (!ahead || near < ahead.near) ahead = { near, far };
          } else if (far <= -0.2) behind = Math.min(behind, -far);
        }
        // On the ground: hold our place against the carry and drift back to the crest, but never
        // towards a hole that just went by under us.
        let mx = -carry / 8.5;
        const toTop = -p.x;
        const intoBehind = up !== 0 && Math.sign(toTop) === -up && behind < 2.8;
        if (!intoBehind) mx += Math.max(-0.45, Math.min(0.45, toTop * 0.35));
        const mz = Math.max(-1, Math.min(1, (lane - p.z) * 0.6));
        // A hole coming at us: hop when its near edge reaches our feet, with just enough speed
        // relative to the surface to land past it. Worse players jump a little early or late.
        const slop = (1 - (bot.mem.skill ?? 0.7)) * 0.5;
        const late = (bot.rng() - 0.5) * slop;
        if (ahead && up !== 0 && bot.body.grounded && ahead.near < 1.0 + late && ahead.near > -0.3) {
          // Clear the near edge, the hole and a body length, minus what the drum brings us.
          const span = ahead.far - Math.max(0, ahead.near) + 1.6;
          out.jump = true;
          // …but never so far that it lands on the steep side of the drum.
          const room = Math.max(0.2, (3.2 - up * p.x * mirror) / 6.4);
          mx = up * Math.min(1, room, Math.max(0.2, (span - Math.abs(carry) * 0.75) / 0.75 / 8.5));
          bot.mem.hopMx = mx;
          bot.mem.hopUntil = bot.t + 0.7;
        } else if (!bot.body.grounded && (bot.mem.hopUntil ?? -1) > bot.t) mx = bot.mem.hopMx ?? mx;
        const l = Math.hypot(mx, mz) || 1;
        const k = Math.min(1, l);
        out.mx = (mx / l) * k;
        out.mz = (mz / l) * k;
        humanize(bot, out, { precise: true });
      },
    };
  },
  ['ocean', 'jungle', 'meadow'],
);
