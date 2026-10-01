/**
 * Golden traces for the Rust port (rust/core/fb_arena/tests/golden): builds maps with the TS code,
 * runs a few bodies through a scripted round and records everything the Rust side must reproduce.
 *   bun scripts/golden.ts [map…]   (default: the maps ported so far)
 * The loop is the body part of server/rooms/arena.ts step(): events cleared, world moved, bodies
 * stepped against each other, bonuses, falls.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import * as THREE from 'three';
import { BTN } from '../src/shared/codec';
import { DT } from '../src/shared/consts';
import { Bonuses } from '../src/sim/bonus';
import { Builder } from '../src/sim/builder';
import type { MapCtx, MapModule } from '../src/sim/map';
import { BODY_STATES, type OtherBody, PlayerBody } from '../src/sim/physics';

const OUT = 'rust/core/fb_arena/tests/golden';
const SEEDS = [1, 777, 123456789];
const INTRO_TICKS = 720;
const PLAY_TICKS = 1800;
const IDS = [1, 2, 3];
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
  const d = DIRS[(Math.floor(k / 90) * 5 + id * 3) % DIRS.length]!;
  let buttons = 0;
  if (k % 70 === id * 7) buttons |= BTN.jump;
  if (k % 250 === id * 31) buttons |= BTN.dive;
  return { mx: d[0], mz: d[1], buttons };
}

function trace(id: string, mod: MapModule, seed: number) {
  const b = new Builder(seed, null);
  const ctx: MapCtx = {
    server: true,
    seed,
    participants: [],
    now: () => 0,
    emit: () => {},
    score: () => 0,
    setScore: () => {},
    bodies: () => new Map(),
    sfx: () => {},
    me: () => -1,
    decorate: () => {},
  };
  const spec = mod.build(b, ctx);
  const bonuses = new Bonuses(b, seed, { arena: !spec.finish, duration: mod.meta.duration });
  const tick0 = -INTRO_TICKS;
  const world = b.world;
  world.finalize(tick0 * DT);
  const staticHash = world.hash(true);
  const colliders = world.colliders.map((c) => ({
    shape: c.shape,
    isStatic: c.isStatic,
    enabled: c.enabled,
    cur: [...c.cur.elements],
  }));
  const faceYaw = (p: THREE.Vector3) => (spec.faceCenter ? Math.atan2(-p.x, -p.z) : 0);
  const pawns = IDS.map((pid, i) => {
    const spawn = spec.spawns[i % spec.spawns.length]!.clone();
    const body = new PlayerBody(pid);
    body.reset(spawn, faceYaw(spawn));
    return { id: pid, body, spawn };
  });
  const frames: number[][] = [];
  const hashes: [number, string][] = [];
  const events: unknown[] = [];
  for (let k = tick0 + 1; k <= PLAY_TICKS; k++) {
    const t = k * DT;
    const moving = t >= 0;
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
      const f = moving ? script(p.id, k) : { mx: 0, mz: 0, buttons: 0 };
      const input = { mx: f.mx / 127, mz: f.mz / 127, jump: (f.buttons & BTN.jump) !== 0, dive: (f.buttons & BTN.dive) !== 0 };
      p.body.step(
        DT,
        input,
        world,
        t,
        others.filter((o) => o.id !== p.id),
      );
    }
    if (t >= 0)
      bonuses.check(
        t,
        pawns.map((p) => p.body),
        (_name, data) => {
          bonuses.onEvent(data);
          events.push({ k, ...(data as object) });
        },
      );
    for (const p of pawns) {
      if (p.body.pos.y >= spec.killY) continue;
      const to = p.spawn;
      const jitter = ((p.id * 7919) % 100) / 100 - 0.5;
      p.body.reset(new THREE.Vector3(to.x + jitter * 2, to.y + 0.5, to.z), faceYaw(to));
    }
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
    frames.push(row);
    if (k % 60 === 0) hashes.push([k, world.hash()]);
  }
  return {
    map: id,
    seed,
    intro: INTRO_TICKS,
    ids: IDS,
    staticHash,
    colliders,
    bonuses: bonuses.list.map((x) => ({ i: x.i, x: x.x, y: x.y, z: x.z, kind: x.kind, appearAt: x.appearAt })),
    events,
    hashes,
    frames,
  };
}

const maps = process.argv.slice(2);
const ids = maps.length ? maps : ['jump-club'];
mkdirSync(OUT, { recursive: true });
for (const id of ids) {
  const mod = (await import(`../src/games/${id}/map.ts`)).default as MapModule;
  const runs = [];
  for (const seed of SEEDS) runs.push(trace(id, mod, seed));
  const file = `${OUT}/${id}.json`;
  writeFileSync(file, JSON.stringify({ runs }));
  console.log(`${file}: ${runs.length} seeds × ${runs[0]!.frames.length} ticks`);
}
