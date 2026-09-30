import * as THREE from 'three';
import { shuffle } from '../../shared/rng';
import { arenaBrain } from '../../sim/bots';
import { PAL, type Palette } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import { armContactEta, spinUp } from '../../sim/props';
import meta from './meta';

const PLATE = 3.6;
const GAP = 0.25;
const N = 7;
const WARN = 2.2;

interface Plate {
  x: number;
  z: number;
  fallAt: number;
  obj: THREE.Object3D;
  col: Collider;
}

/** Starts 10° past a spawn and eases in (see jump-club). */
const LOW = spinUp(-0.22, 0.9, 0.004);
export const lowAngle = LOW.angle;
const HIGH_AT = 45;
const highAngle = (t: number) => (t <= HIGH_AT ? 0 : -(0.6 * (t - HIGH_AT) + 0.003 * (t - HIGH_AT) ** 2));

export default defineMap(meta, (b) => {
  b.style.pattern = 'checker';
  const plates: Plate[] = [];
  const pals: Palette[] = [PAL.purple, PAL.blue, PAL.pink, PAL.teal];
  const half = (N - 1) / 2;
  for (let i = 0; i < N; i++)
    for (let k = 0; k < N; k++) {
      const x = (i - half) * (PLATE + GAP);
      const z = (k - half) * (PLATE + GAP);
      if (Math.hypot(x, z) > (half + 0.6) * (PLATE + GAP)) continue;
      if (i === half && k === half) continue;
      const p = b.box(x, -0.5, z, PLATE, 1, PLATE, pals[(i + k) % pals.length]!, { dynamic: true });
      plates.push({ x, z, fallAt: Number.POSITIVE_INFINITY, obj: p.obj, col: p.col });
    }
  // The centre never falls: it carries the pillar with the beams.
  b.box(0, -0.5, 0, PLATE, 1, PLATE, PAL.yellow);
  b.hub(0, 0, 0, 0.9);
  const reach = (half + 0.5) * (PLATE + GAP);
  b.rotor(0, 0.6, 0, reach, 2, lowAngle, 0.7);
  b.rotor(0, 2.45, 0, reach, 1, highAngle, 0.7);

  // Deterministic drop order: identical on server and clients, no events needed.
  let at = 10;
  shuffle([...plates], b.rng).forEach((p, k) => {
    p.fallAt = at;
    at += Math.max(2.4, 6 - k * 0.14);
  });

  const base = new Map(plates.map((p) => [p, p.obj.position.clone()]));
  b.move((t) => {
    for (const p of plates) {
      const pos = base.get(p)!;
      const left = p.fallAt - t;
      p.col.enabled = left > 0;
      if (left > WARN) p.obj.position.copy(pos);
      else if (left > 0) p.obj.position.set(pos.x + Math.sin(t * 40 + p.x) * 0.06 * (1 - left / WARN), pos.y, pos.z);
      else p.obj.position.set(pos.x, pos.y - 12 * left * left, pos.z);
      p.obj.visible = left > -2;
    }
  });
  if (b.view) {
    const warnMat = b.view.plain('#ff6070', { emissive: new THREE.Color('#ff2040'), emissiveIntensity: 0.35 });
    const normal = new Map(plates.map((p) => [p, p.obj instanceof THREE.Mesh ? p.obj.material : null]));
    b.anim((t) => {
      for (const p of plates) {
        if (!(p.obj instanceof THREE.Mesh)) continue;
        const left = p.fallAt - t;
        const blink = left < WARN && left > 0 && Math.sin(t * (10 + (WARN - left) * 8)) > 0;
        p.obj.material = blink ? warnMat : (normal.get(p) ?? p.obj.material);
      }
    });
  }
  b.clouds(0, 0, 45);

  const plateAt = (x: number, z: number) => plates.find((p) => Math.abs(p.x - x) < PLATE / 2 && Math.abs(p.z - z) < PLATE / 2);
  const brain = arenaBrain({
    radius: reach - 1,
    retarget: 1.5,
    safe: (x, z, t) => {
      // Inner ring: slow beams, far from the edge.
      const r = Math.hypot(x, z);
      if (r < 2.8 || r > 9.5) return false;
      const p = plateAt(x, z);
      return !!p && p.fallAt - t > WARN + 1.5;
    },
    jumpWhen: (bot) => {
      const t = Math.max(0, bot.t);
      if (t <= 0) return false;
      const eta = armContactEta(bot, lowAngle(t), LOW.omega(t), 2);
      const high = t > HIGH_AT ? armContactEta(bot, highAngle(t), -(0.6 + 0.006 * (t - HIGH_AT)), 1) : 9;
      return eta > 0.1 && eta < 0.15 + (bot.mem.react ?? 0.2) * 0.3 && high > 0.7;
    },
  });

  return {
    spawns: b.ringSpawns(8, 8, 0.1, Math.PI / 8),
    killY: -12,
    faceCenter: true,
    view: new THREE.Vector3(0, 2, 0),
    bot: brain,
  };
});
