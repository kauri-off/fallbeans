import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { mulberry32, type Rng } from '../../shared/rng';
import { clone, sharedGeometries } from '../engine/assets';
import { Collider, type ColliderOpts, type Shape } from '../engine/physics';

const timeUniform = { value: 0 };
const matCache = new Map<string, THREE.Material>();

export function setMapTime(t: number) {
  timeUniform.value = t;
}

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

export function patternMaterial(c1: string, c2: string, freq = 0.25, dir: [number, number] = [1, 1], speed = 0): THREE.Material {
  const key = [c1, c2, freq, dir, speed].join('|');
  const cached = matCache.get(key);
  if (cached) return cached;
  const m = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.6 });
  m.onBeforeCompile = (sh) => {
    sh.uniforms.uC1 = { value: new THREE.Color(c1) };
    sh.uniforms.uC2 = { value: new THREE.Color(c2) };
    sh.uniforms.uF = { value: freq };
    sh.uniforms.uDir = { value: new THREE.Vector2(dir[0], dir[1]).normalize() };
    sh.uniforms.uSpeed = { value: speed };
    sh.uniforms.uTime = timeUniform;
    sh.vertexShader = sh.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vWP;')
      .replace('#include <project_vertex>', '#include <project_vertex>\nvWP = (modelMatrix * vec4(transformed, 1.0)).xyz;');
    sh.fragmentShader = sh.fragmentShader
      .replace(
        '#include <common>',
        '#include <common>\nvarying vec3 vWP; uniform vec3 uC1; uniform vec3 uC2; uniform float uF; uniform vec2 uDir; uniform float uSpeed; uniform float uTime;',
      )
      .replace(
        '#include <color_fragment>',
        '#include <color_fragment>\nfloat stp = smoothstep(0.46, 0.54, fract(dot(vWP.xz, uDir) * uF + uTime * uSpeed));\ndiffuseColor.rgb *= mix(uC1, uC2, stp);',
      );
  };
  m.customProgramCacheKey = () => key;
  matCache.set(key, m);
  return m;
}

export function plainMaterial(color: string, opts: THREE.MeshStandardMaterialParameters = {}): THREE.Material {
  const key = `p|${color}|${JSON.stringify(opts)}`;
  let m = matCache.get(key);
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.55, ...opts });
    matCache.set(key, m);
  }
  return m;
}

export type Updater = (t: number, dt: number) => void;

export interface PrimOpts extends ColliderOpts {
  material?: THREE.Material;
  freq?: number;
  rot?: [number, number, number];
  castShadow?: boolean;
  parent?: THREE.Object3D;
  noCollide?: boolean;
  dynamic?: boolean;
  seg?: number;
}

export interface Prim {
  mesh: THREE.Mesh;
  col: Collider;
}

export class Builder {
  readonly group = new THREE.Group();
  readonly colliders: Collider[] = [];
  readonly updaters: Updater[] = [];
  readonly rng: Rng;
  private readonly owned: THREE.Material[] = [];

  constructor(
    scene: THREE.Scene,
    readonly seed: number,
  ) {
    scene.add(this.group);
    this.rng = mulberry32(seed);
  }

  mat(pal: Palette | string, freq?: number): THREE.Material {
    return typeof pal === 'string' ? plainMaterial(pal) : patternMaterial(pal[0], pal[1], freq);
  }

  ownMaterial<M extends THREE.Material>(m: M): M {
    this.owned.push(m);
    return m;
  }

  update(fn: Updater) {
    this.updaters.push(fn);
  }

  private place(mesh: THREE.Mesh, x: number, y: number, z: number, opts: PrimOpts) {
    mesh.position.set(x, y, z);
    if (opts.rot) mesh.rotation.set(...opts.rot);
    mesh.castShadow = opts.castShadow ?? true;
    mesh.receiveShadow = true;
    (opts.parent ?? this.group).add(mesh);
  }

  box(x: number, y: number, z: number, sx: number, sy: number, sz: number, pal: Palette | string = PAL.blue, opts: PrimOpts = {}): Prim {
    const r = Math.min(0.25, sx / 4, sy / 4, sz / 4);
    const mesh = new THREE.Mesh(new RoundedBoxGeometry(sx, sy, sz, 2, r), opts.material ?? this.mat(pal, opts.freq));
    this.place(mesh, x, y, z, opts);
    return { mesh, col: this.attach(mesh, { type: 'box', hx: sx / 2, hy: sy / 2, hz: sz / 2 }, opts) };
  }

  cyl(x: number, y: number, z: number, r: number, h: number, pal: Palette | string = PAL.purple, opts: PrimOpts = {}): Prim {
    const mesh = new THREE.Mesh(new THREE.CylinderGeometry(r, r, h, opts.seg ?? 48), opts.material ?? this.mat(pal, opts.freq));
    this.place(mesh, x, y, z, opts);
    return { mesh, col: this.attach(mesh, { type: 'cyl', r, hh: h / 2 }, opts) };
  }

  sphere(x: number, y: number, z: number, r: number, pal: Palette | string = PAL.pink, opts: PrimOpts = {}): Prim {
    const mesh = new THREE.Mesh(new THREE.SphereGeometry(r, 32, 20), opts.material ?? this.mat(pal, opts.freq ?? 0.6));
    this.place(mesh, x, y, z, opts);
    return { mesh, col: this.attach(mesh, { type: 'sphere', r }, opts) };
  }

  private attach(obj: THREE.Object3D, shape: Shape, opts: PrimOpts): Collider {
    const col = new Collider(obj, shape, { isStatic: !opts.dynamic, ...opts });
    if (opts.noCollide) col.enabled = false;
    else this.colliders.push(col);
    return col;
  }

  collider(obj: THREE.Object3D, shape: Shape, opts: ColliderOpts = {}): Collider {
    const c = new Collider(obj, shape, opts);
    this.colliders.push(c);
    return c;
  }

  anchor(x: number, y: number, z: number, parent: THREE.Object3D = this.group): THREE.Object3D {
    const o = new THREE.Object3D();
    o.position.set(x, y, z);
    parent.add(o);
    return o;
  }

  ramp(x: number, z0: number, y0: number, z1: number, y1: number, width: number, pal: Palette | string = PAL.blue, thick = 1, opts: PrimOpts = {}) {
    const ang = Math.atan2(y1 - y0, z1 - z0);
    const len = Math.hypot(z1 - z0, y1 - y0);
    return this.box(x, (y0 + y1) / 2 - thick / 2 / Math.cos(ang), (z0 + z1) / 2, width, thick, len, pal, { ...opts, rot: [-ang, 0, 0] });
  }

  rails(z0: number, z1: number, halfWidth: number, y = 0, pal: Palette | string = PAL.pink) {
    const len = Math.abs(z1 - z0);
    for (const s of [-1, 1]) this.box(s * (halfWidth + 0.4), y + 0.6, (z0 + z1) / 2, 0.8, 1.2, len, pal);
  }

  hub(x: number, y: number, z: number, scale = 1) {
    const h = clone('hub');
    h.position.set(x, y, z);
    h.scale.setScalar(scale);
    this.group.add(h);
    this.collider(this.anchor(x, y + 1.6 * scale, z), { type: 'cyl', r: 1.1 * scale, hh: 1.6 * scale }, { isStatic: true });
  }

  rotor(x: number, y: number, z: number, len: number, count: number, angleFn: (t: number) => number, hit = 1) {
    const rotor = this.anchor(x, y, z);
    for (let k = 0; k < count; k++) {
      const pivot = new THREE.Object3D();
      pivot.rotation.y = (k / count) * Math.PI * 2;
      rotor.add(pivot);
      const arm = clone('arm');
      arm.scale.set(len, 1, 1);
      pivot.add(arm);
      this.collider(this.anchor(len / 2 + 0.3, 0, 0, pivot), { type: 'box', hx: len / 2 - 0.3, hy: 0.36, hz: 0.36 }, { hit });
    }
    this.update((t) => {
      rotor.rotation.y = angleFn(t);
    });
    return rotor;
  }

  bumper(x: number, y: number, z: number, s = 1, power = 13) {
    const b = clone('bumper');
    b.position.set(x, y, z);
    b.scale.setScalar(s);
    this.group.add(b);
    this.collider(this.anchor(x, y + 0.95 * s, z), { type: 'cyl', r: 0.9 * s, hh: 0.9 * s }, { isStatic: true, bounce: power });
    return b;
  }

  hammer(x: number, y: number, z: number, speed: number, phase: number, amp = 1.05, withFrame = true) {
    if (withFrame) {
      for (const sx of [-4.4, 4.4]) this.box(x + sx, y - 3.6, z, 0.8, 8.4, 0.8, PAL.purple);
      this.box(x, y + 0.8, z, 9.6, 0.8, 1.2, PAL.purple);
    }
    const h = clone('hammer');
    h.position.set(x, y, z);
    this.group.add(h);
    this.collider(this.anchor(0, -6, 0, h), { type: 'box', hx: 1.35, hy: 0.95, hz: 0.95 }, { hit: 0.9 });
    this.update((t) => {
      h.rotation.z = Math.sin(t * speed + phase) * amp;
    });
    return h;
  }

  pad(x: number, y: number, z: number, r = 1.4, power = 17) {
    this.cyl(x, y - 0.3, z, r + 0.2, 0.6, '#5a3fb8');
    const top = this.cyl(x, y + 0.05, z, r, 0.2, PAL.teal, { pad: power, freq: 1.2 });
    const base = top.mesh.position.y;
    this.update((t) => {
      top.mesh.position.y = base + Math.max(0, Math.sin(t * 8)) * 0.04;
    });
    return top;
  }

  finish(x: number, y: number, z: number) {
    const f = clone('finish');
    f.position.set(x, y, z);
    this.group.add(f);
    for (const sx of [-8.5, 8.5]) this.collider(this.anchor(x + sx, y + 3, z), { type: 'cyl', r: 0.6, hh: 3 }, { isStatic: true });
  }

  clouds(cx: number, cz: number, spread: number, n = 26, yMin = -30, yMax = -4) {
    for (let i = 0; i < n; i++) {
      const c = clone('cloud');
      const a = this.rng() * Math.PI * 2;
      const d = spread * (0.55 + this.rng() * 0.8);
      c.position.set(cx + Math.cos(a) * d, yMin + this.rng() * (yMax - yMin), cz + Math.sin(a) * d * 1.2);
      c.scale.setScalar(1.5 + this.rng() * 3);
      c.rotation.y = this.rng() * 6;
      c.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.castShadow = false;
          o.receiveShadow = false;
        }
      });
      this.group.add(c);
    }
  }

  startArea(z0 = 0): THREE.Vector3[] {
    this.box(0, -1, z0, 18, 2, 14, PAL.purple);
    this.box(-9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(0, 0.6, z0 - 7.4, 19.6, 1.2, 0.8, PAL.pink);
    const gate = this.box(0, 1.8, z0 + 7.1, 18, 3.6, 0.4, PAL.pink, {
      material: this.ownMaterial(new THREE.MeshStandardMaterial({ color: '#ff5fa2', transparent: true, opacity: 0.35 })),
      castShadow: false,
    });
    this.update((t) => {
      gate.col.enabled = t < 0;
      gate.mesh.visible = t < 0;
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
    this.group.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.InstancedMesh) {
        if (!sharedGeometries.has(o.geometry)) o.geometry.dispose();
      }
    });
    for (const m of this.owned) m.dispose();
    this.group.removeFromParent();
  }
}
