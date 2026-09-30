import * as THREE from 'three';
import { GAMES, MAPS } from '../games';
import { ServerArena } from '../server/arena';
import { planGame } from '../server/director';
import { BTN, decodeInput, decodeSnapshot, encodeInput, encodeSnapshot, type InputFrame } from '../shared/codec';
import { DT, MAX_PLAYERS, TICK_MS } from '../shared/consts';
import type { Genre } from '../shared/game';
import { ROUND_COUNTS } from '../shared/protocol';
import { mulberry32 } from '../shared/rng';
import { emptyStats, type RoundView, scoreRound, TOP_POINTS } from '../shared/rules';
import { Builder } from '../sim/builder';
import { type BodyInput, PlayerBody, RUN_SPEED } from '../sim/physics';
import type { World } from '../sim/world';
import { type GlobalAudit, r3 } from './types';

// ------------------------------------------------------------------ physics feel

/** A flat test floor with ledges of increasing height along +x, and an ice patch. */
function testWorld(): World {
  const b = new Builder(1, null);
  b.box(0, -1, 0, 400, 2, 400);
  [0.2, 0.35, 0.5, 0.65, 0.8, 1.0, 1.3, 1.6, 2.0, 2.4].forEach((h, i) => {
    b.box(-100 - i * 20, h / 2, 0, 6, h, 6);
  });
  b.box(100, 0.1, 0, 40, 0.2, 40, undefined, { slip: 1 });
  b.world.finalize(0);
  return b.world;
}

function run(
  world: World,
  body: PlayerBody,
  ticks: number,
  input: (k: number) => BodyInput,
  each?: (k: number) => boolean | undefined,
) {
  for (let k = 1; k <= ticks; k++) {
    const t = k * DT;
    body.clearEvents();
    body.beforeWorldUpdate();
    world.setTime(t);
    body.afterWorldUpdate();
    body.step(DT, input(k), world, t, []);
    if (each?.(k)) return k;
  }
  return ticks;
}

const IDLE: BodyInput = { mx: 0, mz: 0, jump: false, dive: false };
const FWD: BodyInput = { mx: 0, mz: 1, jump: false, dive: false };
const speed = (b: PlayerBody) => Math.hypot(b.vel.x, b.vel.z);

/** Measures how the bean handles (accelerate, stop, turn, jump, dive, climb, ice) and flags big changes. */
export const physicsAudit: GlobalAudit = {
  name: 'physics',
  perMap: false,
  run(_ctx, out) {
    const world = testWorld();
    const fresh = (x = 0, z = 0) => {
      const b = new PlayerBody(1);
      b.reset(new THREE.Vector3(x, 0.02, z), 0);
      run(world, b, 30, () => IDLE);
      return b;
    };
    // Acceleration to 90% of running speed.
    let b = fresh();
    const accel =
      run(
        world,
        b,
        240,
        () => FWD,
        () => speed(b) >= RUN_SPEED * 0.9,
      ) * DT;
    // Stopping from full speed.
    run(world, b, 60, () => FWD);
    const stop =
      run(
        world,
        b,
        240,
        () => IDLE,
        () => speed(b) < 0.5,
      ) * DT;
    // Turning around at full speed.
    run(world, b, 60, () => FWD);
    const turn =
      run(
        world,
        b,
        240,
        () => ({ ...IDLE, mz: -1 }),
        () => b.vel.z < -RUN_SPEED * 0.9,
      ) * DT;
    // Jump: apex and air time.
    b = fresh();
    const y0 = b.pos.y;
    let apex = 0;
    let left = false;
    const air =
      run(
        world,
        b,
        240,
        (k) => ({ ...IDLE, jump: k === 1 }),
        () => {
          apex = Math.max(apex, b.pos.y - y0);
          if (!b.grounded) left = true;
          return left && b.grounded;
        },
      ) * DT;
    // Running jump distance.
    b = fresh();
    run(world, b, 90, () => FWD);
    const jx = b.pos.z;
    let jumped = false;
    run(
      world,
      b,
      240,
      (k) => ({ ...FWD, jump: k === 1 }),
      () => {
        if (!b.grounded) jumped = true;
        return jumped && b.grounded;
      },
    );
    const jumpDist = b.pos.z - jx;
    // Dive from a run: distance until back on the feet.
    b = fresh();
    run(world, b, 90, () => FWD);
    const dz0 = b.pos.z;
    const diveTime =
      run(
        world,
        b,
        480,
        (k) => ({ ...FWD, dive: k === 1 }),
        (k) => k > 5 && b.state === 'normal',
      ) * DT;
    const diveDist = b.pos.z - dz0;
    // Climbing: the highest ledge walked onto, and jumped onto.
    const ledges = [0.2, 0.35, 0.5, 0.65, 0.8, 1.0, 1.3, 1.6, 2.0, 2.4];
    let walk = 0;
    let jumpUp = 0;
    ledges.forEach((h, i) => {
      const x = -100 - i * 20;
      for (const withJump of [false, true]) {
        const lb = new PlayerBody(1);
        lb.reset(new THREE.Vector3(x, 0.02, -8), 0);
        run(world, lb, 20, () => IDLE);
        // On top at some point while over the ledge (the bean runs on past it).
        let top = -1;
        run(
          world,
          lb,
          180,
          () => ({ ...FWD, jump: withJump && lb.grounded && lb.pos.z > -4.6 && lb.pos.z < -3.2 }),
          () => {
            if (Math.abs(lb.pos.z) < 2.5 && lb.grounded) top = Math.max(top, lb.pos.y);
            return undefined;
          },
        );
        const on = top > h - 0.2;
        if (on && !withJump) walk = Math.max(walk, h);
        if (on && withJump) jumpUp = Math.max(jumpUp, h);
      }
    });
    // Ice: run onto it at full speed, let go, and see how much speed is left half a second later.
    b = fresh(100, -34);
    run(
      world,
      b,
      360,
      () => FWD,
      () => b.pos.z > -15 && b.groundCol?.slip === 1,
    );
    const iceV0 = speed(b);
    run(world, b, 60, () => IDLE);
    const iceKeep = speed(b) / Math.max(0.01, iceV0);

    const m = {
      accel: r3(accel),
      stop: r3(stop),
      turn: r3(turn),
      jumpApex: r3(apex),
      airTime: r3(air),
      jumpDist: r3(jumpDist),
      diveDist: r3(diveDist),
      diveRecover: r3(diveTime),
      stepWalk: walk,
      stepJump: jumpUp,
      iceKeep: r3(iceKeep),
    };
    for (const [k, v] of Object.entries(m)) out.metric(k, v);
    // Expected ranges: outside them the controls feel different from what the maps were built for.
    const want: Record<keyof typeof m, [number, number]> = {
      accel: [0.05, 0.3],
      stop: [0.02, 0.3],
      turn: [0.05, 0.45],
      jumpApex: [1.4, 2.6],
      airTime: [0.55, 1.0],
      jumpDist: [4, 9],
      diveDist: [3, 9],
      diveRecover: [0.3, 1.6],
      stepWalk: [0.2, 0.65],
      stepJump: [1.0, 2.4],
      iceKeep: [0.3, 1],
    };
    for (const [k, [lo, hi]] of Object.entries(want)) {
      const v = m[k as keyof typeof m];
      if (v < lo || v > hi) out.warn(`${k} = ${v} is outside the expected ${lo}…${hi}`);
    }
  },
};

// ------------------------------------------------------------------ input handling and the wire format

export const inputAudit: GlobalAudit = {
  name: 'input',
  perMap: false,
  run(ctx, out) {
    const rng = mulberry32(ctx.seed);
    const n = ctx.quick ? 2000 : 20000;
    // Garbage never crashes the decoders.
    let crashes = 0;
    for (let i = 0; i < n; i++) {
      const len = Math.floor(rng() * 120);
      const data = new Uint8Array(len);
      for (let j = 0; j < len; j++) data[j] = Math.floor(rng() * 256);
      if (len && rng() < 0.5) data[0] = rng() < 0.5 ? 1 : 2;
      try {
        decodeInput(data);
        decodeSnapshot(data);
      } catch {
        crashes++;
      }
    }
    if (crashes) out.error(`decoders threw on ${crashes} of ${n} random packets`);
    // Round trips keep every value.
    let bad = 0;
    for (let i = 0; i < n; i++) {
      const frames: InputFrame[] = Array.from({ length: 1 + Math.floor(rng() * 32) }, () => ({
        mx: Math.round(rng() * 254 - 127),
        mz: Math.round(rng() * 254 - 127),
        buttons: Math.floor(rng() * 8),
      }));
      const p = { arena: Math.floor(rng() * 65536), firstTick: Math.floor(rng() * 2e9 - 1e9), frames };
      const d = decodeInput(encodeInput(p));
      if (!d || d.arena !== p.arena || d.firstTick !== p.firstTick || JSON.stringify(d.frames) !== JSON.stringify(frames)) bad++;
    }
    if (bad) out.error(`${bad} input packets did not survive encode → decode`);
    const snap = { arena: 7, tick: 1234, own: null, bodies: [] };
    if (!decodeSnapshot(encodeSnapshot(snap))) out.error('an empty snapshot does not decode');

    // The server acts on a press in the tick it is for, and rejects packets far in the future.
    const mod = MAPS.find((m) => m.meta.id === 'jump-club') ?? MAPS[0]!;
    const a = new ServerArena({
      id: 5,
      kind: 'lobby',
      module: mod,
      seed: 1,
      startAt: 0,
      participants: [1],
      now: 0,
      hooks: {
        onFinish() {},
        onKo() {},
        onEvent() {},
        onScore() {},
        onSnapshot() {},
        warn() {},
      },
    });
    a.addPawn(1, false, 0);
    a.advance(1000);
    const p = a.pawns.get(1)!;
    const k = a.tick + 1;
    const ok = a.input(1, { arena: 5, firstTick: k, frames: [{ mx: 0, mz: 0, buttons: BTN.jump }] }, 1000);
    a.advance(1000 + TICK_MS * 1.01);
    if (!ok) out.error('a valid input packet was rejected');
    if (!(p.body.vel.y > 0)) out.error('a jump pressed for tick k did not start in tick k');
    const far = a.input(1, { arena: 5, firstTick: a.tick + 10_000, frames: [{ mx: 127, mz: 0, buttons: 0 }] }, 1100);
    if (far) out.error('an input packet 10 000 ticks ahead was accepted');
    const wrong = a.input(1, { arena: 6, firstTick: a.tick + 1, frames: [{ mx: 127, mz: 0, buttons: 0 }] }, 1100);
    if (wrong) out.error('an input packet for another arena was accepted');
    // Oversized movement is clamped to unit length.
    a.input(1, { arena: 5, firstTick: a.tick + 1, frames: [{ mx: 127, mz: 127, buttons: 0 }] }, 1100);
    const f = p.inputs.get(a.tick + 1);
    if (f && Math.hypot(f.mx, f.mz) > 127.5) out.error('diagonal input is not clamped to unit length');
    a.dispose();
    out.metric('fuzzPackets', n);
  },
};

// ------------------------------------------------------------------ game rules and planning

export const rulesAudit: GlobalAudit = {
  name: 'rules',
  perMap: false,
  run(ctx, out) {
    const rng = mulberry32(ctx.seed);
    // Planning: every player count, mode and length gives a valid game.
    let plans = 0;
    for (let players = 1; players <= MAX_PLAYERS; players++)
      for (const mode of ['mix', 'races', 'survival'] as const)
        for (const rounds of ROUND_COUNTS) {
          const plan = planGame(players, { mode, games: [], rounds }, rng);
          plans++;
          if (plan.length !== rounds) out.error(`${mode}, ${rounds} rounds, ${players} players: planned ${plan.length} rounds`);
          for (const id of plan) {
            const g = GAMES.find((x) => x.id === id);
            if (!g) out.error(`planned an unknown game ${id}`);
            else if ((g.minPlayers ?? 1) > players) out.error(`planned ${id} for ${players} players (needs ${g.minPlayers})`);
            if (mode === 'races' && g && g.genre !== 'race') out.error(`races mode planned ${id} (${g.genre})`);
          }
          if (new Set(plan).size < Math.min(plan.length, 5))
            out.warn(`${mode} ${rounds} rounds repeats a game: ${plan.join(', ')}`);
        }
    out.metric('plans', plans);

    // Scoring: points within 0…10, totals never negative, better placement never scores less.
    const genres: Genre[] = ['race', 'survival', 'points'];
    let rounds = 0;
    for (let i = 0; i < (ctx.quick ? 300 : 3000); i++) {
      const n = 1 + Math.floor(rng() * 8);
      const ids = Array.from({ length: n }, (_, k) => k + 1);
      const genre = genres[i % 3]!;
      const shuffled = [...ids].sort(() => rng() - 0.5);
      const cut = Math.floor(rng() * (n + 1));
      const scores = new Map(ids.map((id) => [id, Math.floor(rng() * 5)]));
      const view: RoundView = {
        genre,
        participants: ids,
        connected: () => true,
        finished: genre === 'race' ? shuffled.slice(0, cut) : [],
        out: genre === 'survival' ? shuffled.slice(0, cut) : [],
        scores,
        progress: (id) => id * 3,
        timeUp: true,
        solo: n === 1,
      };
      const stats = new Map(
        ids.map((id) => [id, { ...emptyStats(), falls: Math.floor(rng() * 6), shortcuts: Math.floor(rng() * 2) }]),
      );
      const totals = new Map(ids.map((id) => [id, Math.floor(rng() * 30)]));
      const rows = scoreRound(view, stats, totals, new Set());
      rounds++;
      for (const r of rows) {
        if (r.points < 0 || r.points > TOP_POINTS) out.error(`round scored ${r.points} placement points`);
        if (r.total < 0) out.error(`a total went negative (${r.total})`);
        if (r.place < 1 || r.place > n) out.error(`place ${r.place} of ${n}`);
      }
      const byPlace = [...rows].sort((a, b) => a.place - b.place);
      for (let k = 1; k < byPlace.length; k++)
        if (byPlace[k]!.points > byPlace[k - 1]!.points) {
          out.error(`place ${byPlace[k]!.place} got more points than place ${byPlace[k - 1]!.place}`);
          break;
        }
    }
    out.metric('scoredRounds', rounds);
  },
};

export const GLOBAL_AUDITS: GlobalAudit[] = [physicsAudit, inputAudit, rulesAudit];
