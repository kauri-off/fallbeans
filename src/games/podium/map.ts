import * as THREE from 'three';
import { defineGame } from '../../shared/game';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';

/** Not a game: the stage at the end of a game, players standing on podiums by final place. */
export const PODIUM_META = defineGame({
  id: 'podium',
  title: 'Итоги',
  genre: 'points',
  desc: '',
  goal: '',
  duration: 1e6,
});

/** Podium x and top height for each final place (1st in the middle, then alternating sides). */
export const PODIUM_SLOTS: readonly { x: number; h: number }[] = [
  { x: 0, h: 3 },
  { x: -3, h: 2.2 },
  { x: 3, h: 1.6 },
  { x: -6, h: 0.8 },
  { x: 6, h: 0.8 },
  { x: -8.6, h: 0.4 },
  { x: 8.6, h: 0.4 },
  { x: 0, h: 0.4 },
];

export default defineMap(PODIUM_META, (b) => {
  b.cyl(0, -1, 0, 16, 2, PAL.purple, { freq: 0.3 });
  b.cyl(0, 0.03, 0, 16.05, 0.1, PAL.yellow, { noCollide: true });
  const pals = [PAL.yellow, PAL.white, PAL.orange, PAL.blue, PAL.blue, PAL.teal, PAL.teal, PAL.pink];
  const spawns = PODIUM_SLOTS.map((s, i) => {
    // The 8th place stands in front, on the floor.
    const z = i === 7 ? 4 : 0;
    if (i < 7) b.box(s.x, s.h / 2, z, 2.6, s.h, 2.6, pals[i]!);
    return new THREE.Vector3(s.x, s.h + 0.05, z);
  });
  if (b.view) {
    // Medals on the podium fronts.
    const labels = ['🥇', '🥈', '🥉'];
    labels.forEach((l, i) => {
      const s = PODIUM_SLOTS[i]!;
      const tex = b.view!.own(b.view!.emojiTexture(l, i === 0 ? '#ffd84a' : i === 1 ? '#f4f1ff' : '#ff9f4a'));
      const m = new THREE.Mesh(
        b.view!.own(new THREE.PlaneGeometry(1.4, 1.4)),
        b.view!.own(new THREE.MeshStandardMaterial({ map: tex, roughness: 0.6 })),
      );
      m.position.set(s.x, s.h / 2, 1.32);
      b.group.add(m);
    });
  }
  if (b.view) {
    // Confetti: small tumbling pieces falling on a loop over the podiums.
    const N = 260;
    const geo = b.view.own(new THREE.PlaneGeometry(0.16, 0.26));
    const mat = b.view.own(new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }));
    const conf = b.view.own(new THREE.InstancedMesh(geo, mat, N));
    conf.frustumCulled = false;
    const cols = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a', '#a66bff', '#ff8a3d', '#ffffff'].map((c) => new THREE.Color(c));
    const seeds = Array.from({ length: N }, (_, i) => ({
      x: (Math.random() - 0.5) * 22,
      z: (Math.random() - 0.5) * 10,
      speed: 1.2 + Math.random() * 1.4,
      phase: Math.random() * 20,
      spin: 2 + Math.random() * 6,
      c: cols[i % cols.length]!,
    }));
    seeds.forEach((s, i) => {
      conf.setColorAt(i, s.c);
    });
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const e = new THREE.Euler();
    const p = new THREE.Vector3();
    const one = new THREE.Vector3(1, 1, 1);
    b.group.add(conf);
    b.anim((t) => {
      seeds.forEach((s, i) => {
        const y = 14 - ((t * s.speed + s.phase) % 16);
        p.set(s.x + Math.sin(t * 1.3 + s.phase) * 0.6, y, s.z + Math.cos(t + s.phase) * 0.4);
        q.setFromEuler(e.set(t * s.spin, t * s.spin * 0.7, s.phase));
        conf.setMatrixAt(i, m.compose(p, q, one));
      });
      conf.instanceMatrix.needsUpdate = true;
    });
  }
  // Stage dressing behind the podiums: fans, flags and stars.
  for (const sx of [-1, 1]) {
    b.prop('fan', sx * 11.5, 0, -5.5, { yaw: -sx * 0.5, scale: 1.3 });
    b.prop('flag', sx * 13.5, 0, -1, { tint: sx < 0 ? '#ff5fa2' : '#3fa9ff', yaw: sx < 0 ? Math.PI : 0 });
    b.prop('star', sx * 4.5, 7.2, -2, { scale: 1.2 });
  }
  b.prop('star', 0, 8.4, -2.5, { scale: 1.8 });
  b.clouds(0, 0, 45);
  return { spawns, killY: -15, view: new THREE.Vector3(0, 2.4, 0) };
});
