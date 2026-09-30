import * as THREE from 'three';
import { follow, steer, type Waypoint } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { edgeJump, movingPlatforms, pickSections, raceCourse, type Segment, seesaws, withRests } from '../../sim/course';
import { type BotInput, type BotView, defineMap } from '../../sim/map';
import { armContactEta } from '../../sim/props';
import meta from './meta';

const ICE_PAL = ['#bfe9ff', '#e4f6ff'] as const;

/** A slope of ice down between rails, bumpers on the way, and a run-out of ice at the bottom. */
function iceSlope(drop = 5, len = 26): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 12;
    b.box(0, s.y - 1, s.z + 2, w, 2, 4, PAL.purple);
    const z0 = s.z + 4;
    const z1 = z0 + len;
    const y1 = s.y - drop;
    b.ramp(0, z0, s.y, z1, y1, w, ICE_PAL, 1, { slip: 0.9, surface: 'ice' });
    const ang = Math.atan2(drop, len);
    for (const sx of [-1, 1])
      b.box(sx * (w / 2 + 0.4), (s.y + y1) / 2 + 0.6, (z0 + z1) / 2, 0.8, 1.2, Math.hypot(len, drop), PAL.pink, {
        rot: [ang, 0, 0],
      });
    const yAt = (z: number) => s.y + ((z - z0) / len) * (y1 - s.y);
    for (let k = 0; k < 4; k++) {
      const bz = z0 + 5 + k * 5.5;
      b.bumper((k % 2 ? 1 : -1) * (1.2 + rng() * 2.8), yAt(bz) - 0.1, bz, 0.8, 9);
    }
    b.box(0, y1 - 1, z1 + 3, w, 2, 6, ICE_PAL, { slip: 0.9, surface: 'ice' });
    b.rails(z1, z1 + 6, w / 2, y1, PAL.pink);
    b.bonus(0, y1, z1 + 3);
    return {
      z: z1 + 6,
      y: y1,
      routes: [
        [
          { x: 0, z: z0 + len * 0.5, w: 3 },
          { x: 0, z: z1 + 5.5, w: 2 },
        ],
      ],
      forbidden: (p) => p.z > z0 && p.z < z1 + 6 && Math.abs(p.x) > w / 2 + 0.05,
    };
  };
}

/** Gaps a jump alone falls short of: jump from the edge, dive in the air to reach the other side. */
function diveGaps(n = 3, gap = 7.5): Segment {
  return (s) => {
    const { b } = s;
    const len = 6;
    const route: Waypoint[] = [];
    let z = s.z;
    for (let k = 0; k <= n; k++) {
      b.box(0, s.y - 1, z + len / 2, 9, 2, len, k % 2 ? PAL.teal : PAL.blue);
      // Take-off line near the edge.
      b.box(0, s.y + 0.01, z + len - 0.6, 9, 0.02, 0.35, PAL.yellow, { noCollide: true });
      if (k === n) break;
      const edge = z + len;
      const next = edge + gap;
      route.push(
        { x: 0, z: edge - 2, w: 0.5 },
        {
          x: 0,
          z: next + 2,
          w: 0,
          jumpWhen: edgeJump(edge, 0.9),
          // In the air past the edge, on the way down: dive for the far side.
          drive: (bot: BotView, out: BotInput) => {
            const body = bot.body;
            if (body.grounded || body.state !== 'normal' || body.pos.z < edge || body.vel.y > 2.5) return false;
            steer(bot, 0, next + 2, out);
            if (body.pos.z < next - 0.5) out.dive = true;
            return true;
          },
        },
      );
      z = next;
    }
    route.push({ x: 0, z: z + len - 1, w: 0.5 });
    return {
      z: z + len,
      y: s.y,
      routes: [route],
      checkpoint: { from: z + 0.5, p: new THREE.Vector3(0, s.y + 0.1, z + 3) },
    };
  };
}

/**
 * Low bars across an icy run: nobody gets under them standing up; a dive from the yellow stripes
 * slides under.
 */
function diveBars(n = 3): Segment {
  return (s) => {
    const { b } = s;
    const w = 12;
    const gapZ = 11;
    const len = n * gapZ + 4;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, ICE_PAL, { slip: 0.5, surface: 'ice' });
    b.rails(s.z, s.z + len, w / 2, s.y, PAL.pink);
    const route: Waypoint[] = [];
    for (let k = 0; k < n; k++) {
      const bz = s.z + 9 + k * gapZ;
      b.box(0, s.y + 1.2 + 1.5, bz, w + 1.6, 3, 0.8, k % 2 ? PAL.orange : PAL.purple);
      // Where to dive from.
      for (const dz of [7, 5.5]) b.box(0, s.y + 0.01, bz - dz, w, 0.02, 0.4, PAL.yellow, { noCollide: true });
      const key = `bar${Math.round(bz)}`;
      route.push({
        x: 0,
        z: bz + 2.5,
        w: 0,
        drive: (bot, out) => {
          const body = bot.body;
          const p = body.pos;
          if (p.z > bz + 0.6) return false;
          if (body.state !== 'normal') {
            out.mx = 0;
            out.mz = 1;
            return true;
          }
          // Stopped at the bar: back off for another run.
          if ((bot.mem[key] ?? -1) > bot.t) {
            follow(bot, 0, bz - 9, out);
            return true;
          }
          if (body.grounded && p.z > bz - 2.2 && Math.hypot(body.vel.x, body.vel.z) < 2) {
            bot.mem[key] = bot.t + 1;
            return true;
          }
          follow(bot, 0, bz + 3, out);
          out.mz = 1;
          if (body.grounded && p.z > bz - 7.2 && p.z < bz - 5.3) out.dive = true;
          return true;
        },
      });
    }
    route.push({ x: 0, z: s.z + len - 0.5, w: 0.5 });
    return {
      z: s.z + len,
      y: s.y,
      routes: [route],
      forbidden: (p) => p.z > s.z && p.z < s.z + len && p.y > s.y + 3.5,
    };
  };
}

/** Round decks of ice with sweeping bars: jump them without sliding off. */
function iceRotors(n = 2): Segment {
  return (s) => {
    const { b, rng } = s;
    let zz = s.z;
    const route: Waypoint[] = [];
    for (let i = 0; i < n; i++) {
      const r = 5.8 + rng();
      const c = zz + 4 + r;
      b.box(0, s.y - 1, (zz + c - r + 0.3) / 2, 3.6, 2, c - r + 0.3 - zz, PAL.yellow);
      b.cyl(0, s.y - 1, c, r + 0.3, 2, ICE_PAL, { slip: 0.35, surface: 'ice' });
      b.hub(0, s.y, c, 1);
      const sp = (0.9 + rng() * 0.4) * (rng() < 0.5 ? -1 : 1);
      const ph = rng() * 6;
      const ang = (t: number) => (t <= 0 ? ph : ph + t * sp);
      b.rotor(0, s.y + 0.6, c, r, 2, ang, 0.45);
      const jumpWhen = (bot: BotView) => {
        const p = bot.body.pos;
        if (bot.t <= 0 || Math.hypot(p.x, p.z - c) < 1.2 || Math.hypot(p.x, p.z - c) > r + 1) return false;
        const eta = armContactEta(bot, ang(bot.t), sp, 2, 0, c);
        return eta > 0.1 && eta < 0.24;
      };
      const side = rng() < 0.5 ? -1 : 1;
      route.push(
        { x: 0, z: c - r - 1.5, w: 0.3, jumpWhen },
        { x: side * 2.6, z: c - 2.5, w: 0.3, jumpWhen },
        { x: side * 2.6, z: c + 2.5, w: 0.3, jumpWhen },
        { x: 0, z: c + r + 1, w: 0.3, jumpWhen },
      );
      zz = c + r - 0.3;
    }
    b.box(0, s.y - 1, zz + 2, 3.6, 2, 4, PAL.yellow);
    return { z: zz + 4, y: s.y, routes: [route] };
  };
}

/** A platform shuttling between two docks (and up), resting at each end for a moment. */
interface Shuttle {
  pos(t: number): { x: number; y: number; z: number };
  /** −1 resting at the near dock, 1 at the far one, 0 on the way. */
  at(t: number): number;
}

function shuttle(
  x: number,
  zA: number,
  zB: number,
  yA: number,
  yB: number,
  dwell: number,
  travel: number,
  phase: number,
): Shuttle {
  const period = 2 * (dwell + travel);
  const ease = (u: number) => u * u * (3 - 2 * u);
  const u = (t: number) => {
    const f = (((t + phase) % period) + period) % period;
    if (f < dwell) return 0;
    if (f < dwell + travel) return ease((f - dwell) / travel);
    if (f < 2 * dwell + travel) return 1;
    return 1 - ease((f - 2 * dwell - travel) / travel);
  };
  return {
    pos: (t) => {
      const k = u(t);
      return { x, y: yA + (yB - yA) * k, z: zA + (zB - zA) * k };
    },
    at: (t) => {
      const k = u(t);
      return k <= 0 ? -1 : k >= 1 ? 1 : 0;
    },
  };
}

/**
 * Bots across a gap on shuttles: wait at the near dock for one to rest there, step on, keep to its
 * middle, step off when it rests at the far dock.
 */
function rideDrive(ferries: readonly Shuttle[], nearEdge: number, farEdge: number, farY: number) {
  return (bot: BotView, out: BotInput): boolean => {
    const body = bot.body;
    const p = body.pos;
    const t = bot.t;
    if (body.grounded && p.z > farEdge + 0.3 && Math.abs(p.y - farY) < 0.5) return false;
    let ride = ferries[0]!;
    for (const f of ferries) if (Math.abs(f.pos(t).x - p.x) < Math.abs(ride.pos(t).x - p.x)) ride = f;
    const fp = ride.pos(t);
    const aboard = p.z > nearEdge + 0.1 && p.z < farEdge - 0.1 && Math.abs(p.y - fp.y) < 0.8;
    if (aboard) {
      if (ride.at(t) === 1 && ride.at(t + 0.5) === 1) steer(bot, fp.x, farEdge + 2.5, out);
      else follow(bot, fp.x, fp.z, out);
      return true;
    }
    if (!body.grounded) {
      steer(bot, fp.x, fp.z, out);
      return true;
    }
    // On the near dock: the shuttle that rests here (and stays a moment), or the one that comes next.
    let next = ferries[0]!;
    let soonest = Infinity;
    for (const f of ferries)
      for (let dt = 0; dt < 8; dt += 0.25)
        if (f.at(t + dt) === -1 && f.at(t + dt + 0.9) === -1) {
          if (dt < soonest) {
            soonest = dt;
            next = f;
          }
          break;
        }
    const g = next.pos(t);
    // Lined up with its lane on the dock first, then straight on board.
    if (soonest > 0 || Math.abs(p.x - g.x) > 0.8) follow(bot, g.x, nearEdge - 1.2, out);
    else steer(bot, g.x, g.z, out);
    return true;
  };
}

/** Flying platforms: shuttles over the clouds to an island higher up, and others on to a higher dock. */
function skyShuttles(): Segment {
  return (s) => {
    const { b, rng } = s;
    const w = 14;
    const size = 4.4;
    const gap = 16;
    b.box(0, s.y - 1, s.z + 3, w, 2, 6, PAL.purple);
    const legs = [
      { near: s.z + 6, y0: s.y, y1: s.y + 2 },
      { near: s.z + 6 + gap + 6, y0: s.y + 2, y1: s.y + 4 },
    ];
    const route: Waypoint[] = [{ x: 0, z: s.z + 2.5, w: 1 }];
    const dwell = 1.8;
    const travel = 3.4;
    legs.forEach((leg, k) => {
      const far = leg.near + gap;
      const zA = leg.near + 0.35 + size / 2;
      const zB = far - 0.35 - size / 2;
      const ph = rng() * 10;
      const ferries = [-3, 3].map((x, i) => {
        const sh = shuttle(x, zA, zB, leg.y0 - 0.5, leg.y1 - 0.5, dwell, travel, ph + i * (dwell + travel));
        const m = b.box(x, leg.y0 - 0.5, zA, size, 1, size, i ? PAL.orange : PAL.green, { dynamic: true });
        const pod = b.cyl(0, -0.8, 0, 1.1, 0.7, '#39406b', { parent: m.obj, noCollide: true, surface: 'metal', seg: 20 });
        pod.obj.userData.dynamic = true;
        b.move((t) => {
          const q = sh.pos(t);
          m.obj.position.set(q.x, q.y, q.z);
        });
        return {
          pos: (t: number) => {
            const q = sh.pos(t);
            return { x: q.x, y: q.y + 0.5, z: q.z };
          },
          at: sh.at,
        };
      });
      const len = k === legs.length - 1 ? 7 : 6;
      b.box(0, leg.y1 - 1, far + len / 2, w, 2, len, k ? PAL.pink : PAL.teal);
      route.push({ x: 0, z: far + 2.5, w: 0.5, drive: rideDrive(ferries, leg.near, far, leg.y1) });
    });
    const end = legs[1]!.near + gap + 7;
    const topY = s.y + 4;
    route.push({ x: 0, z: end - 1, w: 1 });
    return {
      z: end,
      y: topY,
      routes: [route],
      checkpoint: { from: end - 6.5, p: new THREE.Vector3(0, topY + 0.1, end - 3.5) },
    };
  };
}

/**
 * Down an icy slope, over gaps only a dive makes, sliding under low bars, a section from the seed,
 * another slope, then flying shuttles over the clouds to the finish.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const extra = pickSections(b.rng, [iceRotors(2), movingPlatforms(4), seesaws(3)], 1);
    return raceCourse(b, ctx, {
      sections: withRests([iceSlope(5), diveGaps(3), diveBars(3), ...extra, iceSlope(4, 22), skyShuttles()]),
    });
  },
  ['snow', 'starlight', 'ocean'],
);
