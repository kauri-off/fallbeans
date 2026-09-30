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
const N = 9;
/** Warning (shaking, reddening) before a plate drops, and how fast it falls away. */
const WARN = 1.1;
const DROP = 48;
const TINT_STEPS = 24;

interface Plate {
  x: number;
  z: number;
  fallAt: number;
  pal: Palette;
  obj: THREE.Object3D;
  col: Collider;
}

/**
 * Plates drop two or three at a time (a quick shake, then gone), over a bigger arena; the barriers
 * turning over it knock over whoever they catch and pass over them. The centre never drops.
 */
export default defineMap(
  meta,
  (b) => {
    const rng = b.rng;
    const plates: Plate[] = [];
    const pals: Palette[] = [PAL.purple, PAL.blue, PAL.pink, PAL.teal];
    const half = (N - 1) / 2;
    for (let i = 0; i < N; i++)
      for (let k = 0; k < N; k++) {
        const x = (i - half) * (PLATE + GAP);
        const z = (k - half) * (PLATE + GAP);
        if (Math.hypot(x, z) > (half + 0.6) * (PLATE + GAP)) continue;
        if (i === half && k === half) continue;
        const pal = pals[(i + k) % pals.length]!;
        const p = b.box(x, -0.5, z, PLATE, 1, PLATE, pal, { dynamic: true });
        plates.push({ x, z, fallAt: Number.POSITIVE_INFINITY, pal, obj: p.obj, col: p.col });
      }
    // The centre never falls: it carries the pillar with the beams.
    b.box(0, -0.5, 0, PLATE, 1, PLATE, PAL.yellow);
    b.hub(0, 0, 0, 0.9);
    const reach = (half + 0.5) * (PLATE + GAP);
    const dir = rng() < 0.5 ? 1 : -1;
    const LOW = spinUp(-0.22, 0.8 + rng() * 0.2, 0.004);
    const lowAngle = (t: number) => dir * LOW.angle(t);
    const HIGH_AT = 35 + rng() * 15;
    const highAngle = (t: number) => (t <= HIGH_AT ? 0 : -dir * (0.6 * (t - HIGH_AT) + 0.003 * (t - HIGH_AT) ** 2));
    b.rotor(0, 0.6, 0, reach, 2, lowAngle, 0.7);
    b.rotor(0, 2.45, 0, reach, 1, highAngle, 0.7);

    // Deterministic drop order, in groups of two or three: identical on server and clients, no events needed.
    let at = 8;
    const order = shuffle([...plates], rng);
    for (let k = 0; k < order.length; ) {
      const group = rng() < 0.5 ? 2 : 3;
      for (let g = 0; g < group && k < order.length; g++, k++) order[k]!.fallAt = at;
      at += Math.max(2.3, 5.2 - k * 0.07);
    }
    for (const r of [3, 7])
      for (let a = 0; a < 4; a++) b.bonus(Math.cos(a * 1.57 + r) * r * 1.3, 0, Math.sin(a * 1.57 + r) * r * 1.3);

    const base = new Map(plates.map((p) => [p, p.obj.position.clone()]));
    b.move((t) => {
      for (const p of plates) {
        const pos = base.get(p)!;
        const left = p.fallAt - t;
        p.col.enabled = left > 0;
        if (left > WARN) p.obj.position.copy(pos);
        else if (left > 0) p.obj.position.set(pos.x + Math.sin(t * 50 + p.x) * 0.08 * (1 - left / WARN), pos.y, pos.z);
        else p.obj.position.set(pos.x, pos.y - DROP * left * left, pos.z);
        p.obj.visible = left > -1;
      }
    });
    if (b.view) {
      // A steady one-way tint toward red instead of blinking (photosensitivity: no flashes).
      const view = b.view;
      const warn = new THREE.Color('#ff4a3a');
      const c = new THREE.Color();
      const tint = (hex: string, k: number) => `#${c.set(hex).lerp(warn, k).getHexString()}`;
      const ramps = new Map(
        pals.map((pal) => {
          const [c1, c2] = b.pal(pal) as Palette;
          const steps = Array.from({ length: TINT_STEPS + 1 }, (_, i) => {
            const k = (0.8 * i) / TINT_STEPS;
            return view.material([tint(c1, k), tint(c2, k)], undefined, 'plastic', b.style.pattern);
          });
          return [pal, steps] as const;
        }),
      );
      b.anim((t) => {
        for (const p of plates) {
          if (!(p.obj instanceof THREE.Mesh)) continue;
          const u = Math.min(1, Math.max(0, 1 - (p.fallAt - t) / WARN));
          const k = u * u * (3 - 2 * u);
          p.obj.material = ramps.get(p.pal)![Math.round(k * TINT_STEPS)]!;
        }
      });
    }
    b.clouds(0, 0, 55);

    const plateAt = (x: number, z: number) => plates.find((p) => Math.abs(p.x - x) < PLATE / 2 && Math.abs(p.z - z) < PLATE / 2);
    const brain = arenaBrain({
      radius: reach - 1,
      retarget: 1.2,
      ignoreNav: true,
      floor: (x, z, t) => {
        if (Math.abs(x) < PLATE / 2 + 0.1 && Math.abs(z) < PLATE / 2 + 0.1) return true;
        const p = plateAt(x, z);
        return !!p && p.fallAt - t > 0.3;
      },
      safe: (x, z, t) => {
        const r = Math.hypot(x, z);
        if (r < 2.8 || r > reach - 3) return false;
        const p = plateAt(x, z);
        return !!p && p.fallAt - t > WARN + 2;
      },
      jumpWhen: (bot) => {
        const t = Math.max(0, bot.t);
        if (t <= 0) return false;
        const eta = armContactEta(bot, lowAngle(t), dir * LOW.omega(t), 2);
        const high = t > HIGH_AT ? armContactEta(bot, highAngle(t), -dir * (0.6 + 0.006 * (t - HIGH_AT)), 1) : 9;
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
  },
  ['candy', 'ocean', 'circus'],
);
