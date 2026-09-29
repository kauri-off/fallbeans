import * as THREE from 'three';
import { type Builder, PAL, type Palette } from '../../client/world/builder';
import { pathBrain, type Waypoint } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import meta from './meta';

function drum(b: Builder, z: number, r: number, len: number, speed: number, pal: Palette) {
  const d = b.cyl(0, -r, z, r, len, pal, { dynamic: true, rot: [0, 0, Math.PI / 2], seg: 32 });
  for (let k = 0; k < 8; k++) {
    const a = (k / 8) * Math.PI * 2;
    b.box(Math.cos(a) * r, 0, Math.sin(a) * r, 0.18, len - 0.2, 0.35, '#ffffff', { parent: d.mesh, noCollide: true, castShadow: false });
  }
  b.update((t) => {
    d.mesh.rotation.set(t * speed, 0, Math.PI / 2);
  });
}

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 12.2, 14, 2, 10.4, PAL.purple);

  const path: Waypoint[] = [{ x: 0, z: 12, w: 3 }];
  const pals = [PAL.orange, PAL.teal, PAL.pink, PAL.green];
  [18.6, 22.6, 26.6, 30.6].forEach((z, i) => {
    drum(b, z, 1.6, 12, (i % 2 ? 1 : -1) * 1.4, pals[i % 4]!);
    path.push({ x: 0, z, w: 3, jump: true });
  });
  b.box(0, -1, 36.5, 12, 2, 8, PAL.purple);

  b.pad(0, 0, 42.3, 1.6, 17);
  b.pad(0, 0, 52, 1.6, 17);
  b.box(0, -1, 63, 12, 2, 10, PAL.purple);
  path.push({ x: 0, z: 42.3, w: 0.3 }, { x: 0, z: 52, w: 0.3 }, { x: 0, z: 62, w: 1 });

  b.box(0, -1, 69, 4, 2, 4, PAL.yellow);
  b.cyl(0, -1, 76, 7, 2, PAL.blue, { freq: 0.35 });
  b.hub(0, 0, 76, 1);
  b.rotor(0, 0.6, 76, 6.6, 2, (t) => t * 1.3, 0.7);
  b.box(0, -1, 84.5, 4, 2, 5, PAL.yellow);
  path.push({ x: 3, z: 73, w: 0.5, jump: true }, { x: 3, z: 79, w: 0.5, jump: true }, { x: 0, z: 84, w: 0.4 });

  [89, 92.6, 96.2, 99.8, 103.4].forEach((z, i) => {
    drum(b, z, 1.4, 8, (i % 2 ? -1 : 1) * (1.8 + i * 0.2), pals[(i + 1) % 4]!);
    path.push({ x: 0, z, w: 2, jump: true });
  });

  b.box(0, -1, 120.25, 18, 2, 29.5, PAL.yellow);
  b.finish(0, 0, 128);
  b.clouds(0, 60, 60, 36);
  path.push({ x: 0, z: 110, w: 3 }, { x: 0, z: 132, w: 3 });

  return {
    spawns,
    killY: -12,
    finish: { z: 128, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
      { z: 58, p: new THREE.Vector3(0, 0.1, 62) },
      { z: 83, p: new THREE.Vector3(0, 0.1, 85) },
    ],
    bot: pathBrain(path),
  };
});
