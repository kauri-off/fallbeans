import * as THREE from 'three';
import { pathBrain, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import meta from './meta';

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 30, 7, 2, 46, PAL.blue);
  const hammers = [
    [14, 1.9, 0],
    [22, 2.2, 1.6],
    [30, 1.7, 3.1],
    [38, 2.4, 0.8],
    [46, 2.0, 2.4],
  ] as const;
  for (const [z, w, ph] of hammers) b.hammer(0, 7.4, z, w, ph);
  const headX = (w: number, ph: number, t: number) => 6 * Math.sin(Math.sin(t * w + ph) * 1.05);

  b.box(0, -1, 57, 10, 2, 8, PAL.purple);
  [67, 78, 89].forEach((z, i) => {
    const s = b.box(0, -0.5, z, 9, 1, 9, i % 2 ? PAL.pink : PAL.green, { dynamic: true });
    b.move((t) => {
      s.obj.rotation.z = Math.sin(t * 1.1 + i * 2) * 0.32;
      s.obj.rotation.x = Math.sin(t * 0.7 + i) * 0.12;
    });
  });
  b.box(0, -1, 98, 10, 2, 8, PAL.purple);

  const conv = b.view?.pattern('#8a8f9e', '#c7ccd8', 0.9, [0, 1], 3.5 * 0.9);
  b.box(0, -1, 118, 8, 2, 32, PAL.white, { material: conv, conveyor: new THREE.Vector3(0, 0, -3.5) });
  b.box(-4.4, 0.6, 118, 0.8, 1.2, 32, PAL.yellow);
  b.box(4.4, 0.6, 118, 0.8, 1.2, 32, PAL.yellow);
  for (const [x, z] of [
    [-2, 106],
    [2.2, 111],
    [-1, 121],
    [2.5, 127],
    [-2.5, 131],
  ] as const)
    b.bumper(x, 0, z, 0.8, 10);
  const sliders = [
    [109, 1.8, 0],
    [116, 2.1, 2],
    [124, 1.6, 4],
  ] as const;
  const sliderX = (w: number, ph: number, t: number) => Math.sin(t * w + ph) * 2.4;
  for (const [z, w, ph] of sliders) {
    const p = b.box(0, 0.8, z, 3, 1.6, 1, PAL.orange, { dynamic: true, hit: 0.8 });
    b.move((t) => {
      p.obj.position.x = sliderX(w, ph, t);
    });
  }

  b.box(0, -1, 142, 18, 2, 16, PAL.yellow);
  b.finish(0, 0, 142);
  b.clouds(0, 70, 60, 36);

  const edge = (e: number) => (bot: BotView) => bot.body.pos.z > e - 1.1 && bot.body.pos.z < e + 0.3;
  const path: Waypoint[] = [
    { x: 0, z: 9, w: 1 },
    ...hammers.flatMap(([z, w, ph]): Waypoint[] => [
      { x: 0, z: z - 2.6, w: 0 },
      { x: 0, z: z + 2, w: 0, wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(w, ph, bot.t + dt)) > 2.4) },
    ]),
    { x: 0, z: 58, w: 1 },
    { x: 0, z: 67, w: 0.5, jumpWhen: edge(61) },
    { x: 0, z: 78, w: 0.5, jumpWhen: edge(71.5) },
    { x: 0, z: 89, w: 0.5, jumpWhen: edge(82.5) },
    { x: 0, z: 99, w: 0.5, jumpWhen: edge(93.5) },
    { x: 0, z: 103.5, w: 0.3 },
    ...sliders.flatMap(([z, w, ph]): Waypoint[] => {
      // Pass on the side the slider is not, once it has moved out of the middle.
      const side = (t: number) => (sliderX(w, ph, t + 0.3) > 0 ? -2.3 : 2.3);
      return [
        { x: 0, z: z - 2.2, w: 0 },
        {
          x: side,
          z: z + 1.2,
          // Go when the slider is out of the middle and still moving outwards.
          wait: (bot) => {
            const x = sliderX(w, ph, bot.t + 0.3);
            return Math.abs(x) > 1.2 && x * (sliderX(w, ph, bot.t + 0.35) - x) > 0;
          },
        },
        { x: 0, z: z + 2.8, w: 0.3 },
      ];
    }),
    { x: 0, z: 136, w: 2 },
    { x: 0, z: 146, w: 4 },
  ];

  return {
    spawns,
    killY: -14,
    finish: { z: 142, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 54, p: new THREE.Vector3(0, 0.1, 57) },
      { z: 95, p: new THREE.Vector3(0, 0.1, 98) },
    ],
    bot: pathBrain(path),
  };
});
