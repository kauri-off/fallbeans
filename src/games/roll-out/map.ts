import * as THREE from 'three';
import { PAL } from '../../client/world/builder';
import { steer } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import meta from './meta';

const R = 7;
const N = 20;
const CY = -R;

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
    for (const dz of [-4.1, 4.1]) {
      const rim = new THREE.Mesh(new THREE.TorusGeometry(R, 0.25, 8, 48), b.mat('#5a3fb8'));
      rim.position.z = dz;
      group.add(rim);
    }
    const angle = (t: number) => (t <= 0 ? 0 : r.dir * (0.35 * t + 0.002 * t * t));
    b.update((t) => {
      group.rotation.z = angle(t);
    });
    return { ...r, missing, angle };
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
      bot.mem.spd ??= 0.8 + bot.rng() * 0.2;
      steer(bot, 0, ring.z + ((bot.id % 3) - 1) * 1.5, out, bot.mem.spd);
      const phi = Math.atan2(p.x, p.y - CY);
      const t = Math.max(0, bot.t);
      const ahead = 0.25 + (bot.id % 4) * 0.08;
      const k = ((Math.round((phi + ring.angle(t + ahead)) / ((2 * Math.PI) / N)) % N) + N) % N;
      if (ring.missing.has(k) && bot.body.grounded) out.jump = true;
    },
  };
});
