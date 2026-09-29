import * as THREE from 'three';
import { arenaBrain } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import { armContactEta } from '../../sim/props';
import meta from './meta';

export const lowAng = (t: number) => (t <= 0 ? 0 : 1.15 * t + 0.009 * t * t);
const highAng = (t: number) => Math.PI / 2 - (t <= 0 ? 0 : 0.7 * t + 0.006 * t * t);

export default defineMap(meta, (b) => {
  b.cyl(0, -1, 0, 13, 2, PAL.blue, { freq: 0.35 });
  b.cyl(0, 0.03, 0, 13.05, 0.1, PAL.yellow, { noCollide: true });
  b.cyl(0, 0.06, 0, 11.5, 0.1, PAL.blue, { noCollide: true, freq: 0.35 });
  b.hub(0, 0, 0, 1.2);
  b.rotor(0, 0.6, 0, 12.6, 2, lowAng, 0.6);
  b.rotor(0, 2.45, 0, 12.6, 2, highAng, 0.6);
  b.clouds(0, 0, 40);
  const spawns = [25, 65, 115, 155, 205, 245, 295, 335].map((d) => {
    const a = (d * Math.PI) / 180;
    return new THREE.Vector3(Math.cos(a) * 6, 0.1, Math.sin(a) * 6);
  });
  const brain = arenaBrain({
    radius: 10,
    safe: (x, z) => Math.hypot(x, z) > 3.5,
    jumpWhen: (bot) => {
      const t = Math.max(0, bot.t);
      if (t <= 0) return false;
      const eta = armContactEta(bot, lowAng(t), 1.15 + 0.018 * t, 2);
      const high = armContactEta(bot, highAng(t), -(0.7 + 0.012 * t), 2);
      // Worse bots react late (and sometimes too late).
      return eta > 0.02 && eta < 0.08 + (bot.mem.react ?? 0.2) * 0.35 && high > 0.7;
    },
  });
  return { spawns, killY: -6, faceCenter: true, view: new THREE.Vector3(0, 3, 0), bot: brain };
});
