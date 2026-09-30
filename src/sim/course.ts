import * as THREE from 'three';
import { z } from 'zod';
import { type Rng, shuffle } from '../shared/rng';
import { BOT_DT, initBot, pathBrain, type Waypoint } from './bots';
import { type Builder, PAL, type Palette } from './builder';
import type { BotBrain, BotView, Checkpoint, MapCtx, MapSpec } from './map';
import type { Collider } from './physics';
import { armContactEta, glovePuncher } from './props';

/**
 * Race courses built from sections. A map lists its sections (fixed ones, and pools the seed draws
 * from and shuffles); each section builds itself from where the last one ended, with its own timings
 * drawn from the seed, and tells the bots how to get through (one or more routes). Rest platforms
 * between them are checkpoints. Everything is deterministic: server and clients build the same course.
 */

export interface SegCtx {
  b: Builder;
  ctx: MapCtx;
  rng: Rng;
  /** Where the section starts (the end of the previous one) and the floor height there. */
  z: number;
  y: number;
  /** Event handlers and server ticks of this section (names are made unique per section). */
  on(name: string, fn: (data: unknown) => void): void;
  emit(name: string, data: unknown): void;
  tick(fn: (t: number) => void): void;
}

export interface SegOut {
  /** Where the next section starts, and its floor height. */
  z: number;
  y: number;
  /** Ways through for bots (each from z to the end); a bot picks one per section. */
  routes: Waypoint[][];
  /** Standing here counts as a shortcut (tops of walls, frames). */
  forbidden?: (p: THREE.Vector3) => boolean;
  /** This section ends on a platform worth a checkpoint (respawn at `p`, active past `from`). */
  checkpoint?: { from: number; p: THREE.Vector3 };
}

export type Segment = (s: SegCtx) => SegOut;

export interface CourseOpts {
  /** Sections in order (use pickSections for random ones). */
  sections: Segment[];
  /** Palette of the rest platforms. */
  restPal?: Palette;
  /** Length of the finish platform. */
  finishLen?: number;
  clouds?: number;
  /** A special end instead of the finish platform (e.g. a summit with a crown): builds it from (z, y). */
  finishWith?: (b: Builder, z: number, y: number) => { finish: NonNullable<MapSpec['finish']>; route: Waypoint[] };
}

/** A few sections drawn from a pool (without repeats) in a seeded order. */
export function pickSections(rng: Rng, pool: Segment[], n: number): Segment[] {
  return shuffle([...pool], rng).slice(0, n);
}

/** Alternates rest platforms between sections. */
export function withRests(list: Segment[], len = 7): Segment[] {
  return list.flatMap((s, i) => (i ? [rest(len), s] : [s]));
}

export const edgeJump =
  (edge: number, before = 1.1) =>
  (bot: BotView) =>
    bot.body.pos.z > edge - before && bot.body.pos.z < edge + 0.3;

const SegEvent = z.object({ k: z.string().max(64), d: z.unknown() });

/** Builds a race course and returns its spec (spawns, finish, checkpoints, events, bots). */
export function raceCourse(b: Builder, ctx: MapCtx, o: CourseOpts): MapSpec {
  const spawns = b.startArea(0);
  const handlers = new Map<string, (d: unknown) => void>();
  const ticks: ((t: number) => void)[] = [];
  const routes: Waypoint[][][] = [];
  const forbidden: ((p: THREE.Vector3) => boolean)[] = [];
  const checkpoints: Checkpoint[] = [{ z: -100, p: new THREE.Vector3(0, 0.1, 10) }];
  // Out of the start pen onto a platform (the first respawn point is on it).
  b.box(0, -1, 10, 18, 2, 6, PAL.purple);
  let zz = 13;
  let y = 0;
  let minY = 0;
  let maxY = 0;
  o.sections.forEach((seg, i) => {
    const key = `s${i}:`;
    const out = seg({
      b,
      ctx,
      rng: b.rng,
      z: zz,
      y,
      on: (name, fn) => handlers.set(key + name, fn),
      emit: (name, data) => ctx.emit('seg', { k: key + name, d: data }),
      tick: (fn) => ticks.push(fn),
    });
    routes.push(out.routes);
    if (out.forbidden) forbidden.push(out.forbidden);
    if (out.checkpoint) checkpoints.push({ z: out.checkpoint.from, p: out.checkpoint.p });
    zz = out.z;
    y = out.y;
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  });
  let finish: NonNullable<MapSpec['finish']>;
  if (o.finishWith) {
    const end = o.finishWith(b, zz, y);
    finish = end.finish;
    routes.push([end.route]);
    maxY = Math.max(maxY, end.finish.y + 1);
  } else {
    const len = o.finishLen ?? 16;
    b.box(0, y - 1, zz + len / 2, 18, 2, len, PAL.yellow);
    const finishZ = zz + 3;
    b.finish(0, y, finishZ);
    finish = { z: finishZ, y: y - 1 };
    routes.push([
      [
        { x: 0, z: finishZ - 1, w: 2 },
        { x: 0, z: finishZ + 5, w: 3 },
      ],
    ]);
  }
  b.clouds(0, zz / 2, Math.max(60, zz * 0.45), o.clouds ?? 40, minY - 30, maxY + 6);

  return {
    spawns,
    killY: minY - 14,
    finish,
    checkpoints,
    forbidden: (p) => forbidden.some((f) => f(p)),
    onEvent(name, data) {
      if (name !== 'seg') return;
      const e = SegEvent.safeParse(data);
      if (e.success) handlers.get(e.data.k)?.(e.data.d);
    },
    tick: (t) => {
      for (const f of ticks) f(t);
    },
    bot: courseBrain(routes),
  };
}

/** Each bot takes one route per section (chosen when it starts) and follows the joined path. */
export function courseBrain(sections: Waypoint[][][]): BotBrain {
  const brains = new Map<string, BotBrain>();
  return (bot, out) => {
    initBot(bot);
    let key = '';
    sections.forEach((r, i) => {
      const k = `r${i}`;
      if (bot.mem[k] === undefined) bot.mem[k] = r.length > 1 ? Math.floor(bot.rng() * r.length) : 0;
      key += `${bot.mem[k]}.`;
    });
    let brain = brains.get(key);
    if (!brain) {
      const pts = sections.flatMap((r, i) => r[bot.mem[`r${i}`] ?? 0] ?? r[0] ?? []);
      brain = pathBrain(pts);
      brains.set(key, brain);
    }
    brain(bot, out);
  };
}

// ------------------------------------------------------------------ sections

/** A plain platform (a checkpoint). */
export function rest(len = 7, w = 16, pal: Palette = PAL.purple): Segment {
  return (s) => {
    s.b.box(0, s.y - 1, s.z + len / 2, w, 2, len, pal);
    return {
      z: s.z + len,
      y: s.y,
      routes: [[{ x: 0, z: s.z + len / 2, w: 1.5 }]],
      checkpoint: { from: s.z + 0.5, p: new THREE.Vector3(0, s.y + 0.1, s.z + len / 2) },
    };
  };
}

/** A narrow bridge (connector). */
function bridge(b: Builder, z0: number, z1: number, y: number, w = 3.6, x = 0, pal: Palette = PAL.yellow) {
  b.box(x, y - 1, (z0 + z1) / 2, w, 2, z1 - z0, pal);
}

/**
 * Round decks with a hub and sweeping arms (a low one to jump, sometimes a high one to stay under),
 * joined by bridges. Speeds and directions from the seed.
 */
export function rotorDecks(n = 2): Segment {
  return (s) => {
    const { b, rng } = s;
    let zz = s.z;
    const route: Waypoint[] = [];
    for (let i = 0; i < n; i++) {
      const r = 5.6 + rng() * 1;
      const c = zz + 4 + r;
      bridge(b, zz, c - r + 0.3, s.y);
      b.cyl(0, s.y - 1, c, r + 0.3, 2, i % 2 ? PAL.pink : PAL.purple, { freq: 0.35 });
      b.hub(0, s.y, c, 1);
      const arms = rng() < 0.5 ? 2 : 3;
      const sp = (1.1 + rng() * 0.8) * (rng() < 0.5 ? -1 : 1);
      const ph = rng() * 6;
      const low = (t: number) => (t <= 0 ? ph : ph + t * sp);
      b.rotor(0, s.y + 0.6, c, r, arms, low, 0.45);
      const high = rng() < 0.55;
      const hsp = -Math.sign(sp) * (0.8 + rng() * 0.5);
      const highAng = (t: number) => (t <= 0 ? ph + 1.3 : ph + 1.3 + t * hsp);
      if (high) b.rotor(0, s.y + 2.45, c, r, 1, highAng, 0.45);
      if (i === 0) b.bonus(-r * 0.55, s.y, c);
      const jumpWhen = (bot: BotView) => {
        const p = bot.body.pos;
        const d = Math.hypot(p.x, p.z - c);
        if (d > r + 1.2 || d < 1.2 || bot.t <= 0) return false;
        const eta = armContactEta(bot, low(bot.t), sp, arms, 0, c);
        return eta > 0.1 && eta < 0.24 && (!high || armContactEta(bot, highAng(bot.t), hsp, 1, 0, c) > 0.8);
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
    bridge(b, zz, zz + 4, s.y);
    return { z: zz + 4, y: s.y, routes: [route] };
  };
}

/** Platforms sliding (or swinging) from side to side over a gap: jump across when the next one comes. */
export function movingPlatforms(n = 5): Segment {
  return (s) => {
    const { b, rng } = s;
    b.box(0, s.y - 1, s.z + 3, 10, 2, 6, PAL.purple);
    let zz = s.z + 6;
    const route: Waypoint[] = [{ x: 0, z: s.z + 4, w: 0.2 }];
    let edge = zz;
    for (let i = 0; i < n; i++) {
      const c = zz + 2 + 2.25;
      const sp = 0.9 + rng() * 0.7;
      const ph = rng() * 6;
      const amp = 3 + rng() * 1.5;
      const swing = rng() < 0.35;
      const fx = (t: number) => Math.sin(t * sp + ph) * amp;
      const fy = (t: number) => (swing ? -(1 - Math.cos(Math.sin(t * sp + ph) * 0.6)) * 3 : 0);
      const m = b.box(0, s.y - 0.5, c, 4.5, 1, 4.5, i % 2 ? PAL.orange : PAL.green, { dynamic: true });
      b.move((t) => {
        m.obj.position.x = fx(t);
        m.obj.position.y = s.y - 0.5 + fy(t);
      });
      if (i === Math.floor(n / 2)) b.bonus(0, s.y, c);
      const e = edge;
      route.push({
        x: fx,
        z: c,
        wait: (bot) => Math.abs(fx(bot.t + 0.6) - bot.body.pos.x) < 1.2 && Math.abs(fy(bot.t + 0.6)) < 0.5,
        jumpWhen: edgeJump(e, 1),
      });
      edge = c + 2.25;
      zz = c + 2.25;
    }
    const e = edge;
    zz += 2;
    b.box(0, s.y - 1, zz + 3, 10, 2, 6, PAL.purple);
    route.push({ x: 0, z: zz + 3, w: 0.5, jumpWhen: edgeJump(e, 1) });
    return {
      z: zz + 6,
      y: s.y,
      routes: [route],
      checkpoint: { from: zz + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zz + 3) },
    };
  };
}

/** Narrow bridges under swinging hammers (two of them side by side: pick one), timings from the seed. */
export function hammerBridges(n = 4): Segment {
  return (s) => {
    const { b, rng } = s;
    const len = n * 7 + 6.5;
    const routes: Waypoint[][] = [];
    for (const bx of [-4.5, 4.5]) {
      b.box(bx, s.y - 1, s.z + len / 2, 3.2, 2, len, bx < 0 ? PAL.blue : PAL.teal);
      const route: Waypoint[] = [{ x: bx, z: s.z + 0.8, w: 0 }];
      for (let k = 0; k < n; k++) {
        // The two bridges' hammers are staggered (their heads swing over the other bridge).
        const hz = s.z + 3.5 + k * 7 + (bx > 0 ? 3.5 : 0);
        const w = 1.7 + rng() * 0.9;
        const ph = rng() * 6.28;
        b.hammer(bx, s.y + 7.4, hz, w, ph, 1.12);
        const headX = (t: number) => bx + 6 * Math.sin(Math.sin(t * w + ph) * 1.12);
        route.push(
          { x: bx, z: hz - 2.6, w: 0 },
          {
            x: bx,
            z: hz + 2,
            w: 0,
            wait: (bot) => [0, 0.2, 0.4, 0.6, 0.8].every((dt) => Math.abs(headX(bot.t + dt) - bx) > 2.6),
          },
        );
      }
      route.push({ x: bx, z: s.z + len + 1, w: 0 });
      routes.push(route);
    }
    const zEnd = s.z + len;
    b.box(0, s.y - 1, zEnd + 3, 16, 2, 6, PAL.purple);
    return {
      z: zEnd + 6,
      y: s.y,
      routes,
      // On top of the hammer frames.
      forbidden: (p) => p.z > s.z && p.z < zEnd && p.y > s.y + 2,
      checkpoint: { from: zEnd + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zEnd + 3) },
    };
  };
}

/** Open fraction (0 shut … 1 open) of a door that opens `share` of every `period`, smoothly. */
const cycleOpen = (t: number, period: number, phase: number, share: number) => {
  const f = ((((t + phase) % period) + period) % period) / period;
  const ramp = 0.08;
  if (f < share) return Math.min(1, f / ramp, (share - f) / ramp);
  return 0;
};

/**
 * Walls across the course with sliding doors that open and shut on their own rhythms: time your run
 * through a door that is about to open (and do not get caught when it shuts).
 */
export function timedDoors(rows = 3, w = 16): Segment {
  return (s) => {
    const { b, rng } = s;
    const gapZ = 7;
    const len = rows * gapZ + 2;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, PAL.blue);
    b.rails(s.z, s.z + len, w / 2, s.y, PAL.pink);
    const routes: Waypoint[][] = [[], []];
    const doorW = 3.2;
    for (let r = 0; r < rows; r++) {
      const wz = s.z + 4 + r * gapZ;
      const doors = [-4.2, 4.2].map((dx) => ({
        x: dx + (rng() - 0.5) * 1.5,
        period: 3.2 + rng() * 1.8,
        phase: rng() * 5,
        share: 0.38 + rng() * 0.12,
      }));
      // Wall pieces around the two doorways.
      const edges = [
        -w / 2,
        doors[0]!.x - doorW / 2,
        doors[0]!.x + doorW / 2,
        doors[1]!.x - doorW / 2,
        doors[1]!.x + doorW / 2,
        w / 2,
      ];
      for (let k = 0; k < edges.length; k += 2) {
        const a = edges[k]!;
        const c = edges[k + 1]!;
        b.box((a + c) / 2, s.y + 1.6, wz, c - a, 3.2, 0.8, r % 2 ? PAL.orange : PAL.purple);
      }
      b.box(0, s.y + 3.5, wz, w, 0.6, 0.9, PAL.yellow);
      doors.forEach((d, di) => {
        for (const side of [-1, 1]) {
          const leaf = b.box(d.x + (side * doorW) / 4, s.y + 1.6, wz, doorW / 2, 3.2, 0.4, PAL.yellow, {
            dynamic: true,
            sinks: true,
            navSkip: true,
          });
          b.move((t) => {
            leaf.obj.position.x = d.x + side * (doorW / 4 + cycleOpen(t, d.period, d.phase, d.share) * (doorW / 2 - 0.05));
          });
        }
        const open = (t: number) => cycleOpen(t, d.period, d.phase, d.share);
        routes[di]!.push(
          { x: d.x, z: wz - 2.2, w: 0 },
          {
            x: d.x,
            z: wz + 1.6,
            w: 0,
            wait: (bot) => open(bot.t + 0.15) > 0.7 && open(bot.t + 0.55) > 0.7,
          },
        );
      });
    }
    b.bonus(0, s.y, s.z + 4 + gapZ / 2);
    for (const r of routes) r.push({ x: 0, z: s.z + len + 0.5, w: 1 });
    return { z: s.z + len, y: s.y, routes, forbidden: (p) => p.z > s.z && p.z < s.z + len && p.y > s.y + 2.5 };
  };
}

/**
 * A heavy gate that only opens now and then by itself, or while somebody stands on one of the
 * buttons beside it: hold it for the others (and lose time), or wait for your turn.
 */
export function coopGate(w = 16): Segment {
  return (s) => {
    const { b, rng } = s;
    const len = 16;
    const wz = s.z + 10;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, PAL.teal);
    b.rails(s.z, s.z + len, w / 2, s.y, PAL.pink);
    const gw = 4.4;
    for (const side of [-1, 1]) b.box(side * (gw / 2 + (w / 2 - gw / 2) / 2), s.y + 1.8, wz, w / 2 - gw / 2, 3.6, 1, PAL.purple);
    b.box(0, s.y + 3.9, wz, w, 0.6, 1.1, PAL.yellow);
    const period = 8 + rng() * 3;
    const phase = rng() * period;
    const RATE = 2.2;
    // Button state: transitions [time, pressed] from the server; the gate's lift follows from them.
    let pressed = false;
    let level0 = 0;
    let at = -1e9;
    const held = (t: number) =>
      pressed ? Math.min(1, level0 + (t - at) * RATE) : Math.max(0, level0 - Math.max(0, t - at) * RATE);
    const open = (t: number) => Math.max(cycleOpen(t, period, phase, 0.22), held(t));
    const setPressed = (on: boolean, t: number) => {
      level0 = held(t);
      at = t;
      pressed = on;
    };
    s.on('btn', (d) => {
      const e = z.object({ on: z.boolean(), at: z.number() }).safeParse(d);
      if (e.success) setPressed(e.data.on, e.data.at);
    });
    for (const side of [-1, 1]) {
      const leaf = b.box((side * gw) / 4, s.y + 1.8, wz, gw / 2, 3.6, 0.5, PAL.orange, {
        dynamic: true,
        sinks: true,
        navSkip: true,
      });
      b.move((t) => {
        leaf.obj.position.x = side * (gw / 4 + open(t) * (gw / 2 - 0.05));
      });
    }
    // Buttons: round plates at both sides of the approach.
    const buttons: Collider[] = [];
    const bx = w / 2 - 2;
    const bz = s.z + 4;
    for (const side of [-1, 1]) {
      b.cyl(side * bx, s.y + 0.02, bz, 1.25, 0.1, '#5a3fb8', { noCollide: true, surface: 'rubber' });
      const top = b.cyl(side * bx, s.y + 0.1, bz, 1, 0.2, PAL.red, { surface: 'rubber' });
      buttons.push(top.col);
      const base = top.obj.position.y;
      b.anim((t) => {
        top.obj.position.y = base - (pressed || held(t) > 0 ? 0.08 : 0);
      });
    }
    // A lamp over the gate: green while it is open.
    if (b.view) {
      const lamp = b.sphere(0, s.y + 4.6, wz, 0.35, PAL.red, { noCollide: true });
      const on = b.view.plain('#4fdc6a', { emissive: new THREE.Color('#4fdc6a'), emissiveIntensity: 0.9 });
      const off = b.view.plain('#ff6070', { emissive: new THREE.Color('#ff2040'), emissiveIntensity: 0.5 });
      b.anim((t) => {
        if (lamp.mesh) lamp.mesh.material = open(t) > 0.5 ? on : off;
      });
    }
    // Server: somebody standing on a button holds the gate open.
    s.tick((t) => {
      let any = false;
      for (const body of s.ctx.bodies().values()) if (body.grounded && buttons.includes(body.groundCol as Collider)) any = true;
      if (any !== pressed && t >= 0) {
        setPressed(any, t);
        s.emit('btn', { on: any, at: t });
      }
    });
    const route: Waypoint[] = [
      { x: 0, z: s.z + 2, w: 1 },
      {
        x: 0,
        z: wz - 2,
        w: 0.8,
        // Helpful bots go and stand on a button for a while when the gate is shut and nobody helps.
        detour: (bot) => {
          const k = 'help';
          if ((bot.mem[k] ?? -1e9) > bot.t) {
            const side = (bot.mem.helpSide ?? 1) as number;
            return { x: side * bx, z: bz };
          }
          if (bot.mem.helped === wz || bot.body.pos.z > wz - 3.5 || bot.t <= 0) return null;
          if (open(bot.t) < 0.3 && !pressed && (bot.mem.aggro ?? 0.5) < 0.35 && bot.rng() < 0.4 * BOT_DT * 10) {
            bot.mem.helped = wz;
            bot.mem.helpSide = bot.body.pos.x < 0 ? -1 : 1;
            bot.mem[k] = bot.t + 3 + bot.rng() * 3;
            return { x: bot.mem.helpSide * bx, z: bz };
          }
          return null;
        },
      },
      { x: 0, z: wz + 2, w: 0.3, wait: (bot) => open(bot.t) > 0.75 && open(bot.t + 0.5) > 0.7 },
      { x: 0, z: s.z + len - 1, w: 1 },
    ];
    return { z: s.z + len, y: s.y, routes: [route], forbidden: (p) => Math.abs(p.z - wz) < 1 && p.y > s.y + 2 };
  };
}

/** Rows of doors: most are solid, some burst open when you run into them (the first one through finds out). */
export function doorRows(rows = 2, w = 17): Segment {
  return (s) => {
    const { b, rng, ctx } = s;
    const gapZ = 9;
    const len = rows * gapZ + 3;
    b.box(0, s.y - 1, s.z + len / 2, w + 1, 2, len, PAL.blue);
    b.rails(s.z, s.z + len, (w + 1) / 2, s.y, PAL.pink);
    interface Door {
      obj: THREE.Object3D;
      breakable: boolean;
      broken: boolean;
      t: number;
      col: Collider;
      x: number;
      row: number;
    }
    const doors: Door[] = [];
    const n = 5;
    const dw = w / n;
    const rowZ: number[] = [];
    for (let r = 0; r < rows; r++) {
      const wz = s.z + 5 + r * gapZ;
      rowZ.push(wz);
      const nBreak = rng() < 0.5 ? 1 : 2;
      const idx = shuffle([0, 1, 2, 3, 4], rng).slice(0, nBreak);
      for (let i = 0; i < n; i++) {
        const x = -w / 2 + dw / 2 + i * dw;
        const obj = b.model('door');
        obj.position.set(x, s.y, wz);
        obj.scale.x = dw / 3.1;
        obj.rotation.y = Math.PI;
        const id = doors.length;
        const d: Door = {
          obj,
          breakable: idx.includes(i),
          broken: false,
          t: 0,
          col: b.collider(
            b.anchor(x, s.y + 1.6, wz),
            { type: 'box', hx: dw / 2, hy: 1.6, hz: 0.3 },
            { isStatic: true, navSkip: true },
          ),
          x,
          row: r,
        };
        if (d.breakable)
          d.col.onTouch = () => {
            if (ctx.server) s.emit('door', id);
            else breakDoor(id);
          };
        doors.push(d);
      }
      b.box(0, s.y + 3.6, wz, w + 0.4, 0.8, 1.0, PAL.yellow);
    }
    function breakDoor(id: number) {
      const d = doors[id];
      if (!d || d.broken || !d.breakable) return;
      d.broken = true;
      d.col.enabled = false;
      ctx.sfx('break');
    }
    s.on('door', (d) => {
      if (typeof d === 'number') breakDoor(d);
    });
    b.anim((_t, dt) => {
      for (const d of doors) {
        if (!d.broken || !d.obj.visible) continue;
        d.t += dt;
        d.obj.rotation.x = Math.min(Math.PI / 2, d.t * d.t * 7);
        if (d.t > 1.4) d.obj.visible = false;
      }
    });
    const route: Waypoint[] = [];
    rowZ.forEach((wz, r) => {
      const row = doors.filter((d) => d.row === r);
      const pickKey = `door${Math.round(wz)}`;
      const triedKey = `tried${Math.round(wz)}`;
      route.push({
        x: 0,
        z: wz + 1.5,
        w: 0.5,
        // Like a player: take a door someone broke, or barge into one; if it holds, try the next.
        detour: (bot) => {
          const p = bot.body.pos;
          if (p.z > wz + 0.6) return null;
          let pick = bot.mem[pickKey];
          const open = row.map((d, i) => (d.broken ? i : -1)).filter((i) => i >= 0);
          if (open.length && (pick === undefined || !row[pick]?.broken)) {
            const nearest = open.reduce((a, i) => (Math.abs(row[i]!.x - p.x) < Math.abs(row[a]!.x - p.x) ? i : a));
            if (Math.abs(row[nearest]!.x - p.x) < 7) pick = nearest;
          }
          if (pick === undefined) {
            const mask = bot.mem[triedKey] ?? 0;
            const options = [0, 1, 2, 3, 4].filter((i) => !(mask & (1 << i)));
            const pref = p.x + (bot.mem.off ?? 0) * 3;
            options.sort((a, c) => Math.abs(row[a]!.x - pref) - Math.abs(row[c]!.x - pref));
            pick = options[bot.rng() < 0.7 ? 0 : Math.min(options.length - 1, 1)] ?? Math.floor(bot.rng() * 5);
          }
          bot.mem[pickKey] = pick;
          const door = row[pick]!;
          if (!door.broken && p.z > wz - 1.05 && Math.abs(p.x - door.x) < 1.2) {
            bot.mem.push = (bot.mem.push ?? 0) + BOT_DT;
            if ((bot.mem.push ?? 0) > 0.25 + (bot.mem.react ?? 0.15)) {
              bot.mem[triedKey] = (bot.mem[triedKey] ?? 0) | (1 << pick);
              bot.mem[pickKey] = undefined;
              bot.mem.push = 0;
            }
          } else bot.mem.push = 0;
          const aligned = Math.abs(p.x - door.x) < 0.5;
          return door.broken || aligned || p.z > wz - 1.5 ? { x: door.x, z: wz + 2 } : { x: door.x, z: wz - 1.4 };
        },
      });
    });
    route.push({ x: 0, z: s.z + len - 0.5, w: 1 });
    return { z: s.z + len, y: s.y, routes: [route], forbidden: (p) => p.z > s.z && p.z < s.z + len && p.y > s.y + 3 };
  };
}

/** A climb up a ramp through bumpers, with gloves punching out of the rails. */
export function bumperRamp(rise = 4, len = 22): Segment {
  return (s) => {
    const { b, rng } = s;
    const z0 = s.z;
    const z1 = s.z + len;
    const w = 12;
    b.ramp(0, z0, s.y, z1, s.y + rise, w, PAL.blue);
    const ang = Math.atan2(rise, len);
    const yAt = (zz: number) => s.y + ((zz - z0) / len) * rise;
    for (const sx of [-1, 1])
      b.box(sx * (w / 2 + 0.4), (s.y * 2 + rise) / 2 + 0.6, (z0 + z1) / 2, 0.8, 1.2, len + 0.4, PAL.pink, { rot: [-ang, 0, 0] });
    for (let k = 0; k < 5; k++) {
      const bz = z0 + 4 + k * ((len - 7) / 4);
      const bx = (k % 2 ? 1 : -1) * (1.5 + rng() * 2.5);
      b.bumper(bx, yAt(bz) - 0.1, bz, 0.9, 11);
    }
    for (const [f, side] of [
      [0.3, -1],
      [0.62, 1],
    ] as const) {
      const gz = z0 + len * f;
      glovePuncher(b, {
        x: side * 7.6,
        y: yAt(gz) + 0.95,
        z: gz,
        side,
        w: 1 + rng() * 0.4,
        ph: rng() * 6,
        reach: 5.2,
        scale: 1.2,
        postTo: s.y - 2,
      });
    }
    b.box(0, s.y + rise - 1, z1 + 2, 14, 2, 4, PAL.purple);
    return {
      z: z1 + 4,
      y: s.y + rise,
      routes: [
        [
          { x: 0, z: z0 + len * 0.45, w: 3 },
          { x: 0, z: z1 + 2, w: 2 },
        ],
      ],
      forbidden: (p) => p.z > z0 && p.z < z1 && Math.abs(p.x) > w / 2 + 0.05,
    };
  };
}

/** Tilting seesaw platforms in a zig-zag: jump across as they level out. */
export function seesaws(n = 3): Segment {
  return (s) => {
    const { b, rng } = s;
    let zz = s.z + 3;
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 1 }];
    b.box(0, s.y - 1, s.z + 1.5, 12, 2, 3, PAL.purple);
    let edge = s.z + 3;
    for (let i = 0; i < n; i++) {
      const x = (i % 2 ? 1 : -1) * 2.5;
      const c = zz + 2 + 3.75;
      const w1 = 1 + rng() * 0.5;
      const ph = rng() * 6;
      const pl = b.box(x, s.y - 0.5, c, 7.5, 1, 7.5, i % 2 ? PAL.pink : PAL.teal, { dynamic: true });
      b.move((t) => {
        pl.obj.rotation.z = Math.sin(t * w1 + ph) * 0.36;
        pl.obj.rotation.x = Math.sin(t * 0.8 + ph) * 0.12;
      });
      route.push({ x, z: c, w: 0.3, jumpWhen: edgeJump(edge) });
      edge = c + 3.75;
      zz = c + 3.75;
    }
    zz += 2;
    b.box(0, s.y - 1, zz + 3, 12, 2, 6, PAL.purple);
    route.push({ x: 0, z: zz + 3, w: 0.5, jumpWhen: edgeJump(edge) });
    return {
      z: zz + 6,
      y: s.y,
      routes: [route],
      checkpoint: { from: zz + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zz + 3) },
    };
  };
}

/** A conveyor belt running back at you, with punching walls and bumpers. */
export function conveyor(len = 30): Segment {
  return (s) => {
    const { b, rng } = s;
    const speed = 3.6 + rng() * 1.2;
    const conv = b.view?.pattern('#8a8f9e', '#c7ccd8', 0.9, [0, 1], speed * 0.9, 'rubber');
    const cz = s.z + len / 2;
    b.box(0, s.y - 1, cz, 9, 2, len, PAL.white, { material: conv, conveyor: new THREE.Vector3(0, 0, -speed) });
    for (const sx of [-1, 1]) b.box(sx * 4.9, s.y + 0.6, cz, 0.8, 1.2, len, PAL.yellow);
    const route: Waypoint[] = [];
    const punches = Math.floor(len / 9);
    for (let k = 0; k < punches; k++) {
      const pz = s.z + 5 + k * 8.5;
      const side = k % 2 ? 1 : -1;
      const w = 1.4 + rng() * 0.6;
      const ph = rng() * 6;
      const px = (t: number) => side * (5.6 - 3.2 * Math.max(0, Math.sin(t * w + ph)));
      const m = b.box(0, s.y + 0.9, pz, 3.4, 1.8, 1.2, PAL.orange, { dynamic: true, hit: 0.9, tag: 'pusher', sinks: true });
      b.move((t) => {
        m.obj.position.x = px(t);
      });
      b.bumper(-side * 2.9, s.y, pz + 4, 0.75, 10);
      const lane = -side * 1.9;
      route.push(
        { x: lane, z: pz - 2, w: 0 },
        { x: lane, z: pz + 1.5, w: 0, wait: (bot) => Math.abs(px(bot.t + 0.4)) > 3.4 || side * lane < 0 },
      );
    }
    b.bonus(0, s.y, s.z + len * 0.5);
    route.push({ x: 0, z: s.z + len + 0.5, w: 0.5 });
    return {
      z: s.z + len,
      y: s.y,
      routes: [route],
      forbidden: (p) => p.z > s.z + 1 && p.z < s.z + len - 1 && Math.abs(p.x) > 4.3 && p.y > s.y + 0.6,
    };
  };
}

/**
 * A gap with a trampoline down in it: bounce up to the platform on the other side (a raised one).
 * Missing just means another bounce.
 */
export function trampolineGap(): Segment {
  return (s) => {
    const { b } = s;
    const rise = 2;
    const basinY = s.y - 3;
    const gap = 9;
    b.box(0, s.y - 1, s.z + 2, 12, 2, 4, PAL.purple);
    // A basin under the gap (with a rim), and the trampoline in it.
    b.box(0, basinY - 1, s.z + 4 + gap / 2, 12, 2, gap, PAL.blue);
    for (const sx of [-1, 1]) b.box(sx * 6.4, basinY + 1, s.z + 4 + gap / 2, 0.8, 4, gap, PAL.pink);
    const tz = s.z + 4 + gap * 0.42;
    for (const tx of [-2.8, 2.8]) b.trampoline(tx, basinY, tz, 1.9, 19.5);
    const far = s.z + 4 + gap;
    b.box(0, s.y + rise - 2.5, far + 4, 12, 5, 8, PAL.purple);
    const routes: Waypoint[][] = [-2.8, 2.8].map((tx) => [
      { x: tx, z: tz, w: 0 },
      { x: tx * 0.5, z: far + 2.5, w: 0.3 },
      { x: 0, z: far + 5, w: 1 },
    ]);
    return {
      z: far + 8,
      y: s.y + rise,
      routes,
      checkpoint: { from: far + 0.5, p: new THREE.Vector3(0, s.y + rise + 0.1, far + 4) },
      forbidden: (p) => p.z > s.z + 4 && p.z < far && Math.abs(p.x) > 6 && p.y > basinY + 2.5,
    };
  };
}

/**
 * A fork: the long way round (a zig-zag between walls), or a leap over a gap to a portal that
 * puts you at the far end.
 */
export function portalFork(): Segment {
  return (s) => {
    const { b, rng } = s;
    const len = 26;
    // Right: the long zig-zag.
    b.box(4.5, s.y - 1, s.z + len / 2, 7, 2, len, PAL.green);
    for (const sx of [1, 8]) b.box(sx, s.y + 1.2, s.z + len / 2, 0.8, 2.4, len, PAL.pink);
    const zig: Waypoint[] = [{ x: 4.5, z: s.z + 1, w: 0 }];
    for (let k = 0; k < 4; k++) {
      const wz = s.z + 4 + k * 5.5;
      const left = k % 2 === 0;
      b.box(left ? 3.5 : 5.5, s.y + 1.2, wz, 4.2, 2.4, 0.8, PAL.purple);
      const gx = left ? 6.3 : 2.7;
      zig.push({ x: gx, z: wz - 1.4, w: 0 }, { x: gx, z: wz + 1.4, w: 0 });
    }
    zig.push({ x: 4.5, z: s.z + len + 1, w: 0 });
    // Left: a short run, a gap, the portal.
    b.box(-4.5, s.y - 1, s.z + 4, 6, 2, 8, PAL.blue);
    const gapEnd = s.z + 8 + 3 + rng() * 0.6;
    b.box(-4.5, s.y - 1, gapEnd + 2, 5, 2, 4, PAL.blue);
    const pz = gapEnd + 2.4;
    b.box(0, s.y - 1, s.z + len + 3, 16, 2, 6, PAL.purple);
    b.portal({ x: -4.5, y: s.y, z: pz, yaw: Math.PI }, { x: -4.5, y: s.y, z: s.z + len + 3.2, yaw: 0 }, '#39e0d0');
    b.bonus(-4.5, s.y, s.z + 5);
    const hop: Waypoint[] = [
      { x: -4.5, z: s.z + 5, w: 0 },
      { x: -4.5, z: gapEnd + 1.2, w: 0, jumpWhen: edgeJump(s.z + 8) },
      { x: -4.5, z: pz + 0.5, w: 0 },
      { x: 0, z: s.z + len + 5, w: 1 },
    ];
    zig.push({ x: 0, z: s.z + len + 5, w: 1 });
    return {
      z: s.z + len + 6,
      y: s.y,
      routes: [zig, hop],
      forbidden: (p) => p.z > s.z && p.z < s.z + len && p.x > 0 && p.y > s.y + 2,
      checkpoint: { from: s.z + len + 0.5, p: new THREE.Vector3(2, s.y + 0.1, s.z + len + 4) },
    };
  };
}

/** A walkway with gloves punching across it from both sides, out of rhythm with each other. */
export function gloveAlley(n = 4): Segment {
  return (s) => {
    const { b, rng } = s;
    const len = n * 5 + 4;
    const w = 7;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, PAL.teal);
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 0 }];
    for (let k = 0; k < n; k++) {
      const gz = s.z + 3 + k * 5;
      const side = k % 2 ? 1 : -1;
      const gx = glovePuncher(b, {
        x: side * (w / 2 + 2.6),
        y: s.y + 0.95,
        z: gz,
        side,
        w: 1.1 + rng() * 0.5,
        ph: rng() * 6,
        reach: w * 0.8,
        scale: 1.3,
        postTo: s.y - 4,
      });
      const rest = side * (w / 2 + 2.6);
      route.push(
        { x: 0, z: gz - 2.2, w: 0 },
        {
          x: 0,
          z: gz + 1.8,
          w: 0,
          wait: (bot) => [0, 0.25, 0.5].every((dt) => Math.abs(gx(bot.t + dt) - rest) < 1.2),
        },
      );
    }
    route.push({ x: 0, z: s.z + len + 0.5, w: 0.5 });
    return { z: s.z + len, y: s.y, routes: [route] };
  };
}

/** A ramp (or flat run) through walls with a gap sliding from side to side. */
export function slidingGates(n = 4, rise = 5): Segment {
  return (s) => {
    const { b, rng } = s;
    const len = n * 8 + 4;
    const w = 16;
    const z0 = s.z;
    const z1 = s.z + len;
    const yAt = (zz: number) => s.y + ((zz - z0) / len) * rise;
    b.ramp(0, z0, s.y, z1, s.y + rise, w, PAL.teal);
    const ang = Math.atan2(rise, len);
    for (const sx of [-1, 1])
      b.box(sx * (w / 2 + 0.4), (2 * s.y + rise) / 2 + 0.6, (z0 + z1) / 2, 0.8, 1.2, Math.hypot(len, rise), PAL.pink, {
        rot: [-ang, 0, 0],
      });
    const route: Waypoint[] = [];
    for (let k = 0; k < n; k++) {
      const gz = z0 + 5 + k * 8;
      const gw = 0.8 + rng() * 0.6;
      const ph = rng() * 6;
      const gap = 3.6 - k * 0.15;
      const gx = (t: number) => Math.sin(t * gw + ph) * (w / 2 - gap / 2 - 0.4);
      const gate = b.anchor(0, yAt(gz), gz);
      for (const side of [-1, 1])
        b.box(side * (gap / 2 + w / 2), 1.4, 0, w, 3.4, 0.8, k % 2 ? PAL.orange : PAL.purple, {
          parent: gate,
          dynamic: true,
          tag: 'gate',
        });
      b.move((t) => {
        gate.position.x = gx(t);
      });
      route.push(
        { x: 0, z: gz - 3, w: 0.5 },
        { x: gx, z: gz + 1.2, wait: (bot: BotView) => Math.abs(gx(bot.t + 0.45) - bot.body.pos.x) < 1.2 },
        { x: gx, z: gz + 2.5 },
      );
    }
    b.box(0, s.y + rise - 1, z1 + 3, 16, 2, 6, PAL.purple);
    route.push({ x: 0, z: z1 + 3, w: 1 });
    return {
      z: z1 + 6,
      y: s.y + rise,
      routes: [route],
      forbidden: (p) => p.z > z0 && p.z < z1 && (Math.abs(p.x) > w / 2 + 0.05 || p.y > yAt(p.z) + 2.8),
      checkpoint: { from: z1 + 0.5, p: new THREE.Vector3(0, s.y + rise + 0.1, z1 + 3) },
    };
  };
}

/**
 * A narrow bridge of flaps that tip over now and then (dropping whoever is on them): cross each
 * while it stays level.
 */
export function tippingBridge(n = 6): Segment {
  return (s) => {
    const { b, rng } = s;
    const flap = 3;
    b.box(0, s.y - 1, s.z + 1, 6, 2, 2, PAL.purple);
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 0 }];
    let zz = s.z + 2;
    for (let k = 0; k < n; k++) {
      const c = zz + flap / 2 + 0.15;
      const period = 3 + rng() * 2;
      const ph = rng() * period;
      // Tipped for 0.9 s of every period.
      const tip = (t: number) => {
        if (t <= 0) return 0;
        const f = (((t + ph) % period) + period) % period;
        return f < 0.9 ? Math.sin((f / 0.9) * Math.PI) : 0;
      };
      const pivot = b.anchor(0, s.y - 0.25, c);
      b.box(0, 0, 0, 3.4, 0.5, flap, k % 2 ? PAL.orange : PAL.yellow, { parent: pivot, dynamic: true });
      const dir = rng() < 0.5 ? -1 : 1;
      b.move((t) => {
        pivot.rotation.z = dir * tip(t) * 1.35;
      });
      route.push({ x: 0, z: c + flap / 2 - 0.3, w: 0, wait: (bot) => [0.1, 0.4, 0.7].every((dt) => tip(bot.t + dt) < 0.05) });
      zz += flap + 0.3;
    }
    b.box(0, s.y - 1, zz + 2, 12, 2, 4, PAL.purple);
    route.push({ x: 0, z: zz + 2, w: 0.5 });
    return {
      z: zz + 4,
      y: s.y,
      routes: [route],
      checkpoint: { from: zz + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zz + 2) },
    };
  };
}

/** Blocks shooting out of both side walls across the floor, row after row: dash between them. */
export function pistons(rows = 4, w = 14): Segment {
  return (s) => {
    const { b, rng } = s;
    const gapZ = 5;
    const len = rows * gapZ + 4;
    b.box(0, s.y - 1, s.z + len / 2, w, 2, len, PAL.blue);
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 0.5 }];
    for (let k = 0; k < rows; k++) {
      const pz = s.z + 3 + k * gapZ;
      const period = 2.2 + rng() * 1.2;
      const ph = rng() * period;
      // Out for 40% of the period (a fast shove, a slower pull back).
      const out = (t: number) => {
        if (t <= 0) return 0;
        const f = ((((t + ph) % period) + period) % period) / period;
        return f < 0.1 ? f / 0.1 : f < 0.4 ? 1 : f < 0.55 ? 1 - (f - 0.4) / 0.15 : 0;
      };
      for (const side of [-1, 1]) {
        b.box(side * (w / 2 + 1.5), s.y + 1, pz, 3, 2.4, 2.2, PAL.purple, { noCollide: true });
        const m = b.box(side * (w / 2 + 1.5), s.y + 0.8, pz, w / 2 + 0.5, 1.6, 1.8, PAL.orange, {
          dynamic: true,
          hit: 0.9,
          tag: 'pusher',
          sinks: true,
        });
        b.move((t) => {
          m.obj.position.x = side * (w / 2 + (w / 4 + 0.25) - out(t) * (w / 2 - 0.1));
        });
      }
      route.push(
        { x: 0, z: pz - 2, w: 0.5 },
        { x: 0, z: pz + 1.6, w: 0.5, wait: (bot) => out(bot.t + 0.1) < 0.02 && out(bot.t + 0.45) < 0.02 },
      );
    }
    b.bonus(w / 2 - 2, s.y, s.z + 3 + gapZ * 1.5);
    route.push({ x: 0, z: s.z + len + 0.5, w: 0.5 });
    return { z: s.z + len, y: s.y, routes: [route] };
  };
}
