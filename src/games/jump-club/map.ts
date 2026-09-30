import * as THREE from 'three';
import { arenaBrain } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import { armContactEta, spinUp } from '../../sim/props';
import meta from './meta';

/**
 * Jump the low bar, stay under the high one. The bars are barriers that knock over whoever they catch
 * (and pass over them: nobody gets dragged along). Direction, the number of bars and how fast they
 * speed up come from the seed.
 */
export default defineMap(meta, (b) => {
  b.style.pattern = 'checker';
  const rng = b.rng;
  const dir = rng() < 0.5 ? 1 : -1;
  const lowArms = 2;
  const highArms = rng() < 0.35 ? 1 : 2;
  const acc = 0.009 + rng() * 0.005;
  // The low bar starts 10° past a spawn pair and eases in: the first bean it reaches has over a second.
  const LOW = spinUp(-0.26, 1.1 + rng() * 0.15, acc);
  const lowAng = (t: number) => dir * LOW.angle(t);
  const lowOmega = (t: number) => dir * LOW.omega(t);
  const hk = 0.006 + rng() * 0.004;
  const highAng = (t: number) => dir * (Math.PI / 2 - (t <= 0 ? 0 : 0.7 * t + hk * t * t));
  const highOmega = (t: number) => -dir * (0.7 + 2 * hk * t);

  b.cyl(0, -1, 0, 13, 2, PAL.blue, { freq: 0.35 });
  b.cyl(0, 0.03, 0, 13.05, 0.1, PAL.yellow, { noCollide: true });
  b.cyl(0, 0.06, 0, 11.5, 0.1, PAL.blue, { noCollide: true, freq: 0.35 });
  b.hub(0, 0, 0, 1.2);
  b.rotor(0, 0.6, 0, 12.6, lowArms, lowAng, 0.6);
  b.rotor(0, 2.45, 0, 12.6, highArms, highAng, 0.6);
  for (const a of [0, 1, 2, 3]) b.bonus(Math.cos(a * 1.57 + 0.8) * 8, 0, Math.sin(a * 1.57 + 0.8) * 8);
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
      const eta = armContactEta(bot, lowAng(t), lowOmega(t), lowArms);
      const high = armContactEta(bot, highAng(t), highOmega(t), highArms);
      // Worse bots react late (and sometimes too late).
      return eta > 0.1 && eta < 0.15 + (bot.mem.react ?? 0.2) * 0.3 && high > 0.7;
    },
  });
  return { spawns, killY: -6, faceCenter: true, view: new THREE.Vector3(0, 3, 0), bot: brain };
});
