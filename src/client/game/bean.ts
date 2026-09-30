import * as THREE from 'three';
import { ANIM, RAINBOW } from '../../shared/consts';
import { shotMode } from '../state';
import { clone } from './assets';
import { type Expr, Face } from './face';
import { lod } from './lod';
import { applySurface } from './materials';

/** A colour three.js understands (the rainbow starts red and is recoloured every frame: tickRainbow). */
const baseColor = (color: string) => new THREE.Color(color === RAINBOW ? '#ff5f5f' : color);
const WHITE = new THREE.Color('#ffffff');
const hue = new THREE.Color();

/** The rainbow suit: its colour runs round the colour wheel (`t` in seconds). */
export function tickRainbow(t: number) {
  const body = bodyMats.get(RAINBOW);
  const belly = bellyMats.get(RAINBOW);
  if (!body && !belly) return;
  hue.setHSL((t * 0.15) % 1, 0.85, 0.6);
  body?.color.copy(hue);
  belly?.color.copy(hue).lerp(WHITE, 0.62);
}

const bodyMats = new Map<string, THREE.MeshStandardMaterial>();
/** Smooth, solid suit colour: soft plastic with a faint clearcoat, no surface texture; `ao` is the model's baked occlusion. */
function bodyMaterial(color: string, ao: THREE.Texture | null) {
  let m = bodyMats.get(color);
  if (!m) {
    m = new THREE.MeshPhysicalMaterial({
      aoMap: ao,
      color: baseColor(color),
      roughness: 0.5,
      clearcoat: 0.3,
      clearcoatRoughness: 0.35,
      sheen: 0.35,
      sheenRoughness: 0.6,
      sheenColor: new THREE.Color('#ffffff'),
    });
    applySurface(m, null);
    bodyMats.set(color, m);
  }
  return m;
}

const bellyMats = new Map<string, THREE.MeshStandardMaterial>();
/** The belly patch: the player colour, washed towards white. */
function bellyMaterial(color: string, ao: THREE.Texture | null) {
  let m = bellyMats.get(color);
  if (!m) {
    m = applySurface(
      new THREE.MeshStandardMaterial({ aoMap: ao, color: baseColor(color).lerp(WHITE, 0.62), roughness: 0.55 }),
      null,
    );
    bellyMats.set(color, m);
  }
  return m;
}

let tailProto: THREE.Object3D | null = null;
function makeTail(): THREE.Object3D {
  if (!tailProto) {
    const g = new THREE.Group();
    const mat = applySurface(new THREE.MeshStandardMaterial({ color: '#ff9f1c', roughness: 0.7 }), null);
    const tip = applySurface(new THREE.MeshStandardMaterial({ color: '#fff4d6', roughness: 0.8 }), null);
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
  /** World position of the bean being held (its feet), if any, and its size. */
  grabAt?: THREE.Vector3 | null;
  grabSize?: number;
  /** Size (1, or bigger while a giant). */
  size?: number;
  /** Bonus in effect (physics POWER), for the glow at the feet. */
  power?: number;
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
const _aim = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _q2 = new THREE.Quaternion();
const _axis = new THREE.Vector3();
const smooth = THREE.MathUtils.smoothstep;
const clamp = THREE.MathUtils.clamp;

/** Height of the tip-over pivot (the lower collision sphere). */
const PIVOT_Y = 0.5;
const CROWN_SCALE = 0.62;
/** Ground covered by one full run cycle (two steps), m. */
const STRIDE = 2.2;
/** Shoulders (model space) and the length from shoulder to hand. */
const SHOULDER_X = 0.5;
const SHOULDER_Y = 1.08;
const ARM_LEN = 0.5;

export class Bean {
  readonly root = new THREE.Group();
  /** Rotates around the lower body when tumbling. */
  private readonly pivot = new THREE.Group();
  readonly model: THREE.Object3D;
  private readonly limbs: Record<'ArmL' | 'ArmR' | 'LegL' | 'LegR', Limb>;
  private readonly hands: THREE.Object3D[];
  /** Stretchy arms (grabbing, reaching out): length scale of each arm. */
  private readonly stretch = [new Spring(), new Spring()];
  private readonly face: Face;
  /** A face shown for a moment whatever else happens (finishing, being knocked out…). */
  private reactExpr: Expr | null = null;
  private reactT = 0;
  private fallT = 0;
  private readonly grow = new Spring();
  private aura: THREE.Mesh | null = null;
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
    this.hands = ['HandL', 'HandR'].map((n) => this.model.getObjectByName(n) ?? new THREE.Object3D());
    for (const st of this.stretch) st.x = 1;
    this.face = new Face(this.model);
    this.grow.x = 1;
    // The visor carries the face: always at full detail, or the face patch would cut into it.
    this.model.traverse((o) => {
      if (o.name === 'Visor') o.userData.noLod = true;
    });
    this.setColor(color);
    this.name = name;
    lod.register(this.model, this, 0.95);
  }

  setColor(color: string) {
    this.color = color;
    // The model's baked AO (one map for all its materials) goes on the player's materials too.
    let ao: THREE.Texture | null = null;
    this.model.traverse((o) => {
      if (o instanceof THREE.Mesh) ao ??= (o.material as THREE.MeshStandardMaterial).aoMap ?? null;
    });
    const m = bodyMaterial(color, ao);
    const belly = bellyMaterial(color, ao);
    this.model.traverse((o) => {
      if (!(o instanceof THREE.Mesh) || o.userData.lodGhost) return;
      const name = (o.material as THREE.Material).name;
      if (name === 'Body') o.userData.part = 'body';
      if (name === 'Belly') o.userData.part = 'belly';
      if (o.userData.part === 'body') o.material = m;
      if (o.userData.part === 'belly') o.material = belly;
    });
  }

  setCrown(on: boolean) {
    if (on && !this.crown) {
      this.crown = clone('crown');
      // The band (radius 0.5 in the model) rests on the head where it is 0.31 m from the axis (y ≈ 1.49).
      this.crown.scale.setScalar(CROWN_SCALE);
      this.crown.position.set(0, 1.465, -0.01);
      this.crown.rotation.x = -0.06;
      this.model.add(this.crown);
      lod.register(this.crown, this.crown);
    } else if (!on && this.crown) {
      lod.drop(this.crown);
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

  /** Shows a face for a while (e.g. joy on finishing, tears when knocked out). */
  react(e: Expr, seconds: number) {
    this.reactExpr = e;
    this.reactT = seconds;
  }

  private pickExpr(a: number, f: BeanFrame, speed: number): Expr {
    if (this.reactT > 0 && this.reactExpr) return this.reactExpr;
    if (this.pose === 'cheer') return 'laugh';
    if (this.pose === 'clap') return 'grin';
    if (this.pose === 'sad') return 'cry';
    if (this.emoteT > 0 && speed < 1.2 && (a === ANIM.idle || a === ANIM.air)) {
      const byEmote: Record<number, Expr> = { 1: 'grin', 2: 'grin', 3: 'laugh', 4: 'cry', 5: 'scared' };
      return byEmote[this.emote] ?? 'smile';
    }
    if (a === ANIM.tumble) return 'scared';
    if (a === ANIM.stun) return 'dizzy';
    if (a === ANIM.getup) return 'strain';
    if (a === ANIM.grab || a === ANIM.reach) return 'strain';
    if (a === ANIM.dive || a === ANIM.slide) return 'determined';
    // A long fall: fright.
    if (a === ANIM.air && this.fallT > 0.45) return 'scared';
    if (a === ANIM.air && f.vel.y > 8) return 'surprised';
    if (speed > 6) return 'grin';
    return 'smile';
  }

  dispose() {
    if (this.crown) lod.drop(this.crown);
    lod.drop(this);
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
    this.reactT -= dt;
    this.airT = a === ANIM.air ? this.airT + dt : 0;
    this.fallT = a === ANIM.air && f.vel.y < -9 ? this.fallT + dt : 0;

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

    // Grabbing or reaching out: the arms point at the target and stretch to it.
    const reach = this.aimArms(T, f, a, t);
    for (let i = 0; i < 2; i++) {
      const k = this.stretch[i]!.step(reach[i]!, 260, 22, dt);
      const arm = i ? L.ArmR : L.ArmL;
      arm.o.scale.y = Math.max(0.6, k);
      this.hands[i]!.scale.y = 1 / Math.max(0.6, k);
    }

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
    // (Screenshots: no blinking, the same picture every time.)
    const blink = this.blinkAt < 0.12 && this.blinkAt > 0 && !shotMode.value ? 0.12 : 1;
    if (this.blinkAt < 0) this.blinkAt = 2 + Math.random() * 3.5;
    this.face.set(this.pickExpr(a, f, speed));
    this.face.update(dt, t, blink, a === ANIM.tumble ? 0.3 : 1);
    // Giants grow (and shrink back) with a wobble.
    const size = this.grow.step(f.size ?? 1, 90, 9, dt);
    this.root.scale.setScalar(Math.max(0.5, size));
    this.updateAura(f.power ?? 0, t);

    if (this.tail) {
      // Each segment follows the one before: a wagging, trailing tail.
      let seg = this.tail as THREE.Object3D;
      for (let i = 0; i < 5 && seg; i++) {
        seg.rotation.y = Math.sin(t * 8 - i * 0.7) * (0.12 + Math.min(0.25, speed * 0.03));
        seg.rotation.x = clamp(-fwd * 0.02, -0.3, 0.2) + (a === ANIM.air ? 0.15 : 0);
        seg = seg.children.find((c) => c.name.startsWith('tail')) as THREE.Object3D;
      }
    }
  }

  /**
   * Arm targets for grabbing (hands on the held bean) and reaching out (grasping ahead), written
   * into T as rotations; returns the stretch of each arm (1 = normal length).
   */
  private aimArms(T: Targets, f: BeanFrame, a: number, t: number): [number, number] {
    const grabbing = !!f.grabAt && (a === ANIM.grab || a === ANIM.reach);
    if (!grabbing && a !== ANIM.reach) return [1, 1];
    this.root.updateMatrixWorld();
    if (grabbing) {
      // Both hands on the near side of the held bean, about its middle.
      const hs = f.grabSize ?? 1;
      const at = f.grabAt!;
      _aim.subVectors(this.root.position, at).setY(0);
      const d = _aim.length();
      if (d > 1e-3) _aim.multiplyScalar((0.42 * hs) / d);
      _aim.add(at);
      _aim.y = at.y + 0.85 * hs;
      this.model.worldToLocal(_aim);
    } else {
      // Grasping at the air ahead, the hands opening and closing.
      _aim.set(0, 0.98, 0.62 + Math.abs(Math.sin(t * 9)) * 0.38);
    }
    const out: [number, number] = [1, 1];
    for (let i = 0; i < 2; i++) {
      const side = i ? 1 : -1;
      const limb = i ? this.limbs.ArmR : this.limbs.ArmL;
      _v.set(_aim.x + side * 0.2 - side * SHOULDER_X, _aim.y - SHOULDER_Y, _aim.z);
      const len = Math.max(0.2, _v.length());
      _v.divideScalar(len);
      // Euler XYZ that turns the arm (hanging along −y) to point along _v.
      const rz = Math.asin(clamp(_v.x, -1, 1));
      const rx = Math.atan2(-_v.z, -_v.y);
      if (i) {
        T.armRx = rx - limb.base.x;
        T.armRz = rz - limb.base.z;
      } else {
        T.armLx = rx - limb.base.x;
        T.armLz = limb.base.z - rz;
      }
      out[i] = clamp(len / ARM_LEN, 0.8, 7);
    }
    T.k = Math.max(T.k, 260);
    return out;
  }

  /** A soft glow at the feet while a bonus is in effect. */
  private updateAura(power: number, t: number) {
    if (!power) {
      if (this.aura) this.aura.visible = false;
      return;
    }
    if (!this.aura) {
      this.aura = new THREE.Mesh(
        new THREE.RingGeometry(0.45, 0.8, 40),
        new THREE.MeshBasicMaterial({
          transparent: true,
          opacity: 0.7,
          depthWrite: false,
          toneMapped: false,
          side: THREE.DoubleSide,
        }),
      );
      this.aura.rotation.x = -Math.PI / 2;
      this.aura.position.y = 0.05;
      this.aura.userData.noLod = true;
      this.root.add(this.aura);
    }
    const colors = ['#ffffff', '#ff6f91', '#58d68d', '#ffd23f'];
    (this.aura.material as THREE.MeshBasicMaterial).color.set(colors[power] ?? '#ffffff');
    this.aura.visible = true;
    this.aura.scale.setScalar(1 + Math.sin(t * 6) * 0.12);
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
      // Superman: arms stretched ahead, legs straight back. The body itself lies along the flight
      // (the physics tilt, applied at the pivot like the collision spheres).
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
      T.roll = clamp(-side * 0.04, -0.3, 0.3);
      T.k = 200;
      return T;
    }
    if (a === ANIM.climb || a === ANIM.climbOver) {
      // Hanging on and pulling up, then a knee over the edge and a push down. The phase comes from the
      // physics (guessing it from the motion flickered for others' beans on a poor connection).
      const pulling = a === ANIM.climb;
      T.k = 200;
      T.c = 16;
      if (pulling) {
        const scramble = Math.sin(t * 17);
        T.armLx = -2.75;
        T.armRx = -2.75;
        T.armLz = 0.32;
        T.armRz = 0.32;
        T.legLx = -0.5 + scramble * 0.45;
        T.legRx = -0.5 - scramble * 0.45;
        T.lean = 0.12;
      } else {
        T.armLx = -0.75;
        T.armRx = -0.75;
        T.armLz = 0.5;
        T.armRz = 0.5;
        T.legLx = -1.3;
        T.legRx = 0.35;
        T.lean = 0.5;
      }
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
      // Leaning into the flight: the faster forward and down, the more.
      if (vy < 0) T.lean += clamp(Math.atan2(Math.max(0, fwd), -vy + 4) * 0.7, 0, 0.45);
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
    // Long strides: each leg swings far, and lifts its knee on the way forward (the swing phase).
    const swing = 0.3 + 0.95 * run;
    const knee = 0.55 * run;
    T.legLx = s * swing * back - Math.max(0, -c) * knee;
    T.legRx = -s * swing * back - Math.max(0, c) * knee;
    T.legLz = 0.03 + 0.06 * run;
    T.legRz = 0.03 + 0.06 * run;
    T.armLx = -s * (0.3 + 1.05 * run) * back + 0.1 * run;
    T.armRx = s * (0.3 + 1.05 * run) * back + 0.1 * run;
    T.armLz = 0.22 + 0.28 * run;
    T.armRz = 0.22 + 0.28 * run;
    T.lean = 0.26 * run * back + clamp(aFwd * 0.012, -0.22, 0.3);
    // Two bounces per cycle: up in the flight of each stride, down as the legs pass; hips twist and roll.
    T.lift = (1 - Math.abs(c)) * 0.13 * run;
    T.twist = s * 0.2 * run;
    T.roll = c * 0.08 * run + clamp(-this.yawRate * speed * 0.012, -0.32, 0.32);
    if (Math.abs(side) > 1 && Math.abs(fwd) < 2) {
      // Strafing: side steps.
      T.legLz = 0.05 + Math.max(0, s) * 0.35;
      T.legRz = 0.05 + Math.max(0, -s) * 0.35;
      T.roll -= clamp(side * 0.02, -0.15, 0.15);
    }
    if (a === ANIM.reach) {
      T.armLx = -1.5 + Math.sin(t * 9) * 0.15;
      T.armRx = -1.5 - Math.sin(t * 9) * 0.15;
      T.armLz = 0.18;
      T.armRz = 0.18;
      T.lean = 0.15 + T.lean * 0.5;
      return T;
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
    if (this.fidgetAt < 0 && !shotMode.value) {
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
    } else if (this.emote === 4) {
      // Crying: slumped, hands rubbing the eyes, shoulders shaking.
      const sob = Math.sin(t * 14) * 0.06;
      T.lean = 0.3 + sob;
      T.armLx = -2.3 + sob;
      T.armRx = -2.3 - sob;
      T.armLz = -0.35;
      T.armRz = -0.35;
      T.lift = Math.abs(sob) * 0.3;
    } else if (this.emote === 5) {
      // Fright: arms thrown up, knees knocking, trembling.
      const tr = Math.sin(t * 30) * 0.05;
      T.armLx = -2.8 + tr;
      T.armRx = -2.8 - tr;
      T.armLz = 0.9;
      T.armRz = 0.9;
      T.lean = -0.2;
      T.legLz = -0.15 + tr;
      T.legRz = -0.15 - tr;
      T.roll = tr;
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
}
