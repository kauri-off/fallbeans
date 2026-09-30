import * as THREE from 'three';
import { GAMES } from '../games';
import { DT, MAX_PLAYERS } from '../shared/consts';
import { GameMetaSchema } from '../shared/game';
import { NavGrid } from '../sim/nav';
import { type BodyInput, type Collider, PlayerBody, R, SPHERES } from '../sim/physics';
import type { World } from '../sim/world';
import { harness, mapOrThrow, median, quantile } from './harness';
import { type MapAudit, r1, r3, v3 } from './types';

/** Beans never get closer than this (sim/physics BEAN_GAP); spawns must be further apart. */
const MIN_SPAWN_GAP = 1.1;
const IDLE: BodyInput = { mx: 0, mz: 0, jump: false, dive: false };
const contact = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };

/** A map built on its own (no beans), for geometry checks. */
function built(mapId: string, seed: number) {
  const h = harness(mapOrThrow(mapId), { seed, players: 0 });
  return { h, world: h.arena.world, spec: h.arena.spec, meta: h.mod.meta };
}

/** Steps a lone bean like the arena does, from sim time t0 for `seconds`; calls `each` after every tick. */
function simulate(
  body: PlayerBody,
  world: World,
  t0: number,
  seconds: number,
  input: BodyInput = IDLE,
  each?: (t: number) => boolean | undefined,
) {
  const n = Math.round(seconds / DT);
  for (let i = 1; i <= n; i++) {
    const t = t0 + i * DT;
    body.clearEvents();
    body.beforeWorldUpdate();
    world.setTime(t);
    body.afterWorldUpdate();
    body.step(DT, input, world, t, []);
    if (each?.(t)) return;
  }
}

/** Deepest overlap of the bean's spheres with solid colliders, ignoring the ground it stands on. */
function overlap(body: PlayerBody, world: World): { depth: number; col: Collider | null } {
  const near: Collider[] = [];
  const c = new THREE.Vector3();
  let best = { depth: 0, col: null as Collider | null };
  world.query(body.pos.x, body.pos.z, 1.5, near);
  for (const col of near) {
    if (!col.enabled || col.trigger) continue;
    for (const h of SPHERES) {
      c.set(body.pos.x, body.pos.y + h, body.pos.z);
      if (!col.contact(c, R, contact)) continue;
      if (contact.normal.y > 0.7 && h === SPHERES[0]) continue;
      if (contact.depth > best.depth) best = { depth: contact.depth, col };
    }
  }
  return best;
}

const label = (c: Collider | null) =>
  c ? `#${c.index} ${c.shape.type}${c.tag ? ` (${c.tag})` : ''}${c.isStatic ? '' : ' moving'}` : '?';

// ------------------------------------------------------------------ meta & spec rules

export const metaAudit: MapAudit = {
  name: 'meta',
  perMap: true,
  run(mapId, _ctx, out) {
    const meta = mapOrThrow(mapId).meta;
    const r = GameMetaSchema.safeParse(meta);
    if (!r.success) for (const i of r.error.issues) out.error(`meta.${i.path.join('.')}: ${i.message}`);
    if (GAMES.filter((g) => g.title === meta.title).length > 1) out.error(`title "${meta.title}" is used twice`);
    out.metric('duration', meta.duration);
    out.metric('genre', meta.genre);
  },
};

export const specAudit: MapAudit = {
  name: 'spec',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, spec, meta, world } = built(mapId, ctx.seed);
    try {
      const sp = spec.spawns;
      out.metric('spawns', sp.length);
      if (sp.length < MAX_PLAYERS) out.error(`${sp.length} spawns for up to ${MAX_PLAYERS} players (beans would share a spawn)`);
      sp.forEach((p, i) => {
        if (!Number.isFinite(p.x + p.y + p.z)) out.error(`spawn ${i} is not a finite position`);
        if (spec.forbidden?.(p)) out.error(`spawn ${i} is inside a forbidden (shortcut) zone`, { at: v3(p) });
        for (let j = 0; j < i; j++) {
          const d = p.distanceTo(sp[j]!);
          if (d < MIN_SPAWN_GAP) out.error(`spawns ${j} and ${i} are ${r3(d)} m apart`, { at: v3(p) });
        }
      });
      const lowest = Math.min(...sp.map((p) => p.y));
      if (spec.killY > lowest - 2) out.error(`killY ${spec.killY} is within 2 m of the lowest spawn (y ${r3(lowest)})`);
      const cps = spec.checkpoints ?? [];
      out.metric('checkpoints', cps.length);
      cps.forEach((c, i) => {
        if (i && c.z < cps[i - 1]!.z)
          out.error(`checkpoint ${i} threshold (${c.z}) is below checkpoint ${i - 1} (${cps[i - 1]!.z})`);
        if (spec.forbidden?.(c.p)) out.error(`checkpoint ${i} respawn point is in a forbidden zone`, { at: v3(c.p) });
        if (c.p.y < spec.killY + 2) out.error(`checkpoint ${i} respawn point is below the kill height`, { at: v3(c.p) });
      });
      if (meta.genre === 'race') {
        if (!spec.finish) out.error('a race without a finish');
        else {
          const start = Math.max(...sp.map((p) => spec.progress?.(p) ?? p.z));
          if (spec.finish.z <= start) out.error('the finish is not ahead of the spawns');
          out.metric('length', r1(spec.finish.z - start));
          if (!cps.length) out.warn('a race without checkpoints: every fall goes back to the start');
        }
      } else {
        if (spec.finish) out.warn(`a ${meta.genre} game with a finish line`);
        if (!spec.view) out.warn('arena without a view point: the camera looks at the origin');
      }
      if (!spec.bot) out.error('no bot brain: bots will stand still');
      out.metric('colliders', world.colliders.length);
      out.metric('moving', world.dynamic.length);
      out.metric('movers', world.movers.length);
      out.metric('hash', world.hash());
    } finally {
      h.dispose();
    }
  },
};

// ------------------------------------------------------------------ spawns and respawns

/** Does a bean put at `p` at time t0 stand safely for `seconds`? */
function standTest(world: World, killY: number, p: THREE.Vector3, t0: number, seconds: number) {
  const b = new PlayerBody(99);
  b.reset(p, 0);
  world.setTime(t0);
  const start = overlap(b, world);
  let hit: string | null = null;
  let hitAt = 0;
  let fell = false;
  simulate(b, world, t0, seconds, IDLE, (t) => {
    if (!hit && (b.hazard || b.knocked || b.stunned)) {
      hit = b.hazard ?? (b.knocked ? 'knocked' : 'stunned');
      hitAt = t - t0;
    }
    if (b.pos.y < killY) {
      fell = true;
      return true;
    }
    return undefined;
  });
  const drift = Math.hypot(b.pos.x - p.x, b.pos.z - p.z);
  return { start, hit, hitAt, fell, drift, grounded: b.grounded, ground: b.groundCol, dy: b.pos.y - p.y, end: b.pos.clone() };
}

export const spawnAudit: MapAudit = {
  name: 'spawn',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, spec, world } = built(mapId, ctx.seed);
    try {
      let worst = 0;
      spec.spawns.forEach((p, i) => {
        const r = standTest(world, spec.killY, p, 0, 2);
        worst = Math.max(worst, r.start.depth);
        if (r.start.depth > 0.05)
          out.error(`spawn ${i} starts ${r3(r.start.depth)} m inside ${label(r.start.col)}`, { at: v3(p) });
        if (r.fell) out.error(`a bean on spawn ${i} falls off without moving`, { at: v3(p) });
        else if (!r.hit && !r.grounded) out.warn(`a bean on spawn ${i} is not standing after the start`, { at: v3(r.end) });
        // (On moving floors, a drum that rolls, the bean is carried away: nothing to measure.)
        else if (!r.hit && r.ground?.isStatic && r.dy < -0.6)
          out.warn(`spawn ${i} is ${r3(-r.dy)} m above the ground`, { at: v3(p) });
        // A hazard reaching a bean that stands still at the start: under a second leaves no time to react.
        if (r.hit)
          (r.hitAt < 1 ? out.warn : out.info)(
            `a bean standing on spawn ${i} is hit (${r.hit}) ${r1(r.hitAt)} s after the start`,
            {
              at: v3(p),
              t: r3(r.hitAt),
            },
          );
        if (r.drift > 1 && !r.fell) out.info(`a bean on spawn ${i} drifts ${r1(r.drift)} m by itself`, { at: v3(p) });
      });
      out.metric('worstStartOverlap', r3(worst));
    } finally {
      h.dispose();
    }
  },
};

export const respawnAudit: MapAudit = {
  name: 'respawn',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, spec, world, meta } = built(mapId, ctx.seed);
    try {
      const cps = spec.checkpoints ?? [];
      const times = ctx.quick ? [10] : [5, 20, meta.duration * 0.6];
      let tests = 0;
      cps.forEach((cp, i) => {
        for (const dx of [-1, 0, 1])
          for (const t0 of times) {
            // Same place the arena respawns at (± the per-player jitter).
            const p = new THREE.Vector3(cp.p.x + dx, cp.p.y + 0.5, cp.p.z);
            const r = standTest(world, spec.killY, p, t0, 1.5);
            tests++;
            if (r.start.depth > 0.1)
              out.error(`checkpoint ${i} respawn (x${dx >= 0 ? '+' : ''}${dx}) starts inside ${label(r.start.col)}`, {
                at: v3(p),
                t: t0,
              });
            if (r.fell)
              out.error(`a bean respawned at checkpoint ${i} (x${dx >= 0 ? '+' : ''}${dx}) falls off right away`, {
                at: v3(p),
                t: t0,
              });
            else if (r.hit) out.warn(`a bean respawned at checkpoint ${i} is hit (${r.hit}) within 1.5 s`, { at: v3(p), t: t0 });
          }
      });
      out.metric('tests', tests);
    } finally {
      h.dispose();
    }
  },
};

// ------------------------------------------------------------------ clipping (moving parts through others)

function samplePoints(c: Collider): THREE.Vector3[] {
  const s = c.shape;
  const pts: THREE.Vector3[] = [];
  if (s.type === 'box') {
    for (const x of [-1, 0, 1])
      for (const y of [-1, 0, 1])
        for (const z of [-1, 0, 1]) if (x || y || z) pts.push(new THREE.Vector3(x * s.hx, y * s.hy, z * s.hz));
  } else if (s.type === 'cyl') {
    for (let i = 0; i < 12; i++) {
      const a = (i / 12) * Math.PI * 2;
      for (const y of [-1, 0, 1]) pts.push(new THREE.Vector3(Math.cos(a) * s.r, y * s.hh, Math.sin(a) * s.r));
    }
    pts.push(new THREE.Vector3(0, s.hh, 0), new THREE.Vector3(0, -s.hh, 0));
  } else {
    for (const d of [
      [1, 0, 0],
      [-1, 0, 0],
      [0, 1, 0],
      [0, -1, 0],
      [0, 0, 1],
      [0, 0, -1],
    ] as const)
      pts.push(new THREE.Vector3(d[0] * s.r, d[1] * s.r, d[2] * s.r));
  }
  return pts;
}

/** Parts of one prop (hammer head and arm, rotor and hub) share an ancestor below the map root. */
function related(a: Collider, b: Collider, root: THREE.Object3D): boolean {
  const up = new Set<THREE.Object3D>();
  for (let o: THREE.Object3D | null = a.obj; o && o !== root; o = o.parent) up.add(o);
  for (let o: THREE.Object3D | null = b.obj; o && o !== root; o = o.parent) if (up.has(o)) return true;
  return false;
}

export const clipAudit: MapAudit = {
  name: 'clip',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, world, meta } = built(mapId, ctx.seed);
    try {
      const root = h.arena.builder.group;
      const dyn = world.dynamic.filter((c) => c.enabled);
      const local = new Map(world.colliders.map((c) => [c, samplePoints(c)]));
      const pairs = new Map<
        string,
        { a: Collider; b: Collider; depth: number; t: number; at: THREE.Vector3; hits: number; times: Set<number> }
      >();
      const step = ctx.quick ? 0.25 : 0.1;
      const end = Math.min(meta.duration, ctx.quick ? 20 : 60);
      const near: Collider[] = [];
      const w = new THREE.Vector3();
      let checks = 0;
      const test = (a: Collider, b: Collider, t: number) => {
        // Points of a inside b.
        for (const p of local.get(a)!) {
          w.copy(p).applyMatrix4(a.cur);
          checks++;
          if (!b.contact(w, 0.001, contact) || contact.depth < 0.06) continue;
          const key = a.index < b.index ? `${a.index}-${b.index}` : `${b.index}-${a.index}`;
          const cur = pairs.get(key);
          if (!cur) pairs.set(key, { a, b, depth: contact.depth, t, at: w.clone(), hits: 1, times: new Set([t]) });
          else {
            cur.hits++;
            cur.times.add(t);
            if (contact.depth > cur.depth) Object.assign(cur, { depth: contact.depth, t, at: w.clone() });
          }
        }
      };
      for (let t = 0; t <= end; t += step) {
        world.setTime(t);
        for (const a of dyn) {
          world.query(a.center.x, a.center.z, a.radius, near);
          for (const b of near) {
            if (b === a || !b.enabled || a.sinks || b.sinks || related(a, b, root)) continue;
            if (!b.isStatic && b.index < a.index) continue;
            test(a, b, t);
            test(b, a, t);
          }
        }
      }
      const samples = Math.floor(end / step) + 1;
      // Overlapping nearly all the time: an axle or a hinge (a rotor arm in its hub), not clipping.
      const attached = [...pairs.values()].filter((p) => p.times.size >= samples * 0.9);
      const list = [...pairs.values()].filter((p) => p.times.size < samples * 0.9).sort((x, y) => y.depth - x.depth);
      if (attached.length) out.metric('attached', attached.length);
      for (const p of list.slice(0, 12))
        out.warn(`${label(p.a)} and ${label(p.b)} pass through each other by ${r3(p.depth)} m`, {
          at: v3(p.at),
          t: r3(p.t),
          data: { samples: p.hits, share: r3(p.times.size / samples) },
        });
      if (list.length > 12) out.info(`${list.length - 12} more clipping pairs`);
      out.metric('pairs', list.length);
      out.metric('worstDepth', r3(list[0]?.depth ?? 0));
      out.metric('checks', checks);
    } finally {
      h.dispose();
    }
  },
};

// ------------------------------------------------------------------ navigation

export const navAudit: MapAudit = {
  name: 'nav',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, spec, world, meta } = built(mapId, ctx.seed);
    try {
      world.setTime(0);
      const t0 = performance.now();
      const nav = NavGrid.build(world, spec.forbidden);
      out.metric('buildMs', r1(performance.now() - t0));
      spec.spawns.forEach((p, i) => {
        // Moving floors (platforms, tiles that drop) are not in the grid by design.
        const r = standTest(world, spec.killY, p, 0, 0.3);
        if (r.ground && !r.ground.isStatic) return;
        if (nav.groundAt(p.x, p.z, p.y) === null) out.warn(`spawn ${i} is not on walkable ground for bots`, { at: v3(p) });
      });
      (spec.checkpoints ?? []).forEach((c, i) => {
        if (c.z > -50 && nav.groundAt(c.p.x, c.p.z, c.p.y) === null)
          out.warn(`checkpoint ${i} respawn point is not on walkable ground`, { at: v3(c.p) });
      });
      if (meta.genre === 'race' && spec.finish) {
        const from = spec.spawns[0]!;
        const path = nav.path(from, 0, spec.finish.z + 1, spec.finish.y);
        out.metric('staticPathToFinish', !!path);
        if (!path) out.info('no path to the finish over static ground alone (jumps or moving parts are needed)');
      }
    } finally {
      h.dispose();
    }
  },
};

// ------------------------------------------------------------------ determinism

export const determinismAudit: MapAudit = {
  name: 'determinism',
  perMap: true,
  run(mapId, ctx, out) {
    const mod = mapOrThrow(mapId);
    const seconds = ctx.quick ? 8 : 25;
    // The server simulation must not use Math.random (clients rebuild maps from the seed).
    const real = Math.random;
    let calls = 0;
    Math.random = () => {
      // three.js names every object with a random UUID: harmless.
      if (!new Error().stack?.includes('generateUUID')) calls++;
      return real();
    };
    const hashes: string[][] = [[], []];
    const worlds: string[] = [];
    try {
      for (const run of [0, 1]) {
        const h = harness(mod, { seed: ctx.seed });
        worlds.push(h.arena.world.hash());
        for (let t = 1; t <= seconds; t++) {
          h.runTo(t);
          hashes[run]!.push(h.arena.stateHash());
        }
        h.dispose();
      }
    } finally {
      Math.random = real;
    }
    if (calls) out.error(`the server-side build or simulation called Math.random ${calls} times (must use the seeded rng)`);
    if (worlds[0] !== worlds[1]) out.error('building the map twice with the same seed gives different geometry');
    const diverge = hashes[0]!.findIndex((x, i) => x !== hashes[1]![i]);
    if (diverge >= 0) out.error(`two runs with the same seed diverge after ${diverge + 1} s`, { t: diverge + 1 });
    out.metric('seconds', seconds);
  },
};

// ------------------------------------------------------------------ bots: balance, stuck spots, cost

export const balanceAudit: MapAudit = {
  name: 'balance',
  perMap: true,
  run(mapId, ctx, out) {
    const mod = mapOrThrow(mapId);
    const meta = mod.meta;
    const seeds = ctx.quick ? [ctx.seed] : [ctx.seed, 23, 37, 51, 77];
    const finishTimes: number[] = [];
    const outTimes: number[] = [];
    const firstOuts: number[] = [];
    const survivors: number[] = [];
    const topScores: number[] = [];
    const spots = new Map<string, number>();
    let falls = 0;
    let botSeconds = 0;
    let finishers = 0;
    let stuck = 0;
    let simMs = 0;
    let ticks = 0;
    for (const seed of seeds) {
      const h = harness(mod, { seed });
      const last = new Map<number, { p: THREE.Vector3; t: number }>();
      const end = meta.duration;
      const t0 = performance.now();
      for (let t = 0; t <= end; t += 1) {
        h.runTo(t);
        // Stuck: in play, but has not moved half a metre in 12 s.
        for (const id of h.ids) {
          const p = h.arena.pawns.get(id);
          if (p?.status !== 'play') continue;
          const l = last.get(id);
          if (!l || l.p.distanceTo(p.body.pos) > 0.5) last.set(id, { p: p.body.pos.clone(), t });
          else if (t - l.t >= 12) {
            stuck++;
            out.warn(`bot ${id} (seed ${seed}) has not moved for 12 s`, { at: v3(p.body.pos), t });
            last.set(id, { p: p.body.pos.clone(), t });
          }
        }
        if (!h.alive()) break;
      }
      simMs += performance.now() - t0;
      ticks += h.arena.tick;
      botSeconds += h.ids.length * Math.max(1, h.arena.time);
      for (const w of new Set(h.warnings)) out.error(`arena warning: ${w}`);
      finishers += h.finishes.length;
      finishTimes.push(...h.finishes.map((f) => f.t));
      const outs = h.falls.filter((f) => f.out).map((f) => f.t);
      outTimes.push(...outs);
      if (outs.length) firstOuts.push(Math.min(...outs));
      survivors.push(h.alive());
      topScores.push(Math.max(0, ...h.arena.scores.values()));
      for (const f of h.falls) {
        if (f.out) continue;
        falls++;
        const key = `z≈${Math.round(f.progress / 5) * 5} ${f.cause}`;
        spots.set(key, (spots.get(key) ?? 0) + 1);
      }
      h.dispose();
    }
    const runs = seeds.length;
    const beans = 8 * runs;
    out.metric('seeds', runs);
    out.metric('fallsPerBotMin', r1((falls / botSeconds) * 60));
    out.metric('msPerTick8Bots', r3(simMs / Math.max(1, ticks)));
    if (stuck) out.metric('stuck', stuck);
    const hot = [...spots].sort((a, b) => b[1] - a[1]).slice(0, 3);
    if (hot.length) out.metric('fallHotspots', hot.map(([k, v]) => `${k} ×${v}`).join('; '));
    if (meta.genre === 'race') {
      const rate = finishers / beans;
      out.metric('finishRate', r3(rate));
      out.metric('finishP50', r1(median(finishTimes)));
      out.metric('finishP90', r1(quantile(finishTimes, 0.9)));
      if (rate < 0.3) out.error(`only ${Math.round(rate * 100)}% of bots finish`);
      else if (rate < 0.6) out.warn(`only ${Math.round(rate * 100)}% of bots finish`);
      const p50 = median(finishTimes);
      if (p50 > meta.duration * 0.85) out.warn(`median finish ${r1(p50)} s is close to the ${meta.duration} s limit`);
      if (p50 < 20) out.info(`median finish in ${r1(p50)} s: a short course`);
    } else if (meta.genre === 'survival') {
      out.metric('firstOut', r1(Math.min(...firstOuts, meta.duration)));
      out.metric('outP50', r1(median(outTimes)));
      out.metric('survivorsAtEnd', r1(survivors.reduce((a, b) => a + b, 0) / runs));
      if (firstOuts.some((t) => t < 3)) out.error('a bot is eliminated in the first 3 seconds');
      if (outTimes.length === 0) out.warn('no bot is ever eliminated: the round always runs to the time limit');
    } else {
      out.metric('topScoreAvg', r1(topScores.reduce((a, b) => a + b, 0) / runs));
      if (topScores.every((s) => s <= 0)) out.warn('bots never score in this points game');
    }
  },
};

export const limitsAudit: MapAudit = {
  name: 'limits',
  perMap: true,
  run(mapId, ctx, out) {
    const { h, world } = built(mapId, ctx.seed);
    h.dispose();
    const n = world.colliders.length;
    if (n > 2000) out.warn(`${n} colliders (budget 2000)`);
    if (world.dynamic.length > 200) out.warn(`${world.dynamic.length} moving colliders (budget 200)`);
    // Simulation cost with 8 bots, measured after a warm-up (JIT, bot navigation grid) of 3 s.
    const h2 = harness(mapOrThrow(mapId), { seed: ctx.seed });
    h2.runTo(3);
    const tick0 = h2.arena.tick;
    const t0 = performance.now();
    h2.runTo(3 + (ctx.quick ? 4 : 12));
    const perTick = (performance.now() - t0) / Math.max(1, h2.arena.tick - tick0);
    h2.dispose();
    out.metric('msPerTick', r3(perTick));
    // 120 ticks/s on one core shared with the other rooms: keep a round well under 10% of a core.
    if (perTick > 0.8) out.warn(`${r3(perTick)} ms per tick with 8 bots (budget 0.8 ms)`);
  },
};

export const MAP_AUDITS: MapAudit[] = [
  metaAudit,
  specAudit,
  spawnAudit,
  respawnAudit,
  clipAudit,
  navAudit,
  determinismAudit,
  limitsAudit,
  balanceAudit,
];
