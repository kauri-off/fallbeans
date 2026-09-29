import * as THREE from 'three';
import { pathBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import { rollingBalls, yOnRamp } from '../../sim/props';
import meta from './meta';

/**
 * An icy slope you cannot run up: only the zig-zag carpet strips give grip, and balls roll down
 * across them. Slip off and you slide back down. Then a grippy ramp with sliding gates.
 */

// Section A: the ice slope.
const A0 = { z: 15, y: 0 };
const A1 = { z: 55, y: 10 };
const A_W = 20;
// Section B: gates.
const B0 = { z: 63, y: 10 };
const B1 = { z: 103, y: 18 };
const B_W = 16;

const yA = (z: number) => yOnRamp(z, A0.z, A0.y, A1.z, A1.y);
const yB = (z: number) => yOnRamp(z, B0.z, B0.y, B1.z, B1.y);

/** Carpet corners (x, z): a zig-zag up the ice. */
const CARPET: readonly [number, number][] = [
  [0, 15.5],
  [-7, 21],
  [7, 31],
  [-7, 41],
  [6, 50],
  [0, 55],
];
const CARPET_W = 3.2;

/** Gates: a wall across the ramp with one gap that slides from side to side. */
const GATES = [
  { z: 72, w: 0.75, ph: 0, gap: 3.8 },
  { z: 82, w: 0.95, ph: 2, gap: 3.6 },
  { z: 92, w: 1.1, ph: 4.1, gap: 3.4 },
] as const;
const gateX = (g: (typeof GATES)[number], t: number) => Math.sin(t * g.w + g.ph) * (B_W / 2 - g.gap / 2 - 0.4);

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 11, 18, 2, 8, PAL.purple);

  // --- A: ice with carpets
  const angA = Math.atan2(A1.y - A0.y, A1.z - A0.z);
  const cosA = Math.cos(angA);
  const ice = b.view?.plain('#d6f2ff', { roughness: 0.08, metalness: 0.05 }, 'ice');
  b.ramp(0, A0.z, A0.y, A1.z, A1.y, A_W, PAL.blue, 1, { material: ice, slip: 1 });
  const lenA = Math.hypot(A1.z - A0.z, A1.y - A0.y);
  for (const sx of [-1, 1])
    b.box(sx * (A_W / 2 + 0.4), (A0.y + A1.y) / 2 + 0.6, (A0.z + A1.z) / 2, 0.8, 1.2, lenA, PAL.pink, { rot: [-angA, 0, 0] });
  for (let i = 0; i < CARPET.length - 1; i++) {
    const [x0, z0] = CARPET[i]!;
    const [x1, z1] = CARPET[i + 1]!;
    const cx = (x0 + x1) / 2;
    const cz = (z0 + z1) / 2;
    const dx = x1 - x0;
    const dzSlope = (z1 - z0) / cosA;
    const len = Math.hypot(dx, dzSlope) + CARPET_W * 0.7;
    const yaw = Math.atan2(dx, dzSlope);
    b.box(cx, yA(cz) + 0.1 / cosA, cz, CARPET_W, 0.2, len, i % 2 ? PAL.green : PAL.yellow, {
      rot: [-angA, yaw, 0],
      freq: 0.8,
    });
  }
  const balls = rollingBalls(b, {
    lanes: [-5.5, 0, 5.5],
    zTop: A1.z - 1,
    yTop: A1.y,
    zBottom: A0.z + 1,
    yBottom: A0.y,
    radius: 1.1,
    speed: (t) => 8 + t * 0.03,
    period: 5.5,
    perLane: 1,
  });

  b.box(0, A1.y - 1, 59, A_W, 2, 8, PAL.purple);

  // --- B: gates
  const angB = Math.atan2(B1.y - B0.y, B1.z - B0.z);
  b.ramp(0, B0.z, B0.y, B1.z, B1.y, B_W, PAL.teal);
  const lenB = Math.hypot(B1.z - B0.z, B1.y - B0.y);
  for (const sx of [-1, 1])
    b.box(sx * (B_W / 2 + 0.4), (B0.y + B1.y) / 2 + 0.6, (B0.z + B1.z) / 2, 0.8, 1.2, lenB, PAL.pink, { rot: [-angB, 0, 0] });
  GATES.forEach((g, i) => {
    const y = yB(g.z);
    const gate = b.anchor(0, y, g.z);
    const piece = B_W;
    const pal = i % 2 ? PAL.orange : PAL.purple;
    for (const side of [-1, 1])
      b.box(side * (g.gap / 2 + piece / 2), 1.4, 0, piece, 3.4, 0.8, pal, { parent: gate, dynamic: true, tag: 'gate' });
    b.move((t) => {
      gate.position.x = gateX(g, t);
    });
  });
  for (const [x, z] of [
    [-4, 77],
    [4, 87],
    [0, 97],
  ] as const)
    b.bumper(x, yB(z) - 0.1, z, 0.8, 10);

  b.box(0, B1.y - 1, 114, 18, 2, 22, PAL.yellow);
  b.finish(0, B1.y, 118);
  b.clouds(0, 70, 70, 40, -30, 10);

  // --- bots: along the carpets, waiting for a gap between balls; then through the gates.
  const path: Waypoint[] = [{ x: 0, z: 13, w: 1 }];
  for (let i = 1; i < CARPET.length; i++) {
    const [x0, z0] = CARPET[i - 1]!;
    const [x1, z1] = CARPET[i]!;
    // Stop just before each ball lane the carpet crosses and go when it is clear.
    for (const lane of [-5.5, 0, 5.5]) {
      if ((lane - x0) * (lane - x1) >= 0) continue;
      const f = (lane - x0) / (x1 - x0);
      const z = z0 + (z1 - z0) * f;
      const before = Math.max(0, f - 2.4 / Math.hypot(x1 - x0, z1 - z0));
      path.push({ x: x0 + (x1 - x0) * before, z: z0 + (z1 - z0) * before, w: 0.2 });
      path.push({ x: lane, z, w: 0.2, wait: (bot) => !balls.danger(lane - 1.5, lane + 1.5, z - 2.5, z + 2.5, bot.t, 1.1) });
    }
    path.push({ x: x1, z: z1, w: 0.2 });
  }
  path.push({ x: 0, z: 60, w: 1 });
  for (const g of GATES) {
    path.push({ x: 0, z: g.z - 3, w: 0.5 });
    path.push({
      x: (t) => gateX(g, t),
      z: g.z + 1.2,
      wait: (bot: BotView) => Math.abs(gateX(g, bot.t + 0.45) - bot.body.pos.x) < 1.2,
    });
    path.push({ x: (t) => gateX(g, t), z: g.z + 2.5 });
  }
  path.push({ x: 0, z: 110, w: 2 }, { x: 0, z: 122, w: 3 });

  return {
    spawns,
    killY: -14,
    finish: { z: 118, y: B1.y - 1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: 56, p: new THREE.Vector3(0, A1.y + 0.1, 60) },
    ],
    // Riding the rails or the gate tops instead of the course.
    forbidden: (p) =>
      (p.z > A0.z && p.z < A1.z && (Math.abs(p.x) > A_W / 2 + 0.05 || p.y > yA(p.z) + 2.6)) ||
      (p.z > B0.z && p.z < B1.z && (Math.abs(p.x) > B_W / 2 + 0.05 || p.y > yB(p.z) + 2.8)),
    bot: pathBrain(path),
  };
});
