import * as THREE from 'three';
import { aimLanding, type Waypoint } from '../../sim/bots';
import { PAL, type PortalPair } from '../../sim/builder';
import {
  movingPlatforms,
  pickSections,
  pistons,
  portalFork,
  raceCourse,
  rotorDecks,
  type Segment,
  withRests,
} from '../../sim/course';
import { type BotView, defineMap } from '../../sim/map';
import { armContactEta } from '../../sim/props';
import meta from './meta';

const ONE_WAY = '#39e0d0';
const CANNON = '#ff8a3d';
const BLINK = '#a66bff';

/** Shut right now (after a trip, or out of its rhythm)? */
const shut = (pair: PortalPair, open: ((t: number) => boolean) | undefined, t: number) =>
  (t > pair.at && t < pair.closedUntil) || (!!open && !open(t));

/** A stretch of the route that walks into a portal at (x, z) and carries on at (x, zOut). */
function through(x: number, z: number): Waypoint[] {
  return [
    { x, z: z - 2.4, w: 0 },
    { x, z: z + 0.6, w: 0 },
  ];
}

/**
 * Islands over gaps nobody can jump, each swept by a low bar: two one-way portals at the far end of
 * each lead on to the next island (left to left, right to right).
 */
function portalIslands(n = 2): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 14;
    const gap = 13;
    const islands: { z: number; len: number }[] = [{ z: s.z, len: 7 }];
    for (let k = 1; k <= n; k++) {
      const prev = islands[k - 1]!;
      islands.push({ z: prev.z + prev.len + gap, len: 11 });
    }
    const routes: Waypoint[][] = [-1, 1].map(() => []);
    islands.forEach((isl, k) => {
      b.box(0, s.y - 1, isl.z + isl.len / 2, w, 2, isl.len, k % 2 ? PAL.teal : PAL.blue);
      let jumpWhen = (_bot: BotView) => false;
      if (k > 0) {
        const c = isl.z + isl.len / 2 + 0.5;
        const sp = (0.9 + rng() * 0.5) * (rng() < 0.5 ? -1 : 1);
        const ph = rng() * 6;
        const ang = (t: number) => (t <= 0 ? ph : ph + t * sp);
        b.hub(0, s.y, c, 0.8);
        b.rotor(0, s.y + 0.6, c, 6.4, 2, ang, 0.45);
        jumpWhen = (bot) => {
          const p = bot.body.pos;
          if (bot.t <= 0 || Math.abs(p.z - c) > 7 || Math.hypot(p.x, p.z - c) < 1.2) return false;
          const eta = armContactEta(bot, ang(bot.t), sp, 2, 0, c);
          return eta > 0.1 && eta < 0.24;
        };
      }
      [-1, 1].forEach((sx, r) => {
        const route = routes[r]!;
        const x = sx * 3.6;
        if (k === 0) route.push({ x, z: isl.z + 2, w: 0 });
        else route.push({ x, z: isl.z + isl.len / 2, w: 0, jumpWhen }, { x, z: isl.z + isl.len - 3.9, w: 0, jumpWhen });
        if (k === n) return;
        const ez = isl.z + isl.len - 1.5;
        const next = islands[k + 1]!;
        b.portal({ x, y: s.y, z: ez, yaw: 0 }, { x, y: s.y, z: next.z + 0.8, yaw: 0 }, ONE_WAY, { oneWay: true, closed: 0.3 });
        route.push({ x, z: ez + 0.6, w: 0, jumpWhen });
      });
    });
    const last = islands[n]!;
    const end = last.z + last.len;
    b.bonus(0, s.y, islands[1]!.z + 2);
    for (const r of routes) r.push({ x: 0, z: end + 0.5, w: 1 });
    return { z: end, y: s.y, routes };
  };
}

/**
 * A gap with three one-way portals in a row, each open for a moment in turn (their sashes shut in
 * between): go for the one that is about to open.
 */
function blinkingPortals(): Segment {
  return (s) => {
    const { b, rng } = s;
    const near = 9;
    const gap = 14;
    const far = s.z + near + gap;
    b.box(0, s.y - 1, s.z + near / 2, 17, 2, near, PAL.purple);
    b.box(0, s.y - 1, far + 4, 17, 2, 8, PAL.pink);
    const period = 3.4 + rng() * 1.2;
    const share = 0.42;
    const start = rng() * period;
    const routes: Waypoint[][] = [];
    [-5.2, 0, 5.2].forEach((x, k) => {
      const ph = start + (k * period) / 3 + (rng() - 0.5) * 0.3;
      const open = (t: number) => t > 0 && ((((t + ph) % period) + period) % period) / period < share;
      const ez = s.z + near - 1.6;
      const pair = b.portal({ x, y: s.y, z: ez, yaw: 0 }, { x, y: s.y, z: far + 0.6, yaw: 0 }, BLINK, {
        oneWay: true,
        closed: 0.3,
        open,
      });
      // A lamp over each: green while it is open.
      if (b.view) {
        const lamp = b.sphere(x, s.y + 3.35, ez, 0.26, PAL.red, { noCollide: true });
        const on = b.view.plain('#4fdc6a', { emissive: new THREE.Color('#4fdc6a'), emissiveIntensity: 0.9 });
        const off = b.view.plain('#ff6070', { emissive: new THREE.Color('#ff2040'), emissiveIntensity: 0.5 });
        b.anim((t) => {
          if (lamp.mesh) lamp.mesh.material = open(t) ? on : off;
        });
      }
      routes.push([
        { x, z: ez - 3.2, w: 0 },
        { x, z: ez + 0.6, w: 0, wait: (bot) => !shut(pair, open, bot.t + 0.15) && !shut(pair, open, bot.t + 0.5) },
        { x: 0, z: far + 5, w: 1 },
      ]);
    });
    b.bonus(0, s.y, s.z + 2.5);
    return {
      z: far + 8,
      y: s.y,
      routes,
      checkpoint: { from: far + 0.5, p: new THREE.Vector3(0, s.y + 0.1, far + 5) },
    };
  };
}

/**
 * A wall too high to climb: portals at its foot come out on top of it, and throw whoever comes out
 * over it and down onto the landing beyond. Bumpers stand in the way to the portals.
 */
function portalWall(): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 16;
    const h = 7;
    const wz = s.z + 12;
    b.box(0, s.y - 1, s.z + 6, w, 2, 12, PAL.blue);
    b.box(0, s.y + h / 2 - 1, wz, w, h + 2, 3, PAL.purple);
    b.box(0, s.y - 1, wz + 1.5 + 7, w, 2, 14, PAL.teal);
    for (const [x, z] of [
      [-2.4 - rng(), s.z + 4.5],
      [2.4 + rng(), s.z + 4.5],
    ] as const)
      b.bumper(x, s.y, z, 0.8, 10);
    const routes: Waypoint[][] = [];
    for (const x of [-5, 0, 5]) {
      const ez = wz - 3;
      b.portal({ x, y: s.y, z: ez, yaw: 0 }, { x, y: s.y + h, z: wz - 1.1, yaw: 0 }, CANNON, {
        oneWay: true,
        closed: 0.3,
        speed: 9,
        lift: 6,
      });
      const land = wz + 9;
      routes.push([
        ...through(x, ez),
        {
          x,
          z: land,
          w: 0,
          drive: (bot, out) => !bot.body.grounded && aimLanding(bot, x, s.y, land, out),
        },
        { x: 0, z: wz + 13, w: 1 },
      ]);
    }
    return {
      z: wz + 15.5,
      y: s.y,
      routes,
      checkpoint: { from: wz + 2, p: new THREE.Vector3(0, s.y + 0.1, wz + 11) },
    };
  };
}

/**
 * Portal cannons: step into a portal in the middle of a ledge and be shot out of the one at its edge,
 * across the gap and down onto the next ledge (steer in the air to land).
 */
function portalCannons(n = 2): Segment {
  return (s) => {
    const { b } = s;
    const len = 10;
    const gap = 11;
    const drop = 2.5;
    const routes: Waypoint[][] = [[], []];
    let z = s.z;
    let y = s.y;
    for (let k = 0; k <= n; k++) {
      b.box(0, y - 1, z + len / 2, 14, 2, len, k % 2 ? PAL.pink : PAL.blue);
      if (k === n) break;
      const ny = y - drop;
      const nz = z + len + gap;
      [-3.5, 3.5].forEach((x, r) => {
        const ez = z + 3.5;
        b.portal({ x, y, z: ez, yaw: 0 }, { x, y, z: z + len - 1.8, yaw: 0 }, CANNON, {
          oneWay: true,
          closed: 0.3,
          speed: 13,
          lift: 10,
        });
        const land = nz + 3.5;
        const yy = ny;
        routes[r]!.push(...through(x, ez), {
          x,
          z: land,
          w: 0,
          drive: (bot, out) => !bot.body.grounded && aimLanding(bot, x, yy, land, out),
        });
      });
      z = nz;
      y = ny;
    }
    for (const r of routes) r.push({ x: 0, z: z + len - 0.5, w: 1 });
    return {
      z: z + len,
      y,
      routes,
      checkpoint: { from: z + 0.5, p: new THREE.Vector3(0, y + 0.1, z + 5) },
    };
  };
}

/** Before the finish: a lone one-way portal over a last gap (a jump and a dive make it too, just). */
function lastHop(): Segment {
  return (s) => {
    const { b } = s;
    b.box(0, s.y - 1, s.z + 3, 10, 2, 6, PAL.purple);
    const far = s.z + 6 + 9;
    b.box(0, s.y - 1, far + 3, 10, 2, 6, PAL.purple);
    b.portal({ x: 0, y: s.y, z: s.z + 4, yaw: 0 }, { x: 0, y: s.y, z: far + 0.5, yaw: 0 }, ONE_WAY, {
      oneWay: true,
      closed: 0.3,
    });
    return {
      z: far + 6,
      y: s.y,
      routes: [[...through(0, s.z + 4), { x: 0, z: far + 5, w: 0.5 }]],
    };
  };
}

/**
 * Gaps that only portals cross: one-way portals from island to island (under sweeping bars), portals
 * that open in turn, portals at the foot of a wall that throw you over it, portal cannons across the
 * void; in between, a few more challenges drawn from the seed.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const middle = pickSections(b.rng, [portalFork(), rotorDecks(2), movingPlatforms(4), pistons(3)], 2);
    return raceCourse(b, ctx, {
      sections: withRests([
        portalIslands(2),
        blinkingPortals(),
        middle[0]!,
        portalWall(),
        middle[1]!,
        portalCannons(2),
        lastHop(),
      ]),
    });
  },
  ['neon', 'starlight', 'candy'],
);
