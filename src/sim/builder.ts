import * as THREE from 'three';
import { mulberry32, type Rng } from '../shared/rng';
import { Collider, type ColliderOpts, type Shape } from './physics';
import { type Mover, World } from './world';

export const MODEL_NAMES = ['bean', 'crown', 'hub', 'arm', 'hammer', 'hex', 'door', 'finish', 'bumper', 'cloud'] as const;
export type ModelName = (typeof MODEL_NAMES)[number];

export type Palette = readonly [string, string];

export const PAL = {
  blue: ['#7ccfff', '#9bdcff'],
  purple: ['#a98bff', '#bca4ff'],
  pink: ['#ff8cc8', '#ffa6d6'],
  yellow: ['#ffd84a', '#ffe47a'],
  green: ['#6fe08a', '#8ceaa2'],
  white: ['#f4f1ff', '#ffffff'],
  orange: ['#ff9f4a', '#ffb673'],
  red: ['#ff6070', '#ff8490'],
  teal: ['#39e0d0', '#6ff0e4'],
} as const satisfies Record<string, Palette>;

export type PrimKind = 'box' | 'cyl' | 'sphere';

/**
 * Rendering side of a map, implemented by the client. The server builds maps without it: the same
 * build code then only creates colliders and movers.
 */
export interface View {
  prim(kind: PrimKind, dims: readonly number[], material: THREE.Material): THREE.Mesh;
  /** Many copies of one primitive in one draw call (per-instance matrix and colour). */
  instanced(kind: PrimKind, dims: readonly number[], material: THREE.Material, count: number): THREE.InstancedMesh;
  material(pal: Palette | string, freq?: number): THREE.Material;
  pattern(c1: string, c2: string, freq?: number, dir?: [number, number], speed?: number): THREE.Material;
  plain(color: string, opts?: THREE.MeshStandardMaterialParameters): THREE.Material;
  model(name: ModelName): THREE.Object3D;
  /** Instanced copies of a model (for scenery like clouds); returns the group holding them. */
  scatter(name: ModelName, matrices: readonly THREE.Matrix4[], shadows?: boolean): THREE.Object3D;
  emojiTexture(emoji: string, bg: string): THREE.Texture;
  /** Materials, textures and geometries made for this map only; disposed with it. */
  own<T extends { dispose(): void }>(x: T): T;
  disposeOwned(): void;
}

export type Anim = (t: number, dt: number) => void;

export interface PrimOpts extends ColliderOpts {
  material?: THREE.Material | undefined;
  freq?: number;
  rot?: [number, number, number];
  castShadow?: boolean;
  parent?: THREE.Object3D;
  noCollide?: boolean;
  /** The primitive moves (a mover changes it): its collider is re-read every tick. */
  dynamic?: boolean;
  seg?: number;
}

export interface Prim {
  obj: THREE.Object3D;
  mesh: THREE.Mesh | null;
  col: Collider;
}

export class Builder {
  readonly group = new THREE.Group();
  readonly world = new World(this.group);
  readonly anims: Anim[] = [];
  readonly rng: Rng;

  constructor(
    readonly seed: number,
    readonly view: View | null,
  ) {
    this.rng = mulberry32(seed);
  }

  get server() {
    return this.view === null;
  }

  mat(pal: Palette | string, freq?: number): THREE.Material | undefined {
    return this.view?.material(pal, freq);
  }

  /** Collision-relevant motion: a pure function of time, run on server and client. */
  move(fn: Mover) {
    this.world.movers.push(fn);
  }

  /** Visual-only animation (client). */
  anim(fn: Anim) {
    if (this.view) this.anims.push(fn);
  }

  private place(obj: THREE.Object3D, x: number, y: number, z: number, opts: PrimOpts) {
    obj.position.set(x, y, z);
    if (opts.rot) obj.rotation.set(...opts.rot);
    if (obj instanceof THREE.Mesh) {
      obj.castShadow = opts.castShadow ?? true;
      obj.receiveShadow = true;
    }
    (opts.parent ?? this.group).add(obj);
  }

  private prim(kind: PrimKind, dims: readonly number[], pal: Palette | string, opts: PrimOpts, freq?: number): THREE.Object3D {
    const v = this.view;
    if (!v) return new THREE.Object3D();
    return v.prim(kind, dims, opts.material ?? v.material(pal, opts.freq ?? freq));
  }

  box(
    x: number,
    y: number,
    z: number,
    sx: number,
    sy: number,
    sz: number,
    pal: Palette | string = PAL.blue,
    opts: PrimOpts = {},
  ): Prim {
    const obj = this.prim('box', [sx, sy, sz], pal, opts);
    this.place(obj, x, y, z, opts);
    return this.result(obj, { type: 'box', hx: sx / 2, hy: sy / 2, hz: sz / 2 }, opts);
  }

  cyl(x: number, y: number, z: number, r: number, h: number, pal: Palette | string = PAL.purple, opts: PrimOpts = {}): Prim {
    const obj = this.prim('cyl', [r, h, opts.seg ?? 48], pal, opts);
    this.place(obj, x, y, z, opts);
    return this.result(obj, { type: 'cyl', r, hh: h / 2 }, opts);
  }

  sphere(x: number, y: number, z: number, r: number, pal: Palette | string = PAL.pink, opts: PrimOpts = {}): Prim {
    const obj = this.prim('sphere', [r], pal, opts, 0.6);
    this.place(obj, x, y, z, opts);
    return this.result(obj, { type: 'sphere', r }, opts);
  }

  private result(obj: THREE.Object3D, shape: Shape, opts: PrimOpts): Prim {
    const col = new Collider(obj, shape, { ...opts, isStatic: !opts.dynamic });
    if (opts.noCollide) col.enabled = false;
    else this.world.add(col);
    return { obj, mesh: obj instanceof THREE.Mesh ? obj : null, col };
  }

  collider(obj: THREE.Object3D, shape: Shape, opts: ColliderOpts = {}): Collider {
    return this.world.add(new Collider(obj, shape, opts));
  }

  anchor(x: number, y: number, z: number, parent: THREE.Object3D = this.group): THREE.Object3D {
    const o = new THREE.Object3D();
    o.position.set(x, y, z);
    parent.add(o);
    return o;
  }

  /** A model from the asset pack, or an empty object on the server. */
  model(name: ModelName, parent: THREE.Object3D = this.group): THREE.Object3D {
    const o = this.view ? this.view.model(name) : new THREE.Object3D();
    parent.add(o);
    return o;
  }

  ramp(
    x: number,
    z0: number,
    y0: number,
    z1: number,
    y1: number,
    width: number,
    pal: Palette | string = PAL.blue,
    thick = 1,
    opts: PrimOpts = {},
  ) {
    const ang = Math.atan2(y1 - y0, z1 - z0);
    const len = Math.hypot(z1 - z0, y1 - y0);
    return this.box(x, (y0 + y1) / 2 - thick / 2 / Math.cos(ang), (z0 + z1) / 2, width, thick, len, pal, {
      ...opts,
      rot: [-ang, 0, 0],
    });
  }

  rails(z0: number, z1: number, halfWidth: number, y = 0, pal: Palette | string = PAL.pink) {
    const len = Math.abs(z1 - z0);
    for (const s of [-1, 1]) this.box(s * (halfWidth + 0.4), y + 0.6, (z0 + z1) / 2, 0.8, 1.2, len, pal);
  }

  hub(x: number, y: number, z: number, scale = 1) {
    const h = this.model('hub');
    h.position.set(x, y, z);
    h.scale.setScalar(scale);
    this.collider(this.anchor(x, y + 1.6 * scale, z), { type: 'cyl', r: 1.1 * scale, hh: 1.6 * scale }, { isStatic: true });
  }

  rotor(x: number, y: number, z: number, len: number, count: number, angleFn: (t: number) => number, hit = 1) {
    const rotor = this.anchor(x, y, z);
    for (let k = 0; k < count; k++) {
      const pivot = new THREE.Object3D();
      pivot.rotation.y = (k / count) * Math.PI * 2;
      rotor.add(pivot);
      const arm = this.model('arm', pivot);
      arm.scale.set(len, 1, 1);
      this.collider(this.anchor(len / 2 + 0.3, 0, 0, pivot), { type: 'box', hx: len / 2 - 0.3, hy: 0.36, hz: 0.36 }, { hit });
    }
    this.move((t) => {
      rotor.rotation.y = angleFn(t);
    });
    return rotor;
  }

  bumper(x: number, y: number, z: number, s = 1, power = 13) {
    const b = this.model('bumper');
    b.position.set(x, y, z);
    b.scale.setScalar(s);
    this.collider(this.anchor(x, y + 0.95 * s, z), { type: 'cyl', r: 0.9 * s, hh: 0.9 * s }, { isStatic: true, bounce: power });
    return b;
  }

  hammer(x: number, y: number, z: number, speed: number, phase: number, amp = 1.05, withFrame = true) {
    if (withFrame) {
      for (const sx of [-4.4, 4.4]) this.box(x + sx, y - 3.6, z, 0.8, 8.4, 0.8, PAL.purple);
      this.box(x, y + 0.8, z, 9.6, 0.8, 1.2, PAL.purple);
    }
    const h = this.model('hammer');
    h.position.set(x, y, z);
    this.collider(this.anchor(0, -6, 0, h), { type: 'box', hx: 1.35, hy: 0.95, hz: 0.95 }, { hit: 0.9 });
    this.move((t) => {
      h.rotation.z = Math.sin(t * speed + phase) * amp;
    });
    return h;
  }

  pad(x: number, y: number, z: number, r = 1.4, power = 17) {
    this.cyl(x, y - 0.3, z, r + 0.2, 0.6, '#5a3fb8');
    const top = this.cyl(x, y + 0.05, z, r, 0.2, PAL.teal, { pad: power, freq: 1.2 });
    const base = top.obj.position.y;
    this.anim((t) => {
      top.obj.position.y = base + Math.max(0, Math.sin(t * 8)) * 0.04;
    });
    return top;
  }

  finish(x: number, y: number, z: number) {
    const f = this.model('finish');
    f.position.set(x, y, z);
    for (const sx of [-8.5, 8.5])
      this.collider(this.anchor(x + sx, y + 3, z), { type: 'cyl', r: 0.6, hh: 3 }, { isStatic: true });
  }

  /** Decorative clouds below the course (instanced; nothing on the server). */
  clouds(cx: number, cz: number, spread: number, n = 26, yMin = -30, yMax = -4) {
    const mats: THREE.Matrix4[] = [];
    const q = new THREE.Quaternion();
    const e = new THREE.Euler();
    for (let i = 0; i < n; i++) {
      const a = this.rng() * Math.PI * 2;
      const d = spread * (0.55 + this.rng() * 0.8);
      const p = new THREE.Vector3(cx + Math.cos(a) * d, yMin + this.rng() * (yMax - yMin), cz + Math.sin(a) * d * 1.2);
      const s = 1.5 + this.rng() * 3;
      q.setFromEuler(e.set(0, this.rng() * 6, 0));
      mats.push(new THREE.Matrix4().compose(p, q, new THREE.Vector3(s, s, s)));
    }
    if (this.view) this.group.add(this.view.scatter('cloud', mats, false));
  }

  /** Start pen with a gate that opens at t = 0; returns 8 spawn points. */
  startArea(z0 = 0): THREE.Vector3[] {
    this.box(0, -1, z0, 18, 2, 14, PAL.purple);
    this.box(-9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(0, 0.6, z0 - 7.4, 19.6, 1.2, 0.8, PAL.pink);
    const gateMat = this.view?.own(new THREE.MeshStandardMaterial({ color: '#ff5fa2', transparent: true, opacity: 0.35 }));
    const gate = this.box(0, 1.8, z0 + 7.1, 18, 3.6, 0.4, PAL.pink, { material: gateMat, castShadow: false });
    this.move((t) => {
      gate.col.enabled = t < 0;
      gate.obj.visible = t < 0;
    });
    return Array.from({ length: 8 }, (_, i) => new THREE.Vector3(-7 + i * 2, 0.05, z0 - 2));
  }

  ringSpawns(n: number, radius: number, y = 0.1, offset = 0): THREE.Vector3[] {
    return Array.from({ length: n }, (_, k) => {
      const a = (k / n) * Math.PI * 2 + offset;
      return new THREE.Vector3(Math.cos(a) * radius, y, Math.sin(a) * radius);
    });
  }

  dispose() {
    this.view?.disposeOwned();
    this.group.removeFromParent();
  }
}
