import * as THREE from 'three';
import type { BodyFullState } from '../shared/codec';

const _l = new THREE.Vector3();
const _q = new THREE.Vector3();
const _n = new THREE.Vector3();
const _c = new THREE.Vector3();
const _p = new THREE.Vector3();
const _w = new THREE.Vector3();
const _sv = new THREE.Vector3();
const _dir = new THREE.Vector3();
const _ro = new THREE.Vector3();
const _rd = new THREE.Vector3();

/** The bean is two spheres of radius R, at these heights above its feet. */
export const R = 0.5;
export const SPHERES = [0.5, 1.1] as const;
export const HEIGHT = 1.6;
export const GRAVITY = 28;
export const RUN_SPEED = 8.5;
export const JUMP_V = 10.5;
export const DIVE_SPEED = 12.5;

/**
 * Bonuses picked up on the course: giant (bigger and four times as heavy), mega jump, speed.
 * A body has at most one at a time; `powerUntil` is the sim time it wears off.
 */
export const POWER = { none: 0, giant: 1, jump: 2, speed: 3 } as const;
/** How long each bonus lasts (s), by kind. */
export const POWER_TIME: readonly number[] = [0, 9, 10, 8];
/** Size of a giant (×) and its mass (a bean weighs 1). */
export const GIANT_SIZE = 1.8;
export const GIANT_MASS = 4;
const MEGA_JUMP = 1.5;
const SPEED_UP = 1.4;

export type Shape =
  | { type: 'box'; hx: number; hy: number; hz: number }
  | { type: 'cyl'; r: number; hh: number }
  | { type: 'sphere'; r: number };

export interface ColliderOpts {
  isStatic?: boolean;
  bounce?: number;
  hit?: number;
  pad?: number;
  conveyor?: THREE.Vector3 | null;
  /** Hazard name for knockout credit ('hammer', 'rotor', …); touching it from the side is remembered. */
  tag?: string;
  /** Ice: 0 = normal grip, 1 = almost none (and beans slide down slopes). */
  slip?: number;
  onTouch?: ((col: Collider, n: THREE.Vector3, body: PlayerBody) => void) | null;
  onGround?: ((col: Collider, body: PlayerBody) => void) | null;
  /** Left out of bot navigation (the bots' own logic deals with it, e.g. doors that may break). */
  navSkip?: boolean;
  /** Meant to slide into other geometry (retracting walls, sinking gates): the clipping audit skips it. */
  sinks?: boolean;
  /**
   * A sweeping arm (rotors, barriers): a hit always knocks the bean over, and a bean that is down
   * passes under it instead of being dragged along.
   */
  sweep?: boolean;
  /** Not solid: only reports touches (onTouch), e.g. portals, fake glass panes. */
  trigger?: boolean;
  /** A ladder (a trigger in front of a wall, facing its local +z): walk into it to climb up. */
  ladder?: boolean;
  /** A pad throws the bean this way too (horizontal velocity, m/s), not only up. */
  launch?: THREE.Vector3 | null;
}

export interface Contact {
  local: THREE.Vector3;
  point: THREE.Vector3;
  normal: THREE.Vector3;
  depth: number;
}

export class Collider {
  enabled = true;
  /** Position in the world's collider list; identical on server and clients (same build). */
  index = -1;
  /** Query de-duplication mark (see World.query). */
  stamp = 0;
  readonly isStatic: boolean;
  bounce: number;
  hit: number;
  pad: number;
  conveyor: THREE.Vector3 | null;
  tag: string | null;
  slip: number;
  onTouch: ColliderOpts['onTouch'];
  onGround: ColliderOpts['onGround'];
  readonly navSkip: boolean;
  readonly sinks: boolean;
  readonly sweep: boolean;
  readonly trigger: boolean;
  readonly ladder: boolean;
  launch: THREE.Vector3 | null;
  readonly cur = new THREE.Matrix4();
  readonly prev = new THREE.Matrix4();
  readonly inv = new THREE.Matrix4();
  readonly center = new THREE.Vector3();
  readonly radius: number;
  private synced = false;

  constructor(
    readonly obj: THREE.Object3D,
    readonly shape: Shape,
    opts: ColliderOpts = {},
  ) {
    this.isStatic = !!opts.isStatic;
    this.bounce = opts.bounce ?? 0;
    this.hit = opts.hit ?? 0;
    this.pad = opts.pad ?? 0;
    this.conveyor = opts.conveyor ?? null;
    this.tag = opts.tag ?? null;
    this.slip = opts.slip ?? 0;
    this.onTouch = opts.onTouch ?? null;
    this.onGround = opts.onGround ?? null;
    this.navSkip = !!opts.navSkip;
    this.sinks = !!opts.sinks;
    this.sweep = !!opts.sweep;
    this.trigger = !!opts.trigger || !!opts.ladder;
    this.ladder = !!opts.ladder;
    this.launch = opts.launch ?? null;
    this.radius =
      shape.type === 'box'
        ? Math.hypot(shape.hx, shape.hy, shape.hz)
        : shape.type === 'cyl'
          ? Math.hypot(shape.r, shape.hh)
          : shape.r;
  }

  /** Reads the object's world matrix; the previous one is kept for surface velocity. */
  sync() {
    if (this.isStatic && this.synced) {
      this.prev.copy(this.cur);
      return;
    }
    this.obj.updateWorldMatrix(true, false);
    if (!this.synced) this.prev.copy(this.obj.matrixWorld);
    else this.prev.copy(this.cur);
    this.cur.copy(this.obj.matrixWorld);
    this.inv.copy(this.cur).invert();
    this.center.setFromMatrixPosition(this.cur);
    this.synced = true;
  }

  /** World-space AABB half extents in x and z (for the static grid). */
  extentXZ(): [number, number] {
    const e = this.cur.elements;
    const s = this.shape;
    const [hx, hy, hz] = s.type === 'box' ? [s.hx, s.hy, s.hz] : s.type === 'cyl' ? [s.r, s.hh, s.r] : [s.r, s.r, s.r];
    const ex = Math.abs(e[0]!) * hx + Math.abs(e[4]!) * hy + Math.abs(e[8]!) * hz;
    const ez = Math.abs(e[2]!) * hx + Math.abs(e[6]!) * hy + Math.abs(e[10]!) * hz;
    return [ex, ez];
  }

  contact(center: THREE.Vector3, r: number, out: Contact): boolean {
    if (center.distanceToSquared(this.center) > (this.radius + r) ** 2) return false;
    _l.copy(center).applyMatrix4(this.inv);
    const s = this.shape;
    let depth: number;
    if (s.type === 'box') {
      _q.set(
        THREE.MathUtils.clamp(_l.x, -s.hx, s.hx),
        THREE.MathUtils.clamp(_l.y, -s.hy, s.hy),
        THREE.MathUtils.clamp(_l.z, -s.hz, s.hz),
      );
      _n.subVectors(_l, _q);
      const d = _n.length();
      if (d > 1e-6) {
        if (d >= r) return false;
        _n.divideScalar(d);
        depth = r - d;
      } else {
        const dx = s.hx - Math.abs(_l.x);
        const dy = s.hy - Math.abs(_l.y);
        const dz = s.hz - Math.abs(_l.z);
        _n.set(0, 0, 0);
        if (dy <= dx && dy <= dz) {
          _n.y = Math.sign(_l.y) || 1;
          _q.y = _n.y * s.hy;
          depth = dy + r;
        } else if (dx <= dz) {
          _n.x = Math.sign(_l.x) || 1;
          _q.x = _n.x * s.hx;
          depth = dx + r;
        } else {
          _n.z = Math.sign(_l.z) || 1;
          _q.z = _n.z * s.hz;
          depth = dz + r;
        }
      }
    } else if (s.type === 'cyl') {
      const rl = Math.hypot(_l.x, _l.z);
      const inR = rl <= s.r;
      const inY = Math.abs(_l.y) <= s.hh;
      if (inR && inY) {
        const side = s.r - rl;
        const top = s.hh - _l.y;
        const bot = s.hh + _l.y;
        _q.copy(_l);
        if (top <= side && top <= bot) {
          _n.set(0, 1, 0);
          _q.y = s.hh;
          depth = top + r;
        } else if (bot <= side) {
          _n.set(0, -1, 0);
          _q.y = -s.hh;
          depth = bot + r;
        } else {
          if (rl < 1e-6) _n.set(1, 0, 0);
          else _n.set(_l.x / rl, 0, _l.z / rl);
          _q.set(_n.x * s.r, _l.y, _n.z * s.r);
          depth = side + r;
        }
      } else {
        const k = inR ? 1 : s.r / rl;
        _q.set(_l.x * k, THREE.MathUtils.clamp(_l.y, -s.hh, s.hh), _l.z * k);
        _n.subVectors(_l, _q);
        const d = _n.length();
        if (d >= r || d < 1e-9) return false;
        _n.divideScalar(d);
        depth = r - d;
      }
    } else {
      const d = _l.length();
      if (d >= s.r + r) return false;
      if (d < 1e-6) _n.set(0, 1, 0);
      else _n.copy(_l).divideScalar(d);
      _q.copy(_n).multiplyScalar(s.r);
      depth = s.r + r - d;
    }
    out.local.copy(_q);
    out.point.copy(_q).applyMatrix4(this.cur);
    out.normal.copy(_n).transformDirection(this.cur);
    out.depth = depth;
    return true;
  }

  /**
   * Distance along a ray (unit `dir`) to where it enters the shape, or −1 (no hit within `maxT`, or
   * the ray starts inside). `normal` gets the world surface normal there.
   */
  raycast(origin: THREE.Vector3, dir: THREE.Vector3, maxT: number, normal: THREE.Vector3): number {
    const o = _ro.copy(origin).applyMatrix4(this.inv);
    const d = _rd.copy(dir).transformDirection(this.inv);
    const s = this.shape;
    let t = -1;
    if (s.type === 'box') {
      const h = [s.hx, s.hy, s.hz];
      const oa = [o.x, o.y, o.z];
      const da = [d.x, d.y, d.z];
      let t0 = Number.NEGATIVE_INFINITY;
      let t1 = Number.POSITIVE_INFINITY;
      let axis = -1;
      for (let a = 0; a < 3; a++) {
        const ha = h[a]!;
        const o1 = oa[a]!;
        const d1 = da[a]!;
        if (Math.abs(d1) < 1e-9) {
          if (Math.abs(o1) > ha) return -1;
          continue;
        }
        let ta = (-ha - o1) / d1;
        let tb = (ha - o1) / d1;
        if (ta > tb) [ta, tb] = [tb, ta];
        if (ta > t0) {
          t0 = ta;
          axis = a;
        }
        t1 = Math.min(t1, tb);
        if (t0 > t1) return -1;
      }
      if (axis < 0 || t0 < 0 || t0 > maxT) return -1;
      t = t0;
      normal.set(0, 0, 0).setComponent(axis, -Math.sign(da[axis]!));
    } else if (s.type === 'cyl') {
      let best = Number.POSITIVE_INFINITY;
      if (Math.hypot(o.x, o.z) <= s.r && Math.abs(o.y) <= s.hh) return -1;
      // Caps.
      if (Math.abs(d.y) > 1e-9)
        for (const cy of [s.hh, -s.hh]) {
          const tc = (cy - o.y) / d.y;
          if (tc < 0 || tc >= best) continue;
          if (Math.hypot(o.x + d.x * tc, o.z + d.z * tc) <= s.r) {
            best = tc;
            normal.set(0, Math.sign(cy), 0);
          }
        }
      // Side.
      const a = d.x * d.x + d.z * d.z;
      if (a > 1e-12) {
        const b = o.x * d.x + o.z * d.z;
        const c = o.x * o.x + o.z * o.z - s.r * s.r;
        const disc = b * b - a * c;
        if (disc >= 0) {
          const ts = (-b - Math.sqrt(disc)) / a;
          if (ts >= 0 && ts < best && Math.abs(o.y + d.y * ts) <= s.hh) {
            best = ts;
            normal.set((o.x + d.x * ts) / s.r, 0, (o.z + d.z * ts) / s.r);
          }
        }
      }
      if (best > maxT) return -1;
      t = best;
    } else {
      const b = o.dot(d);
      const c = o.lengthSq() - s.r * s.r;
      if (c <= 0) return -1;
      const disc = b * b - c;
      if (disc < 0) return -1;
      t = -b - Math.sqrt(disc);
      if (t < 0 || t > maxT) return -1;
      normal.copy(d).multiplyScalar(t).add(o).divideScalar(s.r);
    }
    normal.transformDirection(this.cur);
    return t;
  }

  surfaceVelocity(local: THREE.Vector3, dt: number, out: THREE.Vector3) {
    _p.copy(local).applyMatrix4(this.cur);
    _w.copy(local).applyMatrix4(this.prev);
    return out.subVectors(_p, _w).divideScalar(Math.max(dt, 1e-4));
  }
}

/** Uniform 2D (x/z) grid of static colliders; moving colliders are checked separately. */
export class ColliderGrid {
  private readonly cells = new Map<number, Collider[]>();
  constructor(readonly cell = 4) {}

  private key(ix: number, iz: number) {
    return (ix + 32768) * 65536 + (iz + 32768);
  }

  insert(col: Collider) {
    const [ex, ez] = col.extentXZ();
    const c = col.center;
    const x0 = Math.floor((c.x - ex) / this.cell);
    const x1 = Math.floor((c.x + ex) / this.cell);
    const z0 = Math.floor((c.z - ez) / this.cell);
    const z1 = Math.floor((c.z + ez) / this.cell);
    for (let ix = x0; ix <= x1; ix++)
      for (let iz = z0; iz <= z1; iz++) {
        const k = this.key(ix, iz);
        const list = this.cells.get(k);
        if (list) list.push(col);
        else this.cells.set(k, [col]);
      }
  }

  query(x: number, z: number, r: number, stamp: number, out: Collider[]) {
    const x0 = Math.floor((x - r) / this.cell);
    const x1 = Math.floor((x + r) / this.cell);
    const z0 = Math.floor((z - r) / this.cell);
    const z1 = Math.floor((z + r) / this.cell);
    for (let ix = x0; ix <= x1; ix++)
      for (let iz = z0; iz <= z1; iz++) {
        const list = this.cells.get(this.key(ix, iz));
        if (!list) continue;
        for (const c of list) {
          if (c.stamp === stamp) continue;
          c.stamp = stamp;
          out.push(c);
        }
      }
  }

  get size() {
    return this.cells.size;
  }
}

/** What a body collides with: the world's colliders near a point. */
export interface CollisionWorld {
  readonly colliders: readonly Collider[];
  query(x: number, z: number, r: number, out: Collider[]): Collider[];
}

export interface BodyInput {
  mx: number;
  mz: number;
  jump: boolean;
  dive: boolean;
}

export interface OtherBody {
  id: number;
  x: number;
  y: number;
  z: number;
  /** Horizontal velocity (bumps exchange momentum). */
  vx: number;
  vz: number;
  touching: boolean;
  /** Size (1, or GIANT_SIZE for a giant). */
  size?: number;
}

/**
 * normal · stun (short daze) · dive → slide (belly slide) · tumble (knocked over: the body tips over,
 * slides and rolls with little control) → getup · climb (caught a ledge in the air, pulling up onto it) ·
 * portal (inside a portal: out of sight, travelling to the other end, touching nothing) · ladder (on a
 * ladder: up and down with the stick, off with a jump, over the top onto what it leans on).
 */
export type BodyState = 'normal' | 'stun' | 'dive' | 'slide' | 'tumble' | 'getup' | 'climb' | 'portal' | 'ladder';
export const BODY_STATES: readonly BodyState[] = [
  'normal',
  'stun',
  'dive',
  'slide',
  'tumble',
  'getup',
  'climb',
  'portal',
  'ladder',
];

/** Seconds a trip through a portal takes. */
export const PORTAL_T = 0.5;

/** Lying down: the tilt a tumbling body settles at (a little under 90°). */
const LIE = 1.4;
/** Diving, the body lies along its flight (between these tilts); sliding, flat on the belly. */
const DIVE_TILT_MIN = 0.95;
const DIVE_TILT_MAX = 1.75;
const SLIDE_TILT = 1.45;
/** A sweeping arm catching a bean that is already down tosses it up and over itself (m/s). */
const SCOOP_V = 7;
/** Ledges: a top this high above the feet (× size) can be caught in the air, then climbed. */
const LEDGE_MIN = 0.5;
const LEDGE_MAX = 1.75;
/** Climbing: a moment hanging, then up (m/s) and over the edge (m/s); given up after CLIMB_T. */
const CLIMB_HANG = 0.08;
const CLIMB_UP = 5.5;
const CLIMB_OVER = 4.5;
const CLIMB_T = 1.2;
/** Ladders: climbing speed (m/s), how far the body keeps from the rungs' plane, and the leap off (m/s back). */
const LADDER_SPEED = 4.2;
const LADDER_GAP = 0.17;
const LADDER_LEAP = 4;
/** After leaping off a ladder, this long (s) before one is caught again. */
const LADDER_AGAIN = 0.35;
/** Distance between the two collision spheres. */
const SPINE = SPHERES[1] - SPHERES[0];
/** Closest two beans get (centre to centre, horizontally). */
export const BEAN_GAP = 1.05;
/** Bounciness of bean-on-bean bumps and of a tumbling bean hitting things. */
const BUMP_E = 0.45;
const TUMBLE_E = 0.4;
const AIR_ACCEL = 24;
/** Largest move per collision substep, as a share of the bean's radius (see PlayerBody.step). */
const SUBSTEP_REACH = 0.6;
const MAX_SUBSTEPS = 8;
/** Moving ground tipping under the feet (a drum): firm up to STEEP_FROM (n.y), then the feet slip downhill, up to STEEP_SLIP m/s. */
const STEEP_FROM = 0.77;
const STEEP_SLIP = 7;

const _gv = new THREE.Vector3();
const _gv2 = new THREE.Vector3();
const hitInfo: Contact = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
const nearby: Collider[] = [];
const probe: Collider[] = [];
const groundN = new THREE.Vector3();
const wallN = new THREE.Vector3();
const _rn = new THREE.Vector3();
const DOWN = new THREE.Vector3(0, -1, 0);

/** Centre of collision sphere i of a body with its feet at `pos`, tipped by `tilt` towards `tiltDir`. */
function sphereAt(pos: THREE.Vector3, tilt: number, tiltDir: number, size: number, i: number, out: THREE.Vector3) {
  out.set(pos.x, pos.y + SPHERES[0] * size, pos.z);
  if (i === 0) return out;
  const s = Math.sin(tilt) * SPINE * size;
  return out.set(out.x + Math.sin(tiltDir) * s, out.y + Math.cos(tilt) * SPINE * size, out.z + Math.cos(tiltDir) * s);
}

const drawn: Collider[] = [];
const _dc = new THREE.Vector3();
const drawnHit: Contact = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };

/**
 * Moves where a body is drawn (feet at `pos`) out of solid colliders, as its simulation would. Other
 * beans are drawn a little in the past and the world in the present: without this they sink into
 * walls coming at them.
 */
export function pushOut(world: CollisionWorld, pos: THREE.Vector3, tilt = 0, tiltDir = 0, size = 1) {
  const r = R * size;
  const cols = world.query(pos.x, pos.z, r + 1.2 * size + (tilt > 0 ? SPINE * size : 0), drawn);
  for (let iter = 0; iter < 2; iter++) {
    let any = false;
    for (const col of cols) {
      if (!col.enabled || col.trigger) continue;
      for (let si = 0; si < 2; si++) {
        if (!col.contact(sphereAt(pos, tilt, tiltDir, size, si, _dc), r, drawnHit)) continue;
        pos.addScaledVector(drawnHit.normal, drawnHit.depth);
        any = true;
      }
    }
    if (!any) break;
  }
  return pos;
}

/** Solid ground a hand can hold on to: no hazards, pads, bumpers, ice or triggers. */
function holdable(c: Collider) {
  return c.enabled && c.isStatic && !c.trigger && !c.hit && !c.tag && !c.sweep && !c.bounce && !c.pad && c.slip < 0.5;
}

/** A moving platform the body rides on (its motion carries over); not a hazard like a rotor arm or a hammer. */
function rides(c: Collider | null): c is Collider {
  return !!c && !c.isStatic && c.enabled && !c.hit && !c.tag;
}

export class PlayerBody {
  readonly pos = new THREE.Vector3();
  readonly vel = new THREE.Vector3();
  yaw = 0;
  grounded = false;
  groundCol: Collider | null = null;
  private carryCol: Collider | null = null;
  private readonly groundLocal = new THREE.Vector3();
  private hasGroundLocal = false;
  state: BodyState = 'normal';
  stateT = 0;
  coyote = 0;
  jumpBuf = 0;
  /** Sim time (s) until which the body is slowed (grabbing or being grabbed), and by how much. */
  slowUntil = -1e9;
  slowK = 1;
  landImpact = 0;
  /** Tumbling: how far the body is tipped over (0 upright … LIE lying) and towards which yaw. */
  tilt = 0;
  tiltDir = 0;
  // Events of the last step, for sounds and effects.
  jumped = false;
  bounced = false;
  hitSomething = false;
  stunned = false;
  knocked = false;
  /** Bumped into another bean this step (relative speed, 0 if not). */
  bumped = 0;
  /** Tag of a hazard touched from the side during the last step. */
  hazard: string | null = null;
  /** Stepped into a portal this step. */
  portalIn = false;
  /** Came out of a portal this step. */
  portalOut = false;
  /** Bonus in effect (POWER) and the sim time it wears off. */
  power: number = POWER.none;
  powerUntil = -1e9;
  /** Current size (1, or GIANT_SIZE while a giant); follows the bonus at every step. */
  size = 1;
  /** Climbing: where the feet end up on the ledge. */
  readonly climbTo = new THREE.Vector3();

  constructor(readonly actor: number) {}

  reset(p: THREE.Vector3, yaw = 0) {
    this.pos.copy(p);
    this.vel.set(0, 0, 0);
    this.yaw = yaw;
    this.state = 'normal';
    this.stateT = 0;
    this.grounded = false;
    this.groundCol = null;
    this.carryCol = null;
    this.hasGroundLocal = false;
    this.coyote = 0;
    this.jumpBuf = 0;
    this.slowUntil = -1e9;
    this.slowK = 1;
    this.landImpact = 0;
    this.tilt = 0;
    this.tiltDir = 0;
    this.power = POWER.none;
    this.powerUntil = -1e9;
    this.size = 1;
    this.climbTo.set(0, 0, 0);
  }

  get down() {
    return this.state === 'tumble' || this.state === 'getup';
  }

  get mass() {
    return this.size > 1 ? GIANT_MASS : 1;
  }

  /** Climbing and already up at the edge (going over it), past hanging on and pulling up. */
  get climbingOver() {
    return this.state === 'climb' && this.stateT <= CLIMB_T - CLIMB_HANG && this.pos.y >= this.climbTo.y - 1e-4;
  }

  /** Gives a bonus from sim time t. */
  givePower(kind: number, t: number) {
    this.power = kind;
    this.powerUntil = t + (POWER_TIME[kind] ?? 0);
  }

  stun(t = 1.1) {
    if (this.down) return;
    this.state = 'stun';
    this.stateT = t;
  }

  /**
   * Knocked over: tips over in the direction of (vx, vz) and tumbles for about `t` seconds. A giant
   * shrugs most of it off (a small push and a daze), unless `force` (a sweeping arm fells anyone).
   */
  knock(vx: number, vz: number, vy: number, t = 1, force = false) {
    if (this.size > 1) {
      const k = force ? 0.5 : 0.25;
      vx *= k;
      vz *= k;
      vy *= force ? 0.7 : 0.4;
      if (!force) {
        this.vel.x += vx;
        this.vel.z += vz;
        this.vel.y = Math.max(this.vel.y, vy);
        this.stun(0.35);
        return;
      }
    }
    if (force) {
      // Caught by a sweeping arm: its shove replaces whatever the bean was doing.
      this.vel.x = vx;
      this.vel.z = vz;
    } else {
      this.vel.x += vx;
      this.vel.z += vz;
    }
    this.vel.y = Math.max(this.vel.y, vy);
    this.grounded = false;
    if (Math.hypot(vx, vz) > 0.1) this.tiltDir = Math.atan2(vx, vz);
    const was = this.state === 'tumble';
    if (!was) this.knocked = true;
    this.state = 'tumble';
    this.stateT = Math.max(was ? this.stateT : 0, t);
  }

  /** Centre of collision sphere i (0 feet, 1 head); the head swings over when tipped. */
  sphere(i: number, out: THREE.Vector3) {
    return sphereAt(this.pos, this.tilt, this.tiltDir, this.size, i, out);
  }

  /**
   * Into a portal: for PORTAL_T the body is out of play, gliding to the exit `p` (so the camera
   * follows it there), then comes out facing `yaw` with (at least `minSpeed` of) its speed, thrown
   * up at `lift` m/s when given. The exit is kept in climbTo, which travels with the full state
   * (client prediction).
   */
  enterPortal(p: THREE.Vector3, yaw: number, minSpeed = 6, lift?: number) {
    if (this.state === 'portal') return;
    const sp = Math.max(minSpeed, Math.hypot(this.vel.x, this.vel.z));
    this.climbTo.copy(p);
    this.vel.set(Math.sin(yaw) * sp, lift ?? Math.max(this.vel.y, 3), Math.cos(yaw) * sp);
    this.yaw = yaw;
    this.state = 'portal';
    this.stateT = PORTAL_T;
    this.tilt = 0;
    this.grounded = false;
    this.groundCol = null;
    this.carryCol = null;
    this.hasGroundLocal = false;
    this.portalIn = true;
  }

  /** In the portal: a straight glide to the exit, arriving when the time is up. */
  private portalStep(dt: number) {
    if (this.stateT <= dt + 1e-9) {
      this.pos.copy(this.climbTo);
      this.state = 'normal';
      this.stateT = 0;
      this.portalOut = true;
      return;
    }
    this.pos.lerp(this.climbTo, dt / this.stateT);
    this.stateT -= dt;
  }

  /** Inside a portal: not drawn, not touched, not grabbed. */
  get inPortal() {
    return this.state === 'portal';
  }

  /** Before moving the world: remember where we stand on the ground collider. */
  beforeWorldUpdate() {
    this.hasGroundLocal = false;
    // A sweeping arm is no ride: it moves on under whoever lands on top, and they drop off behind it.
    if (this.grounded && this.groundCol?.enabled && !this.groundCol.sweep) {
      this.groundLocal.copy(this.pos).applyMatrix4(this.groundCol.inv);
      this.hasGroundLocal = true;
      this.carryCol = this.groundCol;
    }
  }

  /** After moving the world: ride along with a moving platform. */
  afterWorldUpdate() {
    const c = this.carryCol;
    if (!this.hasGroundLocal || !c?.enabled) return;
    this.pos.copy(this.groundLocal).applyMatrix4(c.cur);
    _dir.set(0, 0, 1).transformDirection(c.cur);
    _w.set(0, 0, 1).transformDirection(c.prev);
    const dYaw = Math.atan2(_dir.x, _dir.z) - Math.atan2(_w.x, _w.z);
    if (Math.abs(dYaw) < 0.5) {
      this.yaw += dYaw;
      this.tiltDir += dYaw;
    }
  }

  clearEvents() {
    this.jumped = false;
    this.bounced = false;
    this.hitSomething = false;
    this.stunned = false;
    this.knocked = false;
    this.bumped = 0;
    this.hazard = null;
    this.portalIn = false;
    this.portalOut = false;
  }

  /** Velocity of the ground under the feet (moving platforms, drums); zero on static ground. */
  private groundVelocity(dt: number, out: THREE.Vector3): THREE.Vector3 {
    out.set(0, 0, 0);
    const c = this.groundCol;
    // Platforms only: standing on a hazard (a rotor arm, a hammer) must not fling you off with it.
    if (!rides(c)) return out;
    _l.copy(this.pos).applyMatrix4(c.inv);
    c.surfaceVelocity(_l, dt, out);
    const l = out.length();
    return l > 6 ? out.multiplyScalar(6 / l) : out;
  }

  /** Accelerates the horizontal velocity towards (tx, tz) by at most `acc`. */
  private accelerate(tx: number, tz: number, acc: number) {
    const dx = tx - this.vel.x;
    const dz = tz - this.vel.z;
    const dl = Math.hypot(dx, dz);
    if (dl <= acc) {
      this.vel.x = tx;
      this.vel.z = tz;
    } else {
      this.vel.x += (dx / dl) * acc;
      this.vel.z += (dz / dl) * acc;
    }
  }

  step(dt: number, input: BodyInput, world: CollisionWorld, t: number, others: readonly OtherBody[] = []) {
    if (this.state === 'portal') return this.portalStep(dt);
    if (this.power !== POWER.none && t >= this.powerUntil) this.power = POWER.none;
    const pw = this.power;
    this.size = pw === POWER.giant ? GIANT_SIZE : 1;
    const size = this.size;
    const r = R * size;
    const slow = (t < this.slowUntil ? this.slowK : 1) * (pw === POWER.speed ? SPEED_UP : 1);
    const g = this.grounded;
    const slip = g ? (this.groundCol?.slip ?? 0) : 0;
    this.coyote = g ? 0.12 : Math.max(0, this.coyote - dt);
    this.jumpBuf = input.jump ? 0.12 : Math.max(0, this.jumpBuf - dt);
    this.stateT -= dt;
    const stateBefore = this.state;
    // Leaving a moving surface keeps its motion (jumping off a platform or a drum).
    const platV = g ? this.groundVelocity(dt, _gv) : _gv.set(0, 0, 0);

    if (this.state === 'tumble') {
      // Little control while tumbling; the ground drags a bit (less on ice).
      this.accelerate(input.mx * RUN_SPEED * 0.3, input.mz * RUN_SPEED * 0.3, 5 * dt);
      if (g) {
        const f = Math.exp(-(2.4 - slip * 2) * dt);
        this.vel.x *= f;
        this.vel.z *= f;
      }
      this.tilt += (LIE - this.tilt) * Math.min(1, 9 * dt);
      const settled = g && Math.hypot(this.vel.x, this.vel.z) < 3;
      if ((this.stateT <= 0 && settled) || this.stateT < -2.5) {
        this.state = 'getup';
        this.stateT = 0.4;
      }
    } else if (this.state === 'getup') {
      const f = Math.exp(-8 * dt);
      this.vel.x *= f;
      this.vel.z *= f;
      this.tilt *= Math.exp(-10 * dt);
      // Jump pressed while getting up: spring to the feet (the buffered jump follows).
      if (this.stateT <= 0 || (g && this.jumpBuf > 0 && this.stateT < 0.3)) {
        this.state = 'normal';
        this.tilt = 0;
      }
    } else if (this.state === 'stun' || this.state === 'slide') {
      const f = g ? Math.exp(-(this.state === 'slide' ? 3.5 : 5) * (1 - slip * 0.8) * dt) : 1;
      this.vel.x *= f;
      this.vel.z *= f;
      if (this.stateT <= 0 && (g || this.state === 'slide')) this.state = 'normal';
      // Jump pressed while sliding on the belly: spring back up (the buffered jump follows).
      if (this.state === 'slide' && g && this.jumpBuf > 0 && this.stateT < 0.33) this.state = 'normal';
      if (this.stateT <= -3) this.state = 'normal';
    } else if (this.state === 'dive') {
      if (g && this.stateT < 0.25) {
        this.state = 'slide';
        this.stateT = 0.45;
      }
    } else if (this.state === 'climb') {
      this.climb(dt, input);
    } else if (this.state === 'ladder') {
      this.ladderStep(dt, input, world);
    } else {
      const acc = (g ? 60 - 58.5 * slip : AIR_ACCEL) * dt;
      // Flying faster than a run (thrown by a pad or a portal): the stick steers, it does not brake.
      const cap = RUN_SPEED * slow;
      const fly = g ? 0 : Math.hypot(this.vel.x, this.vel.z);
      const top = fly > cap ? fly : cap;
      this.accelerate(input.mx * top, input.mz * top, acc);
      if (Math.hypot(input.mx, input.mz) > 0.1) {
        let d = Math.atan2(input.mx, input.mz) - this.yaw;
        d = Math.atan2(Math.sin(d), Math.cos(d));
        this.yaw += d * Math.min(1, 14 * dt);
      }
      if (this.jumpBuf > 0 && this.coyote > 0) {
        this.vel.y = JUMP_V * (slow < 1 ? 0.8 : 1) * (pw === POWER.jump ? MEGA_JUMP : 1);
        this.coyote = 0;
        this.jumpBuf = 0;
        this.grounded = false;
        this.jumped = true;
        if (g) {
          this.vel.x += platV.x;
          this.vel.z += platV.z;
          this.vel.y += Math.max(0, platV.y);
        }
      }
      if (input.dive) {
        this.state = 'dive';
        this.stateT = 0.6;
        const l = Math.hypot(input.mx, input.mz);
        let fx = Math.sin(this.yaw);
        let fz = Math.cos(this.yaw);
        if (l > 0.1) {
          fx = input.mx / l;
          fz = input.mz / l;
          this.yaw = Math.atan2(fx, fz);
        }
        // Keep some of the speed already going the same way (no dive may be slower than running).
        const along = Math.max(0, this.vel.x * fx + this.vel.z * fz);
        const sp = Math.max(DIVE_SPEED * slow, Math.min(along, DIVE_SPEED * 1.25));
        this.vel.x = fx * sp + (g ? platV.x : 0);
        this.vel.z = fz * sp + (g ? platV.z : 0);
        this.vel.y = g ? 6 + Math.max(0, platV.y) : Math.max(this.vel.y, 3);
        this.grounded = false;
      }
    }
    if (this.state === 'dive' || this.state === 'slide') {
      // The body lies along its motion: the head sphere swings forward, as the bean is drawn.
      const along = this.vel.x * Math.sin(this.yaw) + this.vel.z * Math.cos(this.yaw);
      const want =
        this.state === 'dive'
          ? THREE.MathUtils.clamp(Math.atan2(Math.max(2, along), this.vel.y), DIVE_TILT_MIN, DIVE_TILT_MAX)
          : SLIDE_TILT;
      this.tilt += (want - this.tilt) * Math.min(1, 12 * dt);
      this.tiltDir = this.yaw;
    } else if (!this.down) this.tilt = this.tilt > 0.02 ? this.tilt * Math.exp(-12 * dt) : 0;

    const climbing = this.state === 'climb';
    const onLadder = this.state === 'ladder';
    const still = climbing || onLadder;
    if (!still) this.vel.y = Math.max(this.vel.y - GRAVITY * dt, -32);
    // Continuous collision, conservatively: a move longer than SUBSTEP_REACH of the radius is split
    // into substeps, each resolved against the colliders, so nothing is passed through or pushed
    // out the wrong side at any speed. (At the speeds the game has today a tick moves at most
    // about half a radius: one step, exactly as before.)
    const travel = still ? 0 : this.vel.length() * dt;
    const substeps = Math.min(MAX_SUBSTEPS, Math.max(1, Math.ceil(travel / (r * SUBSTEP_REACH))));

    const vyBefore = this.vel.y;
    this.grounded = false;
    let newGround: Collider | null = null;
    groundN.set(0, 0, 0);
    wallN.set(0, 0, 0);
    const hitDone = new Set<Collider>();
    let ladder: Collider | null = null;
    for (let sub = 0; sub < substeps; sub++) {
      if (!still) this.pos.addScaledVector(this.vel, dt / substeps);
      const cols = world.query(this.pos.x, this.pos.z, r + 1.2 * size + (this.tilt > 0 ? SPINE * size : 0), nearby);
      for (let iter = 0; iter < 3; iter++) {
        let any = false;
        for (const col of cols) {
          if (!col.enabled) continue;
          // Climbing moves the body along the ledge by itself; only hazards still get at it.
          if (climbing && !col.hit && !col.sweep && !col.bounce && !col.trigger) continue;
          for (let si = 0; si < SPHERES.length; si++) {
            this.sphere(si, _c);
            if (!col.contact(_c, r, hitInfo)) continue;
            if (col.trigger) {
              if (!hitDone.has(col)) {
                hitDone.add(col);
                if (col.ladder) ladder = col;
                else col.onTouch?.(col, hitInfo.normal, this);
              }
              break;
            }
            any = true;
            const n = hitInfo.normal;
            this.pos.addScaledVector(n, hitInfo.depth);
            const vn = this.vel.dot(n);
            if (col.bounce && !hitDone.has(col)) {
              hitDone.add(col);
              const push = Math.max(col.bounce, -vn * 0.8);
              this.vel.addScaledVector(n, push - vn);
              this.vel.y = Math.max(this.vel.y, 4);
              // A fast run into a bumper knocks you over; a light touch only dazes.
              if (n.y < 0.5) {
                if (-vn > 11 && this.state !== 'tumble') this.knock(n.x * 2, n.z * 2, 5, 0.7);
                else this.stun(0.5);
              }
              this.hitSomething = true;
            } else if (vn < 0) {
              // Tumbling beans bounce off like rag dolls; on your feet you just stop.
              const e = this.state === 'tumble' && vn < -3 ? TUMBLE_E : 0;
              this.vel.addScaledVector(n, -vn * (1 + e));
            }
            // Knockback once per hit: a bean pinned against a mover (tumbling, or just dazed by it)
            // is pushed along, not re-launched every tick.
            const fresh = this.state !== 'tumble' && !(this.state === 'stun' && this.stateT > 0.2);
            if (col.sweep && n.y < 0.55 && !hitDone.has(col)) {
              hitDone.add(col);
              col.surfaceVelocity(hitInfo.local, dt, _sv);
              const sp = Math.hypot(_sv.x, _sv.z);
              // Running into the back of an arm that is moving away: just a wall (it fells only what it catches).
              const behind = _sv.x * n.x + _sv.z * n.z < -0.3 * sp;
              if (!behind && this.state === 'tumble') {
                // Already down: scooped up and over the arm (not dragged along with it, not passed through).
                if (g && this.vel.y < SCOOP_V * 0.6) {
                  this.vel.y = SCOOP_V;
                  this.hitSomething = true;
                }
                this.stateT = Math.max(this.stateT, 0.6);
              } else if (!behind && sp > 1) {
                // A sweeping arm fells whoever it catches, shoved along its swing.
                // (hit sets how hard: 0.6 is a full shove)
                const k = Math.min(1, 11 / sp) * 1.5 * col.hit;
                this.knock(_sv.x * k + n.x * 1.5, _sv.z * k + n.z * 1.5, 5 + Math.min(2, sp * 0.15), 1.1, true);
                this.hitSomething = true;
              }
            } else if (col.hit && !col.sweep && !hitDone.has(col) && fresh) {
              hitDone.add(col);
              col.surfaceVelocity(hitInfo.local, dt, _sv);
              // Only the part of the obstacle's motion that comes at us counts.
              const sp = Math.max(0, _sv.dot(n)) * 0.6 + _sv.length() * 0.4;
              if (sp > 2.2) {
                // Hard hits knock the bean over (away from the surface too); glancing ones only daze it.
                const k = col.hit;
                const away = Math.min(2.5, sp * 0.2);
                if (sp * k > 4.2)
                  this.knock(_sv.x * k + n.x * away, _sv.z * k + n.z * away, 4.5 + sp * 0.3, 0.8 + Math.min(0.8, sp * 0.06));
                else {
                  this.vel.x += _sv.x * k + n.x * away * 0.5;
                  this.vel.z += _sv.z * k + n.z * away * 0.5;
                  this.vel.y = Math.max(this.vel.y, 3.5);
                  this.stun(0.6);
                }
                this.hitSomething = true;
              }
            }
            if (n.y > 0.55) {
              this.grounded = true;
              newGround = col;
              if (n.y > groundN.y) groundN.copy(n);
            } else if (col.tag) this.hazard = col.tag;
            else if (Math.abs(n.y) < 0.35 && holdable(col)) wallN.copy(n);
            col.onTouch?.(col, n, this);
          }
        }
        if (!any) break;
      }
    }
    // Over the curve of a moving surface (a drum) the feet stay on it instead of skipping off every tick.
    if (g && !this.grounded && !this.jumped && this.state !== 'dive' && this.vel.y < 1 && rides(this.groundCol)) {
      const col = this.snapDown(world, r, dt);
      if (col) {
        this.grounded = true;
        newGround = col;
      }
    }

    if (
      stateBefore === 'normal' &&
      this.state === 'normal' &&
      !g &&
      !this.grounded &&
      this.vel.y < 4 &&
      wallN.lengthSq() > 0 &&
      this.intoWall(input, wallN) > 0.5
    )
      this.grabLedge(world, wallN);
    if (ladder && stateBefore === 'normal' && this.state === 'normal' && this.stateT <= 0 && !this.jumped)
      this.grabLadder(ladder, input);
    // Stepping off the foot of a ladder (pulling back with the feet on the ground).
    if (onLadder && this.state === 'ladder' && this.grounded) {
      const back = input.mx * Math.sin(this.yaw) + input.mz * Math.cos(this.yaw);
      if (back < -0.3) {
        this.state = 'normal';
        this.stateT = LADDER_AGAIN;
      }
    }

    const myMass = this.mass;
    for (const o of others) {
      const dx = this.pos.x - o.x;
      const dz = this.pos.z - o.z;
      const dy = this.pos.y - o.y;
      const d = Math.hypot(dx, dz);
      const os = o.size ?? 1;
      const gap = (BEAN_GAP * (size + os)) / 2;
      if (d < gap && dy < HEIGHT * os && -dy < HEIGHT * size) {
        if (climbing) {
          o.touching = true;
          continue;
        }
        if (dy > SPHERES[1] * os && this.vel.y <= 0) {
          this.pos.y = o.y + HEIGHT * os;
          this.vel.y = 0;
          this.grounded = true;
        } else {
          const nx = d > 1e-4 ? dx / d : Math.sin(this.actor * 2.4);
          const nz = d > 1e-4 ? dz / d : Math.cos(this.actor * 2.4);
          // Each bean moves out by its share of the overlap (the lighter one more); the other one
          // resolves its share in its own step.
          const oMass = os > 1 ? GIANT_MASS : 1;
          const share = oMass / (myMass + oMass);
          const push = (gap - d) * share;
          this.pos.x += nx * push;
          this.pos.z += nz * push;
          // An elastic-ish exchange of the approaching speed along the normal, by mass.
          const vrel = (this.vel.x - o.vx) * nx + (this.vel.z - o.vz) * nz;
          if (vrel < 0) {
            const j = -(1 + BUMP_E) * vrel * share;
            this.vel.x += nx * j;
            this.vel.z += nz * j;
            // A proper collision pops you up a little.
            if (-vrel > 6 && this.grounded) this.vel.y = Math.max(this.vel.y, Math.min(4, -vrel * 0.3));
            this.bumped = Math.max(this.bumped, -vrel);
          }
        }
        o.touching = true;
      }
    }

    const oldGround = this.groundCol;
    this.groundCol = newGround;
    // On the ground the velocity is relative to it (the ride itself is applied in afterWorldUpdate).
    const relOld = g && !this.jumped && this.state !== 'dive';
    if (relOld && !this.grounded) {
      // Walked (or got pushed) off a moving surface: keep its motion.
      this.vel.x += platV.x;
      this.vel.z += platV.z;
      this.vel.y += Math.max(0, platV.y);
    } else if (this.grounded && newGround !== oldGround) {
      // Landed on (or stepped onto) a moving surface: from now on relative to it.
      this.groundVelocity(dt, _gv2);
      this.vel.x += (relOld ? platV.x : 0) - _gv2.x;
      this.vel.z += (relOld ? platV.z : 0) - _gv2.z;
    }
    if (this.grounded) {
      this.vel.y = Math.max(this.vel.y, 0);
      if (newGround?.conveyor) this.pos.addScaledVector(newGround.conveyor, dt);
      // Moving ground tipping steeply under the feet slips away beneath them: a slide downhill, not a ride up the wall.
      if (rides(newGround) && groundN.y < STEEP_FROM && groundN.y > 0) {
        const k = Math.min(1, (STEEP_FROM - groundN.y) / (STEEP_FROM - 0.55));
        const s = Math.sqrt(1 - groundN.y * groundN.y);
        const d = (STEEP_SLIP * k * dt) / s;
        this.pos.x += groundN.x * groundN.y * d;
        this.pos.y += (groundN.y * groundN.y - 1) * d;
        this.pos.z += groundN.z * groundN.y * d;
      }
      // On ice (and when tumbling) gravity pulls the bean down the slope.
      const slide = Math.max((newGround?.slip ?? 0) * 1.3, this.state === 'tumble' ? 0.6 : 0);
      if (slide > 0 && groundN.y > 0) {
        this.vel.x += GRAVITY * groundN.x * groundN.y * slide * dt;
        this.vel.z += GRAVITY * groundN.z * groundN.y * slide * dt;
      }
      newGround?.onGround?.(newGround, this);
      if (newGround?.pad) {
        this.vel.y = newGround.pad;
        if (newGround.launch) {
          this.vel.x = newGround.launch.x;
          this.vel.z = newGround.launch.z;
        }
        this.grounded = false;
        this.groundCol = null;
        this.bounced = true;
      } else if (!g) this.landImpact = Math.min(1, -vyBefore / 20);
    }
    if (stateBefore !== 'stun' && this.state === 'stun') this.stunned = true;
  }

  /** How squarely the stick pushes into a wall with normal n (−1 … 1; 0 without input). */
  private intoWall(input: BodyInput, n: THREE.Vector3) {
    const l = Math.hypot(input.mx, input.mz);
    const h = Math.hypot(n.x, n.z);
    if (l < 0.3 || h < 1e-3) return 0;
    return -(input.mx * n.x + input.mz * n.z) / (l * h);
  }

  /** Ground within a short drop below the feet: the body is set down on it (straight down) and it is returned. */
  private snapDown(world: CollisionWorld, r: number, dt: number): Collider | null {
    const reach = Math.min(0.3, 0.05 + Math.hypot(this.vel.x, this.vel.z) * dt * 1.5) * this.size;
    const y0 = this.pos.y;
    this.pos.y -= reach;
    this.sphere(0, _c);
    let best: Collider | null = null;
    let lift = 0;
    for (const col of world.query(this.pos.x, this.pos.z, r + 0.5, probe)) {
      if (!col.enabled || col.trigger || col.bounce || col.pad) continue;
      if (!col.contact(_c, r, hitInfo) || hitInfo.normal.y <= 0.55) continue;
      const up = hitInfo.depth / hitInfo.normal.y;
      if (up > lift) {
        lift = up;
        best = col;
        groundN.copy(hitInfo.normal);
      }
    }
    if (!best) {
      this.pos.y = y0;
      return null;
    }
    this.pos.y = Math.min(y0, this.pos.y + lift);
    this.vel.y = Math.max(this.vel.y, 0);
    return best;
  }

  /** Room for the upright body with its feet at (x, y, z)? */
  private fits(world: CollisionWorld, x: number, y: number, z: number): boolean {
    const k = this.size;
    const cols = world.query(x, z, R * k + 0.2, probe);
    for (const si of [0, 1]) {
      _c.set(x, y + SPHERES[si]! * k, z);
      for (const col of cols) {
        if (!col.enabled || col.trigger) continue;
        if (col.contact(_c, R * k, hitInfo) && hitInfo.depth > 0.03) return false;
      }
    }
    return true;
  }

  /**
   * Airborne against a wall (n: its normal): a flat top within reach above it is caught, and the body
   * climbs onto it. The top must be solid and still, with room to stand on it and on the way up.
   */
  private grabLedge(world: CollisionWorld, n: THREE.Vector3) {
    const k = this.size;
    const h = Math.hypot(n.x, n.z);
    const nx = n.x / h;
    const nz = n.z / h;
    const reach = R * k + 0.3 * k;
    const px = this.pos.x - nx * reach;
    const pz = this.pos.z - nz * reach;
    const y0 = this.pos.y + LEDGE_MAX * k;
    const span = (LEDGE_MAX - LEDGE_MIN) * k;
    _p.set(px, y0, pz);
    let best = -1;
    let bestCol: Collider | null = null;
    for (const col of world.query(px, pz, 0.1, probe)) {
      if (!col.enabled || col.trigger) continue;
      // Something at hand height already (a taller wall): no ledge here.
      if (col.contact(_p, 0.05, hitInfo)) return;
      const t = col.raycast(_p, DOWN, span, _rn);
      if (t < 0 || (best >= 0 && t >= best)) continue;
      best = t;
      bestCol = col;
      groundN.copy(_rn);
    }
    if (!bestCol || !holdable(bestCol) || groundN.y < 0.8) return;
    const top = y0 - best + 0.02;
    if (!this.fits(world, px, top, pz) || !this.fits(world, this.pos.x, top, this.pos.z)) return;
    this.state = 'climb';
    this.stateT = CLIMB_T;
    this.climbTo.set(px, top, pz);
    this.vel.set(0, 0, 0);
    this.yaw = Math.atan2(-nx, -nz);
    this.tilt = 0;
    this.coyote = 0;
    this.jumpBuf = 0;
  }

  /** One step of climbing: hang, pull up the wall, then over the edge onto the ledge. */
  private climb(dt: number, input: BodyInput) {
    const to = this.climbTo;
    const dx = to.x - this.pos.x;
    const dz = to.z - this.pos.z;
    const d = Math.hypot(dx, dz);
    const rising = this.pos.y < to.y - 1e-4;
    // Pulling away while still hanging lets go.
    const away = d > 1e-3 && Math.hypot(input.mx, input.mz) > 0.3 && (input.mx * dx + input.mz * dz) / d < -0.5;
    if ((rising && away) || this.stateT <= 0) {
      this.state = 'normal';
      if (d > 1e-3) this.vel.set((-dx / d) * 2, 0, (-dz / d) * 2);
      return;
    }
    const x0 = this.pos.x;
    const y0 = this.pos.y;
    const z0 = this.pos.z;
    if (this.stateT > CLIMB_T - CLIMB_HANG) {
      // Hanging on for a moment.
    } else if (rising) this.pos.y = Math.min(to.y, this.pos.y + CLIMB_UP * this.size * dt);
    else {
      const step = CLIMB_OVER * this.size * dt;
      if (d <= step) {
        this.pos.copy(to);
        this.state = 'normal';
        // Up and over: carry on the way the bean was going.
        if (d > 1e-3) this.vel.set((dx / d) * 3, 0, (dz / d) * 3);
        else this.vel.set(Math.sin(this.yaw) * 3, 0, Math.cos(this.yaw) * 3);
        return;
      }
      this.pos.x += (dx / d) * step;
      this.pos.z += (dz / d) * step;
    }
    this.vel.set((this.pos.x - x0) / dt, (this.pos.y - y0) / dt, (this.pos.z - z0) / dt);
    if (d > 1e-3) this.yaw = Math.atan2(dx, dz);
  }

  /**
   * Walking (or jumping) into a ladder catches it: the body faces the rungs and keeps its height.
   * The ladder's top (its trigger's top) and the spot in front of the rungs are kept in climbTo.
   */
  private grabLadder(col: Collider, input: BodyInput) {
    _dir.set(0, 0, 1).transformDirection(col.cur);
    const h = Math.hypot(_dir.x, _dir.z);
    if (h < 1e-3 || col.shape.type !== 'box') return;
    const nx = _dir.x / h;
    const nz = _dir.z / h;
    if (-(input.mx * nx + input.mz * nz) < 0.5 * Math.hypot(input.mx, input.mz) || Math.hypot(input.mx, input.mz) < 0.3) return;
    if (this.vel.x * nx + this.vel.z * nz > 2 || this.size > 1) return;
    const top = col.center.y + col.shape.hy;
    if (this.pos.y > top - 0.9 || this.pos.y < col.center.y - col.shape.hy - 0.4) return;
    this.state = 'ladder';
    this.stateT = 0;
    this.climbTo.set(col.center.x + nx * LADDER_GAP, top, col.center.z + nz * LADDER_GAP);
    this.vel.set(0, 0, 0);
    this.yaw = Math.atan2(-nx, -nz);
    this.tilt = 0;
    this.coyote = 0;
    this.jumpBuf = 0;
  }

  /** One step on a ladder: up or down with the stick, a leap off with jump, over the top at the top. */
  private ladderStep(dt: number, input: BodyInput, world: CollisionWorld) {
    const fx = Math.sin(this.yaw);
    const fz = Math.cos(this.yaw);
    const to = this.climbTo;
    if (this.jumpBuf > 0) {
      this.state = 'normal';
      this.stateT = LADDER_AGAIN;
      this.vel.set(-fx * LADDER_LEAP, JUMP_V * 0.8, -fz * LADDER_LEAP);
      this.jumpBuf = 0;
      this.jumped = true;
      return;
    }
    const x0 = this.pos.x;
    const y0 = this.pos.y;
    const z0 = this.pos.z;
    // Drawn to the middle of the rungs; pushed far off them (by others), the hands let go.
    const dx = to.x - this.pos.x;
    const dz = to.z - this.pos.z;
    const d = Math.hypot(dx, dz);
    if (d > 1.3) {
      this.state = 'normal';
      return;
    }
    const pull = Math.min(d, 5 * dt);
    if (d > 1e-4) {
      this.pos.x += (dx / d) * pull;
      this.pos.z += (dz / d) * pull;
    }
    const up = input.mx * fx + input.mz * fz;
    const climb = Math.abs(up) > 0.3 ? up * LADDER_SPEED * this.size : 0;
    this.pos.y = Math.min(this.pos.y + climb * dt, to.y - 0.85);
    if (climb > 0 && this.pos.y >= to.y - 0.85 - 1e-6) {
      // At the top: over onto what the ladder leans on, if there is room there.
      const ox = to.x + fx * 1.05;
      const oz = to.z + fz * 1.05;
      if (this.fits(world, ox, to.y + 0.02, oz)) {
        this.state = 'climb';
        this.stateT = CLIMB_T - CLIMB_HANG - 1e-3;
        this.climbTo.set(ox, to.y + 0.02, oz);
      }
    }
    this.vel.set((this.pos.x - x0) / dt, (this.pos.y - y0) / dt, (this.pos.z - z0) / dt);
  }

  toFull(teleport = false): BodyFullState {
    return {
      px: this.pos.x,
      py: this.pos.y,
      pz: this.pos.z,
      vx: this.vel.x,
      vy: this.vel.y,
      vz: this.vel.z,
      yaw: this.yaw,
      state: BODY_STATES.indexOf(this.state),
      stateT: this.stateT,
      grounded: this.grounded,
      coyote: this.coyote,
      jumpBuf: this.jumpBuf,
      slowUntil: Math.max(this.slowUntil, -1e6),
      slowK: this.slowK,
      groundCol: this.groundCol?.index ?? -1,
      landImpact: this.landImpact,
      tilt: this.tilt,
      tiltDir: this.tiltDir,
      power: this.power,
      powerUntil: Math.max(this.powerUntil, -1e6),
      cx: this.climbTo.x,
      cy: this.climbTo.y,
      cz: this.climbTo.z,
      teleport,
    };
  }

  fromFull(s: BodyFullState, world: CollisionWorld) {
    this.pos.set(s.px, s.py, s.pz);
    this.vel.set(s.vx, s.vy, s.vz);
    this.yaw = s.yaw;
    this.state = BODY_STATES[s.state] ?? 'normal';
    this.stateT = s.stateT;
    this.grounded = s.grounded;
    this.coyote = s.coyote;
    this.jumpBuf = s.jumpBuf;
    this.slowUntil = s.slowUntil;
    this.slowK = s.slowK;
    this.groundCol = s.groundCol >= 0 ? (world.colliders[s.groundCol] ?? null) : null;
    this.carryCol = null;
    this.hasGroundLocal = false;
    this.landImpact = s.landImpact;
    this.tilt = s.tilt;
    this.tiltDir = s.tiltDir;
    this.power = s.power;
    this.powerUntil = s.powerUntil;
    this.size = s.power === POWER.giant ? GIANT_SIZE : 1;
    this.climbTo.set(s.cx, s.cy, s.cz);
  }
}
