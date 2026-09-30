import * as THREE from 'three';
import type { Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import {
  conveyor,
  gloveAlley,
  hammerBridges,
  movingPlatforms,
  pickSections,
  raceCourse,
  rotorDecks,
  type Segment,
  seesaws,
  tippingBridge,
  withRests,
} from '../../sim/course';
import { type BotView, defineMap } from '../../sim/map';
import meta from './meta';

/**
 * The fork: a narrow bridge under swinging hammers (short, but you must time it) or a walled
 * zig-zag with pushers (safe, but longer). Hammer speeds and pusher rhythms come from the seed.
 */
function fork(): Segment {
  return (s) => {
    const { b, rng } = s;
    const z0 = s.z;
    const len = 30;
    const z1 = z0 + len;
    const BX = -5;
    b.box(BX, s.y - 1, z0 + len / 2, 3.2, 2, len, PAL.blue);
    const hammers = [5, 12, 19, 26].map((dz) => ({ z: z0 + dz, w: 1.8 + rng() * 0.8, ph: rng() * 6.28 }));
    for (const h of hammers) b.hammer(BX, s.y + 7.4, h.z, h.w, h.ph, 1.12);
    const headX = (h: (typeof hammers)[number], t: number) => BX + 6 * Math.sin(Math.sin(t * h.w + h.ph) * 1.12);

    b.box(5, s.y - 1, z0 + len / 2, 5, 2, len, PAL.green);
    for (const sx of [2.1, 7.9]) b.box(sx, s.y + 1.2, z0 + len / 2, 0.8, 2.4, len, PAL.pink);
    const zig = [5, 11.5, 18, 24.5].map((dz, k) => ({ z: z0 + dz, x0: k % 2 ? 4.5 : 2.5, x1: k % 2 ? 7.5 : 5.5 }));
    for (const w of zig) b.box((w.x0 + w.x1) / 2, s.y + 1.2, w.z, w.x1 - w.x0, 2.4, 0.8, PAL.purple);
    const pushers = zig.map((w) => ({ z: w.z + 3.2, w: 1.4 + rng() * 0.7, ph: rng() * 6 }));
    const pusherX = (p: (typeof pushers)[number], t: number) => 5 + Math.sin(t * p.w + p.ph) * 1.6;
    for (const p of pushers) {
      const m = b.box(5, s.y + 0.8, p.z, 1.4, 1.6, 1.4, PAL.orange, { dynamic: true, hit: 0.7, tag: 'pusher' });
      b.move((t) => {
        m.obj.position.x = pusherX(p, t);
      });
    }
    b.bonus(BX, s.y, z0 + 15.5);
    b.box(0, s.y - 1, z1 + 4, 18, 2, 8, PAL.purple);

    const bridge: Waypoint[] = [{ x: BX, z: z0 + 0.5, w: 0 }];
    for (const h of hammers)
      bridge.push(
        { x: BX, z: h.z - 2.6, w: 0 },
        {
          x: BX,
          z: h.z + 2,
          w: 0,
          wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(h, bot.t + dt) - BX) > 2.6),
        },
      );
    const walls: Waypoint[] = [{ x: 5, z: z0 + 0.5, w: 0.2 }];
    zig.forEach((w, k) => {
      const gx = w.x0 > 3 ? 3.4 : 6.6;
      const p = pushers[k]!;
      walls.push(
        { x: gx, z: w.z - 1.4, w: 0 },
        { x: gx, z: w.z + 1.4, w: 0 },
        {
          x: gx,
          z: p.z + 1.4,
          w: 0,
          wait: (bot: BotView) => Math.abs(pusherX(p, bot.t + 0.35) - gx) > 1.6 && Math.abs(pusherX(p, bot.t) - gx) > 1.6,
        },
      );
    });
    for (const r of [bridge, walls]) r.push({ x: 0, z: z1 + 4, w: 1 });
    return {
      z: z1 + 8,
      y: s.y,
      routes: [bridge, walls],
      // On top of the zig-zag walls or the hammer frames.
      forbidden: (p) => p.z > z0 && p.z < z1 && p.y > s.y + 1.9,
      checkpoint: { from: z1 + 0.5, p: new THREE.Vector3(0, s.y + 0.1, z1 + 4) },
    };
  };
}

export default defineMap(
  meta,
  (b, ctx) => {
    const middle = pickSections(
      b.rng,
      [seesaws(3), conveyor(30), hammerBridges(3), tippingBridge(6), gloveAlley(4), movingPlatforms(4)],
      4,
    );
    return raceCourse(b, ctx, { sections: withRests([fork(), ...middle, rotorDecks(1)]) });
  },
  ['factory', 'desert', 'lava'],
);
