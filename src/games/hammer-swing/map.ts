import * as THREE from 'three';
import { routesBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import meta from './meta';

/**
 * A fork: a narrow bridge under swinging hammers (short, but you must time it) or a walled
 * zig-zag with pushers (safe, but longer). Then tilting seesaws in a zig-zag, and a conveyor
 * belt running back at you, with punching walls and bumpers.
 */

const FORK_Z0 = 15;
const FORK_Z1 = 45;
const HAMMERS = [
  { z: 20, w: 1.9, ph: 0 },
  { z: 27, w: 2.2, ph: 1.7 },
  { z: 34, w: 1.7, ph: 3.2 },
  { z: 41, w: 2.4, ph: 0.9 },
] as const;
const BRIDGE_X = -5;
/** Zig-zag walls on the right path: [z, wall from x, to x]. */
const ZIG = [
  { z: 20, x0: 2.5, x1: 5.5 },
  { z: 26.5, x0: 4.5, x1: 7.5 },
  { z: 33, x0: 2.5, x1: 5.5 },
  { z: 39.5, x0: 4.5, x1: 7.5 },
] as const;
const PUSHERS = [
  { z: 23.2, w: 1.6, ph: 0 },
  { z: 36.2, w: 1.9, ph: 2 },
] as const;
const pusherX = (p: (typeof PUSHERS)[number], t: number) => 5 + Math.sin(t * p.w + p.ph) * 1.6;

const SEESAWS = [
  { x: -2.5, z: 59 },
  { x: 2.5, z: 69 },
  { x: -2.5, z: 79 },
] as const;

const CONV_Z0 = 91;
const CONV_Z1 = 123;
const PUNCH = [
  { z: 99, side: -1, w: 1.3, ph: 0 },
  { z: 107, side: 1, w: 1.5, ph: 1.5 },
  { z: 115, side: -1, w: 1.7, ph: 3 },
] as const;
/** Punching wall: pulled back into the rail, then out across half the belt. */
const punchX = (p: (typeof PUNCH)[number], t: number) => p.side * (5.6 - 3.2 * Math.max(0, Math.sin(t * p.w + p.ph)));

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 11, 18, 2, 8, PAL.purple);

  // --- A: the fork
  const len = FORK_Z1 - FORK_Z0;
  b.box(BRIDGE_X, -1, (FORK_Z0 + FORK_Z1) / 2, 3.2, 2, len, PAL.blue);
  for (const h of HAMMERS) b.hammer(BRIDGE_X, 7.4, h.z, h.w, h.ph);
  const headX = (h: (typeof HAMMERS)[number], t: number) => BRIDGE_X + 6 * Math.sin(Math.sin(t * h.w + h.ph) * 1.05);

  b.box(5, -1, (FORK_Z0 + FORK_Z1) / 2, 5, 2, len, PAL.green);
  for (const sx of [2.1, 7.9]) b.box(sx, 1.2, (FORK_Z0 + FORK_Z1) / 2, 0.8, 2.4, len, PAL.pink);
  for (const w of ZIG) b.box((w.x0 + w.x1) / 2, 1.2, w.z, w.x1 - w.x0, 2.4, 0.8, PAL.purple);
  for (const p of PUSHERS) {
    const m = b.box(5, 0.8, p.z, 1.4, 1.6, 1.4, PAL.orange, { dynamic: true, hit: 0.7, tag: 'pusher' });
    b.move((t) => {
      m.obj.position.x = pusherX(p, t);
    });
  }

  b.box(0, -1, 49, 18, 2, 8, PAL.purple);

  // --- B: seesaws in a zig-zag
  SEESAWS.forEach((s, i) => {
    const pl = b.box(s.x, -0.5, s.z, 7.5, 1, 7.5, i % 2 ? PAL.pink : PAL.teal, { dynamic: true });
    b.move((t) => {
      pl.obj.rotation.z = Math.sin(t * 1.1 + i * 2) * 0.3;
      pl.obj.rotation.x = Math.sin(t * 0.7 + i) * 0.1;
    });
  });
  b.box(0, -1, 88, 12, 2, 6, PAL.purple);

  // --- C: conveyor with punching walls and bumpers
  const conv = b.view?.pattern('#8a8f9e', '#c7ccd8', 0.9, [0, 1], 3.5 * 0.9, 'rubber');
  const cl = CONV_Z1 - CONV_Z0;
  const cz = (CONV_Z0 + CONV_Z1) / 2;
  b.box(0, -1, cz, 9, 2, cl, PAL.white, { material: conv, conveyor: new THREE.Vector3(0, 0, -3.5) });
  for (const sx of [-1, 1]) b.box(sx * 4.9, 0.6, cz, 0.8, 1.2, cl, PAL.yellow);
  for (const p of PUNCH) {
    const m = b.box(0, 0.9, p.z, 3.4, 1.8, 1.2, PAL.orange, { dynamic: true, hit: 0.9, tag: 'pusher' });
    b.move((t) => {
      m.obj.position.x = punchX(p, t);
    });
  }
  for (const [x, z] of [
    [-2.9, 95],
    [2.9, 103],
    [-2.9, 111],
    [2.9, 119],
  ] as const)
    b.bumper(x, 0, z, 0.75, 10);

  b.box(0, -1, 131, 18, 2, 16, PAL.yellow);
  b.finish(0, 0, 130);
  b.clouds(0, 70, 60, 36);

  // --- bots
  const start: Waypoint[] = [{ x: 0, z: 9, w: 1 }];
  const bridge: Waypoint[] = [
    { x: BRIDGE_X, z: FORK_Z0 + 0.5, w: 0 },
    ...HAMMERS.flatMap((h): Waypoint[] => [
      { x: BRIDGE_X, z: h.z - 2.6, w: 0 },
      {
        x: BRIDGE_X,
        z: h.z + 2,
        w: 0,
        wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(h, bot.t + dt) - BRIDGE_X) > 2.6),
      },
    ]),
  ];
  const zig: Waypoint[] = [{ x: 5, z: FORK_Z0 + 0.5, w: 0.2 }];
  for (const w of ZIG) {
    const gx = w.x0 > 3 ? 3.4 : 6.6;
    const push = PUSHERS.find((p) => Math.abs(p.z - (w.z + 3.2)) < 0.2);
    zig.push({ x: gx, z: w.z - 1.4, w: 0 }, { x: gx, z: w.z + 1.4, w: 0 });
    if (push)
      zig.push({
        x: gx,
        z: push.z + 1.4,
        w: 0,
        wait: (bot: BotView) => Math.abs(pusherX(push, bot.t + 0.35) - gx) > 1.6 && Math.abs(pusherX(push, bot.t) - gx) > 1.6,
      });
  }
  const rest: Waypoint[] = [{ x: 0, z: 49, w: 1 }];
  let edge = 53;
  for (const s of SEESAWS) {
    const e = edge;
    rest.push({ x: s.x, z: s.z, w: 0.3, jumpWhen: (bot) => bot.body.pos.z > e - 1.1 && bot.body.pos.z < e + 0.3 });
    edge = s.z + 3.75;
  }
  rest.push({ x: 0, z: 87, w: 0.5, jumpWhen: (bot) => bot.body.pos.z > edge - 1.1 && bot.body.pos.z < edge + 0.3 });
  // The belt: weave between bumpers, pass a punch when it is pulled back.
  for (const p of PUNCH) {
    const lane = -p.side * 1.9;
    rest.push({ x: lane, z: p.z - 2, w: 0 });
    rest.push({
      x: lane,
      z: p.z + 1.5,
      w: 0,
      wait: (bot: BotView) => Math.abs(punchX(p, bot.t + 0.4)) > 3.4 || p.side * lane < 0,
    });
  }
  rest.push({ x: 0, z: 126, w: 2 }, { x: 0, z: 134, w: 3 });
  const brain = routesBrain([
    { x: -3, points: [...start, ...bridge, ...rest] },
    { x: 3, points: [...start, ...zig, ...rest] },
  ]);

  return {
    spawns,
    killY: -14,
    finish: { z: 130, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: 46, p: new THREE.Vector3(0, 0.1, 49) },
      { z: 86, p: new THREE.Vector3(0, 0.1, 88) },
    ],
    forbidden: (p) =>
      // On top of the zig-zag walls or the hammer frames.
      (p.z > FORK_Z0 && p.z < FORK_Z1 && p.y > 1.9) ||
      // Walking along the belt rails.
      (p.z > CONV_Z0 + 1 && p.z < CONV_Z1 - 1 && Math.abs(p.x) > 4.3 && p.y > 0.6),
    bot: brain,
  };
});
