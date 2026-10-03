/**
 * Golden traces for the Rust port (rust/core/fb_arena/tests/golden): plays whole rounds of the TS
 * server arena (scripted players and bots) and records everything the Rust side must reproduce.
 *   cargo xtask golden [map…|scenarios]   (default: every map, and the physics scenarios), or
 *   bun --preload ./scripts/golden-math.ts scripts/golden.ts [map…|scenarios]
 * Written gzipped (<map>.json.gz): the traces of 19 maps are large.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import * as THREE from 'three';
import { getMap, LOBBY, MAPS, PODIUM } from '../src/games';
import { type ArenaHooks, ServerArena } from '../src/server/rooms/arena';
import { BTN } from '../src/shared/codec';
import { DT, TICK_MS } from '../src/shared/consts';
import { Builder, PAL } from '../src/sim/builder';
import type { MapModule } from '../src/sim/map';
import { BODY_STATES, type OtherBody, PlayerBody, POWER } from '../src/sim/physics';

const goldenMath = (globalThis as { __goldenMath?: () => number }).__goldenMath;
if (!goldenMath) throw new Error('run with --preload ./scripts/golden-math.ts (cargo xtask golden does)');
if (goldenMath() !== 1) throw new Error('golden-math: the source rewrite of sim/bots.ts did not happen');

const OUT = 'rust/core/fb_arena/tests/golden';
const SEEDS = [1, 777, 123456789];
/** Seeds that draw a map's sections the common ones miss (frost-sky: ice rotors). */
const EXTRA_SEEDS: Record<string, number[]> = { 'frost-sky': [5] };
const INTRO_TICKS = 720;
/** Players with scripted input, and bots. */
const HUMANS = [1, 2];
const BOTS = [3, 4, 5, 6];
/** Full state rows every this many ticks (the state hash is kept every tick). */
const ROW_EVERY = Number(process.env.GOLDEN_ROWS ?? 120);
/** The lobby and the podium have no end: this many seconds of them. */
const ENDLESS_S = 60;
const DIRS = [
  [127, 0],
  [90, 90],
  [0, 127],
  [-90, 90],
  [-127, 0],
  [-90, -90],
  [0, -127],
  [90, -90],
  [0, 0],
] as const;

/** The scripted input of body `id` at tick k (integers only: both sides compute it identically). */
function script(id: number, k: number) {
  const mod = (a: number, n: number) => ((a % n) + n) % n;
  const d = DIRS[mod(Math.floor(k / 90) * 5 + id * 3, DIRS.length)]!;
  let buttons = 0;
  if (k >= 0 && k % 70 === id * 7) buttons |= BTN.jump;
  if (k >= 0 && k % 250 === id * 31) buttons |= BTN.dive;
  return { mx: d[0], mz: d[1], buttons };
}

type Pawn = { id: number; body: PlayerBody };
type Frame = { mx: number; mz: number; buttons: number };

/** One tick of bodies in a world, as arena.ts (and fb_arena::tick_bodies) does it. */
function tickBodies(world: Builder['world'], t: number, pawns: Pawn[], frame: (p: Pawn) => Frame) {
  for (const p of pawns) {
    p.body.clearEvents();
    p.body.beforeWorldUpdate();
  }
  world.setTime(t);
  for (const p of pawns) p.body.afterWorldUpdate();
  const others: OtherBody[] = pawns
    .filter((p) => !p.body.inPortal)
    .map((p) => ({
      id: p.id,
      x: p.body.pos.x,
      y: p.body.pos.y,
      z: p.body.pos.z,
      vx: p.body.vel.x,
      vz: p.body.vel.z,
      touching: false,
      size: p.body.size,
    }));
  for (const p of pawns) {
    const f = frame(p);
    const input = { mx: f.mx / 127, mz: f.mz / 127, jump: (f.buttons & BTN.jump) !== 0, dive: (f.buttons & BTN.dive) !== 0 };
    p.body.step(
      DT,
      input,
      world,
      t,
      others.filter((o) => o.id !== p.id),
    );
  }
}

/** What the Rust side compares each tick: pos, vel, yaw, tilt, state, ground collider. */
function bodyRow(pawns: Pawn[], k: number) {
  const row = [k];
  for (const p of pawns) {
    const s = p.body;
    row.push(
      s.pos.x,
      s.pos.y,
      s.pos.z,
      s.vel.x,
      s.vel.y,
      s.vel.z,
      s.yaw,
      s.tilt,
      BODY_STATES.indexOf(s.state),
      s.groundCol?.index ?? -1,
    );
  }
  return row;
}

function colliderList(world: Builder['world']) {
  return world.colliders.map((c) => ({
    shape: c.shape,
    isStatic: c.isStatic,
    enabled: c.enabled,
    cur: [...c.cur.elements],
  }));
}

const STATUS = ['play', 'finished', 'out'];

function trace(id: string, mod: MapModule, seed: number, withColliders: boolean) {
  const events: unknown[] = [];
  let k = 0;
  const hooks: ArenaHooks = {
    onFinish: (pid, time) => events.push({ k, e: 'finish', id: pid, t: time }),
    onKo: (ko) => events.push({ k, e: 'ko', ...ko }),
    onEvent: (name, data) => events.push({ k, e: name, data }),
    onScore: (pid, v) => events.push({ k, e: 'score', id: pid, v }),
    onSnapshot: () => {},
    onEmote: (pid, e) => events.push({ k, e: 'emote', id: pid, emote: e }),
    warn: (msg, data) => {
      throw new Error(`${msg} ${JSON.stringify(data)}`);
    },
  };
  const participants = [...HUMANS, ...BOTS];
  const kind = id === 'lobby' ? 'lobby' : id === 'podium' ? 'podium' : 'round';
  const arena = new ServerArena({
    id: 1,
    kind,
    module: mod,
    seed,
    startAt: 0,
    participants,
    now: -INTRO_TICKS * TICK_MS,
    hooks,
  });
  const tick0 = arena.tick;
  const staticHash = arena.staticHash;
  const colliders = colliderList(arena.world);
  for (const p of participants) arena.addPawn(p, BOTS.includes(p));
  const step = (arena as unknown as { step(k: number): void }).step.bind(arena);
  const end = Math.round(Math.min(mod.meta.duration, kind === 'round' ? 1e9 : ENDLESS_S) / DT);
  const states: string[] = [];
  const worlds: [number, string][] = [];
  const rows: number[][] = [];
  for (k = tick0 + 1; k <= end; k++) {
    for (const h of HUMANS) arena.forceInput(h, k, script(h, k));
    step(k);
    states.push(arena.stateHash());
    if (k % 60 === 0) worlds.push([k, arena.world.hash()]);
    if (k % ROW_EVERY === 0) {
      const pawns = [...arena.pawns.values()];
      const row = bodyRow(pawns, k);
      for (const p of pawns) row.push(STATUS.indexOf(p.status), p.grabbing ?? -1);
      rows.push(row);
    }
  }
  const pawns = [...arena.pawns.values()];
  return {
    map: id,
    kind,
    seed,
    tick0,
    end,
    humans: HUMANS,
    bots: BOTS,
    rowEvery: ROW_EVERY,
    staticHash,
    colliders: withColliders ? colliders : null,
    bonuses: (arena.bonuses?.list ?? []).map((x) => ({ i: x.i, x: x.x, y: x.y, z: x.z, kind: x.kind, appearAt: x.appearAt })),
    events,
    states,
    worlds,
    rows,
    finished: arena.finished,
    out: arena.out,
    stats: pawns.map((p) => ({ id: p.id, ...p.stats })),
    scores: [...arena.scores].sort((a, b) => a[0] - b[0]),
  };
}

/**
 * Physics scenarios: small worlds for the branches of the bean's physics that jump-club does not touch
 * (pushes, slopes, ice, conveyors, pads, sweeping arms, hammers, platforms, ledges, ladders, giants).
 * The Rust side builds the same worlds (fb_arena/tests/scenarios.rs) and replays the inputs from here.
 */
interface ScenarioBody {
  id: number;
  at: [number, number, number];
  power?: number;
  /** Stick from tick k on: [k, mx, mz]. */
  stick: [number, number, number][];
  /** Buttons pressed on tick k only: [k, buttons]. */
  press?: [number, number][];
}
interface Scenario {
  name: string;
  ticks: number;
  build: (b: Builder) => void;
  bodies: ScenarioBody[];
}

const J = BTN.jump;
const D = BTN.dive;
const floor = (b: Builder) => b.box(0, -1, 0, 40, 2, 40);
const run = (k: number, mx: number, mz: number, stop: number): [number, number, number][] => [
  [k, mx, mz],
  [stop, 0, 0],
];

const SCENARIOS: Scenario[] = [
  {
    name: 'portal',
    ticks: 600,
    build: (b) => {
      b.box(0, -1, 0, 60, 2, 60);
      b.portal({ x: 0, y: 0, z: 5, yaw: Math.PI }, { x: 20, y: 0, z: 0, yaw: Math.PI / 2 });
      b.portal({ x: -10, y: 0, z: 5, yaw: Math.PI }, { x: -10, y: 0, z: 20, yaw: 0 }, '#ffffff', {
        oneWay: true,
        speed: 9,
        lift: 6,
      });
    },
    bodies: [
      { id: 1, at: [0, 0.02, 0], stick: run(1, 0, 127, 200) },
      { id: 2, at: [0.5, 0.02, -2.5], stick: run(1, 0, 127, 300) },
      { id: 3, at: [-10, 0.02, 0], stick: run(1, 0, 127, 100) },
      { id: 4, at: [-10, 0.02, 24], stick: run(1, 0, -127, 160) },
    ],
  },
  {
    name: 'moves',
    ticks: 720,
    build: (b) => {
      floor(b);
      b.box(0, 2, 16, 40, 4, 1);
    },
    bodies: [
      {
        id: 1,
        at: [0, 0.02, 0],
        stick: run(1, 0, 127, 200),
        press: [
          [30, J],
          [100, D],
        ],
      },
      {
        id: 2,
        at: [-8, 0.02, 0],
        power: POWER.jump,
        stick: [[1, 0, 0]],
        press: [
          [20, J],
          [200, J],
          [400, J],
        ],
      },
      {
        id: 3,
        at: [8, 0.02, -15],
        power: POWER.speed,
        stick: [
          [1, 0, 127],
          [100, 127, 0],
          [150, 0, -127],
          [250, -127, 0],
          [350, 0, 0],
        ],
      },
      {
        id: 4,
        at: [-14, 0.02, -15],
        stick: run(1, 0, 127, 500),
        press: [
          [40, J],
          [60, D],
          [300, J],
          [302, D],
        ],
      },
    ],
  },
  {
    name: 'push',
    ticks: 420,
    build: floor,
    bodies: [
      { id: 1, at: [0, 0.02, -3], stick: run(1, 0, 127, 180) },
      { id: 2, at: [0, 0.02, 3], stick: run(1, 0, -127, 180) },
      { id: 3, at: [6, 0.02, -4], power: POWER.giant, stick: run(1, 0, 127, 240) },
      { id: 4, at: [6, 0.02, 1], stick: [[1, 0, 0]] },
      { id: 5, at: [-6, 0.02, -3], stick: run(1, 0, 127, 200), press: [[30, D]] },
      { id: 6, at: [-6, 0.02, 1], stick: [[1, 0, 0]] },
    ],
  },
  {
    name: 'slopes',
    ticks: 480,
    build: (b) => {
      floor(b);
      b.ramp(-6, 2, 0, 14, 4, 6);
      b.ramp(6, 2, 0, 5, 4.5, 6);
      b.ramp(-15, 5, 0, 15, 4, 6, PAL.white, 1, { slip: 0.9 });
    },
    bodies: [
      {
        id: 1,
        at: [-6, 0.02, 0],
        stick: [
          [1, 0, 127],
          [220, 0, 0],
          [300, 0, -127],
          [400, 0, 0],
        ],
      },
      {
        id: 2,
        at: [6, 0.02, 0],
        stick: run(1, 0, 127, 200),
        press: [
          [60, J],
          [120, J],
        ],
      },
      { id: 3, at: [7, 5, 3.5], stick: [[1, 0, 0]] },
      { id: 4, at: [-15, 0.02, 3], stick: run(1, 0, 127, 150) },
    ],
  },
  {
    name: 'surfaces',
    ticks: 480,
    build: (b) => {
      b.box(-6, -1, 0, 8, 2, 40, PAL.white, { slip: 0.9 });
      b.box(0, -1, 0, 4, 2, 40);
      b.box(6, -1, 0, 8, 2, 40, PAL.white, { conveyor: new THREE.Vector3(0, 0, -4) });
    },
    bodies: [
      {
        id: 1,
        at: [-6, 0.02, -15],
        stick: [
          [1, 0, 127],
          [90, 0, 0],
          [200, 127, 0],
          [240, 0, 0],
        ],
      },
      {
        id: 2,
        at: [0, 0.02, -15],
        stick: [
          [1, 0, 127],
          [90, 0, 0],
          [200, 127, 0],
          [240, 0, 0],
        ],
      },
      { id: 3, at: [6, 0.02, 0], stick: run(120, 0, 127, 360) },
    ],
  },
  {
    name: 'rotor',
    ticks: 720,
    build: (b) => {
      floor(b);
      b.rotor(0, 0.6, 0, 8, 1, (t) => t * 1.2);
    },
    bodies: [
      { id: 1, at: [0.5, 0.02, -5], stick: [[1, 0, 0]] },
      { id: 2, at: [3, 0.02, 1.5], stick: run(1, 0, -127, 97) },
      { id: 3, at: [5, 1, 0], stick: [[1, 0, 0]] },
      { id: 4, at: [-5, 0.02, -3], stick: [[1, 0, 0]], press: Array.from({ length: 15 }, (_, i) => [10 + i * 45, J]) },
    ],
  },
  {
    name: 'hammer',
    ticks: 600,
    build: (b) => {
      floor(b);
      b.hammer(0, 7, 0, 2.2, Math.PI / 2);
    },
    bodies: [
      { id: 1, at: [0, 0.02, -0.6], stick: [[1, 0, 0]] },
      { id: 2, at: [0, 0.02, -8], stick: run(1, 0, 127, 300) },
      { id: 3, at: [0, 0.02, 1.6], power: POWER.giant, stick: [[1, 0, 0]] },
    ],
  },
  {
    name: 'bounce',
    ticks: 480,
    build: (b) => {
      floor(b);
      b.bumper(0, 0, 5);
      b.pad(6, 0, 5);
      b.pad(-6, 0, 5, 1.4, 16, { x: 0, z: 8 });
      b.trampoline(12, 0, 5);
      b.mushroom(-12, 0, 5);
    },
    bodies: [
      { id: 1, at: [0, 0.02, 0], stick: run(1, 0, 127, 90) },
      { id: 2, at: [6, 0.02, 0], stick: run(1, 0, 127, 90) },
      { id: 3, at: [-6, 0.02, 0], stick: run(1, 0, 127, 90) },
      { id: 4, at: [12, 0.02, 0], stick: run(1, 0, 127, 90) },
      { id: 5, at: [-12, 6, 5], stick: [[1, 0, 0]] },
    ],
  },
  {
    name: 'platforms',
    ticks: 600,
    build: (b) => {
      floor(b);
      const slider = b.box(-6, 1, 0, 4, 0.6, 4, PAL.blue, { dynamic: true });
      b.move((t) => {
        slider.obj.position.x = -6 + Math.sin(t * 1.5) * 3;
      });
      const disc = b.cyl(6, 1, 0, 3, 0.6, PAL.purple, { dynamic: true });
      b.move((t) => {
        disc.obj.rotation.y = t * 1.2;
      });
      const lift = b.box(0, 2, 8, 3, 0.6, 3, PAL.blue, { dynamic: true });
      b.move((t) => {
        lift.obj.position.y = 2 + Math.sin(t * 1.3) * 1.2;
      });
    },
    bodies: [
      { id: 1, at: [-6, 1.32, 0], stick: run(400, 0, 127, 460) },
      { id: 2, at: [8, 1.32, 0], stick: [[1, 0, 0]], press: [[300, J]] },
      { id: 3, at: [0, 2.32, 8], stick: [[450, 127, 0]] },
    ],
  },
  {
    name: 'climb',
    ticks: 720,
    build: (b) => {
      floor(b);
      b.box(0, 1.3, 7, 6, 2.6, 6);
      b.box(8, 2.1, 7, 6, 4.2, 6);
      b.box(-8, 2.5, 7, 6, 5, 6);
      b.ladder(-8, 0, 4, 5, Math.PI);
      b.box(-16, 2.5, 7, 6, 5, 6);
      b.ladder(-16, 0, 4, 5, Math.PI);
    },
    bodies: [
      { id: 1, at: [0, 0.02, 0], stick: run(1, 0, 127, 200), press: [[25, J]] },
      { id: 2, at: [8, 0.02, 0], stick: run(1, 0, 127, 200), press: [[25, J]] },
      { id: 3, at: [-8, 0.02, 0], stick: run(1, 0, 127, 330) },
      {
        id: 4,
        at: [-16, 0.02, 0],
        stick: [
          [1, 0, 127],
          [160, 0, -127],
          [220, 0, 0],
        ],
        press: [[150, J]],
      },
    ],
  },
];

function scenario(sc: Scenario) {
  const b = new Builder(1, null);
  sc.build(b);
  const world = b.world;
  world.finalize(0);
  const staticHash = world.hash(true);
  const colliders = colliderList(world);
  const pawns = sc.bodies.map((d) => {
    const body = new PlayerBody(d.id);
    body.reset(new THREE.Vector3(...d.at));
    if (d.power) body.givePower(d.power, 0);
    return { id: d.id, body };
  });
  const frameOf = (d: ScenarioBody, k: number): Frame => {
    let mx = 0;
    let mz = 0;
    for (const [from, x, z] of d.stick)
      if (from <= k) {
        mx = x;
        mz = z;
      }
    const buttons = d.press?.find(([at]) => at === k)?.[1] ?? 0;
    return { mx, mz, buttons };
  };
  const frames: number[][] = [];
  const hashes: [number, string][] = [];
  for (let k = 1; k <= sc.ticks; k++) {
    tickBodies(world, k * DT, pawns, (p) => frameOf(sc.bodies.find((d) => d.id === p.id)!, k));
    frames.push(bodyRow(pawns, k));
    if (k % 60 === 0) hashes.push([k, world.hash()]);
  }
  return {
    name: sc.name,
    ticks: sc.ticks,
    bodies: sc.bodies.map((d) => ({ ...d, power: d.power ?? 0, press: d.press ?? [] })),
    staticHash,
    colliders,
    hashes,
    frames,
  };
}

const args = process.argv.slice(2);
const ids = args.length ? args.filter((a) => a !== 'scenarios') : [...MAPS.map((m) => m.meta.id), LOBBY.meta.id, PODIUM.meta.id];
mkdirSync(OUT, { recursive: true });
for (const id of ids) {
  const mod = getMap(id);
  if (!mod) throw new Error(`unknown map ${id}`);
  const runs = [...SEEDS, ...(EXTRA_SEEDS[id] ?? [])].map((seed, i) => trace(id, mod, seed, i === 0));
  const file = `${OUT}/${id}.json.gz`;
  writeFileSync(file, gzipSync(JSON.stringify({ runs }), { level: 9 }));
  const outcome = runs.map((r) => `seed ${r.seed}: ${r.events.length} events, out ${r.out.join(',') || '-'}`).join('; ');
  console.log(`${file}: ${runs.length} seeds × ${runs[0]!.states.length} ticks (${outcome})`);
}
if (!args.length || args.includes('scenarios')) {
  const file = `${OUT}/scenarios.json.gz`;
  writeFileSync(file, gzipSync(JSON.stringify({ scenarios: SCENARIOS.map(scenario) }), { level: 9 }));
  console.log(`${file}: ${SCENARIOS.map((s) => s.name).join(', ')}`);
}
