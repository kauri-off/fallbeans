import * as THREE from 'three';
import { BTN, type InputFrame, type InputPacket, REMOTE_FLAG, type RemoteState, type Snapshot } from '../shared/codec';
import { ANIM, BOT_EVERY, DT, TICK_MS } from '../shared/consts';
import { type ArenaKind, canMove, type FallBehaviour, fallBehaviour } from '../shared/game';
import type { GameEventRecord } from '../shared/protocol';
import { mulberry32, type Rng } from '../shared/rng';
import { emptyStats, type RoundStats } from '../shared/rules';
import { Builder } from '../sim/builder';
import type { BotInput, BotMem, BotPlan, BotView, Checkpoint, MapCtx, MapModule, MapSpec } from '../sim/map';
import { NavGrid } from '../sim/nav';
import { type OtherBody, PlayerBody } from '../sim/physics';

/** How far ahead of the server a client may send inputs (ticks). */
const MAX_LEAD = 120;
const MAX_PACKETS_PER_SEC = 150;
const GRAB_REACH = 1.5;
/** Holding: rope length, how far it stretches before breaking, how long it lasts (s). */
const HOLD_LEN = 1.35;
const HOLD_BREAK = 2.8;
const HOLD_MAX = 3;
/** Jumps a held bean needs to break free. */
const STRUGGLE = 3;
const GRAB_COOLDOWN = 1.2;
/** Centre distance for a dive to tackle (beans never get closer than BEAN_GAP). */
const DIVE_REACH = 1.6;
const MAX_CATCHUP = 60;
/** A hit (by a player or a hazard) counts for a fall this long after it (s). */
const CREDIT_WINDOW = 3;
/** Time (s) a bean may stand somewhere forbidden before it counts as a shortcut. */
const FORBIDDEN_GRACE = 0.6;

export type PawnStatus = 'play' | 'finished' | 'out';

interface BotState {
  mem: BotMem;
  plan: BotPlan;
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
  /** Jump/dive presses that arrived after their tick was simulated: applied on the next tick. */
  late: number;
  /** Highest tick whose late input has been looked at. */
  lateSeen: number;
  spawn: THREE.Vector3;
  checkpoint: Checkpoint | null;
  progress: number;
  grabbing: number | null;
  holdSince: number;
  grabReadyAt: number;
  /** Jumps while held (breaking free). */
  struggle: number;
  diveHits: Map<number, number>;
  teleported: boolean;
  packetWindow: number;
  packets: number;
  rejected: number;
  stats: RoundStats;
  /** Last thing that hit this bean: another player (by) and/or a hazard (cause). */
  lastHit: { by: number | null; cause: string; t: number } | null;
  /** Sim time of the last real input with movement or buttons. */
  activeAt: number;
  forbiddenFor: number;
}

export interface KoInfo {
  id: number;
  /** Eliminated from the round (survival), not just respawned. */
  out: boolean;
  by: number | null;
  cause: string;
  shortcut: boolean;
}

export interface ArenaHooks {
  onFinish(id: number, time: number): void;
  onKo(ko: KoInfo): void;
  onEvent(name: string, data: unknown): void;
  onScore(id: number, v: number): void;
  onSnapshot(tick: number): void;
  onEmote?(id: number, e: number): void;
  warn(msg: string, data?: Record<string, unknown>): void;
}

export interface ArenaOptions {
  id: number;
  kind: ArenaKind;
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
  readonly kind: ArenaKind;
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
  /** Bot navigation grid, built on first use once the round has started (gates are open). */
  private nav: NavGrid | null = null;

  constructor(o: ArenaOptions) {
    this.id = o.id;
    this.kind = o.kind;
    this.module = o.module;
    this.seed = o.seed;
    this.startAt = o.startAt;
    this.endAt = o.startAt + o.module.meta.duration * 1000;
    this.participants = o.participants;
    this.hooks = o.hooks;
    this.fall = o.kind === 'round' ? fallBehaviour(o.module.meta.genre) : 'spawn';
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
            plan: { path: null, i: 0, tx: 0, tz: 0, at: -1e9 },
            rng: mulberry32(this.seed ^ (id * 2654435761)),
            input: { mx: 0, mz: 0, jump: false, dive: false, grab: false, emote: 0 },
          }
        : null,
      inputs: new Map(),
      last: IDLE,
      ack: this.tick,
      late: 0,
      lateSeen: this.tick,
      spawn,
      checkpoint: null,
      progress: spawn.z,
      grabbing: null,
      holdSince: 0,
      grabReadyAt: -1e9,
      struggle: 0,
      diveHits: new Map(),
      teleported: true,
      packetWindow: 0,
      packets: 0,
      rejected: 0,
      stats: emptyStats(),
      lastHit: null,
      activeAt: Math.max(0, this.time),
      forbiddenFor: 0,
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
      if (k <= this.tick) {
        // Too late to simulate on time, but a press must not get lost: it happens on the next tick.
        if (k > p.ack && k > p.lateSeen && k > this.tick - 30) {
          p.late |= f.buttons & (BTN.jump | BTN.dive);
          p.lateSeen = k;
        }
        return;
      }
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
    const late = p.late;
    p.late = 0;
    const f = p.inputs.get(k);
    if (f) {
      p.inputs.delete(k);
      return { frame: late ? { ...f, buttons: f.buttons | late } : f, real: true };
    }
    // Missing input: keep moving the same way for a moment, never repeating a jump or dive;
    // a client that went silent stops.
    if (k - p.ack > 30) return { frame: late ? { ...IDLE, buttons: late } : IDLE, real: false };
    return { frame: { mx: p.last.mx, mz: p.last.mz, buttons: (p.last.buttons & BTN.grab) | late }, real: false };
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
    out.emote = 0;
    if (!brain) return;
    if (!this.nav && (this.time >= 0 || this.kind !== 'round')) {
      try {
        this.nav = NavGrid.build(this.world, this.spec.forbidden);
      } catch (e) {
        this.hooks.warn('bot navigation failed', { game: this.module.meta.id, err: String(e) });
      }
    }
    const others: BotView['others'][number][] = [];
    for (const o of this.pawns.values())
      if (o !== p && o.status === 'play') others.push({ id: o.id, pos: o.body.pos, vel: o.body.vel, down: o.body.down });
    try {
      brain({ id: p.id, body: p.body, t: this.time, rng: st.rng, mem: st.mem, plan: st.plan, others, nav: this.nav }, out);
    } catch (e) {
      this.hooks.warn('bot brain failed', { game: this.module.meta.id, err: String(e) });
    }
    if (out.emote) this.hooks.onEmote?.(p.id, out.emote);
  }

  private step(k: number) {
    this.tick = k;
    const t = k * DT;
    const active: Pawn[] = [];
    const frames = new Map<Pawn, InputFrame>();
    const moving = canMove(this.kind, t);
    for (const p of this.pawns.values()) {
      if (p.status !== 'play') continue;
      active.push(p);
      const got = this.frameFor(p, k);
      const { real } = got;
      // Before the start (and on the podium) inputs are read and acknowledged, but nobody moves.
      const frame = moving ? got.frame : IDLE;
      frames.set(p, frame);
      p.last = got.frame;
      if (real) p.ack = k;
      if (this.kind === 'round' && t >= 0) {
        if (real && (frame.mx || frame.mz || frame.buttons)) p.activeAt = t;
        p.stats.idle = Math.max(p.stats.idle, t - p.activeAt);
      }
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
      vx: p.body.vel.x,
      vz: p.body.vel.z,
      touching: false,
    }));
    for (const p of active) {
      const f = frames.get(p)!;
      const input = { mx: f.mx / 127, mz: f.mz / 127, jump: (f.buttons & BTN.jump) !== 0, dive: (f.buttons & BTN.dive) !== 0 };
      const rest = others.filter((o) => o.id !== p.id);
      p.body.step(DT, input, this.world, t, rest);
    }
    for (const p of active) {
      const hz = p.body.hazard;
      if (hz) p.lastHit = { by: p.lastHit && t - p.lastHit.t < CREDIT_WINDOW ? p.lastHit.by : null, cause: hz, t };
    }
    for (const p of active) this.interact(p, frames.get(p)!, active, t, frames);
    for (const p of active) this.rules(p, t);
    try {
      this.spec.tick?.(t);
    } catch (e) {
      this.hooks.warn('game tick failed', { game: this.module.meta.id, err: String(e) });
    }
    this.hooks.onSnapshot(k);
  }

  /** Grabbing holds on (pulling the other bean along) until released, broken free or timed out; dives knock over. */
  private interact(p: Pawn, f: InputFrame, active: Pawn[], t: number, frames: Map<Pawn, InputFrame>) {
    const b = p.body;
    const wants = (f.buttons & BTN.grab) !== 0 && b.state === 'normal';
    if (p.grabbing !== null) {
      const o = this.pawns.get(p.grabbing);
      const dist = o ? Math.hypot(o.body.pos.x - b.pos.x, o.body.pos.z - b.pos.z) : 99;
      const escaped = !!o && (o.struggle >= STRUGGLE || o.body.state === 'dive' || o.body.down);
      if (!wants || !o || o.status !== 'play' || escaped || t - p.holdSince > HOLD_MAX || dist > HOLD_BREAK) {
        p.grabbing = null;
        p.grabReadyAt = t + (escaped || t - p.holdSince > HOLD_MAX ? GRAB_COOLDOWN : 0.3);
        if (o) o.struggle = 0;
      } else this.hold(p, o, dist, t, frames.get(o));
    } else if (wants && t >= p.grabReadyAt) {
      const fx = Math.sin(b.yaw);
      const fz = Math.cos(b.yaw);
      let best = GRAB_REACH;
      let target: Pawn | null = null;
      for (const o of active) {
        if (o === p || o.body.down) continue;
        const dx = o.body.pos.x - b.pos.x;
        const dz = o.body.pos.z - b.pos.z;
        const d = Math.hypot(dx, dz);
        if (d > best || Math.abs(o.body.pos.y - b.pos.y) > 1.2) continue;
        if (d > 0.3 && (dx * fx + dz * fz) / d < 0.2) continue;
        best = d;
        target = o;
      }
      if (target) {
        p.grabbing = target.id;
        p.holdSince = t;
        target.struggle = 0;
        if (this.kind === 'round') p.stats.grabs++;
        try {
          this.spec.onGrab?.(p.id, target.id);
        } catch (e) {
          this.hooks.warn('grab handler failed', { err: String(e) });
        }
        this.hold(p, target, best, t, frames.get(target));
      }
    }

    if (b.state === 'dive' || (b.state === 'slide' && Math.hypot(b.vel.x, b.vel.z) > 6)) {
      const fx = Math.sin(b.yaw);
      const fz = Math.cos(b.yaw);
      for (const o of active) {
        if (o === p || o.body.down || (p.diveHits.get(o.id) ?? -1) > t) continue;
        const dx = o.body.pos.x - b.pos.x;
        const dz = o.body.pos.z - b.pos.z;
        const d = Math.hypot(dx, dz);
        if (d > DIVE_REACH || Math.abs(o.body.pos.y - b.pos.y) > 1.3) continue;
        // Only what is ahead of the diver (or right on top of it).
        if (d > 0.4 && (dx * fx + dz * fz) / d < 0.25) continue;
        const nx = d > 1e-3 ? dx / d : fx;
        const nz = d > 1e-3 ? dz / d : fz;
        const sp = Math.hypot(b.vel.x, b.vel.z);
        const k = 5 + Math.min(5, sp * 0.4);
        o.body.knock(nx * k + fx * 2, nz * k + fz * 2, 4.5, 0.9);
        o.lastHit = { by: p.id, cause: 'tackle', t };
        // The tackler spends its momentum on the hit.
        b.vel.x *= 0.45;
        b.vel.z *= 0.45;
        p.diveHits.set(o.id, t + 0.6);
        if (this.kind === 'round') p.stats.tackles++;
      }
    }
  }

  /** One tick of holding: both slow down, the held bean is pulled back within reach; jumping struggles free. */
  private hold(p: Pawn, o: Pawn, dist: number, t: number, of: InputFrame | undefined) {
    const b = p.body;
    const ob = o.body;
    ob.slowUntil = t + 0.15;
    ob.slowK = 0.5;
    b.slowUntil = t + 0.15;
    b.slowK = 0.7;
    o.lastHit = { by: p.id, cause: 'grab', t };
    if (of && (of.buttons & BTN.jump) !== 0) o.struggle++;
    if (dist > HOLD_LEN && dist > 1e-3) {
      const dx = (ob.pos.x - b.pos.x) / dist;
      const dz = (ob.pos.z - b.pos.z) / dist;
      // Spring back towards the grabber; the held bean cannot outrun the hand.
      const away = ob.vel.x * dx + ob.vel.z * dz;
      if (away > 0) {
        ob.vel.x -= dx * away * 0.6;
        ob.vel.z -= dz * away * 0.6;
      }
      const k = Math.min(1, (dist - HOLD_LEN) * 0.5);
      ob.pos.x -= dx * k * 0.1;
      ob.pos.z -= dz * k * 0.1;
    }
    // Face what we hold.
    b.yaw = Math.atan2(ob.pos.x - b.pos.x, ob.pos.z - b.pos.z);
  }

  private rules(p: Pawn, t: number) {
    const b = p.body;
    const pos = b.pos;
    const prog = this.spec.progress?.(pos) ?? pos.z;
    if (t >= 0) p.progress = Math.max(p.progress, prog);
    const round = this.kind === 'round';
    const fin = this.spec.finish;
    if (
      round &&
      fin &&
      t >= 0 &&
      pos.z >= fin.z &&
      pos.y > fin.y - 2 &&
      (fin.halfWidth === undefined || Math.abs(pos.x) <= fin.halfWidth)
    ) {
      if (!this.frozen) {
        p.status = 'finished';
        p.stats.finishAt = t;
        this.finished.push(p.id);
        this.bodyMap.delete(p.id);
        this.hooks.onFinish(p.id, t);
      }
      return;
    }
    if (b.grounded && this.spec.checkpoints) {
      for (const cp of this.spec.checkpoints) if (prog >= cp.z && (!p.checkpoint || cp.z > p.checkpoint.z)) p.checkpoint = cp;
    }
    // Standing where the course does not go (on frames, behind walls): back to the checkpoint, fined.
    // Only while standing there: being knocked off over a rail is a fall, not a shortcut.
    if (b.grounded) p.forbiddenFor = round && t >= 0 && this.spec.forbidden?.(pos) ? p.forbiddenFor + DT : 0;
    const shortcut = p.forbiddenFor > FORBIDDEN_GRACE;
    const fell = shortcut || pos.y < this.spec.killY || (this.spec.isOut?.(pos) ?? false);
    if (!fell) return;
    p.forbiddenFor = 0;
    const hit = p.lastHit && t - p.lastHit.t < CREDIT_WINDOW ? p.lastHit : null;
    p.lastHit = null;
    const by = shortcut ? null : (hit?.by ?? null);
    const cause = shortcut ? 'shortcut' : (hit?.cause ?? this.spec.fallCause ?? 'fall');
    const counts = round && t >= 0 && !this.frozen;
    if (counts) {
      if (by !== null && by !== p.id) {
        const attacker = this.pawns.get(by);
        if (attacker) attacker.stats.kos++;
      }
      if (shortcut) p.stats.shortcuts++;
    }
    if (this.fall === 'out' && !shortcut && round) {
      if (this.frozen) return;
      p.status = 'out';
      p.stats.outAt = Math.max(0, t);
      this.out.push(p.id);
      this.bodyMap.delete(p.id);
      this.hooks.onKo({ id: p.id, out: true, by, cause, shortcut: false });
      return;
    }
    if (counts && !shortcut) p.stats.falls++;
    if (counts || this.kind === 'lobby') this.hooks.onKo({ id: p.id, out: false, by, cause, shortcut });
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
        own = { ack: p.ack, s: p.body.toFull(p.teleported), grab: p.grabbing ?? -1 };
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
        tilt: p.body.tilt,
        tiltDir: p.body.tiltDir,
        grab: p.grabbing ?? -1,
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
  if (b.state === 'tumble') return ANIM.tumble;
  if (b.state === 'getup') return ANIM.getup;
  if (b.state === 'stun') return ANIM.stun;
  if (b.state === 'dive') return ANIM.dive;
  if (b.state === 'slide') return ANIM.slide;
  if (!b.grounded) return ANIM.air;
  if (grabbing) return ANIM.grab;
  return ANIM.idle;
}
