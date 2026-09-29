import * as THREE from 'three';
import { routesBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import { rollingBalls, sweepEta } from '../../sim/props';
import meta from './meta';

const SUMMIT_Y = 18;
const CROWN_Z = 121;

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 11, 18, 2, 8, PAL.purple);

  // 1. Ramp with rolling balls
  b.ramp(0, 15, 0, 45, 8, 14, PAL.blue);
  const ang = Math.atan2(8, 30);
  const len = Math.hypot(30, 8);
  for (const sx of [-1, 1]) b.box(sx * 7.4, 4.6, 30, 0.8, 1.2, len, PAL.pink, { rot: [-ang, 0, 0] });
  // Balls in three lanes; the two strips between them are the safe way up.
  rollingBalls(b, {
    lanes: [-4.5, 0, 4.5],
    zTop: 44,
    yTop: 8,
    zBottom: 16,
    yBottom: 0,
    radius: 1.2,
    speed: (t) => 9 + t * 0.02,
    period: 4,
    perLane: 2,
  });
  b.box(0, 7, 50, 14, 2, 10, PAL.purple);

  // 2. Hammer bridge
  b.box(0, 7, 68, 6, 2, 26, PAL.teal, { freq: 0.3 });
  const hammers = [60, 66, 72, 78].map((z, i) => ({ z, speed: 1.8 + i * 0.2, phase: i * 1.4 }));
  for (const h of hammers) b.hammer(0, 15.4, h.z, h.speed, h.phase);
  /** x of a hammer head at time t (the head hangs 6 m below the pivot). */
  const headX = (h: (typeof hammers)[number], t: number) => 6 * Math.sin(Math.sin(t * h.speed + h.phase) * 1.05);
  b.box(0, 7, 86, 14, 2, 10, PAL.purple);

  // 3. Launch pads up to the rotor deck
  b.pad(-3, 8, 89, 1.3, 19);
  b.pad(3, 8, 89, 1.3, 19);
  b.box(0, 13, 99, 14, 2, 10, PAL.pink, { freq: 0.3 });
  b.hub(0, 14, 99, 0.9);
  b.rotor(0, 14.6, 99, 6.5, 2, (t) => t * 1.4, 0.8);

  // 4. Sliding steps
  const stepX = (k: number, t: number) => Math.sin(t * (0.8 + k * 0.25) + k * 2) * 1.6;
  [0, 1, 2].forEach((k) => {
    const s = b.box(0, 14.5 + k * 1.2, 106.5 + k * 3.5, 4, 1, 3, k % 2 ? PAL.orange : PAL.green, { dynamic: true });
    b.move((t) => {
      s.obj.position.x = stepX(k, t);
    });
  });

  // 5. Summit with the crown
  b.box(0, SUMMIT_Y - 1, CROWN_Z, 12, 2, 10, PAL.yellow);
  b.box(0, SUMMIT_Y + 0.6, CROWN_Z, 2, 1.2, 2, '#fff4d6');
  const crown = b.model('crown');
  crown.position.set(0, SUMMIT_Y + 2.2, CROWN_Z);
  crown.scale.setScalar(1.6);
  b.anim((t) => {
    crown.rotation.y = t * 1.5;
    crown.position.y = SUMMIT_Y + 2.2 + Math.sin(t * 2) * 0.15;
  });
  b.clouds(0, 60, 60, 40, -30, 10);

  const edgeJump = (edge: number) => (bot: BotView) => bot.body.pos.z > edge - 1.1 && bot.body.pos.z < edge + 0.2;
  const route = (lane: number): Waypoint[] => [
    { x: lane, z: 14, w: 0 },
    { x: lane, z: 44, w: 0 },
    { x: 0, z: 48, w: 2 },
    ...hammers.flatMap((h): Waypoint[] => [
      { x: 0, z: h.z - 2.6, w: 0 },
      {
        x: 0,
        z: h.z + 2.2,
        w: 0,
        // Go when the head is far and stays away while we pass under.
        wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(h, bot.t + dt)) > 2.4),
      },
    ]),
    { x: 0, z: 84, w: 1 },
    { x: lane > 0 ? 3 : -3, z: 89, w: 0 },
    { x: 0, z: 96, w: 1 },
    {
      x: 0,
      z: 103.5,
      w: 0.3,
      jumpWhen: (bot) => sweepEta(bot, bot.t * 1.4, 1.4, 2, 0, 99) < 0.15,
    },
    { x: (t) => stepX(0, t), z: 106.5, jumpWhen: edgeJump(104) },
    { x: (t) => stepX(1, t), z: 110, jumpWhen: edgeJump(108) },
    { x: (t) => stepX(2, t), z: 113.5, jumpWhen: edgeJump(111.5) },
    { x: 0, z: 117.5, w: 0.5, jumpWhen: edgeJump(115) },
    { x: 0, z: CROWN_Z, w: 0.2 },
  ];
  const brain = routesBrain([2.25, -2.25].map((x) => ({ x, points: route(x) })));

  return {
    spawns,
    killY: -12,
    finish: { z: CROWN_Z - 1.7, y: SUMMIT_Y + 1, halfWidth: 2.5 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 46, p: new THREE.Vector3(0, 8.1, 50) },
      { z: 82, p: new THREE.Vector3(0, 8.1, 86) },
      { z: 95, p: new THREE.Vector3(-5, 14.1, 95.5) },
    ],
    bot: brain,
  };
});
