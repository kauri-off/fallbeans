import * as THREE from 'three';
import type { Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { hammerBridges, pickSections, raceCourse, type Segment, slidingGates, tippingBridge, withRests } from '../../sim/course';
import { type BotView, defineMap } from '../../sim/map';
import { armContactEta } from '../../sim/props';
import meta from './meta';

const STEP_PALS = [PAL.orange, PAL.yellow, PAL.green, PAL.teal, PAL.blue, PAL.purple];

/** Rock steps a little under the reach of a jump: jump, catch the edge, pull up. Boulders to hop over. */
function ledgeSteps(n = 4, rise = 1.5): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 14;
    const depth = 3.5;
    b.box(0, s.y - 1, s.z + 2, w, 2, 4, PAL.purple);
    let z = s.z + 4;
    let y = s.y;
    const route: Waypoint[] = [];
    for (let k = 0; k < n; k++) {
      y += rise;
      const d = k === n - 1 ? 7 : depth;
      const h = y - s.y + 3;
      b.box(0, y - h / 2, z + d / 2, w, h, d, STEP_PALS[k % STEP_PALS.length]!, { surface: 'rock' });
      if (k % 2 === 1 && k < n - 1) b.box(-4 + rng() * 8, y + 0.4, z + d / 2, 2.4, 0.8, 1.2, PAL.white, { surface: 'rock' });
      route.push({ x: 0, z: z + d / 2, w: 2 });
      z += d;
    }
    b.bonus(3, s.y + rise, s.z + 5.7);
    return {
      z,
      y,
      routes: [route],
      checkpoint: { from: z - 6.5, p: new THREE.Vector3(0, y + 0.1, z - 3.5) },
    };
  };
}

/**
 * A cliff face with ladders up it, and a pendulum swinging along the face at mid height: wait for it
 * to pass (or hang on and let it go by), then up.
 */
function ladderWall(h = 6): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 14;
    const wf = s.z + 10;
    b.box(0, s.y - 1, s.z + 5, w, 2, 10, PAL.blue);
    b.box(0, s.y + (h - 2) / 2, wf + 4, w, h + 2, 8, PAL.purple, { surface: 'rock' });
    const lxs = [-5.25, -1.75, 1.75, 5.25];
    for (const lx of lxs) b.ladder(lx, s.y, wf, s.y + h, Math.PI);
    // The pendulum: its head sweeps along the face over every ladder, at the height of a climber.
    const py = s.y + h + 3.2;
    const pz = wf - 1.2;
    for (const sx of [-7.4, 7.4]) b.box(sx, (s.y + py) / 2, pz, 0.8, py - s.y, 0.8, PAL.pink);
    b.box(0, py + 0.4, pz, 15.6, 0.8, 1.2, PAL.pink);
    const sp = 1.1 + rng() * 0.4;
    const ph = rng() * 6.28;
    b.hammer(0, py, pz, sp, ph, 1, false);
    const headX = (t: number) => 6 * Math.sin(Math.sin(t * sp + ph));
    const safe = (lx: number) => (bot: BotView) => {
      for (let dt = 0.1; dt <= 1.5; dt += 0.1) if (Math.abs(headX(bot.t + dt) - lx) < 2.4) return false;
      return true;
    };
    const top = s.y + h;
    return {
      z: wf + 8,
      y: top,
      routes: lxs.map((lx) => [
        { x: lx, z: wf - 4, w: 0 },
        { x: lx, z: wf + 1.2, w: 0, wait: safe(lx) },
        { x: 0, z: wf + 5, w: 1 },
      ]),
      checkpoint: { from: wf + 0.5, p: new THREE.Vector3(0, top + 0.1, wf + 5) },
    };
  };
}

/**
 * Narrow rock shelves stepping up from side to side over the void: jump across and up to the next
 * one (catch its edge if the jump is short).
 */
function zigzagLedges(n = 6, rise = 1.6): Segment {
  return (s) => {
    const { b, rng } = s;
    b.box(0, s.y - 1, s.z + 2.5, 10, 2, 5, PAL.purple);
    const shelves: { x: number; y: number; z: number }[] = [];
    let y = s.y;
    let z = s.z + 5 + 1.5;
    let side = rng() < 0.5 ? -1 : 1;
    for (let k = 0; k < n; k++) {
      y += rise;
      const x = side * 2.6;
      b.box(x, y - 1.5, z, 4, 3, 3, STEP_PALS[k % STEP_PALS.length]!, { surface: 'rock' });
      shelves.push({ x, y, z });
      side = -side;
      z += 2.4;
    }
    const z0 = z - 0.4;
    b.box(0, y + rise - 1, z0 + 3.5, 12, 2, 7, PAL.pink);
    const topY = y + rise;
    const route: Waypoint[] = [{ x: 0, z: s.z + 3, w: 0.5 }];
    const hops = [...shelves, { x: 0, y: topY, z: z0 + 1.5 }];
    // Jump from the edge of the shelf the bot stands on (it runs at the middle of the next one).
    let from = { x: 0, y: s.y, hx: 5, z1: s.z + 5 };
    hops.forEach((h, k) => {
      const f = from;
      route.push({
        x: h.x,
        z: h.z,
        w: 0,
        jumpWhen: (bot) => {
          const p = bot.body.pos;
          if (!bot.body.grounded || Math.abs(p.y - f.y) > 0.4) return false;
          return Math.abs(p.x - f.x) > f.hx - 0.7 || p.z > f.z1 - 0.7 || Math.hypot(p.x - h.x, p.z - h.z) < 3;
        },
      });
      from = { x: h.x, y: h.y, hx: 2, z1: h.z + 1.5 };
      if (k === hops.length - 1) route.push({ x: 0, z: z0 + 5, w: 1 });
    });
    return {
      z: z0 + 7,
      y: topY,
      routes: [route],
      checkpoint: { from: z0 + 0.5, p: new THREE.Vector3(0, topY + 0.1, z0 + 4) },
    };
  };
}

/**
 * A tower of shelves, each a ladder's climb above the last; sweeping bars on the shelves on the way
 * (jump them, even at the foot of a ladder).
 */
function ladderTower(levels = 3, rise = 4): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 12;
    const depth = 8;
    b.box(0, s.y - 1, s.z + 3.5, w, 2, 7, PAL.blue);
    const routes: Waypoint[][] = [[], []];
    let jumpWhen = (_bot: BotView) => false;
    let end = s.z;
    let top = s.y;
    for (let k = 1; k <= levels; k++) {
      const face = s.z + 7 + (k - 1) * depth;
      const y = s.y + k * rise;
      const d = k === levels ? 9 : depth;
      const h = y - s.y + 3;
      b.box(0, y - h / 2, face + d / 2, w, h, d, STEP_PALS[(k + 2) % STEP_PALS.length]!, { surface: 'rock' });
      const spread = 2 + rng() * 2.5;
      [-1, 1].forEach((sx, r) => {
        const lx = sx * spread;
        b.ladder(lx, y - rise, face, y, Math.PI, k % 2 ? '#ffb347' : '#f4f1ff');
        const jw = jumpWhen;
        routes[r]!.push({ x: lx, z: face - 2.6, w: 0, jumpWhen: jw }, { x: lx, z: face + 1.2, w: 0 });
      });
      jumpWhen = () => false;
      if (k < levels) {
        const c = face + depth / 2;
        const sp = (0.9 + rng() * 0.4) * (rng() < 0.5 ? -1 : 1);
        const ph = rng() * 6;
        const ang = (t: number) => (t <= 0 ? ph : ph + t * sp);
        b.hub(0, y, c, 0.7);
        b.rotor(0, y + 0.6, c, 3.5, 1, ang, 0.45);
        jumpWhen = (bot) => {
          const p = bot.body.pos;
          if (bot.t <= 0 || Math.abs(p.y - y) > 0.5 || Math.hypot(p.x, p.z - c) < 1.2) return false;
          const eta = armContactEta(bot, ang(bot.t), sp, 1, 0, c);
          return eta > 0.1 && eta < 0.24;
        };
      }
      end = face + d;
      top = y;
    }
    for (const r of routes) r.push({ x: 0, z: end - 2, w: 1 });
    return {
      z: end,
      y: top,
      routes,
      checkpoint: { from: end - 7.5, p: new THREE.Vector3(0, top + 0.1, end - 4) },
    };
  };
}

/**
 * Up and up: rock steps to catch the edges of, a cliff with ladders under a pendulum, something from
 * the seed, shelves zig-zagging up over the void, and a tower of ladders to the finish at the top.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const extra = pickSections(b.rng, [tippingBridge(5), hammerBridges(3), slidingGates(3, 3)], 1);
    return raceCourse(b, ctx, {
      sections: withRests([ledgeSteps(4), ladderWall(6), ...extra, zigzagLedges(6), ladderTower(3)]),
    });
  },
  ['snow', 'desert', 'castle'],
);
