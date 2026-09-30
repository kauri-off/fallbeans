import * as THREE from 'three';
import { routesBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import { armContactEta, glovePuncher, rollingBalls, sweepEta, yOnRamp } from '../../sim/props';
import meta from './meta';

const SUMMIT_Y = 18;
const CROWN_Z = 121;
const P1 = { z0: 44, z1: 54, y: 8 };
const P2 = { z0: 78, z1: 91, y: 8 };
const HUB_Z = 122.5;

/** Right fork: sliding steps climbing to the first plateau. */
const STEPS = [0, 1, 2, 3, 4].map((k) => ({ z: 18 + k * 5.4, y: 1.3 + k * 1.33, w: 0.9 + k * 0.15, ph: k * 1.9 }));
const stepX = (s: (typeof STEPS)[number], t: number) => 5 + Math.sin(t * s.w + s.ph) * 2.2;

/** Two beams, two hammers each, out of phase. */
const BEAMS = [
  {
    x: -3,
    hammers: [
      { z: 60, w: 1.8, ph: 0 },
      { z: 70, w: 2.1, ph: 2.2 },
    ],
  },
  {
    x: 3,
    hammers: [
      { z: 64, w: 2.0, ph: 1.1 },
      { z: 74, w: 1.7, ph: 3.3 },
    ],
  },
] as const;
type Hammer = (typeof BEAMS)[number]['hammers'][number];

/** Blocks on the ball ramp's middle strip. */
const BLOCKS = [22, 30, 38];

const slideX = (k: number, t: number) => Math.sin(t * (0.8 + k * 0.25) + k * 2) * 1.6;
const CROWN_W = 1.1;
const crownAng = (t: number) => (t <= 0 ? 0 : t * CROWN_W);
/** Gloves across the second plateau, before the pads. */
const GLOVES = [
  { z: 84.3, side: -1, w: 1.2, ph: 0 },
  { z: 87.2, side: 1, w: 1.35, ph: 2.4 },
] as const;
const GLOVE_REACH = 8.5;

export default defineMap(meta, (b) => {
  b.style.pattern = 'chevron';
  const spawns = b.startArea(0);
  b.box(0, -1, 11, 18, 2, 8, PAL.purple);

  // 1a. Left: a ramp with balls rolling down two lanes.
  b.ramp(-5, 15, 0, P1.z0, P1.y, 7, PAL.blue);
  const ang = Math.atan2(P1.y, P1.z0 - 15);
  const len = Math.hypot(P1.z0 - 15, P1.y);
  b.box(-8.9, P1.y / 2 + 0.6, (15 + P1.z0) / 2, 0.8, 1.2, len, PAL.pink, { rot: [-ang, 0, 0] });
  const balls = rollingBalls(b, {
    lanes: [-7, -3],
    zTop: P1.z0 - 1,
    yTop: P1.y,
    zBottom: 16,
    yBottom: 0,
    radius: 1,
    speed: (t) => 8.5 + t * 0.02,
    period: 3.6,
    perLane: 1,
  });
  // Blocks on the safe middle strip: to pass one you step into a ball lane.
  for (const [i, z] of BLOCKS.entries())
    b.box(-5, yOnRamp(z, 15, 0, P1.z0, P1.y) + 0.6, z, 1.8, 1.8, 1.4, i % 2 ? PAL.orange : PAL.yellow, { rot: [-ang, 0, 0] });
  // 1b. Right: sliding steps.
  STEPS.forEach((s, k) => {
    const m = b.box(5, s.y - 0.5, s.z, 3.2, 1, 3, k % 2 ? PAL.orange : PAL.green, { dynamic: true });
    b.move((t) => {
      m.obj.position.x = stepX(s, t);
    });
  });
  b.box(0, P1.y - 1, (P1.z0 + P1.z1) / 2, 20, 2, P1.z1 - P1.z0, PAL.purple);

  // 2. Hammer beams.
  const headX = (x: number, h: Hammer, t: number) => x + 6 * Math.sin(Math.sin(t * h.w + h.ph) * 1.05);
  for (const beam of BEAMS) {
    b.box(beam.x, P1.y - 1, (P1.z1 + P2.z0) / 2, 2.4, 2, P2.z0 - P1.z1, PAL.teal, { freq: 0.3 });
    for (const h of beam.hammers) b.hammer(beam.x, P1.y + 7.4, h.z, h.w, h.ph);
  }
  b.box(0, P2.y - 1, (P2.z0 + P2.z1) / 2, 14, 2, P2.z1 - P2.z0, PAL.purple);

  // Gloves punching across the plateau (from posts beside it).
  const gloveXs = GLOVES.map((g) =>
    glovePuncher(b, {
      x: g.side * 9,
      y: P2.y + 0.95,
      z: g.z,
      side: g.side,
      w: g.w,
      ph: g.ph,
      reach: GLOVE_REACH,
      scale: 1.3,
      postTo: P2.y - 6,
    }),
  );
  // 3. Launch pads up to the rotor deck.
  b.pad(-3, P2.y, 89.2, 1.3, 19);
  b.pad(3, P2.y, 89.2, 1.3, 19);
  b.box(0, 13, 99, 14, 2, 10, PAL.pink, { freq: 0.3 });
  b.hub(0, 14, 99, 0.9);
  // 5.8 m: the arms clear the first sliding step (z 105).
  b.rotor(0, 14.6, 99, 5.8, 2, (t) => t * 1.4, 0.8);

  // 4. Sliding steps.
  [0, 1, 2].forEach((k) => {
    const s = b.box(0, 14.5 + k * 1.2, 106.5 + k * 3.5, 4, 1, 3, k % 2 ? PAL.orange : PAL.green, { dynamic: true });
    b.move((t) => {
      s.obj.position.x = slideX(k, t);
    });
  });

  // 5. Summit: the crown, guarded by a slow sweeper.
  b.box(0, SUMMIT_Y - 1, CROWN_Z, 14, 2, 10, PAL.yellow);
  b.hub(0, SUMMIT_Y, HUB_Z, 0.6);
  b.rotor(0, SUMMIT_Y + 0.6, HUB_Z, 5, 2, crownAng, 0.6);
  // The crown floats above the finish zone (nothing under it for the sweeper to clip).
  const crown = b.model('crown');
  crown.position.set(0, SUMMIT_Y + 2.6, CROWN_Z - 1);
  crown.scale.setScalar(1.6);
  b.anim((t) => {
    crown.rotation.y = t * 1.5;
    crown.position.y = SUMMIT_Y + 2.6 + Math.sin(t * 2) * 0.15;
  });
  b.clouds(0, 60, 60, 40, -30, 10);

  // --- bots
  const edgeJump = (edge: number) => (bot: BotView) => bot.body.pos.z > edge - 1.1 && bot.body.pos.z < edge + 0.2;
  const ramp = (lane: number): Waypoint[] => {
    // Up the strip between the lanes, stepping aside only when no ball is coming.
    const pts: Waypoint[] = [{ x: -5, z: 15.5, w: 0 }];
    BLOCKS.forEach((zb, i) => {
      const side = i % 2 ? 1 : -1;
      const lx = -5 + side * 1.55;
      pts.push({ x: -5, z: zb - 2.3, w: 0 });
      pts.push({
        x: lx,
        z: zb - 0.8,
        w: 0,
        wait: (bot) => !balls.danger(Math.min(-5, lx) - 0.3, Math.max(-5, lx) + 0.3, zb - 2.5, zb + 2.5, bot.t, 0.9),
      });
      pts.push({ x: lx, z: zb + 0.9, w: 0 }, { x: -5, z: zb + 2.3, w: 0 });
    });
    pts.push({ x: lane, z: P1.z0 + 2, w: 0.3 });
    return pts;
  };
  const steps: Waypoint[] = [{ x: 5, z: 14.2, w: 0 }];
  let edge = 15;
  for (const s of STEPS) {
    const e = edge;
    steps.push({
      x: (t) => stepX(s, t),
      z: s.z,
      wait: (bot) => Math.abs(stepX(s, bot.t + 0.55) - bot.body.pos.x) < 1.0,
      jumpWhen: edgeJump(e),
    });
    edge = s.z + 1.5;
  }
  steps.push({ x: 3, z: P1.z0 + 2, w: 0.3, jumpWhen: edgeJump(edge) });
  const beam = (x: number, hammers: readonly Hammer[]): Waypoint[] => [
    { x, z: P1.z1 - 1, w: 0 },
    ...hammers.flatMap((h): Waypoint[] => [
      { x, z: h.z - 2.6, w: 0 },
      {
        x,
        z: h.z + 2.2,
        w: 0,
        wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(x, h, bot.t + dt) - x) > 2.4),
      },
    ]),
    { x: x > 0 ? 3 : -3, z: P2.z0 + 3, w: 0.3 },
  ];
  const top: Waypoint[] = [
    { x: 0, z: 96, w: 1 },
    {
      x: 0,
      z: 103.5,
      w: 0.3,
      jumpWhen: (bot) => sweepEta(bot, bot.t * 1.4, 1.4, 2, 0, 99) < 0.15,
    },
    { x: (t) => slideX(0, t), z: 106.5, jumpWhen: edgeJump(104) },
    { x: (t) => slideX(1, t), z: 110, jumpWhen: edgeJump(108) },
    { x: (t) => slideX(2, t), z: 113.5, jumpWhen: edgeJump(111.5) },
    {
      x: 0,
      z: CROWN_Z - 1,
      w: 0.2,
      jumpWhen: (bot) => {
        if (bot.body.pos.z < 115) return edgeJump(115)(bot);
        const eta = armContactEta(bot, crownAng(bot.t), CROWN_W, 2, 0, HUB_Z);
        return eta > 0.1 && eta < 0.24;
      },
    },
  ];
  // Across the glove lanes only when both are pulled back for a while.
  const clear = (bot: BotView) =>
    [0, 0.25, 0.5, 0.75, 1].every((dt) =>
      gloveXs.every((gx, i) => Math.abs(gx(bot.t + dt)) > 9 - GLOVE_REACH * 0.3 || !GLOVES[i]),
    );
  const pads = (x: number): Waypoint[] => [{ x, z: 89.2, w: 0, wait: clear }];
  const routes = [
    { x: -6, points: [{ x: -3, z: 12, w: 1 }, ...ramp(-3), ...beam(-3, BEAMS[0].hammers), ...pads(-3), ...top] },
    { x: -2, points: [{ x: -3, z: 12, w: 1 }, ...ramp(3), ...beam(3, BEAMS[1].hammers), ...pads(3), ...top] },
    { x: 3, points: [{ x: 5, z: 12, w: 1 }, ...steps, ...beam(3, BEAMS[1].hammers), ...pads(3), ...top] },
    { x: 7, points: [{ x: 5, z: 12, w: 1 }, ...steps, ...beam(-3, BEAMS[0].hammers), ...pads(-3), ...top] },
  ];

  return {
    spawns,
    killY: -12,
    finish: { z: CROWN_Z - 2.4, y: SUMMIT_Y + 1, halfWidth: 2.2 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: P1.z0 + 1, p: new THREE.Vector3(0, P1.y + 0.1, P1.z0 + 4) },
      { z: P2.z0 + 1, p: new THREE.Vector3(0, P2.y + 0.1, P2.z0 + 4) },
      // Falling off the rotor deck or above: back before the pads (the deck is all within the rotor's reach).
      { z: 95, p: new THREE.Vector3(0, P2.y + 0.1, 86) },
    ],
    // On the hammer frames, or the ramp's rail.
    forbidden: (p) => (p.z > P1.z1 && p.z < P2.z0 && p.y > P1.y + 3) || (p.z > 15 && p.z < P1.z0 && p.x < -8.4),
    bot: routesBrain(routes),
  };
});
