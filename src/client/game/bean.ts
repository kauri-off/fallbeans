import * as THREE from 'three';
import { ANIM } from '../../shared/consts';
import { clone } from './assets';
import { applySurface } from './materials';

const bodyMats = new Map<string, THREE.MeshStandardMaterial>();
function bodyMaterial(color: string) {
  let m = bodyMats.get(color);
  if (!m) {
    m = new THREE.MeshPhysicalMaterial({
      color: new THREE.Color(color),
      roughness: 0.62,
      clearcoat: 0.25,
      clearcoatRoughness: 0.45,
      sheen: 0.6,
      sheenRoughness: 0.55,
      sheenColor: new THREE.Color('#ffffff'),
    });
    applySurface(m, 'fabric', { keepRoughness: true });
    bodyMats.set(color, m);
  }
  return m;
}

const bellyMats = new Map<string, THREE.MeshStandardMaterial>();
/** The belly patch: the player colour, washed towards white. */
function bellyMaterial(color: string) {
  let m = bellyMats.get(color);
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color: new THREE.Color(color).lerp(new THREE.Color('#ffffff'), 0.62), roughness: 0.7 });
    applySurface(m, 'fabric', { keepRoughness: true, strength: 0.7 });
    bellyMats.set(color, m);
  }
  return m;
}

let tailProto: THREE.Object3D | null = null;
function makeTail(): THREE.Object3D {
  if (!tailProto) {
    const g = new THREE.Group();
    const mat = applySurface(new THREE.MeshStandardMaterial({ color: '#ff9f1c', roughness: 0.85 }), 'fabric');
    const tip = applySurface(new THREE.MeshStandardMaterial({ color: '#fff4d6', roughness: 0.95 }), 'fabric');
    const segs = 5;
    let parent: THREE.Object3D = g;
    for (let i = 0; i < segs; i++) {
      const r = 0.17 - i * 0.02;
      const m = new THREE.Mesh(new THREE.SphereGeometry(r, 14, 10), i === segs - 1 ? tip : mat);
      m.position.set(0, i ? 0.1 : 0, i ? -0.15 : 0);
      m.castShadow = true;
      m.name = `tail${i}`;
      parent.add(m);
      parent = m;
    }
    tailProto = g;
  }
  return tailProto.clone(true);
}

/** A damped spring: lags and overshoots, which reads as soft and floppy. */
class Spring {
  x = 0;
  v = 0;
  step(target: number, k: number, c: number, dt: number) {
    // Semi-implicit in small substeps: stays stable with stiff springs and long frames.
    const n = Math.max(1, Math.ceil(dt / (1 / 240)));
    const h = dt / n;
    for (let i = 0; i < n; i++) {
      this.v += (k * (target - this.x) - c * this.v) * h;
      this.x += this.v * h;
    }
    return this.x;
  }
}

interface Limb {
  o: THREE.Object3D;
  base: THREE.Euler;
  sx: Spring;
  sz: Spring;
}

export type Pose = 'cheer' | 'clap' | 'sad' | null;

export interface BeanFrame {
  /** World velocity (m/s). */
  vel: THREE.Vector3;
  anim: number;
  /** Sim time (s), for procedural cycles. */
  t: number;
  landImpact?: number;
  tilt?: number;
  /** World yaw the body tips towards. */
  tiltDir?: number;
  /** World position of the bean being held, if any. */
  grabAt?: THREE.Vector3 | null;
}

interface Targets {
  armLx: number;
  armLz: number;
  armRx: number;
  armRz: number;
  legLx: number;
  legLz: number;
  legRx: number;
  legRz: number;
  lean: number;
  roll: number;
  twist: number;
  lift: number;
  /** Limb spring stiffness and damping. */
  k: number;
  c: number;
}

const _v = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _q2 = new THREE.Quaternion();
const _axis = new THREE.Vector3();
const UP = new THREE.Vector3(0, 1, 0);
const smooth = THREE.MathUtils.smoothstep;
const clamp = THREE.MathUtils.clamp;

/** Height of the tip-over pivot (the lower collision sphere). */
const PIVOT_Y = 0.5;
/** Ground covered by one full run cycle (two steps), m. */
const STRIDE = 1.45;

export class Bean {
  readonly root = new THREE.Group();
  /** Rotates around the lower body when tumbling. */
  private readonly pivot = new THREE.Group();
  readonly model: THREE.Object3D;
  private readonly limbs: Record<'ArmL' | 'ArmR' | 'LegL' | 'LegR', Limb>;
  private readonly eyes: THREE.Object3D[] = [];
  private readonly reach: THREE.Mesh[] = [];
  color = '';
  /** Shown by the HTML name tags (see ui/Tags.tsx). */
  name = '';
  /** Podium pose, overriding idle. */
  pose: Pose = null;
  private crown: THREE.Object3D | null = null;
  private tail: THREE.Object3D | null = null;
  private phase = 0;
  private readonly squash = new Spring();
  private readonly lean = new Spring();
  private readonly roll = new Spring();
  private readonly twist = new Spring();
  private readonly lift = new Spring();
  private tiltNow = 0;
  /** Head-over-heels spin while tumbling through the air (rad), and its rate. */
  private spin = 0;
  private spinRate = 0;
  private emote = 0;
  emoteT = 0;
  private blinkAt = 1 + Math.random() * 3;
  private readonly seed = Math.random() * 100;
  private readonly lastVel = new THREE.Vector3();
  private lastYaw = 0;
  private yawRate = 0;
  private lastAnim = -1;
  private airT = 0;
  private fidget = 0;
  private fidgetAt = 4 + Math.random() * 6;
  private stepSide = 0;

  constructor(color: string, name: string) {
    this.model = clone('bean');
    this.pivot.position.y = PIVOT_Y;
    this.model.position.y = -PIVOT_Y;
    this.pivot.add(this.model);
    this.root.add(this.pivot);
    const limb = (n: string): Limb => {
      const o = this.model.getObjectByName(n);
      if (!o) throw new Error(`bean model is missing ${n}`);
      return { o, base: o.rotation.clone(), sx: new Spring(), sz: new Spring() };
    };
    this.limbs = { ArmL: limb('ArmL'), ArmR: limb('ArmR'), LegL: limb('LegL'), LegR: limb('LegR') };
    for (const n of ['EyeL', 'EyeR']) {
      const e = this.model.getObjectByName(n);
      if (e) this.eyes.push(e);
    }
    // Stretchy arms for grabbing: a tapered tube from each shoulder to the held bean.
    const tube = new THREE.CylinderGeometry(0.1, 0.13, 1, 10, 1, false);
    tube.translate(0, 0.5, 0);
    for (let i = 0; i < 2; i++) {
      const m = new THREE.Mesh(tube, bodyMaterial(color));
      m.visible = false;
      m.castShadow = true;
      this.root.add(m);
      this.reach.push(m);
    }
    this.setColor(color);
    this.name = name;
  }

  setColor(color: string) {
    this.color = color;
    const m = bodyMaterial(color);
    const belly = bellyMaterial(color);
    this.model.traverse((o) => {
      if (!(o instanceof THREE.Mesh)) return;
      const name = (o.material as THREE.Material).name;
      if (name === 'Body') o.userData.part = 'body';
      if (name === 'Belly') o.userData.part = 'belly';
      if (o.userData.part === 'body') o.material = m;
      if (o.userData.part === 'belly') o.material = belly;
    });
    for (const r of this.reach) r.material = m;
  }

  setCrown(on: boolean) {
    if (on && !this.crown) {
      this.crown = clone('crown');
      this.crown.scale.setScalar(0.62);
      this.crown.position.set(0, 1.62, -0.02);
      this.crown.rotation.x = -0.12;
      this.model.add(this.crown);
    } else if (!on && this.crown) {
      this.model.remove(this.crown);
      this.crown = null;
    }
  }

  setTail(on: boolean) {
    if (on && !this.tail) {
      this.tail = makeTail();
      this.tail.position.set(0, 0.45, -0.52);
      this.model.add(this.tail);
    } else if (!on && this.tail) {
      this.model.remove(this.tail);
      this.tail = null;
    }
  }

  get hasTail() {
    return !!this.tail;
  }

  playEmote(e: number) {
    this.emote = e;
    this.emoteT = 2.6;
  }

  dispose() {
    this.root.removeFromParent();
  }

  animate(dt: number, f: BeanFrame) {
    if (dt <= 0) return;
    dt = Math.min(dt, 1 / 20);
    const { anim: a, t } = f;
    const yaw = this.root.rotation.y;
    // Velocity in the bean's own frame: forward (+z) and right (−x is the bean's right).
    const cy = Math.cos(yaw);
    const sy = Math.sin(yaw);
    const fwd = f.vel.x * sy + f.vel.z * cy;
    const side = f.vel.x * cy - f.vel.z * sy;
    const speed = Math.hypot(f.vel.x, f.vel.z);
    const accel = _v.subVectors(f.vel, this.lastVel).divideScalar(dt);
    const aFwd = accel.x * sy + accel.z * cy;
    const aSide = accel.x * cy - accel.z * sy;
    this.lastVel.copy(f.vel);
    let dYaw = yaw - this.lastYaw;
    dYaw = Math.atan2(Math.sin(dYaw), Math.cos(dYaw));
    this.lastYaw = yaw;
    this.yawRate += (dYaw / dt - this.yawRate) * Math.min(1, dt * 10);
    const entered = a !== this.lastAnim;
    const prevAnim = this.lastAnim;
    this.lastAnim = a;
    this.emoteT -= dt;
    this.airT = a === ANIM.air ? this.airT + dt : 0;

    // Squash and stretch impulses on events.
    const impact = f.landImpact ?? 0;
    if (impact > 0.1) this.squash.v -= impact * 11;
    if (entered && a === ANIM.air && f.vel.y > 3) this.squash.v += 4.5;
    if (entered && a === ANIM.dive) this.squash.v += 3;

    const T =
      this.pose && a === ANIM.idle && speed < 1.2 ? this.podium(t) : this.pickPose(a, t, dt, speed, fwd, side, f.vel.y, aFwd);

    // Secondary motion: limbs are thrown against the body's acceleration (strongest when loose).
    const loose = 1 - clamp(T.k / 260, 0, 1);
    const kick = (0.0015 + loose * 0.006) * dt * 60;
    const L = this.limbs;
    for (const l of [L.ArmL, L.ArmR]) l.sx.v += aFwd * kick;
    L.ArmL.sz.v += aSide * kick;
    L.ArmR.sz.v += aSide * kick;
    for (const l of [L.LegL, L.LegR]) l.sx.v += aFwd * kick * 0.6;

    const set = (l: Limb, x: number, z: number) => {
      l.o.rotation.x = l.base.x + l.sx.step(x, T.k, T.c, dt);
      l.o.rotation.z = l.base.z + l.sz.step(z, T.k, T.c, dt);
    };
    set(L.ArmL, T.armLx, -T.armLz);
    set(L.ArmR, T.armRx, T.armRz);
    set(L.LegL, T.legLx, -T.legLz);
    set(L.LegR, T.legRx, T.legRz);

    // Body: tip-over (tumble) around the lower body, spinning through the air, then lean/roll on top.
    const want = f.tilt ?? 0;
    this.tiltNow += (want - this.tiltNow) * Math.min(1, dt * 14);
    const tumbling = a === ANIM.tumble;
    if (tumbling && entered) this.spinRate = clamp(speed * 1.3, 5, 13);
    if (tumbling && Math.abs(f.vel.y) > 1.5 && this.airborne(f)) this.spin += this.spinRate * dt;
    else {
      // On the ground (or getting up): settle to the nearest whole turn.
      const target = Math.round(this.spin / (Math.PI * 2)) * Math.PI * 2;
      this.spin += (target - this.spin) * Math.min(1, dt * (tumbling ? 6 : 10));
      this.spinRate *= Math.exp(-dt * 3);
    }
    if (!tumbling && prevAnim === ANIM.tumble && a !== ANIM.getup) this.spin = 0;
    const localDir = (f.tiltDir ?? 0) - yaw;
    _axis.set(Math.cos(localDir), 0, -Math.sin(localDir));
    _q.setFromAxisAngle(_axis, this.tiltNow + this.spin);
    // Rolling from side to side while sliding along on the back.
    if (tumbling) {
      _q2.setFromAxisAngle(
        _v.set(Math.sin(localDir), 0, Math.cos(localDir)),
        Math.sin(t * 7 + this.seed) * Math.min(0.5, speed * 0.08),
      );
      _q.multiply(_q2);
    }
    this.pivot.quaternion.copy(_q);
    const leanNow = this.lean.step(T.lean, 110, 14, dt);
    const rollNow = this.roll.step(T.roll, 110, 13, dt);
    const twistNow = this.twist.step(T.twist, 90, 12, dt);
    this.model.rotation.set(leanNow, twistNow, rollNow);
    this.model.position.y = -PIVOT_Y + this.lift.step(T.lift, 220, 20, dt);

    // Squash and stretch: a spring around 0, plus stretch with vertical speed in the air.
    const airStretch = a === ANIM.air ? clamp(Math.abs(f.vel.y) / 40, 0, 0.12) : 0;
    const sq = clamp(this.squash.step(0, 260, 12, dt) + airStretch, -0.32, 0.28);
    const breathe = a === ANIM.idle && speed < 1 ? Math.sin(t * 2.6 + this.seed) * 0.014 : 0;
    this.model.scale.set(1 - sq * 0.5 - breathe * 0.5, 1 + sq + breathe, 1 - sq * 0.5 - breathe * 0.5);

    // Eyes: blink every few seconds; squeezed shut when tumbling, droopy when dazed.
    this.blinkAt -= dt;
    const blink = this.blinkAt < 0.12 && this.blinkAt > 0 ? 0.12 : 1;
    if (this.blinkAt < 0) this.blinkAt = 2 + Math.random() * 3.5;
    const eye = a === ANIM.tumble ? 0.25 : a === ANIM.stun ? 0.55 : this.pose === 'sad' ? 0.6 : blink;
    for (const e of this.eyes) e.scale.y += (eye - e.scale.y) * Math.min(1, dt * 30);

    if (this.tail) {
      // Each segment follows the one before: a wagging, trailing tail.
      let seg = this.tail as THREE.Object3D;
      for (let i = 0; i < 5 && seg; i++) {
        seg.rotation.y = Math.sin(t * 8 - i * 0.7) * (0.12 + Math.min(0.25, speed * 0.03));
        seg.rotation.x = clamp(-fwd * 0.02, -0.3, 0.2) + (a === ANIM.air ? 0.15 : 0);
        seg = seg.children.find((c) => c.name.startsWith('tail')) as THREE.Object3D;
      }
    }
    this.updateReach(f.grabAt ?? null);
  }

  private airborne(f: BeanFrame) {
    return f.anim === ANIM.air || f.anim === ANIM.tumble || f.anim === ANIM.dive;
  }

  private base(): Targets {
    return {
      armLx: 0.1,
      armLz: 0.15,
      armRx: 0.1,
      armRz: 0.15,
      legLx: 0,
      legLz: 0.03,
      legRx: 0,
      legRz: 0.03,
      lean: 0,
      roll: 0,
      twist: 0,
      lift: 0,
      k: 240,
      c: 20,
    };
  }

  private pickPose(
    a: number,
    t: number,
    dt: number,
    speed: number,
    fwd: number,
    side: number,
    vy: number,
    aFwd: number,
  ): Targets {
    const T = this.base();
    const w = t * 1 + this.seed;
    if (a === ANIM.tumble) {
      // Ragdoll: loose limbs flung around by the tumble.
      T.k = 30;
      T.c = 2.8;
      T.armLx = -2.2 + Math.sin(w * 9) * 1.2;
      T.armRx = -2.0 + Math.sin(w * 8 + 1.3) * 1.2;
      T.armLz = 1.2 + Math.cos(w * 11) * 0.6;
      T.armRz = 1.1 + Math.cos(w * 10 + 2) * 0.6;
      T.legLx = Math.sin(w * 7 + 1) * 1.0;
      T.legRx = Math.sin(w * 7.5 + 2.4) * 1.0;
      T.legLz = 0.5 + Math.sin(w * 6) * 0.3;
      T.legRz = 0.5 + Math.cos(w * 6.4) * 0.3;
      return T;
    }
    if (a === ANIM.getup) {
      // Arms push against the ground, legs gather, a little hop at the end.
      T.k = 150;
      T.c = 14;
      T.armLx = 0.9;
      T.armRx = 0.9;
      T.armLz = 0.7;
      T.armRz = 0.7;
      T.legLx = -0.6;
      T.legRx = -0.6;
      T.legLz = 0.25;
      T.legRz = 0.25;
      T.lift = 0.08;
      return T;
    }
    if (a === ANIM.dive || a === ANIM.slide) {
      // Superman: arms stretched ahead, legs straight back, belly towards the ground.
      T.lean = a === ANIM.dive ? 1.3 : 1.42;
      T.armLx = -2.95;
      T.armRx = -2.95;
      T.armLz = 0.28;
      T.armRz = 0.28;
      T.legLx = 0.45;
      T.legRx = 0.45;
      T.legLz = 0.15;
      T.legRz = 0.15;
      if (a === ANIM.slide) {
        T.legLx += Math.sin(t * 16) * 0.25 * Math.min(1, speed / 4);
        T.legRx -= Math.sin(t * 16) * 0.25 * Math.min(1, speed / 4);
      }
      T.lift = a === ANIM.dive ? 0.42 : 0.3;
      T.roll = clamp(-side * 0.04, -0.3, 0.3);
      T.k = 200;
      return T;
    }
    if (a === ANIM.stun) {
      // Dazed: the body circles, arms dangle loosely.
      T.k = 70;
      T.c = 6;
      T.lean = Math.sin(t * 6) * 0.22 + 0.1;
      T.roll = Math.cos(t * 6) * 0.22;
      T.armLx = Math.sin(t * 6 + 1) * 0.6;
      T.armRx = Math.sin(t * 6 + 2.5) * 0.6;
      T.armLz = 0.7 + Math.cos(t * 5) * 0.3;
      T.armRz = 0.7 + Math.sin(t * 5) * 0.3;
      T.legLz = 0.2;
      T.legRz = 0.2;
      return T;
    }
    if (a === ANIM.air) {
      const rising = vy > 1.5;
      const falling = vy < -6;
      if (rising) {
        // Take-off: arms thrown up, one knee up.
        T.armLx = -2.5;
        T.armRx = -2.3;
        T.armLz = 0.45;
        T.armRz = 0.5;
        T.legLx = this.stepSide ? -0.9 : 0.2;
        T.legRx = this.stepSide ? 0.2 : -0.9;
        T.lean = -0.05 + clamp(fwd * 0.012, 0, 0.12);
      } else if (falling) {
        // Falling: flailing arms, pedalling legs.
        const k = Math.min(1, (-vy - 6) / 10);
        T.k = 160;
        T.c = 10;
        T.armLx = -2.7 + Math.sin(t * 17) * (0.3 + k * 0.4);
        T.armRx = -2.7 + Math.sin(t * 17 + 2) * (0.3 + k * 0.4);
        T.armLz = 0.8 + Math.cos(t * 13) * 0.25;
        T.armRz = 0.8 + Math.cos(t * 13 + 1) * 0.25;
        T.legLx = Math.sin(t * 12) * 0.7;
        T.legRx = -Math.sin(t * 12) * 0.7;
        T.legLz = 0.2;
        T.legRz = 0.2;
        T.lean = 0.1 + k * 0.15;
      } else {
        // Apex: arms out for balance, legs gathered.
        T.armLx = -1.6;
        T.armRx = -1.5;
        T.armLz = 1.0;
        T.armRz = 1.0;
        T.legLx = -0.5;
        T.legRx = -0.3;
        T.legLz = 0.15;
        T.legRz = 0.15;
        T.lean = 0.08;
      }
      T.roll = clamp(-side * 0.03, -0.25, 0.25);
      return T;
    }
    // Ground: run cycle scaled by speed, grab or idle on top.
    const run = smooth(speed, 0.4, 7);
    this.phase += ((speed * dt) / STRIDE) * Math.PI * 2;
    const s = Math.sin(this.phase);
    const c = Math.cos(this.phase);
    this.stepSide = s > 0 ? 1 : 0;
    const back = fwd < -0.5 ? -1 : 1;
    T.legLx = s * (0.25 + 0.8 * run) * back;
    T.legRx = -s * (0.25 + 0.8 * run) * back;
    T.armLx = -s * (0.2 + 0.85 * run) * back + 0.05;
    T.armRx = s * (0.2 + 0.85 * run) * back + 0.05;
    T.armLz = 0.2 + 0.2 * run;
    T.armRz = 0.2 + 0.2 * run;
    T.lean = 0.2 * run * back + clamp(aFwd * 0.012, -0.22, 0.3);
    // Two bobs per cycle, lowest at footfall; hips twist and roll with the step.
    T.lift = Math.abs(s) * 0.085 * run;
    T.twist = s * 0.12 * run;
    T.roll = c * 0.05 * run + clamp(-this.yawRate * speed * 0.012, -0.32, 0.32);
    if (Math.abs(side) > 1 && Math.abs(fwd) < 2) {
      // Strafing: side steps.
      T.legLz = 0.05 + Math.max(0, s) * 0.35;
      T.legRz = 0.05 + Math.max(0, -s) * 0.35;
      T.roll -= clamp(side * 0.02, -0.15, 0.15);
    }
    if (a === ANIM.grab) {
      T.armLx = -1.55;
      T.armRx = -1.55;
      T.armLz = 0.08;
      T.armRz = 0.08;
      T.lean = -0.18;
      return T;
    }
    if (speed > 1.2) {
      this.fidget = 0;
      return T;
    }
    if (this.emoteT > 0) return this.emotePose(T, t);
    // Idle: breathing, weight shifting, every now and then a look around or a stretch.
    const idle = 1 - smooth(speed, 0.2, 1.2);
    this.fidgetAt -= dt;
    if (this.fidgetAt < 0) {
      this.fidget = 1 + Math.floor(Math.random() * 3);
      this.fidgetAt = 5 + Math.random() * 7;
    }
    const since = 5 + 7 - this.fidgetAt;
    T.armLx = 0.12 + Math.sin(t * 1.9 + this.seed) * 0.05;
    T.armRx = 0.12 + Math.sin(t * 1.9 + this.seed + 0.5) * 0.05;
    T.roll += Math.sin(t * 0.9 + this.seed) * 0.04 * idle;
    T.lift += Math.sin(t * 2.6 + this.seed) * 0.006 * idle;
    if (this.fidget === 1 && since < 2) T.twist = Math.sin(since * Math.PI) * 0.45 * (Math.sin(this.seed) > 0 ? 1 : -1);
    else if (this.fidget === 2 && since < 1.4) {
      const k = Math.sin((since / 1.4) * Math.PI);
      T.armLx = -2.9 * k;
      T.armRx = -2.9 * k;
      T.armLz = 0.3;
      T.armRz = 0.3;
      T.lean = -0.15 * k;
    } else if (this.fidget === 3 && since < 1) {
      T.legRx = -0.4 * Math.sin(since * Math.PI);
      T.roll -= 0.1 * Math.sin(since * Math.PI);
    }
    return T;
  }

  private emotePose(T: Targets, t: number): Targets {
    const e = this.emoteT;
    if (this.emote === 1) {
      // Wave with one arm, the other on the hip, bouncing.
      T.armRx = -2.7;
      T.armRz = 0.6 + Math.sin(t * 14) * 0.45;
      T.armLx = 0.3;
      T.armLz = 0.9;
      T.lift = Math.abs(Math.sin(t * 7)) * 0.12;
      T.roll = Math.sin(t * 7) * 0.06;
    } else if (this.emote === 2) {
      // Dance: hips swing, arms up in turn, feet tapping.
      const b = Math.sin(t * 9);
      T.armLx = -2.4 + b * 0.5;
      T.armRx = -2.4 - b * 0.5;
      T.armLz = 0.5;
      T.armRz = 0.5;
      T.roll = b * 0.25;
      T.twist = Math.sin(t * 4.5) * 0.35;
      T.legLx = Math.max(0, b) * -0.6;
      T.legRx = Math.max(0, -b) * -0.6;
      T.lift = Math.abs(b) * 0.1;
    } else {
      // Laugh: rocking back, hands on the belly.
      T.lean = -0.35 + Math.sin(t * 18) * 0.06;
      T.armLx = -0.8;
      T.armRx = -0.8;
      T.armLz = -0.15;
      T.armRz = -0.15;
      T.lift = Math.abs(Math.sin(t * 9)) * 0.06;
      T.roll = Math.sin(t * 3) * 0.05;
    }
    // Ease out of the emote over the last moment.
    if (e < 0.3) T.lift *= e / 0.3;
    return T;
  }

  private podium(t: number): Targets {
    const T = this.base();
    if (this.pose === 'cheer') {
      const hop = Math.max(0, Math.sin(t * 5));
      T.armLx = -2.9 + Math.sin(t * 12) * 0.25;
      T.armRx = -2.9 + Math.sin(t * 12 + 1) * 0.25;
      T.armLz = 0.55 + Math.sin(t * 6) * 0.2;
      T.armRz = 0.55 + Math.sin(t * 6 + 1) * 0.2;
      T.lift = hop * 0.55;
      T.legLx = -hop * 0.5;
      T.legRx = -hop * 0.3;
      T.roll = Math.sin(t * 5) * 0.1;
    } else if (this.pose === 'clap') {
      const c = Math.abs(Math.sin(t * 9));
      T.armLx = -1.45;
      T.armRx = -1.45;
      T.armLz = -0.3 + c * 0.55;
      T.armRz = -0.3 + c * 0.55;
      T.lift = Math.abs(Math.sin(t * 4.5)) * 0.06;
      T.twist = Math.sin(t * 1.3) * 0.15;
    } else {
      T.lean = 0.42 + Math.sin(t * 1.5) * 0.05;
      T.armLx = 0.3;
      T.armRx = 0.3;
      T.armLz = 0.02;
      T.armRz = 0.02;
      T.roll = Math.sin(t * 0.9) * 0.08;
      T.k = 120;
      T.c = 14;
    }
    return T;
  }

  /** Arms stretching from the shoulders to the held bean. */
  private updateReach(target: THREE.Vector3 | null) {
    for (let i = 0; i < 2; i++) {
      const m = this.reach[i]!;
      if (!target) {
        m.visible = false;
        continue;
      }
      const side = i ? 1 : -1;
      // Shoulder in root space → world.
      _v.set(side * 0.5, 1.08, 0.05);
      this.root.localToWorld(_v);
      const end = target.clone();
      end.y += 0.9;
      end.x += side * 0.25 * Math.cos(this.root.rotation.y);
      end.z -= side * 0.25 * Math.sin(this.root.rotation.y);
      const dir = end.sub(_v);
      const len = dir.length();
      m.visible = len > 0.3;
      if (!m.visible) continue;
      // Position in root space (root is only translated and yawed).
      const local = this.root.worldToLocal(_v.clone());
      m.position.copy(local);
      const localDir = dir.normalize().applyAxisAngle(UP, -this.root.rotation.y);
      m.quaternion.setFromUnitVectors(UP, localDir);
      m.scale.set(1, len, 1);
    }
  }
}
