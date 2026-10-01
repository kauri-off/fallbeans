import * as THREE from 'three';
import {
  BTN,
  type InputFrame,
  type InputPacket,
  POWER_SHIFT,
  REMOTE_FLAG,
  type RemoteState,
  type Snapshot,
} from '../../shared/codec';
import { ANIM, BOT_EVERY, DT, TICK_MS } from '../../shared/consts';
import { type ArenaKind, canMove, type FallBehaviour, fallBehaviour } from '../../shared/game';
import type { Sections } from '../../shared/prof';
import type { GameEventRecord } from '../../shared/protocol';
import { mulberry32, type Rng } from '../../shared/rng';
import { emptyStats, type RoundStats } from '../../shared/rules';
import { Bonuses } from '../../sim/bonus';
import { smoothStick } from '../../sim/bots';
import { Builder } from '../../sim/builder';
import { lookFor } from '../../sim/looks';
import {
  type BotInput,
  type BotMem,
  type BotPlan,
  type BotView,
  type Checkpoint,
  type MapCtx,
  type MapModule,
  type MapSpec,
  specProblems,
} from '../../sim/map';
import { NavGrid } from '../../sim/nav';
import { type OtherBody, PlayerBody } from '../../sim/physics';

/** How far ahead of the server a client may send inputs (ticks). */
const MAX_LEAD = 120;
const MAX_PACKETS_PER_SEC = 150;
/** How far a grab reaches (centre to centre; beans never get closer than BEAN_GAP). */
const GRAB_REACH = 2.1;
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
/**
 * Input missing for a tick (lost or late): the pawn keeps moving the way it did for this many ticks
 * (a Wi-Fi hiccup: the player most likely still holds the same keys), then stops (gone silent).
 */
const INPUT_HOLD = 60;
/** Bots' navigation grid is built this long (s) before the start, while nobody may move yet. */
const NAV_PREBUILD = 1.5;
/** A hit (by a player or a hazard) counts for a fall this long after it (s). */
const CREDIT_WINDOW = 3;
/** Time (s) a bean may stand somewhere forbidden before it counts as a shortcut. */
const FORBIDDEN_GRACE = 0.6;

export type PawnStatus = 'play' | 'finished' | 'out';

/** How far (m) a lobby spawn point must be from every bean to count as free. */
const SPAWN_CLEAR = 2.5;

/**
 * A round as played (dev rooms record them): enough to simulate it again tick by tick and get the
 * same result (see scripts/replay.ts). Human input is stored as the frames the simulation actually
 * used (late presses merged, gaps filled), run-length encoded; bots replay from the seed.
 */
export interface Recording {
  v: 1;
  game: string;
  kind: ArenaKind;
  seed: number;
  startAt: number;
  /** `now` the arena was created with. */
  createdAt: number;
  participants: number[];
  /** Pawns in the order they were added: id, bot, spawn index, tick added at. */
  pawns: [number, boolean, number | null, number][];
  /** Human frames: [tick, mx, mz, buttons] whenever they change. */
  frames: Record<string, [number, number, number, number][]>;
  /** Dev and roster changes applied between ticks: [after tick, op, args]. */
  ops: [number, string, unknown[]][];
  endTick: number;
  /** stateHash() at endTick. */
  hash: string;
}

/** One line of a bean's recent history (debug tracer, 10 per second). */
export interface TraceEntry {
  t: number;
  pos: [number, number, number];
  vel: [number, number, number];
  state: string;
  grounded: boolean;
  /** Input used: move x/z (−127…127) and buttons (1 jump, 2 dive, 4 grab). */
  input: [number, number, number];
  grabbing: number | null;
  hazard: string | null;
}

/** Something that happened in the arena (debug journal). */
export interface JournalEntry {
  t: number;
  what: string;
  id?: number;
  data?: unknown;
}

/** Trace: one entry every TRACE_EVERY ticks, the last TRACE_LEN entries per bean (30 s). */
const TRACE_EVERY = 12;
const TRACE_LEN = 300;
const JOURNAL_LEN = 300;
const r3 = (v: number) => Math.round(v * 1000) / 1000;

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
  /** Newest input tick received. */
  newest: number;
  /**
   * Fewest ticks the received inputs were ahead of the simulation since the last snapshot to this
   * player (taken just before each packet arrived; negative: the simulation had to guess).
   */
  margin: number;
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
  /** Dev: holds on as if the grab button were pressed until this sim time. */
  forceGrabUntil: number;
  /** Grab button held with nobody in hand (the arms reach out). */
  reaching: boolean;
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
  /** CPU profile of the tick (sections: inputs, bots, movers, physics, interact, rules, map, snapshot). */
  prof?: Sections;
  /** Record the round for replays (dev rooms). */
  record?: boolean;
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
  /** Dev: bot brains run (false: bots stand still). */
  private bots = true;
  get botsOn() {
    return this.bots;
  }
  set botsOn(on: boolean) {
    this.op('bots', [on]);
    this.bots = on;
  }
  /** World.hash(true): clients compare it with their own build of the map. */
  readonly staticHash: string;
  /** Dev: the round so far, for replays (null unless recording). */
  readonly recording: Recording | null;
  /** Debug: recent history of every bean, and of the arena. */
  readonly trace = new Map<number, TraceEntry[]>();
  readonly journal: JournalEntry[] = [];
  private readonly hooks: ArenaHooks;
  private readonly prof: Sections | null;
  private readonly bodyMap = new Map<number, PlayerBody>();
  private spawnCursor = 0;
  /** Bot navigation grid, built on first use once the round has started (gates are open). */
  private nav: NavGrid | null = null;
  /** The grid built ahead of the start, and the static world it was built for. */
  private navPre: { nav: NavGrid | null; hash: string } | null = null;
  /** Bonuses lying on the course (rounds only). */
  readonly bonuses: Bonuses | null;

  constructor(o: ArenaOptions) {
    this.id = o.id;
    this.kind = o.kind;
    this.module = o.module;
    this.seed = o.seed;
    this.startAt = o.startAt;
    this.endAt = o.startAt + o.module.meta.duration * 1000;
    this.participants = o.participants;
    this.hooks = o.hooks;
    this.prof = o.prof ?? null;
    this.recording = o.record
      ? {
          v: 1,
          game: o.module.meta.id,
          kind: o.kind,
          seed: o.seed,
          startAt: o.startAt,
          createdAt: o.now,
          participants: [...o.participants],
          pawns: [],
          frames: {},
          ops: [],
          endTick: 0,
          hash: '',
        }
      : null;
    this.fall = o.kind === 'round' ? fallBehaviour(o.module.meta.genre) : 'spawn';
    this.tick = Math.floor((o.now - o.startAt) / TICK_MS) - 1;
    this.builder = new Builder(o.seed, null);
    this.builder.setLook(lookFor(o.module.looks, o.seed));
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
    const bad = specProblems(this.spec);
    if (bad.length) throw new Error(`map ${o.module.meta.id}: ${bad.join(', ')}`);
    this.bonuses =
      o.kind === 'round'
        ? new Bonuses(this.builder, o.seed, { arena: !this.spec.finish, duration: o.module.meta.duration })
        : null;
    // Portals: tell the clients when one was used (it closes; they show the light). Not kept in
    // `events` for late joiners: it is over in a second or two.
    this.builder.onPortal = (pair, from, t) => this.hooks.onEvent('portal', { pair, from, t });
    this.builder.world.finalize(this.tick * DT);
    this.staticHash = this.builder.world.hash(true);
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
    const i = spawnIndex ?? (this.kind === 'lobby' ? this.freeSpawn() : this.spawnCursor++);
    this.recording?.pawns.push([id, bot, i, this.tick]);
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
            // By slot in the round, not id: the same seed plays out the same whoever joined when.
            rng: mulberry32(this.seed ^ ((this.participants.indexOf(id) + 1 || id) * 2654435761)),
            input: { mx: 0, mz: 0, jump: false, dive: false, grab: false, emote: 0 },
          }
        : null,
      inputs: new Map(),
      last: IDLE,
      ack: this.tick,
      late: 0,
      lateSeen: this.tick,
      newest: this.tick,
      margin: Number.POSITIVE_INFINITY,
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
      forceGrabUntil: -1e9,
      reaching: false,
    };
    this.pawns.set(id, pawn);
    this.bodyMap.set(id, body);
  }

  /**
   * A spawn point nobody stands on (the lobby: people come and go, and fall and come back, at any
   * time): the next one in turn that is clear, else the one farthest from every bean.
   */
  private freeSpawn(): number {
    const spawns = this.spec.spawns;
    let best = this.spawnCursor % spawns.length;
    let bestD = -1;
    for (let k = 0; k < spawns.length; k++) {
      const i = (this.spawnCursor + k) % spawns.length;
      const s = spawns[i]!;
      let d = Number.POSITIVE_INFINITY;
      for (const p of this.pawns.values()) if (p.status === 'play') d = Math.min(d, p.body.pos.distanceToSquared(s));
      if (d > SPAWN_CLEAR * SPAWN_CLEAR) {
        best = i;
        break;
      }
      if (d > bestD) {
        bestD = d;
        best = i;
      }
    }
    this.spawnCursor = best + 1;
    return best;
  }

  removePawn(id: number) {
    this.op('remove', [id]);
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
    const newest = pkt.firstTick + pkt.frames.length - 1;
    if (newest <= this.tick + MAX_LEAD) {
      p.margin = Math.min(p.margin, p.newest - this.tick);
      p.newest = Math.max(p.newest, newest);
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

  /**
   * Runs every tick due by `nowMs`; returns the number of ticks stepped. After a stall only the last
   * MAX_CATCHUP ticks are simulated, unless `all` (dev time warps).
   */
  advance(nowMs: number, all = false): number {
    const target = Math.floor((nowMs - this.startAt) / TICK_MS);
    let steps = 0;
    if (!all && target - this.tick > MAX_CATCHUP) {
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
    if (k - p.ack > INPUT_HOLD) return { frame: late ? { ...IDLE, buttons: late } : IDLE, real: false };
    return { frame: { mx: p.last.mx, mz: p.last.mz, buttons: (p.last.buttons & BTN.grab) | late }, real: false };
  }

  /** Bots' navigation grid for the current static world, or null if it cannot be built. */
  private buildNav(): NavGrid | null {
    try {
      return NavGrid.build(this.world, this.spec.forbidden);
    } catch (e) {
      this.hooks.warn('bot navigation failed', { game: this.module.meta.id, err: String(e) });
      return null;
    }
  }

  /**
   * The navigation grid takes tens of milliseconds to build; built at the start it stalled every
   * room's simulation right when everybody starts to move (their first inputs arrived "late"). It
   * is built before the start instead, and used at the start if the static world has not changed.
   */
  private prebuildNav() {
    if (this.nav || this.navPre || this.kind !== 'round' || !this.spec.bot) return;
    if (this.time < -NAV_PREBUILD || this.time >= 0) return;
    if (![...this.pawns.values()].some((p) => p.bot)) return;
    this.navPre = { nav: this.buildNav(), hash: this.world.hash(true) };
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
    if (!brain || !this.botsOn) return;
    this.prof?.start('bots');
    if (!this.nav && (this.time >= 0 || this.kind !== 'round')) {
      // Built ahead during the intro (see prebuildNav): the same grid if the static world is the same.
      const pre = this.navPre;
      this.navPre = null;
      if (pre && pre.hash === this.world.hash(true)) this.nav = pre.nav;
      else this.nav = this.buildNav();
    }
    const others: BotView['others'][number][] = [];
    for (const o of this.pawns.values())
      if (o !== p && o.status === 'play') {
        const ob = o.body;
        const dive = ob.state === 'dive' || (ob.state === 'slide' && Math.hypot(ob.vel.x, ob.vel.z) > 6);
        others.push({ id: o.id, pos: ob.pos, vel: ob.vel, down: ob.down, dive, reach: o.reaching });
      }
    const view: BotView = {
      id: p.id,
      body: p.body,
      t: this.time,
      rng: st.rng,
      mem: st.mem,
      plan: st.plan,
      others,
      nav: this.nav,
      bonuses: this.bonuses?.available(this.time) ?? [],
    };
    try {
      brain(view, out);
      smoothStick(view, out);
    } catch (e) {
      this.hooks.warn('bot brain failed', { game: this.module.meta.id, err: String(e) });
    }
    if (out.emote) this.hooks.onEmote?.(p.id, out.emote);
    this.prof?.start('inputs');
  }

  private step(k: number) {
    const prof = this.prof;
    prof?.start('inputs');
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
      if (this.recording && !p.bot) this.recordFrame(p.id, k, frame);
      p.last = got.frame;
      if (real) p.ack = k;
      if (this.kind === 'round' && t >= 0) {
        if (real && (frame.mx || frame.mz || frame.buttons)) p.activeAt = t;
        p.stats.idle = Math.max(p.stats.idle, t - p.activeAt);
      }
      // Drop inputs that are now in the past (arrived too late).
      if (p.inputs.size > 0) for (const key of p.inputs.keys()) if (key <= k) p.inputs.delete(key);
    }
    prof?.start('movers');
    for (const p of active) {
      p.body.clearEvents();
      p.body.beforeWorldUpdate();
    }
    this.world.setTime(t);
    for (const p of active) p.body.afterWorldUpdate();
    prof?.start('physics');

    // (Someone inside a portal is nowhere to bump into.)
    const others: OtherBody[] = active
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
    prof?.start('interact');
    for (const p of active) this.interact(p, frames.get(p)!, active, t, frames);
    if (this.bonuses && t >= 0 && !this.frozen)
      this.bonuses.check(
        t,
        active.map((p) => p.body),
        (name, data) => {
          this.bonuses?.onEvent(data);
          this.note('bonus', undefined, data);
          this.events.push([name, data]);
          this.hooks.onEvent(name, data);
        },
      );
    prof?.start('rules');
    for (const p of active) this.rules(p, t);
    if (k % TRACE_EVERY === 0) for (const p of active) this.record(p, t, frames.get(p)!);
    prof?.start('map');
    try {
      this.spec.tick?.(t);
    } catch (e) {
      this.hooks.warn('game tick failed', { game: this.module.meta.id, err: String(e) });
    }
    this.prebuildNav();
    prof?.start('snapshot');
    this.hooks.onSnapshot(k);
    prof?.stop();
    prof?.frame();
  }

  private recordFrame(id: number, k: number, f: InputFrame) {
    const list = (this.recording!.frames[id] ??= []);
    const last = list.at(-1);
    if (!last || last[1] !== f.mx || last[2] !== f.mz || last[3] !== f.buttons) list.push([k, f.mx, f.mz, f.buttons]);
  }

  /** Records something that changes the simulation from outside, before the next tick. */
  private op(name: string, args: unknown[]) {
    this.recording?.ops.push([this.tick, name, args]);
  }

  /** The recording so far, closed at the current tick. */
  takeRecording(): Recording | null {
    const r = this.recording;
    if (!r) return null;
    return { ...r, endTick: this.tick, hash: this.stateHash() };
  }

  /** Hash of every bean's state (replays and determinism checks compare it). */
  stateHash(): string {
    let h = 2166136261;
    const mix = (v: number) => {
      const x = Math.round(v * 1e5);
      h = Math.imul(h ^ (x & 0xffff), 16777619);
      h = Math.imul(h ^ (x >>> 16), 16777619);
    };
    for (const [id, p] of [...this.pawns].sort((a, b) => a[0] - b[0])) {
      mix(id);
      const b = p.body;
      for (const v of [b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.yaw, b.tilt]) mix(v);
    }
    return (h >>> 0).toString(16).padStart(8, '0');
  }

  /** Replays: the frame a human pawn uses at tick k (see scripts/replay.ts). */
  forceInput(id: number, k: number, f: InputFrame) {
    this.pawns.get(id)?.inputs.set(k, f);
  }

  /** Replays: applies a recorded operation. */
  applyOp(name: string, args: unknown[]) {
    type V3 = [number, number, number];
    const id = args[0] as number;
    if (name === 'remove') this.removePawn(id);
    else if (name === 'late') {
      const at = args[2] as V3 | null;
      this.addLatePawn(id, args[1] as boolean, at ? new THREE.Vector3(...at) : undefined);
    } else if (name === 'teleport') this.devTeleport(id, new THREE.Vector3(...(args[1] as V3)), args[2] as number | undefined);
    else if (name === 'knock') this.devKnock(id, args[1] as V3);
    else if (name === 'kill') this.devKill(id);
    else if (name === 'grab') this.devGrab(id, args[1] as number, args[2] as number);
    else if (name === 'bots') this.botsOn = args[0] as boolean;
  }

  /** Adds a line to the debug journal. */
  note(what: string, id?: number, data?: unknown) {
    this.journal.push({ t: r3(this.time), what, ...(id !== undefined ? { id } : {}), ...(data !== undefined ? { data } : {}) });
    if (this.journal.length > JOURNAL_LEN) this.journal.shift();
  }

  private record(p: Pawn, t: number, f: InputFrame) {
    let list = this.trace.get(p.id);
    if (!list) {
      list = [];
      this.trace.set(p.id, list);
    }
    const b = p.body;
    list.push({
      t: r3(t),
      pos: [r3(b.pos.x), r3(b.pos.y), r3(b.pos.z)],
      vel: [r3(b.vel.x), r3(b.vel.y), r3(b.vel.z)],
      state: b.state,
      grounded: b.grounded,
      input: [f.mx, f.mz, f.buttons],
      grabbing: p.grabbing,
      hazard: b.hazard,
    });
    if (list.length > TRACE_LEN) list.shift();
  }

  /** Grabbing holds on (pulling the other bean along) until released, broken free or timed out; dives knock over. */
  private interact(p: Pawn, f: InputFrame, active: Pawn[], t: number, frames: Map<Pawn, InputFrame>) {
    const b = p.body;
    const wants = ((f.buttons & BTN.grab) !== 0 || t < p.forceGrabUntil) && b.state === 'normal';
    p.reaching = wants && p.grabbing === null;
    if (p.grabbing !== null) {
      const o = this.pawns.get(p.grabbing);
      const dist = o ? Math.hypot(o.body.pos.x - b.pos.x, o.body.pos.z - b.pos.z) : 99;
      const escaped = !!o && (o.struggle >= STRUGGLE || o.body.state === 'dive' || o.body.down || o.body.inPortal);
      if (!wants || !o || o.status !== 'play' || escaped || t - p.holdSince > HOLD_MAX || dist > HOLD_BREAK) {
        p.grabbing = null;
        p.grabReadyAt = t + (escaped || t - p.holdSince > HOLD_MAX ? GRAB_COOLDOWN : 0.3);
        if (o) o.struggle = 0;
      } else this.hold(p, o, dist, t, frames.get(o));
    } else if (wants && t >= p.grabReadyAt) {
      const fx = Math.sin(b.yaw);
      const fz = Math.cos(b.yaw);
      let best = Number.POSITIVE_INFINITY;
      let target: Pawn | null = null;
      for (const o of active) {
        if (o === p || o.body.down || o.body.inPortal) continue;
        const dx = o.body.pos.x - b.pos.x;
        const dz = o.body.pos.z - b.pos.z;
        const d = Math.hypot(dx, dz);
        // Reach grows with the size of either bean (giants have long arms, and are big targets).
        const reach = GRAB_REACH * Math.max(b.size, o.body.size);
        if (d > reach || Math.abs(o.body.pos.y - b.pos.y) > 1.6 * Math.max(b.size, o.body.size)) continue;
        // Anything in front, or right beside (turning to it): forgiving, as grabbing should feel.
        const facing = d > 0.3 ? (dx * fx + dz * fz) / d : 1;
        if (facing < (d < 1.5 ? -0.35 : 0.1)) continue;
        // Prefer what is ahead over what is merely close.
        const score = d - facing * 0.6;
        if (score > best) continue;
        best = score;
        target = o;
      }
      if (target) {
        this.note('grab', p.id, { target: target.id });
        p.grabbing = target.id;
        p.holdSince = t;
        target.struggle = 0;
        if (this.kind === 'round') p.stats.grabs++;
        try {
          this.spec.onGrab?.(p.id, target.id);
        } catch (e) {
          this.hooks.warn('grab handler failed', { err: String(e) });
        }
        p.reaching = false;
        this.hold(p, target, Math.hypot(target.body.pos.x - b.pos.x, target.body.pos.z - b.pos.z), t, frames.get(target));
      }
    }

    if (b.state === 'dive' || (b.state === 'slide' && Math.hypot(b.vel.x, b.vel.z) > 6)) {
      const fx = Math.sin(b.yaw);
      const fz = Math.cos(b.yaw);
      for (const o of active) {
        // Anyone can be hit, a bean already down or getting up too (each diver once per dive).
        if (o === p || (p.diveHits.get(o.id) ?? -1) > t) continue;
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
    // A giant held by a normal bean is barely slowed (and not dragged much).
    const heavy = ob.mass / b.mass;
    ob.slowUntil = t + 0.15;
    ob.slowK = heavy > 1 ? 0.85 : 0.5;
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
        ob.vel.x -= (dx * away * 0.6) / heavy;
        ob.vel.z -= (dz * away * 0.6) / heavy;
      }
      const k = Math.min(1, (dist - HOLD_LEN) * 0.5) / heavy;
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
        this.note('finish', p.id);
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
      this.note('out', p.id, { by, cause, pos: [r3(pos.x), r3(pos.y), r3(pos.z)] });
      this.out.push(p.id);
      this.bodyMap.delete(p.id);
      this.hooks.onKo({ id: p.id, out: true, by, cause, shortcut: false });
      return;
    }
    if (counts && !shortcut) p.stats.falls++;
    this.note(shortcut ? 'shortcut' : 'fall', p.id, { by, cause, pos: [r3(pos.x), r3(pos.y), r3(pos.z)] });
    if (counts || this.kind === 'lobby') this.hooks.onKo({ id: p.id, out: false, by, cause, shortcut });
    if (counts && !shortcut) this.spec.onFall?.(p.id, by);
    const to =
      this.fall === 'checkpoint' && p.checkpoint
        ? p.checkpoint.p
        : this.kind === 'lobby'
          ? this.spec.spawns[this.freeSpawn()]!
          : p.spawn;
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
        // The worst input margin since the last snapshot (none arrived: how far ahead the newest is now).
        const margin = Math.min(p.margin, p.newest - this.tick);
        p.margin = Number.POSITIVE_INFINITY;
        own = { ack: p.ack, s: p.body.toFull(p.teleported), grab: p.grabbing ?? -1, margin };
        continue;
      }
      bodies.push({
        id: p.id,
        x: p.body.pos.x,
        y: p.body.pos.y,
        z: p.body.pos.z,
        yaw: p.body.yaw,
        anim: animFor(p.body, p.grabbing !== null, p.reaching),
        flags:
          (p.grabbing !== null ? REMOTE_FLAG.grab : 0) |
          (p.reaching ? REMOTE_FLAG.reach : 0) |
          ((p.body.power & 3) << POWER_SHIFT),
        tilt: p.body.tilt,
        tiltDir: p.body.tiltDir,
        grab: p.grabbing ?? -1,
      });
    }
    return { arena: this.id, tick: this.tick, own, bodies };
  }

  // ------------------------------------------------------------------ dev tools (server --dev only)

  /** Adds a pawn to a running arena (a bot joining mid-round), at `at` or the next spawn. */
  addLatePawn(id: number, bot: boolean, at?: THREE.Vector3) {
    if (this.pawns.has(id)) return;
    this.op('late', [id, bot, at ? [at.x, at.y, at.z] : null]);
    const rec = this.recording;
    // The op re-adds it on replay: keep it out of the list of pawns added at the start.
    const n = rec?.pawns.length ?? 0;
    if (!this.participants.includes(id)) this.participants.push(id);
    this.addPawn(id, bot);
    if (rec && rec.pawns.length > n) rec.pawns.pop();
    if (at) this.place(id, at);
  }

  private place(id: number, pos: THREE.Vector3, yaw?: number): boolean {
    const p = this.pawns.get(id);
    if (p?.status !== 'play') return false;
    p.body.reset(pos, yaw ?? p.body.yaw);
    p.teleported = true;
    p.forbiddenFor = 0;
    return true;
  }

  devTeleport(id: number, pos: THREE.Vector3, yaw?: number): boolean {
    this.op('teleport', [id, [pos.x, pos.y, pos.z], yaw]);
    return this.place(id, pos, yaw);
  }

  /** Where `goto` sends a bean: its spawn, a checkpoint, or 3 m before the finish line. */
  devPlace(id: number, to: 'spawn' | 'finish' | number): THREE.Vector3 | null {
    const p = this.pawns.get(id);
    if (!p) return null;
    if (to === 'spawn') return p.spawn.clone();
    if (to === 'finish') {
      const f = this.spec.finish;
      return f ? new THREE.Vector3(0, f.y + 1.5, f.z - 3) : null;
    }
    const cp = this.spec.checkpoints?.[to];
    return cp ? cp.p.clone().setY(cp.p.y + 0.5) : null;
  }

  devKnock(id: number, v: readonly [number, number, number]): boolean {
    this.op('knock', [id, v]);
    const p = this.pawns.get(id);
    if (p?.status !== 'play') return false;
    p.body.knock(v[0], v[2], v[1], 1);
    p.lastHit = { by: null, cause: 'dev', t: this.time };
    return true;
  }

  /** Drops a bean below the kill height: it falls by the map's rules on the next tick. */
  devKill(id: number): boolean {
    this.op('kill', [id]);
    const p = this.pawns.get(id);
    if (p?.status !== 'play') return false;
    p.body.pos.y = this.spec.killY - 1;
    return true;
  }

  devGrab(actor: number, target: number, seconds: number): string | null {
    this.op('grab', [actor, target, seconds]);
    const a = this.pawns.get(actor);
    const o = this.pawns.get(target);
    if (!a || !o || a === o || a.status !== 'play' || o.status !== 'play') return 'no such beans in play';
    const t = this.time;
    a.forceGrabUntil = t + seconds;
    a.grabbing = o.id;
    a.holdSince = t;
    o.struggle = 0;
    return null;
  }

  /** Called after snapshots were sent: one-shot flags are cleared. */
  clearTeleports() {
    for (const p of this.pawns.values()) p.teleported = false;
  }

  dispose() {
    this.builder.dispose();
  }
}

export function animFor(b: PlayerBody, grabbing: boolean, reaching = false): number {
  if (b.state === 'portal') return ANIM.portal;
  if (b.state === 'ladder') return ANIM.ladder;
  if (b.state === 'tumble') return ANIM.tumble;
  if (b.state === 'getup') return ANIM.getup;
  if (b.state === 'stun') return ANIM.stun;
  if (b.state === 'dive') return ANIM.dive;
  if (b.state === 'slide') return ANIM.slide;
  if (b.state === 'climb') return b.climbingOver ? ANIM.climbOver : ANIM.climb;
  if (!b.grounded) return ANIM.air;
  if (grabbing) return ANIM.grab;
  if (reaching) return ANIM.reach;
  return ANIM.idle;
}
