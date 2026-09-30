import * as THREE from 'three';
import { getMap } from '../../games';
import {
  type BodyFullState,
  BTN,
  encodeInput,
  type InputFrame,
  MAX_INPUT_FRAMES,
  POWER_SHIFT,
  quantizeAxis,
  REMOTE_FLAG,
  type RemoteState,
  type Snapshot,
} from '../../shared/codec';
import { ANIM, DT, INPUT_EVERY, INPUT_REDUNDANCY, SNAPSHOT_EVERY, TICK_MS } from '../../shared/consts';
import { canMove } from '../../shared/game';
import { LeadControl } from '../../shared/lead';
import type { ArenaInfo } from '../../shared/protocol';
import { BONUS_EVENT, type Bonus, Bonuses } from '../../sim/bonus';
import { Builder } from '../../sim/builder';
import { lookFor } from '../../sim/looks';
import type { BeanDeco, MapCtx, MapModule, MapSfx, MapSpec } from '../../sim/map';
import { type OtherBody, PlayerBody } from '../../sim/physics';
import { report } from '../debug/capture';
import { lod } from './lod';
import { applySurfaces } from './materials';
import { placeScenery } from './scenery';
import { Statics } from './statics';
import { ClientView, timeUniform } from './view';

const IDLE: InputFrame = { mx: 0, mz: 0, buttons: 0 };
const MAX_HISTORY = 240;
/** Other beans may be extrapolated this far (s) past the newest snapshot when snapshots are late. */
const MAX_EXTRAPOLATE = 0.1;

interface Frame {
  tick: number;
  bodies: Map<number, RemoteState>;
}

export interface RemotePose {
  pos: THREE.Vector3;
  yaw: number;
  anim: number;
  tilt: number;
  tiltDir: number;
  /** Id of the bean held, or −1. */
  grab: number;
  /** Reaching out with the grab button (nobody in hand). */
  reach: boolean;
  /** Bonus in effect (physics POWER). */
  power: number;
}

export interface ArenaHost {
  myId: number;
  sfx(s: MapSfx): void;
  decorate(id: number, deco: BeanDeco): void;
  send(data: Uint8Array<ArrayBuffer>): void;
  /** Server clock (ms). */
  serverNow(): number;
  rtt(): number;
}

/**
 * One map on the client: built with a view, the local bean predicted at 120 Hz from local input and
 * corrected from server snapshots (replaying unacknowledged inputs); other beans interpolated.
 */
export class ClientArena {
  readonly mod: MapModule;
  readonly builder: Builder;
  readonly spec: MapSpec;
  readonly scores = new Map<number, number>();
  readonly finished = new Set<number>();
  readonly out = new Set<number>();
  /** Local predicted body, while the local player is in play. */
  body: PlayerBody | null = null;
  predTick: number;
  private readonly history = new Map<number, InputFrame>();
  private readonly prevPos = new THREE.Vector3();
  private prevYaw = 0;
  /** Prediction error being smoothed away (render position = predicted + offset). */
  readonly offset = new THREE.Vector3();
  /** How far ahead of the server clock the local bean is predicted (see LeadControl). */
  private readonly leadCtl: LeadControl;
  /** Newest tick the server has used our input for. */
  private ackTick = Number.NEGATIVE_INFINITY;
  private readonly frames: Frame[] = [];
  private lastArrival = 0;
  private jitter = 8;
  /**
   * How old snapshots are when they arrive, by the server clock (ms): the one-way latency (and any
   * error of the clock estimate). Other beans are drawn that much further in the past.
   */
  private snapAge = 0;
  private snapAgeSet = false;
  /** Interpolation delay in use (ms), eased towards its target so render time never jumps. */
  private interpMs = SNAPSHOT_EVERY * TICK_MS * 2;
  private readonly bodies = new Map<number, PlayerBody>();
  /** Other beans from the last snapshot (with velocities), and the same extrapolated for one tick. */
  private othersSnap: OtherBody[] = [];
  private othersSnapTick = 0;
  private readonly othersNow: OtherBody[] = [];
  /** Local body events since the last read (sounds, effects). */
  events = { jumped: false, bounced: false, hit: false, knocked: false, landed: 0, dived: false, bumped: 0 };
  teleported = false;
  corrections = 0;
  /** Snapshots received for this arena. */
  snapshots = 0;
  /** Id of the bean the local player holds (from the server), or −1. */
  ownGrab = -1;
  /** The local grab button is held. */
  grabHeld = false;
  readonly bonuses: Bonuses | null;
  /** The map's static meshes (instanced, baked shadows): see statics.ts. */
  readonly statics: Statics;
  /** Called when somebody takes a bonus (sounds, notes). */
  onBonus: ((b: Bonus) => void) | null = null;

  constructor(
    readonly info: ArenaInfo,
    scene: THREE.Scene,
    private readonly host: ArenaHost,
  ) {
    const mod = getMap(info.game);
    if (!mod) throw new Error(`unknown map ${info.game}`);
    this.mod = mod;
    this.builder = new Builder(info.seed, new ClientView());
    this.builder.setLook(lookFor(mod.looks, info.seed));
    this.statics = new Statics(this.builder.group);
    for (const [id, v] of info.scores) this.scores.set(id, v);
    for (const id of info.finished) this.finished.add(id);
    for (const id of info.out) this.out.add(id);
    const ctx: MapCtx = {
      server: false,
      seed: info.seed,
      participants: info.participants,
      now: () => this.builder.world.t,
      emit: () => {},
      score: (id) => this.scores.get(id) ?? 0,
      setScore: () => {},
      bodies: () => this.bodies,
      sfx: (s) => host.sfx(s),
      me: () => host.myId,
      decorate: (id, d) => host.decorate(id, d),
    };
    this.spec = mod.build(this.builder, ctx);
    this.bonuses =
      info.kind === 'round'
        ? new Bonuses(this.builder, info.seed, { arena: !this.spec.finish, duration: mod.meta.duration })
        : null;
    const now = host.serverNow();
    this.predTick = Math.floor(this.tickAt(now)) - 1;
    this.builder.world.finalize(this.predTick * DT);
    // Server and client build the map separately: they must agree on the solid geometry.
    const hash = this.builder.world.hash(true);
    if (info.hash && hash !== info.hash)
      report('desync', `map ${info.game} (seed ${info.seed}) built differently: ${hash} vs server ${info.hash}`);
    placeScenery(this.builder);
    applySurfaces(this.builder.group);
    scene.add(this.builder.group);
    // What never moves: instanced, with a baked shadow (before the LOD system takes the rest).
    const world = this.builder.world;
    const t0 = this.predTick * DT;
    this.statics.prepare(
      world,
      (t) => {
        for (const m of world.movers) m(t);
        for (const a of this.builder.anims) a(t, 0);
      },
      () => {
        world.setTime(t0);
        for (const a of this.builder.anims) a(t0, 0);
      },
    );
    lod.register(this.builder.group);
    for (const [name, data] of info.events) this.onEvent(name, data, true);
    this.leadCtl = new LeadControl(host.rtt() / 2 + 30);
  }

  /** An authoritative event from the server (map events, bonuses); replay: it happened before we joined. */
  onEvent(name: string, data: unknown, replay = false) {
    if (name === 'portal') {
      const d = data as { pair: number; from: number; t: number };
      this.builder.portalUsed(d.pair, d.from, d.t);
      return;
    }
    if (name === BONUS_EVENT) {
      const b = this.bonuses?.onEvent(data);
      if (b && !replay) this.onBonus?.(b);
      return;
    }
    this.spec.onEvent?.(name, data);
  }

  /** How far ahead of the server clock the local bean is predicted (ms). */
  get inputLead() {
    return this.leadCtl.lead;
  }

  private get lead() {
    return this.leadCtl.lead;
  }

  get kind() {
    return this.info.kind;
  }

  tickAt(serverMs: number) {
    return (serverMs - this.info.startAt) / TICK_MS;
  }

  /** Spawn pose for the local player: the server sends the real one with the first snapshot. */
  spawn(index: number): { pos: THREE.Vector3; yaw: number } {
    const s = this.spec.spawns;
    const pos = (s[index % Math.max(1, s.length)] ?? new THREE.Vector3()).clone();
    const yaw = this.spec.faceCenter ? Math.atan2(-pos.x, -pos.z) : 0;
    return { pos, yaw };
  }

  /** Starts predicting the local player (they have a pawn on the server). */
  play(myId: number) {
    const b = new PlayerBody(myId);
    const i = Math.max(0, this.info.participants.indexOf(myId));
    const { pos, yaw } = this.spawn(this.info.kind === 'lobby' ? i : i);
    b.reset(pos, yaw);
    this.body = b;
    this.bodies.set(myId, b);
    this.prevPos.copy(pos);
    this.prevYaw = yaw;
  }

  stopPlaying() {
    if (this.body) this.bodies.delete(this.body.actor);
    this.body = null;
  }

  // ------------------------------------------------------------------ prediction

  private stepOwn(tick: number, input: InputFrame, others: readonly OtherBody[], replay: boolean) {
    const b = this.body!;
    const world = this.builder.world;
    const t = tick * DT;
    // Same rule as the server: nobody moves before the start.
    const f = canMove(this.info.kind, t) ? input : IDLE;
    if (!replay) {
      this.prevPos.copy(b.pos);
      this.prevYaw = b.yaw;
    }
    b.clearEvents();
    b.beforeWorldUpdate();
    world.setTime(t);
    b.afterWorldUpdate();
    const wasGrounded = b.grounded;
    const wasDive = b.state === 'dive';
    b.step(
      DT,
      { mx: f.mx / 127, mz: f.mz / 127, jump: (f.buttons & BTN.jump) !== 0, dive: (f.buttons & BTN.dive) !== 0 },
      world,
      t,
      others,
    );
    if (!replay) {
      if (b.jumped) this.events.jumped = true;
      if (b.bounced) this.events.bounced = true;
      if (b.stunned) this.events.hit = true;
      if (b.knocked) this.events.knocked = true;
      if (b.bumped > this.events.bumped) this.events.bumped = b.bumped;
      if (!wasDive && b.state === 'dive') this.events.dived = true;
      if (!wasGrounded && b.grounded && b.landImpact > 0.3) this.events.landed = Math.max(this.events.landed, b.landImpact);
      // Into a portal (the camera glides after the body to the other end: no cut).
      if (b.portalIn) this.events.bounced = true;
    }
  }

  /**
   * Advances prediction to the current input tick, sampling input per tick.
   * `sample` returns camera-relative input already converted to world space.
   */
  predict(sample: () => { mx: number; mz: number; jump: boolean; dive: boolean; grab: boolean }) {
    const now = this.host.serverNow();
    const target = Math.floor(this.tickAt(now + this.lead));
    if (!this.body) {
      // Spectating: the world simply follows server time.
      const t = Math.floor(this.tickAt(now));
      if (t > this.predTick) {
        this.predTick = t;
        this.builder.world.setTime(t * DT);
      }
      return;
    }
    // Catch up after a hitch (keys held meanwhile still count); only a very long stall skips ahead.
    if (target - this.predTick > 150) this.predTick = target - 1;
    while (this.predTick < target) {
      const k = ++this.predTick;
      const s = sample();
      this.grabHeld = s.grab;
      const f: InputFrame = {
        mx: quantizeAxis(s.mx),
        mz: quantizeAxis(s.mz),
        buttons: (s.jump ? BTN.jump : 0) | (s.dive ? BTN.dive : 0) | (s.grab ? BTN.grab : 0),
      };
      this.history.set(k, f);
      this.history.delete(k - MAX_HISTORY);
      this.stepOwn(k, f, this.othersAt(k), false);
      if (k % INPUT_EVERY === 0) this.sendInputs(k);
    }
  }

  /** Where the other beans probably are at tick k: the last snapshot carried forward a little. */
  private othersAt(k: number): OtherBody[] {
    const ahead = Math.min(0.2, Math.max(0, (k - this.othersSnapTick) * DT));
    const out = this.othersNow;
    out.length = this.othersSnap.length;
    this.othersSnap.forEach((o, i) => {
      const e = out[i] ?? { id: 0, x: 0, y: 0, z: 0, vx: 0, vz: 0, touching: false };
      e.id = o.id;
      e.x = o.x + o.vx * ahead;
      e.y = o.y;
      e.z = o.z + o.vz * ahead;
      e.vx = o.vx;
      e.vz = o.vz;
      e.touching = false;
      out[i] = e;
    });
    return out;
  }

  /**
   * Every input the server has not acknowledged yet (at least the last INPUT_REDUNDANCY, at most a
   * second): packets lost in a burst are made up by the next one that arrives.
   */
  private sendInputs(upTo: number) {
    const unacked = Number.isFinite(this.ackTick) ? this.ackTick + 1 : upTo;
    const first = Math.max(upTo - MAX_INPUT_FRAMES + 1, Math.min(upTo - INPUT_REDUNDANCY + 1, unacked));
    const frames: InputFrame[] = [];
    for (let k = first; k <= upTo; k++) frames.push(this.history.get(k) ?? IDLE);
    this.host.send(encodeInput({ arena: this.info.id, firstTick: first, frames }));
  }

  // ------------------------------------------------------------------ snapshots

  onSnapshot(s: Snapshot) {
    if (s.arena !== this.info.id) return;
    this.snapshots++;
    const arrival = performance.now();
    if (this.lastArrival) {
      const expected = SNAPSHOT_EVERY * TICK_MS;
      this.jitter += (Math.abs(arrival - this.lastArrival - expected) - this.jitter) * 0.1;
    }
    this.lastArrival = arrival;
    // Age at arrival: fast to rise (the delay must cover it), slow to fall.
    const age = this.host.serverNow() - (this.info.startAt + s.tick * TICK_MS);
    if (!this.snapAgeSet) {
      this.snapAge = age;
      this.snapAgeSet = true;
    } else this.snapAge += (age - this.snapAge) * (age > this.snapAge ? 0.25 : 0.02);
    const bodies = new Map(s.bodies.map((b) => [b.id, b]));
    if (!this.frames.length || s.tick > this.frames.at(-1)!.tick) this.frames.push({ tick: s.tick, bodies });
    while (this.frames.length > 40) this.frames.shift();
    const prev = new Map(this.othersSnap.map((o) => [o.id, o]));
    const span = Math.max(1, s.tick - this.othersSnapTick) * DT;
    // (Beans inside a portal are nowhere to bump into.)
    this.othersSnap = s.bodies
      .filter((b) => b.anim !== ANIM.portal)
      .map((b) => {
        const p = prev.get(b.id);
        const vx = p ? (b.x - p.x) / span : 0;
        const vz = p ? (b.z - p.z) / span : 0;
        const ok = Math.hypot(vx, vz) < 30;
        return { id: b.id, x: b.x, y: b.y, z: b.z, vx: ok ? vx : 0, vz: ok ? vz : 0, touching: false };
      });
    this.othersSnapTick = s.tick;
    if (s.own && this.body) {
      this.ownGrab = s.own.grab;
      this.ackTick = Math.max(this.ackTick, s.own.ack);
      // Only while prediction runs ahead: in a hidden tab (no frames) snapshots keep coming, and
      // the lead would chase inputs nobody sends.
      if (this.predTick > s.tick) this.leadCtl.update(s.own.margin, this.host.rtt(), arrival);
      this.reconcile(s.tick, s.own.s);
    }
  }

  private reconcile(tick: number, st: BodyFullState) {
    const b = this.body!;
    const world = this.builder.world;
    if (tick >= this.predTick) {
      b.fromFull(st, world);
      this.predTick = tick;
      world.setTime(tick * DT);
      this.offset.set(0, 0, 0);
      this.prevPos.copy(b.pos);
      this.teleported ||= st.teleport;
      return;
    }
    const before = b.pos.clone();
    b.fromFull(st, world);
    world.setTime((tick - 1) * DT);
    world.setTime(tick * DT);
    for (let k = tick + 1; k <= this.predTick; k++) this.stepOwn(k, this.history.get(k) ?? IDLE, this.othersAt(k), true);
    for (const k of this.history.keys()) if (k <= tick - 30) this.history.delete(k);
    const err = before.sub(b.pos);
    if (st.teleport || err.length() > 3) {
      this.offset.set(0, 0, 0);
      this.prevPos.copy(b.pos);
      if (st.teleport) this.teleported = true;
    } else {
      if (err.lengthSq() > 1e-6) this.corrections++;
      this.offset.add(err);
    }
  }

  // ------------------------------------------------------------------ rendering

  /** Render time of the local prediction (for movers and effects). */
  renderTick(): number {
    const now = this.host.serverNow();
    if (!this.body) return this.tickAt(now);
    const frac = Math.min(1, Math.max(0, this.tickAt(now + this.lead) - this.predTick));
    return this.predTick - 1 + frac;
  }

  /** Local bean pose, interpolated between the last two predicted ticks, with error smoothing. */
  ownPose(dt: number, out: THREE.Vector3): number {
    const b = this.body!;
    const alpha = THREE.MathUtils.clamp(this.renderTick() - (this.predTick - 1), 0, 1);
    this.offset.multiplyScalar(Math.exp(-dt * 12));
    out.lerpVectors(this.prevPos, b.pos, alpha).add(this.offset);
    let dy = b.yaw - this.prevYaw;
    dy = Math.atan2(Math.sin(dy), Math.cos(dy));
    return this.prevYaw + dy * alpha;
  }

  /**
   * Interpolation delay for other beans: how old snapshots are on arrival, plus two snapshot
   * intervals (one may be lost) and the measured jitter. (Without the age, beyond ~60 ms of ping
   * render time ran past the newest snapshot and other beans moved in 30 Hz jerks.)
   */
  private interpDelayTicks() {
    return this.interpMs / TICK_MS;
  }

  /** Eases the interpolation delay towards its target (once per rendered frame). */
  private updateInterpDelay(dt: number) {
    const target = Math.min(400, Math.max(0, this.snapAge) + SNAPSHOT_EVERY * TICK_MS * 2 + Math.min(this.jitter, 60) * 2);
    // Up quickly (starving), down gently (render time slows a little instead of jumping).
    const k = target > this.interpMs ? 1 - Math.exp(-dt * 8) : 1 - Math.exp(-dt * 0.5);
    this.interpMs += (target - this.interpMs) * k;
  }

  remotePoses(dt = 0): Map<number, RemotePose> {
    const out = new Map<number, RemotePose>();
    if (!this.frames.length) return out;
    this.updateInterpDelay(dt);
    const r = this.tickAt(this.host.serverNow()) - this.interpDelayTicks();
    let i = this.frames.length - 1;
    while (i > 0 && this.frames[i]!.tick > r) i--;
    const a = this.frames[i]!;
    const b = this.frames[i + 1];
    for (const [id, sa] of a.bodies) {
      const sb = b?.bodies.get(id);
      let pos: THREE.Vector3;
      let yaw = sa.yaw;
      let anim = sa.anim;
      let tilt = sa.tilt;
      let tiltDir = sa.tiltDir;
      let grab = sa.grab;
      let flags = sa.flags;
      // A jump of metres between two snapshots (a portal, a respawn): no sliding across it.
      const jump = sb ? Math.hypot(sb.x - sa.x, sb.y - sa.y, sb.z - sa.z) > 4 : false;
      if (sb && b && jump) {
        const late = (r - a.tick) / (b.tick - a.tick) > 0.5;
        const s = late ? sb : sa;
        pos = new THREE.Vector3(s.x, s.y, s.z);
        yaw = s.yaw;
        anim = s.anim;
        tilt = s.tilt;
        tiltDir = s.tiltDir;
        grab = s.grab;
        flags = s.flags;
      } else if (sb && b) {
        const f = THREE.MathUtils.clamp((r - a.tick) / (b.tick - a.tick), 0, 1);
        pos = new THREE.Vector3(sa.x + (sb.x - sa.x) * f, sa.y + (sb.y - sa.y) * f, sa.z + (sb.z - sa.z) * f);
        let dy = sb.yaw - sa.yaw;
        dy = Math.atan2(Math.sin(dy), Math.cos(dy));
        yaw = sa.yaw + dy * f;
        tilt = sa.tilt + (sb.tilt - sa.tilt) * f;
        let dd = sb.tiltDir - sa.tiltDir;
        dd = Math.atan2(Math.sin(dd), Math.cos(dd));
        tiltDir = sa.tiltDir + dd * f;
        if (f > 0.5) {
          anim = sb.anim;
          grab = sb.grab;
          flags = sb.flags;
        }
      } else if (!b && r > a.tick && i > 0) {
        // Past the newest snapshot (a late or lost one): carry on along the last motion for a moment.
        const sp = this.frames[i - 1]!.bodies.get(id);
        const span = (a.tick - this.frames[i - 1]!.tick) * DT;
        if (sp && span > 0 && Math.hypot(sa.x - sp.x, sa.z - sp.z) < 4) {
          const k = Math.min(MAX_EXTRAPOLATE, (r - a.tick) * DT) / span;
          pos = new THREE.Vector3(sa.x + (sa.x - sp.x) * k, sa.y, sa.z + (sa.z - sp.z) * k);
        } else pos = new THREE.Vector3(sa.x, sa.y, sa.z);
      } else pos = new THREE.Vector3(sa.x, sa.y, sa.z);
      out.set(id, {
        pos,
        yaw,
        anim,
        tilt,
        tiltDir,
        grab,
        reach: (flags & REMOTE_FLAG.reach) !== 0,
        power: (flags >> POWER_SHIFT) & 3,
      });
    }
    // Newest frame for beans that just appeared.
    const last = this.frames.at(-1)!;
    for (const [id, s] of last.bodies)
      if (!out.has(id))
        out.set(id, {
          pos: new THREE.Vector3(s.x, s.y, s.z),
          yaw: s.yaw,
          anim: s.anim,
          tilt: s.tilt,
          tiltDir: s.tiltDir,
          grab: s.grab,
          reach: (s.flags & REMOTE_FLAG.reach) !== 0,
          power: (s.flags >> POWER_SHIFT) & 3,
        });
    return out;
  }

  /** Positions movers and runs visual animations for render time `t` (seconds). */
  animate(t: number, dt: number) {
    const world = this.builder.world;
    // Movers for the smooth rendered time; collision matrices keep the last simulated tick.
    for (const m of world.movers) m(t);
    for (const a of this.builder.anims) a(t, dt);
    timeUniform.value = t;
  }

  ownAnim(): number {
    const b = this.body;
    if (!b) return ANIM.idle;
    if (b.state === 'tumble') return ANIM.tumble;
    if (b.state === 'getup') return ANIM.getup;
    if (b.state === 'stun') return ANIM.stun;
    if (b.state === 'dive') return ANIM.dive;
    if (b.state === 'slide') return ANIM.slide;
    if (b.state === 'climb') return b.climbingOver ? ANIM.climbOver : ANIM.climb;
    if (b.state === 'portal') return ANIM.portal;
    if (b.state === 'ladder') return ANIM.ladder;
    if (!b.grounded) return ANIM.air;
    if (this.ownGrab >= 0) return ANIM.grab;
    return this.grabHeld && b.state === 'normal' ? ANIM.reach : ANIM.idle;
  }

  /** Prediction and interpolation numbers for the debug probe and overlay. */
  netStats() {
    const now = this.host.serverNow();
    const last = this.frames.at(-1);
    return {
      serverTick: Math.floor(this.tickAt(now)),
      predTick: this.predTick,
      leadMs: this.lead,
      leadTicks: this.lead / TICK_MS,
      jitterMs: this.jitter,
      interpDelayMs: this.interpDelayTicks() * TICK_MS,
      snapshotAgeAtArrivalMs: this.snapAge,
      /** Worst input margin (ticks) the server reported over the last ~2 s. */
      inputMargin: this.leadCtl.worst,
      ackTick: Number.isFinite(this.ackTick) ? this.ackTick : -1,
      snapshotTick: last?.tick ?? -1,
      /** How old the newest snapshot is, in ms of server time. */
      snapshotAgeMs: last ? now - (this.info.startAt + last.tick * TICK_MS) : -1,
      snapshots: this.snapshots,
      buffered: this.frames.length,
      history: this.history.size,
      corrections: this.corrections,
      offset: this.offset.length(),
    };
  }

  takeEvents() {
    const e = this.events;
    this.events = { jumped: false, bounced: false, hit: false, knocked: false, landed: 0, dived: false, bumped: 0 };
    return e;
  }

  dispose() {
    this.statics.dispose();
    lod.drop(this.builder.group);
    this.builder.dispose();
  }
}
