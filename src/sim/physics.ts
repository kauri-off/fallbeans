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

/** The bean is two spheres of radius R, at these heights above its feet. */
export const R = 0.5;
export const SPHERES = [0.5, 1.1] as const;
export const HEIGHT = 1.6;
export const GRAVITY = 28;
export const RUN_SPEED = 8.5;
export const JUMP_V = 10.5;
export const DIVE_SPEED = 12.5;

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
  onTouch?: ((col: Collider, n: THREE.Vector3, body: PlayerBody) => void) | null;
  onGround?: ((col: Collider, body: PlayerBody) => void) | null;
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
  onTouch: ColliderOpts['onTouch'];
  onGround: ColliderOpts['onGround'];
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
    this.onTouch = opts.onTouch ?? null;
    this.onGround = opts.onGround ?? null;
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
  touching: boolean;
}

export type BodyState = 'normal' | 'stun' | 'dive' | 'slide';
export const BODY_STATES: readonly BodyState[] = ['normal', 'stun', 'dive', 'slide'];

const hitInfo: Contact = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
const nearby: Collider[] = [];

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
  /** Sim time (s) until which the body is slowed (being grabbed). */
  slowUntil = -1e9;
  landImpact = 0;
  // Events of the last step, for sounds and effects.
  jumped = false;
  bounced = false;
  hitSomething = false;
  stunned = false;

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
    this.landImpact = 0;
  }

  stun(t = 1.1) {
    this.state = 'stun';
    this.stateT = t;
  }

  /** Before moving the world: remember where we stand on the ground collider. */
  beforeWorldUpdate() {
    this.hasGroundLocal = false;
    if (this.grounded && this.groundCol?.enabled) {
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
    if (Math.abs(dYaw) < 0.5) this.yaw += dYaw;
  }

  clearEvents() {
    this.jumped = false;
    this.bounced = false;
    this.hitSomething = false;
    this.stunned = false;
  }

  step(dt: number, input: BodyInput, world: CollisionWorld, t: number, others: readonly OtherBody[] = []) {
    const slow = t < this.slowUntil ? 0.45 : 1;
    const g = this.grounded;
    this.coyote = g ? 0.12 : Math.max(0, this.coyote - dt);
    this.jumpBuf = input.jump ? 0.12 : Math.max(0, this.jumpBuf - dt);
    this.stateT -= dt;
    const stateBefore = this.state;

    if (this.state === 'stun' || this.state === 'slide') {
      const f = g ? Math.exp(-(this.state === 'slide' ? 3.5 : 5) * dt) : 1;
      this.vel.x *= f;
      this.vel.z *= f;
      if (this.stateT <= 0 && (g || this.state === 'slide')) this.state = 'normal';
      if (this.stateT <= -3) this.state = 'normal';
    } else if (this.state === 'dive') {
      if (g && this.stateT < 0.25) {
        this.state = 'slide';
        this.stateT = 0.45;
      }
    } else {
      const tx = input.mx * RUN_SPEED * slow;
      const tz = input.mz * RUN_SPEED * slow;
      const acc = (g ? 60 : 16) * dt;
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
      if (Math.hypot(input.mx, input.mz) > 0.1) {
        let d = Math.atan2(input.mx, input.mz) - this.yaw;
        d = Math.atan2(Math.sin(d), Math.cos(d));
        this.yaw += d * Math.min(1, 14 * dt);
      }
      if (this.jumpBuf > 0 && this.coyote > 0) {
        this.vel.y = JUMP_V * (slow < 1 ? 0.75 : 1);
        this.coyote = 0;
        this.jumpBuf = 0;
        this.grounded = false;
        this.jumped = true;
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
        this.vel.x = fx * DIVE_SPEED * slow;
        this.vel.z = fz * DIVE_SPEED * slow;
        this.vel.y = g ? 6 : Math.max(this.vel.y, 3);
        this.grounded = false;
      }
    }

    this.vel.y = Math.max(this.vel.y - GRAVITY * dt, -32);
    this.pos.addScaledVector(this.vel, dt);

    const vyBefore = this.vel.y;
    this.grounded = false;
    let newGround: Collider | null = null;
    const hitDone = new Set<Collider>();
    const cols = world.query(this.pos.x, this.pos.z, R + 1.2, nearby);
    for (let iter = 0; iter < 3; iter++) {
      let any = false;
      for (const col of cols) {
        if (!col.enabled) continue;
        for (const oy of SPHERES) {
          _c.set(this.pos.x, this.pos.y + oy, this.pos.z);
          if (!col.contact(_c, R, hitInfo)) continue;
          any = true;
          const n = hitInfo.normal;
          this.pos.addScaledVector(n, hitInfo.depth);
          const vn = this.vel.dot(n);
          if (col.bounce && !hitDone.has(col)) {
            hitDone.add(col);
            const push = Math.max(col.bounce, -vn * 0.8);
            this.vel.addScaledVector(n, push - vn);
            this.vel.y = Math.max(this.vel.y, 4);
            if (n.y < 0.5) this.stun(0.5);
            this.hitSomething = true;
          } else if (vn < 0) {
            this.vel.addScaledVector(n, -vn);
          }
          // Knockback once per hit: a stunned bean pinned against a mover is not re-launched every tick.
          if (col.hit && !hitDone.has(col) && this.state !== 'stun') {
            hitDone.add(col);
            col.surfaceVelocity(hitInfo.local, dt, _sv);
            const sp = _sv.length();
            if (sp > 2.5) {
              this.vel.x += _sv.x * col.hit;
              this.vel.z += _sv.z * col.hit;
              this.vel.y = Math.max(this.vel.y, 4 + sp * 0.25);
              this.stun(1.0 + Math.min(0.8, sp * 0.05));
              this.hitSomething = true;
            }
          }
          if (n.y > 0.55) {
            this.grounded = true;
            newGround = col;
          }
          col.onTouch?.(col, n, this);
        }
      }
      if (!any) break;
    }

    for (const o of others) {
      const dx = this.pos.x - o.x;
      const dz = this.pos.z - o.z;
      const dy = this.pos.y - o.y;
      const d = Math.hypot(dx, dz);
      if (d < 0.95 && Math.abs(dy) < HEIGHT) {
        if (dy > 1.1 && this.vel.y <= 0) {
          this.pos.y = o.y + HEIGHT;
          this.vel.y = 0;
          this.grounded = true;
        } else if (d > 1e-4) {
          const push = (0.95 - d) * 0.6;
          this.pos.x += (dx / d) * push;
          this.pos.z += (dz / d) * push;
        }
        o.touching = true;
      }
    }

    this.groundCol = newGround;
    if (this.grounded) {
      this.vel.y = Math.max(this.vel.y, 0);
      if (newGround?.conveyor) this.pos.addScaledVector(newGround.conveyor, dt);
      newGround?.onGround?.(newGround, this);
      if (newGround?.pad) {
        this.vel.y = newGround.pad;
        this.grounded = false;
        this.groundCol = null;
        this.bounced = true;
      } else if (!g) this.landImpact = Math.min(1, -vyBefore / 20);
    }
    if (stateBefore !== 'stun' && this.state === 'stun') this.stunned = true;
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
      groundCol: this.groundCol?.index ?? -1,
      landImpact: this.landImpact,
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
    this.groundCol = s.groundCol >= 0 ? (world.colliders[s.groundCol] ?? null) : null;
    this.carryCol = null;
    this.hasGroundLocal = false;
    this.landImpact = s.landImpact;
  }
}
