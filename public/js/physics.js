import * as THREE from 'three';

const _l = new THREE.Vector3();
const _q = new THREE.Vector3();
const _n = new THREE.Vector3();
const _c = new THREE.Vector3();
const _p = new THREE.Vector3();
const _w = new THREE.Vector3();

export const R = 0.5;
export const SPHERES = [0.5, 1.1];
export const GRAVITY = 28;
export const RUN_SPEED = 8.5;
export const JUMP_V = 10.5;

export class Collider {
  constructor(obj, shape, opts = {}) {
    this.obj = obj;
    this.shape = shape;
    this.enabled = true;
    this.isStatic = !!opts.isStatic;
    this.bounce = opts.bounce || 0;
    this.hit = opts.hit || 0;
    this.conveyor = opts.conveyor || null;
    this.onTouch = opts.onTouch || null;
    this.onGround = opts.onGround || null;
    this.userData = opts.userData || {};
    this.cur = new THREE.Matrix4();
    this.prev = new THREE.Matrix4();
    this.inv = new THREE.Matrix4();
    this.center = new THREE.Vector3();
    this.radius = shape.type === 'box'
      ? Math.hypot(shape.hx, shape.hy, shape.hz)
      : Math.hypot(shape.r, shape.hh);
    this.synced = false;
  }

  sync() {
    if (this.isStatic && this.synced) { this.prev.copy(this.cur); return; }
    this.obj.updateWorldMatrix(true, false);
    if (!this.synced) this.prev.copy(this.obj.matrixWorld); else this.prev.copy(this.cur);
    this.cur.copy(this.obj.matrixWorld);
    this.inv.copy(this.cur).invert();
    this.center.setFromMatrixPosition(this.cur);
    this.synced = true;
  }

  contact(center, r, out) {
    if (center.distanceToSquared(this.center) > (this.radius + r) ** 2) return false;
    _l.copy(center).applyMatrix4(this.inv);
    const s = this.shape;
    let depth;
    if (s.type === 'box') {
      _q.set(
        THREE.MathUtils.clamp(_l.x, -s.hx, s.hx),
        THREE.MathUtils.clamp(_l.y, -s.hy, s.hy),
        THREE.MathUtils.clamp(_l.z, -s.hz, s.hz));
      _n.subVectors(_l, _q);
      const d = _n.length();
      if (d > 1e-6) {
        if (d >= r) return false;
        _n.divideScalar(d);
        depth = r - d;
      } else {
        const dx = s.hx - Math.abs(_l.x), dy = s.hy - Math.abs(_l.y), dz = s.hz - Math.abs(_l.z);
        _n.set(0, 0, 0);
        if (dy <= dx && dy <= dz) { _n.y = Math.sign(_l.y) || 1; _q.y = _n.y * s.hy; depth = dy + r; }
        else if (dx <= dz) { _n.x = Math.sign(_l.x) || 1; _q.x = _n.x * s.hx; depth = dx + r; }
        else { _n.z = Math.sign(_l.z) || 1; _q.z = _n.z * s.hz; depth = dz + r; }
      }
    } else {
      const rl = Math.hypot(_l.x, _l.z);
      const inR = rl <= s.r, inY = Math.abs(_l.y) <= s.hh;
      if (inR && inY) {
        const side = s.r - rl, top = s.hh - _l.y, bot = s.hh + _l.y;
        _q.copy(_l);
        if (top <= side && top <= bot) { _n.set(0, 1, 0); _q.y = s.hh; depth = top + r; }
        else if (bot <= side) { _n.set(0, -1, 0); _q.y = -s.hh; depth = bot + r; }
        else {
          if (rl < 1e-6) _n.set(1, 0, 0); else _n.set(_l.x / rl, 0, _l.z / rl);
          _q.set(_n.x * s.r, _l.y, _n.z * s.r); depth = side + r;
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
    }
    out.local.copy(_q);
    out.point.copy(_q).applyMatrix4(this.cur);
    out.normal.copy(_n).transformDirection(this.cur);
    out.depth = depth;
    return true;
  }

  surfaceVelocity(local, dt, out) {
    _p.copy(local).applyMatrix4(this.cur);
    _w.copy(local).applyMatrix4(this.prev);
    return out.subVectors(_p, _w).divideScalar(Math.max(dt, 1e-4));
  }
}

const hitInfo = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
const _sv = new THREE.Vector3();
const _dir = new THREE.Vector3();

export class PlayerBody {
  constructor() {
    this.pos = new THREE.Vector3();
    this.vel = new THREE.Vector3();
    this.yaw = 0;
    this.grounded = false;
    this.groundCol = null;
    this.groundLocal = new THREE.Vector3();
    this.hasGroundLocal = false;
    this.state = 'normal';
    this.stateT = 0;
    this.coyote = 0;
    this.jumpBuf = 0;
    this.slowUntil = 0;
    this.landImpact = 0;
    this.jumped = false;
    this.hitSomething = false;
  }

  reset(p, yaw = 0) {
    this.pos.copy(p); this.vel.set(0, 0, 0); this.yaw = yaw;
    this.state = 'normal'; this.stateT = 0; this.grounded = false; this.groundCol = null; this.hasGroundLocal = false;
  }

  stun(t = 1.1) { this.state = 'stun'; this.stateT = t; }

  beforeWorldUpdate() {
    this.hasGroundLocal = false;
    if (this.grounded && this.groundCol && this.groundCol.enabled) {
      this.groundLocal.copy(this.pos).applyMatrix4(this.groundCol.inv);
      this.hasGroundLocal = true;
      this.carryCol = this.groundCol;
    }
  }

  afterWorldUpdate() {
    if (!this.hasGroundLocal || !this.carryCol.enabled) return;
    const c = this.carryCol;
    _p.copy(this.groundLocal).applyMatrix4(c.cur);
    this.pos.copy(_p);
    _dir.set(0, 0, 1).transformDirection(c.cur);
    _w.set(0, 0, 1).transformDirection(c.prev);
    const dYaw = Math.atan2(_dir.x, _dir.z) - Math.atan2(_w.x, _w.z);
    if (Math.abs(dYaw) < 0.5) this.yaw += dYaw;
  }

  step(dt, input, colliders, now, others = [], frameDt = dt) {
    this.jumped = false;
    this.hitSomething = false;
    const slow = now < this.slowUntil ? 0.45 : 1;
    const g = this.grounded;
    this.coyote = g ? 0.12 : Math.max(0, this.coyote - dt);
    this.jumpBuf = input.jump ? 0.12 : Math.max(0, this.jumpBuf - dt);
    this.stateT -= dt;

    if (this.state === 'stun' || this.state === 'slide') {
      const f = g ? Math.exp(-(this.state === 'slide' ? 3.5 : 5) * dt) : 1;
      this.vel.x *= f; this.vel.z *= f;
      if (this.stateT <= 0 && (g || this.state === 'slide')) this.state = 'normal';
      if (this.stateT <= -3) this.state = 'normal';
    } else if (this.state === 'dive') {
      if (g && this.stateT < 0.25) { this.state = 'slide'; this.stateT = 0.45; }
    } else {
      const tx = input.mx * RUN_SPEED * slow, tz = input.mz * RUN_SPEED * slow;
      const acc = (g ? 60 : 16) * dt;
      const dx = tx - this.vel.x, dz = tz - this.vel.z;
      const dl = Math.hypot(dx, dz);
      if (dl <= acc) { this.vel.x = tx; this.vel.z = tz; }
      else { this.vel.x += dx / dl * acc; this.vel.z += dz / dl * acc; }
      if (Math.hypot(input.mx, input.mz) > 0.1) {
        const target = Math.atan2(input.mx, input.mz);
        let d = target - this.yaw;
        d = Math.atan2(Math.sin(d), Math.cos(d));
        this.yaw += d * Math.min(1, 14 * dt);
      }
      if (this.jumpBuf > 0 && this.coyote > 0) {
        this.vel.y = JUMP_V * (slow < 1 ? 0.75 : 1);
        this.coyote = 0; this.jumpBuf = 0; this.grounded = false; this.jumped = true;
      }
      if (input.dive) {
        this.state = 'dive'; this.stateT = 0.6;
        const l = Math.hypot(input.mx, input.mz);
        let fx = Math.sin(this.yaw), fz = Math.cos(this.yaw);
        if (l > 0.1) { fx = input.mx / l; fz = input.mz / l; this.yaw = Math.atan2(fx, fz); }
        this.vel.x = fx * 12.5 * slow; this.vel.z = fz * 12.5 * slow;
        this.vel.y = g ? 6 : Math.max(this.vel.y, 3);
        this.grounded = false;
      }
    }

    this.vel.y = Math.max(this.vel.y - GRAVITY * dt, -32);
    this.pos.addScaledVector(this.vel, dt);

    const wasGrounded = g;
    const vyBefore = this.vel.y;
    this.grounded = false;
    let newGround = null;
    const hitDone = new Set();
    for (let iter = 0; iter < 3; iter++) {
      let any = false;
      for (const col of colliders) {
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
          if (col.hit && !hitDone.has(col)) {
            hitDone.add(col);
            col.surfaceVelocity(hitInfo.local, frameDt, _sv);
            const sp = _sv.length();
            if (sp > 2.5) {
              this.vel.x += _sv.x * col.hit; this.vel.z += _sv.z * col.hit;
              this.vel.y = Math.max(this.vel.y, 4 + sp * 0.25);
              this.stun(1.0 + Math.min(0.8, sp * 0.05));
              this.hitSomething = true;
            }
          }
          if (n.y > 0.55) { this.grounded = true; newGround = col; }
          if (col.onTouch) col.onTouch(col, n);
        }
      }
      if (!any) break;
    }

    for (const o of others) {
      const dx = this.pos.x - o.x, dz = this.pos.z - o.z, dy = this.pos.y - o.y;
      const d = Math.hypot(dx, dz);
      if (d < 0.95 && Math.abs(dy) < 1.5) {
        if (dy > 1.1 && this.vel.y <= 0) {
          this.pos.y = o.y + 1.5; this.vel.y = 0; this.grounded = true;
        } else if (d > 1e-4) {
          const push = (0.95 - d) * 0.6;
          this.pos.x += dx / d * push; this.pos.z += dz / d * push;
        }
        o.touching = true;
      }
    }

    this.groundCol = newGround;
    if (this.grounded) {
      this.vel.y = Math.max(this.vel.y, 0);
      if (newGround.conveyor) this.pos.addScaledVector(newGround.conveyor, dt);
      if (newGround.onGround) newGround.onGround(newGround);
      if (!wasGrounded) this.landImpact = Math.min(1, -vyBefore / 20);
    }
  }
}
