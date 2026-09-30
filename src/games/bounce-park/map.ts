import * as THREE from 'three';
import { aimLanding, type Hop, hopChain, type Waypoint } from '../../sim/bots';
import { type Builder, PAL } from '../../sim/builder';
import { bumperRamp, pickSections, raceCourse, type Segment, seesaws, trampolineGap, withRests } from '../../sim/course';
import { defineMap } from '../../sim/map';
import meta from './meta';

const CAPS = ['#ff5f6d', '#ff9f4a', '#a66bff', '#39c0ff', '#ff5fa2'];

/**
 * Giant mushrooms over the void, each cap a little higher than the last: bounce from cap to cap
 * (steer in the air) up to a ledge.
 */
function mushroomForest(n = 5, power = 16): Segment {
  return (s) => {
    const { b, rng } = s;
    b.box(0, s.y - 1, s.z + 3, 12, 2, 6, PAL.purple);
    const edge = s.z + 6;
    const hops: Hop[] = [];
    let x = 0;
    let z = edge + 3.2;
    let top = s.y - 0.4;
    for (let k = 0; k < n; k++) {
      const sc = 1.3 + rng() * 0.35;
      const base = top - 1.92 * sc;
      b.mushroom(x, base, z, sc, power, CAPS[Math.floor(rng() * CAPS.length)]);
      // A stalk down into the clouds (thinner than the stem: nothing to stand on).
      b.cyl(x, base - 5, z, 0.3 * sc, 10, PAL.green, { surface: 'leaf', seg: 16 });
      hops.push({ x, y: top, z, r: 0.98 * sc, power });
      if (k === n - 1) break;
      z += 5 + rng() * 1.2;
      x = Math.max(-4, Math.min(4, x + (rng() < 0.5 ? -1 : 1) * (1.5 + rng() * 1.8)));
      top += 0.7;
    }
    const landY = top + 1.2;
    const z0 = z + 3.5;
    b.box(0, landY - 1, z0 + 3.5, 12, 2, 7, PAL.pink);
    const land = { x: 0, y: landY, z: z0 + 2.5 };
    const route: Waypoint[] = [
      { x: 0, z: s.z + 2, w: 1 },
      { x: 0, z: land.z, w: 0.5, drive: hopChain(`mf${Math.round(s.z)}`, hops, land, edge) },
      { x: 0, z: z0 + 6, w: 1 },
    ];
    return {
      z: z0 + 7,
      y: landY,
      routes: [route],
      checkpoint: { from: z0 + 0.5, p: new THREE.Vector3(0, landY + 0.1, z0 + 3.5) },
    };
  };
}

/** A trampoline hovering over the void on a little engine, sliding from side to side. */
function hoverTrampoline(b: Builder, y: number, z: number, r: number, power: number, fx: (t: number) => number) {
  const holder = b.anchor(0, y - 0.19, z);
  b.cyl(0, -0.03, 0, r + 0.3, 0.3, PAL.orange, { parent: holder, dynamic: true, surface: 'rubber' });
  const mat = b.view?.pattern('#2b3a8f', '#3f57c9', 1.4, [1, 0], 0, 'fabric', 'dots');
  b.cyl(0, 0.14, 0, r, 0.1, PAL.blue, { parent: holder, dynamic: true, pad: power, material: mat, surface: 'fabric' });
  b.cyl(0, -0.55, 0, r * 0.45, 0.8, '#39406b', { parent: holder, noCollide: true, surface: 'metal', seg: 20 });
  b.cyl(0, -1.0, 0, r * 0.25, 0.25, PAL.yellow, { parent: holder, noCollide: true, surface: 'glossy', seg: 16 });
  b.move((t) => {
    holder.position.x = fx(t);
  });
}

/** Trampolines hovering over a gap, sliding from side to side: jump on, bounce across. */
function hoverTrampolines(n = 3, power = 15): Segment {
  return (s) => {
    const { b, rng } = s;
    b.box(0, s.y - 1, s.z + 3, 12, 2, 6, PAL.purple);
    const edge = s.z + 6;
    const y = s.y - 1;
    const hops: Hop[] = [];
    let z = edge + 4;
    for (let k = 0; k < n; k++) {
      const amp = 2 + rng() * 0.8;
      const w = 0.7 + rng() * 0.5;
      const ph = rng() * 6;
      const fx = (t: number) => Math.sin(t * w + ph) * amp;
      hoverTrampoline(b, y, z, 1.6, power, fx);
      hops.push({ x: fx, y, z, r: 1.6, power });
      if (k < n - 1) z += 5.5;
    }
    const z0 = z + 3.5;
    const landY = s.y + 1;
    b.box(0, landY - 1, z0 + 3.5, 12, 2, 7, PAL.teal);
    const land = { x: 0, y: landY, z: z0 + 2.5 };
    const first = hops[0]!;
    const fx0 = first.x as (t: number) => number;
    const route: Waypoint[] = [
      { x: 0, z: s.z + 2, w: 1 },
      {
        x: 0,
        z: land.z,
        w: 0.5,
        drive: hopChain(`ht${Math.round(s.z)}`, hops, land, edge, (bot) => Math.abs(fx0(bot.t + 0.55) - bot.body.pos.x) < 1),
      },
      { x: 0, z: z0 + 6, w: 1 },
    ];
    return {
      z: z0 + 7,
      y: landY,
      routes: [route],
      checkpoint: { from: z0 + 0.5, p: new THREE.Vector3(0, landY + 0.1, z0 + 3.5) },
    };
  };
}

/**
 * Walls too high to climb, with catapult pads before each (the chevrons show which way they throw):
 * over the wall and down the other side. Bumpers between the pads.
 */
function padCatapults(n = 3): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 12;
    const step = 13;
    const len = n * step + 4;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, PAL.blue);
    b.rails(s.z, s.z + len, w / 2, s.y, PAL.pink);
    const routes: Waypoint[][] = [[], []];
    for (let k = 0; k < n; k++) {
      const pz = s.z + 3 + k * step;
      b.box(0, s.y + 2, pz + 4, w, 4, 1, k % 2 ? PAL.orange : PAL.purple);
      b.bumper(0, s.y, pz + (rng() < 0.5 ? -0.5 : 0.5), 0.7, 9);
      [-3, 3].forEach((x, r) => {
        b.pad(x, s.y, pz, 1.4, 16, { x: 0, z: 8 });
        const land = pz + 9.3;
        routes[r]!.push(
          { x, z: pz - 2, w: 0 },
          { x, z: pz, w: 0 },
          { x, z: land, w: 0, drive: (bot, out) => !bot.body.grounded && aimLanding(bot, x, s.y, land, out) },
        );
      });
    }
    for (const r of routes) r.push({ x: 0, z: s.z + len - 0.5, w: 1 });
    return {
      z: s.z + len,
      y: s.y,
      routes,
      forbidden: (p) => p.y > s.y + 3.5 && p.z > s.z && p.z < s.z + len,
    };
  };
}

/** A big trampoline down in a pit, and a high ledge beyond it: bounce up and steer onto it. */
function bigBounce(rise = 6): Segment {
  return (s) => {
    const { b } = s;
    b.box(0, s.y - 1, s.z + 2.5, 12, 2, 5, PAL.purple);
    const edge = s.z + 5;
    const gap = 8;
    const basinY = s.y - 3;
    b.box(0, basinY - 1, edge + gap / 2, 12, 2, gap, PAL.blue);
    for (const sx of [-1, 1]) b.box(sx * 6.4, basinY + 1, edge + gap / 2, 0.8, 4, gap, PAL.pink);
    const tz = edge + gap * 0.45;
    const power = 23.5;
    const hops: Hop[] = [-2.8, 2.8].map((x) => {
      b.trampoline(x, basinY, tz, 1.9, power);
      return { x, y: basinY + 0.19, z: tz, r: 1.9, power };
    });
    const top = s.y + rise;
    const far = edge + gap;
    b.box(0, top - (rise + 3.5) / 2, far + 4, 12, rise + 3.5, 8, PAL.purple);
    const land = { x: 0, y: top, z: far + 2.5 };
    return {
      z: far + 8,
      y: top,
      routes: [-1, 1].map((k) => [
        { x: k * 2.8, z: s.z + 1.5, w: 0 },
        { x: 0, z: land.z, w: 0.5, drive: hopChain(`bb${k}`, k < 0 ? hops : [...hops].reverse(), land, edge) },
        { x: 0, z: far + 6, w: 1 },
      ]),
      checkpoint: { from: far + 0.5, p: new THREE.Vector3(0, top + 0.1, far + 4) },
      forbidden: (p) => p.z > edge && p.z < far && Math.abs(p.x) > 6 && p.y > basinY + 2.5,
    };
  };
}

/**
 * Bounce all the way: a mushroom forest, hovering trampolines, catapults over walls, one more thing
 * from the seed, a second (higher) forest and a giant trampoline up to the finish.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const extra = pickSections(b.rng, [bumperRamp(4), trampolineGap(), seesaws(3)], 1);
    return raceCourse(b, ctx, {
      sections: withRests([
        mushroomForest(5),
        hoverTrampolines(3),
        padCatapults(3),
        ...extra,
        mushroomForest(6, 17),
        bigBounce(6),
      ]),
    });
  },
  ['jungle', 'candy', 'meadow'],
);
