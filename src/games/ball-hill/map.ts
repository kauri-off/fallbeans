import * as THREE from 'three';
import { PAL } from '../../client/world/builder';
import { pathBrain, type Waypoint } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import { rollingBalls, yOnRamp } from '../../client/world/props';
import meta from './meta';

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 11, 18, 2, 8, PAL.purple);

  const sections = [
    { z0: 15, y0: 0, z1: 65, y1: 10, w: 16, lanes: [-6, -2, 2, 6], cover: [-4, 0, 4], speed: 9, period: 7.5 },
    { z0: 75, y0: 10, z1: 125, y1: 20, w: 12, lanes: [-4, 0, 4], cover: [-2, 2], speed: 11, period: 6 },
  ];
  const path: Waypoint[] = [{ x: 0, z: 13, w: 4 }];
  sections.forEach((s, si) => {
    b.ramp(0, s.z0, s.y0, s.z1, s.y1, s.w, si ? PAL.teal : PAL.blue);
    const ang = Math.atan2(s.y1 - s.y0, s.z1 - s.z0);
    const len = Math.hypot(s.z1 - s.z0, s.y1 - s.y0);
    for (const sx of [-1, 1])
      b.box(sx * (s.w / 2 + 0.4), (s.y0 + s.y1) / 2 + 0.6, (s.z0 + s.z1) / 2, 0.8, 1.2, len, PAL.pink, { rot: [-ang, 0, 0] });
    b.box(0, s.y0 - 0.2, s.z0 + 0.6, s.w, 0.1, 1.2, '#5a3fb8', { noCollide: true });
    const rows = 5;
    for (let r = 0; r < rows; r++) {
      const z = s.z0 + 6 + (r * (s.z1 - s.z0 - 10)) / (rows - 1);
      const y = yOnRamp(z, s.z0, s.y0, s.z1, s.y1);
      const xs = r % 2 ? s.cover.slice(0, -1).map((x, i) => (x + (s.cover[i + 1] ?? x)) / 2) : s.cover;
      for (const x of xs) {
        b.cyl(x, y + 1, z, 0.6, 2.4, PAL.yellow, { seg: 20 });
        path.push({ x, z: z - 1.6, w: 0.2 });
      }
    }
    rollingBalls(b, {
      lanes: s.lanes,
      zTop: s.z1 - 1,
      yTop: s.y1,
      zBottom: s.z0 + 1,
      yBottom: s.y0,
      radius: 1.4,
      speed: (t) => s.speed + t * 0.02,
      period: s.period,
      perLane: 2,
    });
    path.sort((a, c) => a.z - c.z);
  });
  const dedup: Waypoint[] = [];
  for (const p of path) {
    const last = dedup.at(-1);
    if (last && Math.abs(last.z - p.z) < 3) continue;
    dedup.push(p);
  }
  b.box(0, 9, 70, 16, 2, 10, PAL.purple);
  b.box(0, 19, 140, 18, 2, 30, PAL.yellow);
  b.finish(0, 20, 150);
  b.clouds(0, 70, 70, 40, -30, 10);
  dedup.push({ x: 0, z: 68, w: 3 }, { x: 0, z: 128, w: 3 }, { x: 0, z: 152, w: 3 });
  dedup.sort((a, c) => a.z - c.z);

  return {
    spawns,
    killY: -14,
    finish: { z: 150, y: 19 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 66, p: new THREE.Vector3(0, 10.1, 70) },
    ],
    bot: pathBrain(dedup),
  };
});
