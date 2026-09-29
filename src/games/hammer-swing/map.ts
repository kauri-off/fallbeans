import * as THREE from 'three';
import { PAL, patternMaterial } from '../../client/world/builder';
import { pathBrain } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import meta from './meta';

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 30, 7, 2, 46, PAL.blue);
  for (const [z, w, ph] of [
    [14, 1.9, 0],
    [22, 2.2, 1.6],
    [30, 1.7, 3.1],
    [38, 2.4, 0.8],
    [46, 2.0, 2.4],
  ] as const)
    b.hammer(0, 7.4, z, w, ph);

  b.box(0, -1, 57, 10, 2, 8, PAL.purple);
  [67, 78, 89].forEach((z, i) => {
    const s = b.box(0, -0.5, z, 9, 1, 9, i % 2 ? PAL.pink : PAL.green, { dynamic: true });
    b.update((t) => {
      s.mesh.rotation.z = Math.sin(t * 1.1 + i * 2) * 0.32;
      s.mesh.rotation.x = Math.sin(t * 0.7 + i) * 0.12;
    });
  });
  b.box(0, -1, 98, 10, 2, 8, PAL.purple);

  const conv = patternMaterial('#8a8f9e', '#c7ccd8', 0.9, [0, 1], 3.5 * 0.9);
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
  for (const [z, w, ph] of [
    [109, 1.8, 0],
    [116, 2.1, 2],
    [124, 1.6, 4],
  ] as const) {
    const p = b.box(0, 0.8, z, 3, 1.6, 1, PAL.orange, { dynamic: true, hit: 0.8 });
    b.update((t) => {
      p.mesh.position.x = Math.sin(t * w + ph) * 2.4;
    });
  }

  b.box(0, -1, 142, 18, 2, 16, PAL.yellow);
  b.finish(0, 0, 142);
  b.clouds(0, 70, 60, 36);

  return {
    spawns,
    killY: -14,
    finish: { z: 142, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 54, p: new THREE.Vector3(0, 0.1, 57) },
      { z: 95, p: new THREE.Vector3(0, 0.1, 98) },
    ],
    bot: pathBrain([
      { x: 0, z: 9, w: 2 },
      { x: 0, z: 52, w: 1.5 },
      { x: 0, z: 62, w: 1 },
      { x: 0, z: 73, w: 1.5 },
      { x: 0, z: 84, w: 1.5 },
      { x: 0, z: 100, w: 1 },
      { x: 3, z: 108, w: 0.5 },
      { x: -3, z: 114, w: 0.5 },
      { x: 0, z: 124, w: 2 },
      { x: 0, z: 136, w: 2 },
      { x: 0, z: 146, w: 4 },
    ]),
  };
});
