import * as THREE from 'three';
import { BTN, type InputFrame, type InputPacket, REMOTE_FLAG, type RemoteState, type Snapshot } from '../shared/codec';
import { ANIM, BOT_EVERY, DT, TICK_MS } from '../shared/consts';
import { type FallBehaviour, fallBehaviour } from '../shared/game';
import type { GameEventRecord } from '../shared/protocol';
import { mulberry32, type Rng } from '../shared/rng';
import { Builder } from '../sim/builder';
import type { BotInput, BotMem, Checkpoint, MapCtx, MapModule, MapSpec } from '../sim/map';
import { type OtherBody, PlayerBody } from '../sim/physics';

/** How far ahead of the server a client may send inputs (ticks). */
const MAX_LEAD = 120;
const MAX_PACKETS_PER_SEC = 150;
const GRAB_REACH = 1.5;
const DIVE_REACH = 1.0;
const MAX_CATCHUP = 60;

export type PawnStatus = 'play' | 'finished' | 'out';

interface BotState {
  mem: BotMem;
  rng: Rng;
  input: BotInput;
}

export interface Pawn {
  id: number;
  body: PlayerBody;
  status: PawnStatus;
  bot: BotState | null;
  inputs: Map<number, InputFrame>;
  last: InputFrame;
  /** Highest tick for which a real input was used. */
  ack: number;
  spawn: THREE.Vector3;
  checkpoint: Checkpoint | null;
  progress: number;
  grabbing: number | null;
  diveHits: Map<number, number>;
  teleported: boolean;
  packetWindow: number;
  packets: number;
  rejected: number;
}

export interface ArenaHooks {
  onFinish(id: number): void;
  onOut(id: number): void;
  onEvent(name: string, data: unknown): void;
  onScore(id: number, v: number): void;
  onSnapshot(tick: number): void;
  warn(msg: string, data?: Record<string, unknown>): void;
}

export interface ArenaOptions {
  id: number;
  kind: 'lobby' | 'round';
  module: MapModule;
  seed: number;
  startAt: number;
  participants: number[];
  now: number;
  hooks: ArenaHooks;
}

const IDLE: InputFrame = { mx: 0, mz: 0, buttons: 0 };

/**
 * Server-authoritative simulation of one map: every pawn (player or bot) is stepped here at 120 Hz
 * from the inputs players send. Clients only predict their own bean and render everyone else.
 */
export class ServerArena {
  readonly id: number;
  readonly kind: 'lobby' | 'round';
  readonly module: MapModule;
  readonly seed: number;
  readonly startAt: number;
  readonly endAt: number;
  readonly participants: number[];
  readonly builder: Builder;
  readonly spec: MapSpec;
  readonly pawns = new Map<number, Pawn>();
  readonly events: GameEventRecord[] = [];
  readonly scores = new Map<number, number>();
  readonly finished: number[] = [];
  readonly out: number[] = [];
  readonly fall: FallBehaviour;
  /** Last simulated tick. */
  tick: number;
  /** Results are decided: no more finishes/outs are recorded. */
  frozen = false;
  private readonly hooks: ArenaHooks;
  private readonly bodyMap = new Map<number, PlayerBody>();
  private spawnCursor = 0;

  constructor(o: ArenaOptions) {
    this.id = o.id;
    this.kind = o.kind;
    this.module = o.module;
    this.seed = o.seed;
    this.startAt = o.startAt;
    this.endAt = o.startAt + o.module.meta.duration * 1000;
    this.participants = o.participants;
    this.hooks = o.hooks;
    this.fall = o.kind === 'lobby' ? 'spawn' : fallBehaviour(o.module.meta.rules);
    this.tick = Math.floor((o.now - o.startAt) / TICK_MS) - 1;
    this.builder = new Builder(o.seed, null);
    const ctx: MapCtx = {
      server: true,
      seed: o.seed,
      participants: this.participants,
      now: () => this.tick * DT,
      emit: (name, data) => {
        this.events.push([name, data]);
        this.spec?.onEvent?.(name, data);
        this.hooks.onEvent(name, data);
      },
      score: (id) => this.scores.get(id) ?? 0,
      setScore: (id, v) => {
        this.scores.set(id, v);
        this.hooks.onScore(id, v);
      },
      bodies: () => this.bodyMap,
      sfx: () => {},
      me: () => -1,
      decorate: () => {},
    };
    this.spec = o.module.build(this.builder, ctx);
    this.builder.world.finalize(this.tick * DT);
  }

  get world() {
    return this.builder.world;
  }

  get time() {
    return this.tick * DT;
  }

  addPawn(id: number, bot: boolean, spawnIndex?: number) {
    if (this.pawns.has(id)) return;
    const spawns = this.spec.spawns;
    const i = spawnIndex ?? this.spawnCursor++;
    const spawn = (spawns[i % spawns.length] ?? new THREE.Vector3()).clone();
    const body = new PlayerBody(id);
    body.reset(spawn, this.faceYaw(spawn));
    const pawn: Pawn = {
      id,
      body,
      status: 'play',
      bot: bot
        ? {
            mem: {},
            rng: mulberry32(this.seed ^ (id * 2654435761)),
            input: { mx: 0, mz: 0, jump: false, dive: false, grab: false },
          }
        : null,
      inputs: new Map(),
      last: IDLE,
      ack: this.tick,
      spawn,
      checkpoint: null,
      progress: spawn.z,
      grabbing: null,
      diveHits: new Map(),
      teleported: true,
      packetWindow: 0,
      packets: 0,
      rejected: 0,
    };
    this.pawns.set(id, pawn);
    this.bodyMap.set(id, body);
  }

  removePawn(id: number) {
    this.pawns.delete(id);
    this.bodyMap.delete(id);
  }

  private faceYaw(p: THREE.Vector3) {
    if (this.spec.faceCenter) return Math.atan2(-p.x, -p.z);
    return 0;
  }

  /** Accepts an input packet from the owner of `id`. Returns false if it was rejected. */
  input(id: number, pkt: InputPacket, nowMs: number): boolean {
    const p = this.pawns.get(id);
    if (!p || p.bot || pkt.arena !== this.id) return false;
    if (nowMs - p.packetWindow > 1000) {
      p.packetWindow = nowMs;
      p.packets = 0;
    }
    if (++p.packets > MAX_PACKETS_PER_SEC) {
      if (p.packets === MAX_PACKETS_PER_SEC + 1) this.hooks.warn('input flood', { id });
      return false;
    }
    let bad = 0;
    pkt.frames.forEach((f, i) => {
      const k = pkt.firstTick + i;
      if (k <= this.tick) return;
      if (k > this.tick + MAX_LEAD) {
        bad++;
        return;
      }
      // Clamp the movement vector to unit length.
      const l = Math.hypot(f.mx, f.mz);
      const s = l > 127 ? 127 / l : 1;
      p.inputs.set(k, { mx: Math.round(f.mx * s), mz: Math.round(f.mz * s), buttons: f.buttons & 7 });
    });
    if (bad) {
      p.rejected += bad;
      if (p.rejected % 120 === bad) this.hooks.warn('inputs too far ahead', { id, tick: this.tick, first: pkt.firstTick });
    }
    return bad === 0;
  }

  /** Runs every tick due by `nowMs`; returns the number of ticks stepped. */
  advance(nowMs: number): number {
    const target = Math.floor((nowMs - this.startAt) / TICK_MS);
    let steps = 0;
    if (target - this.tick > MAX_CATCHUP) {
      this.hooks.warn('arena fell behind', { behind: target - this.tick });
      this.tick = target - MAX_CATCHUP;
    }
    while (this.tick < target) {
      this.step(this.tick + 1);
      steps++;
    }
    return steps;
  }

  private frameFor(p: Pawn, k: number): { frame: InputFrame; real: boolean } {
    if (p.bot) {
      if (k % BOT_EVERY === 0) this.think(p);
      const b = p.bot.input;
      const fresh = k % BOT_EVERY === 0;
      return {
        frame: {
          mx: Math.round(Math.max(-1, Math.min(1, b.mx)) * 127),
          mz: Math.round(Math.max(-1, Math.min(1, b.mz)) * 127),
          buttons: (fresh && b.jump ? BTN.jump : 0) | (fresh && b.dive ? BTN.dive : 0) | (b.grab ? BTN.grab : 0),
        },
        real: true,
      };
    }
    const f = p.inputs.get(k);
    if (f) {
      p.inputs.delete(k);
      return { frame: f, real: true };
    }
    // Missing input: keep moving the same way for a moment, never repeating a jump or dive;
    // a client that went silent stops.
    if (k - p.ack > 30) return { frame: IDLE, real: false };
    return { frame: { mx: p.last.mx, mz: p.last.mz, buttons: p.last.buttons & BTN.grab }, real: false };
  }

  private think(p: Pawn) {
    const brain = this.spec.bot;
    const st = p.bot!;
    const out = st.input;
    out.mx = 0;
    out.mz = 0;
    out.jump = false;
    out.dive = false;
    out.grab = false;
    if (!brain) return;
    const others: { id: number; pos: THREE.Vector3 }[] = [];
    for (const o of this.pawns.values()) if (o !== p && o.status === 'play') others.push({ id: o.id, pos: o.body.pos });
    try {
      brain({ id: p.id, body: p.body, t: this.time, rng: st.rng, mem: st.mem, others }, out);
    } catch (e) {
      this.hooks.warn('bot brain failed', { game: this.module.meta.id, err: String(e) });
    }
  }

  private step(k: number) {
    this.tick = k;
    const t = k * DT;
    const active: Pawn[] = [];
    const frames = new Map<Pawn, InputFrame>();
    for (const p of this.pawns.values()) {
      if (p.status !== 'play') continue;
      active.push(p);
      const { frame, real } = this.frameFor(p, k);
      frames.set(p, frame);
      p.last = frame;
      if (real) p.ack = k;
      // Drop inputs that are now in the past (arrived too late).
      if (p.inputs.size > 0) for (const key of p.inputs.keys()) if (key <= k) p.inputs.delete(key);
    }
    for (const p of active) {
      p.body.clearEvents();
      p.body.beforeWorldUpdate();
    }
    this.world.setTime(t);
    for (const p of active) p.body.afterWorldUpdate();

    const others: OtherBody[] = active.map((p) => ({
      id: p.id,
      x: p.body.pos.x,
      y: p.body.pos.y,
      z: p.body.pos.z,
      touching: false,
    }));
    for (const p of active) {
      const f = frames.get(p)!;
      const input = { mx: f.mx / 127, mz: f.mz / 127, jump: (f.buttons & BTN.jump) !== 0, dive: (f.buttons & BTN.dive) !== 0 };
      const rest = others.filter((o) => o.id !== p.id);
      p.body.step(DT, input, this.world, t, rest);
    }
    for (const p of active) this.interact(p, frames.get(p)!, active, t);
    for (const p of active) this.rules(p, t);
    try {
      this.spec.tick?.(t);
    } catch (e) {
      this.hooks.warn('game tick failed', { game: this.module.meta.id, err: String(e) });
    }
    this.hooks.onSnapshot(k);
  }

  private interact(p: Pawn, f: InputFrame, active: Pawn[], t: number) {
    const b = p.body;
    const grab = (f.buttons & BTN.grab) !== 0 && b.state === 'normal';
    let target: Pawn | null = null;
    if (grab) {
      const fx = Math.sin(b.yaw);
      const fz = Math.cos(b.yaw);
      let best = GRAB_REACH;
      for (const o of active) {
        if (o === p) continue;
        const dx = o.body.pos.x - b.pos.x;
        const dz = o.body.pos.z - b.pos.z;
        const d = Math.hypot(dx, dz);
        if (d > best || Math.abs(o.body.pos.y - b.pos.y) > 1.2) continue;
        if (d > 0.3 && (dx * fx + dz * fz) / d < 0.2) continue;
        best = d;
        target = o;
      }
    }
    if (target) {
      target.body.slowUntil = Math.max(target.body.slowUntil, t + 0.2);
      if (p.grabbing !== target.id) {
        p.grabbing = target.id;
        try {
          this.spec.onGrab?.(p.id, target.id);
        } catch (e) {
          this.hooks.warn('grab handler failed', { err: String(e) });
        }
      }
    } else p.grabbing = null;

    if (b.state === 'dive') {
      for (const o of active) {
        if (o === p || (p.diveHits.get(o.id) ?? -1) > t) continue;
        const dx = o.body.pos.x - b.pos.x;
        const dz = o.body.pos.z - b.pos.z;
        const d = Math.hypot(dx, dz);
        if (d > DIVE_REACH || Math.abs(o.body.pos.y - b.pos.y) > 1.2) continue;
        const nx = d > 1e-3 ? dx / d : Math.sin(b.yaw);
        const nz = d > 1e-3 ? dz / d : Math.cos(b.yaw);
        o.body.vel.x += nx * 7;
        o.body.vel.z += nz * 7;
        o.body.vel.y = Math.max(o.body.vel.y, 4);
        o.body.grounded = false;
        o.body.stun(0.7);
        p.diveHits.set(o.id, t + 0.6);
      }
    }
  }

  private rules(p: Pawn, t: number) {
    const b = p.body;
    const pos = b.pos;
    p.progress = Math.max(p.progress, pos.z);
    const fin = this.spec.finish;
    if (
      fin &&
      t >= 0 &&
      pos.z >= fin.z &&
      pos.y > fin.y - 2 &&
      (fin.halfWidth === undefined || Math.abs(pos.x) <= fin.halfWidth) &&
      this.kind === 'round'
    ) {
      if (!this.frozen) {
        p.status = 'finished';
        this.finished.push(p.id);
        this.bodyMap.delete(p.id);
        this.hooks.onFinish(p.id);
      }
      return;
    }
    if (b.grounded && this.spec.checkpoints) {
      for (const cp of this.spec.checkpoints) if (pos.z >= cp.z && (!p.checkpoint || cp.z > p.checkpoint.z)) p.checkpoint = cp;
    }
    const fell = pos.y < this.spec.killY || (this.spec.isOut?.(pos) ?? false);
    if (!fell) return;
    if (this.fall === 'out' && !this.frozen) {
      p.status = 'out';
      this.out.push(p.id);
      this.bodyMap.delete(p.id);
      this.hooks.onOut(p.id);
      return;
    }
    const to = this.fall === 'checkpoint' && p.checkpoint ? p.checkpoint.p : p.spawn;
    const jitter = ((p.id * 7919) % 100) / 100 - 0.5;
    b.reset(new THREE.Vector3(to.x + jitter * 2, to.y + 0.5, to.z), this.faceYaw(to));
    p.teleported = true;
  }

  /** Snapshot for one viewer (their own full state + everyone else), or for spectators. */
  snapshotFor(viewer: number | null): Snapshot {
    const bodies: RemoteState[] = [];
    let own: Snapshot['own'] = null;
    for (const p of this.pawns.values()) {
      if (p.status !== 'play') continue;
      if (p.id === viewer) {
        own = { ack: p.ack, s: p.body.toFull(p.teleported) };
        continue;
      }
      bodies.push({
        id: p.id,
        x: p.body.pos.x,
        y: p.body.pos.y,
        z: p.body.pos.z,
        yaw: p.body.yaw,
        anim: animFor(p.body, p.grabbing !== null),
        flags: p.grabbing !== null ? REMOTE_FLAG.grab : 0,
      });
    }
    return { arena: this.id, tick: this.tick, own, bodies };
  }

  /** Called after snapshots were sent: one-shot flags are cleared. */
  clearTeleports() {
    for (const p of this.pawns.values()) p.teleported = false;
  }

  dispose() {
    this.builder.dispose();
  }
}

export function animFor(b: PlayerBody, grabbing: boolean): number {
  if (b.state === 'stun') return ANIM.stun;
  if (b.state === 'dive') return ANIM.dive;
  if (b.state === 'slide') return ANIM.slide;
  if (!b.grounded) return ANIM.air;
  if (grabbing) return ANIM.grab;
  return ANIM.idle;
}
