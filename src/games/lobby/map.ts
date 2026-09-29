import * as THREE from 'three';
import { defineGame } from '../../shared/game';
import { arenaBrain } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';

/** Not a game: the playground players run around in between shows. */
export const LOBBY_META = defineGame({
  id: 'lobby',
  title: 'Лобби',
  genre: 'points',
  desc: '',
  goal: '',
  duration: 1e6,
});

export default defineMap(LOBBY_META, (b) => {
  b.cyl(0, -1, 0, 15, 2, PAL.blue, { freq: 0.3 });
  b.cyl(0, 0.05, 0, 4, 0.2, PAL.yellow, { noCollide: true });
  b.hub(0, 0, 0, 1);
  b.rotor(0, 0.6, 0, 9, 1, (t) => t * 0.6, 0.5);
  b.bumper(10, 0, 4);
  b.bumper(-9, 0, -6);
  b.bumper(4, 0, -11);
  for (let i = 0; i < 5; i++) {
    const a = -0.5 + i * 0.3;
    const h = 0.7 * (i + 1);
    b.box(Math.cos(a) * 12, h / 2, Math.sin(a) * 12, 2.6, h, 2.6, PAL.pink, { rot: [0, -a, 0] });
  }
  b.pad(-6, 0, 9, 1.3, 15);
  b.clouds(0, 0, 40);
  // Bots potter about the playground: up the steps, onto the pad, into the bumpers, after each other.
  const steps = [0, 2, 4].map((i) => {
    const a = -0.5 + i * 0.3;
    return { x: Math.cos(a) * 12, z: Math.sin(a) * 12 };
  });
  const bot = arenaBrain({
    radius: 12,
    retarget: 3,
    social: true,
    pois: [...steps, { x: -6, z: 9 }, { x: 8.6, z: 3.4 }, { x: -7.7, z: -5.1 }, { x: 3.4, z: -9.6 }],
  });
  return { spawns: b.ringSpawns(8, 7, 0.05), killY: -15, faceCenter: true, view: new THREE.Vector3(0, 2, 0), bot };
});
