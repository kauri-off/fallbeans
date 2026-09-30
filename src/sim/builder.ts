import * as THREE from 'three';
import { mulberry32, type Rng } from '../shared/rng';
import { CLASSIC, type Palette, type PalKey, type PatternKind, type ResolvedLook } from './looks';
import { Collider, type ColliderOpts, PORTAL_T, type Shape } from './physics';
import { type Mover, World } from './world';

export const MODEL_NAMES = [
  'bean',
  'crown',
  'hub',
  'arm',
  'hammer',
  'hex',
  'door',
  'finish',
  'bumper',
  'cloud',
  'tree',
  'pine',
  'flag',
  'cone',
  'star',
  'island',
  'mushroom',
  'glove',
  'fan',
] as const;
export type ModelName = (typeof MODEL_NAMES)[number];

export type { Palette, PatternKind };

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
} as const satisfies Record<PalKey, Palette>;

/** Which palette a PAL entry is (the look of a round repaints them). */
const PAL_KEY = new Map<Palette, PalKey>(Object.entries(PAL).map(([k, v]) => [v, k as PalKey]));

export type PrimKind = 'box' | 'cyl' | 'sphere';

/** Surface finish of a material (client: detail maps and base roughness). */
export type SurfaceKind =
  | 'plastic'
  | 'padded'
  | 'rubber'
  | 'metal'
  | 'fabric'
  | 'ice'
  | 'cloud'
  | 'gold'
  | 'wood'
  | 'glossy'
  | 'tile'
  | 'leaf'
  | 'grass'
  | 'rock'
  | 'cloth'
  | 'glass'
  | 'carpet';

/** Client-only decoration placed after the build, clear of everything solid. */
export interface SceneryRequest {
  cx: number;
  cz: number;
  spread: number;
  clouds: number;
  yMin: number;
  yMax: number;
}

/**
 * Rendering side of a map, implemented by the client. The server builds maps without it: the same
 * build code then only creates colliders and movers.
 */
export interface View {
  prim(kind: PrimKind, dims: readonly number[], material: THREE.Material): THREE.Mesh;
  /** Many copies of one primitive in one draw call (per-instance matrix and colour). */
  instanced(kind: PrimKind, dims: readonly number[], material: THREE.Material, count: number): THREE.InstancedMesh;
  material(pal: Palette | string, freq?: number, surface?: SurfaceKind, pattern?: PatternKind): THREE.Material;
  pattern(
    c1: string,
    c2: string,
    freq?: number,
    dir?: [number, number],
    speed?: number,
    surface?: SurfaceKind,
    kind?: PatternKind,
  ): THREE.Material;
  plain(color: string, opts?: THREE.MeshStandardMaterialParameters, surface?: SurfaceKind): THREE.Material;
  model(name: ModelName): THREE.Object3D;
  emojiTexture(emoji: string, bg: string): THREE.Texture;
  /** Materials, textures and geometries made for this map only; disposed with it. */
  own<T extends { dispose(): void }>(x: T): T;
  disposeOwned(): void;
}

export type Anim = (t: number, dt: number) => void;

/** How long (s) both ends of a portal stay shut after the traveller came out. */
export const PORTAL_CLOSED = 1;

/** The last trip through a pair of portals (the same on the server and on clients). */
export interface PortalPair {
  /** Sim time somebody went in, and at which end (0 or 1). */
  at: number;
  from: number;
  /** Both ends are shut until then. */
  closedUntil: number;
  /** How long (s) the ends stay shut after a traveller came out. */
  closeFor: number;
}

export interface PortalOpts {
  /** Only the first end takes beans in; the second only lets them out. */
  oneWay?: boolean;
  /** Beans come out with at least this speed (m/s; default 6), thrown up at `lift` m/s when given. */
  speed?: number;
  lift?: number;
  /** How long (s) the ends stay shut after a trip (default PORTAL_CLOSED). */
  closed?: number;
  /** Open only while this says so (a pure function of sim time): shut in between. */
  open?: (t: number) => boolean;
}

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
  /** Surface finish (default: padded for big floors, rubber for balls, plastic otherwise). */
  surface?: SurfaceKind;
  /** Pattern of a two-colour palette (default: the map's style). */
  pattern?: PatternKind;
}

export interface Prim {
  obj: THREE.Object3D;
  mesh: THREE.Mesh | null;
  col: Collider;
}

/** Rings flowing out, for the discs of one-way exits (client only: needs a canvas). */
function ringsTexture(color: string): THREE.Texture {
  const size = 256;
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d')!;
  const grad = g.createRadialGradient(size / 2, size / 2, 4, size / 2, size / 2, size / 2);
  grad.addColorStop(0, '#1a0f3d');
  grad.addColorStop(0.6, color);
  grad.addColorStop(1, '#ffffff');
  g.fillStyle = grad;
  g.beginPath();
  g.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  g.fill();
  g.strokeStyle = 'rgba(255,255,255,0.6)';
  g.lineWidth = 7;
  for (const r of [0.14, 0.28, 0.42]) {
    g.beginPath();
    g.arc(size / 2, size / 2, r * size, 0, Math.PI * 2);
    g.stroke();
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

/** A spiral for portal discs (client only: needs a canvas). */
function swirlTexture(color: string): THREE.Texture {
  const size = 256;
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d')!;
  const grad = g.createRadialGradient(size / 2, size / 2, 4, size / 2, size / 2, size / 2);
  grad.addColorStop(0, '#ffffff');
  grad.addColorStop(0.35, color);
  grad.addColorStop(1, '#1a0f3d');
  g.fillStyle = grad;
  g.beginPath();
  g.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  g.fill();
  g.strokeStyle = 'rgba(255,255,255,0.55)';
  g.lineWidth = 6;
  for (let arm = 0; arm < 4; arm++) {
    g.beginPath();
    for (let k = 0; k <= 60; k++) {
      const f = k / 60;
      const a = arm * (Math.PI / 2) + f * Math.PI * 2.2;
      const r = f * size * 0.48;
      const px = size / 2 + Math.cos(a) * r;
      const py = size / 2 + Math.sin(a) * r;
      if (k) g.lineTo(px, py);
      else g.moveTo(px, py);
    }
    g.stroke();
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

export class Builder {
  readonly group = new THREE.Group();
  readonly world = new World(this.group);
  readonly anims: Anim[] = [];
  readonly scenery: SceneryRequest[] = [];
  /** Candidate spots for bonuses (sim/bonus.ts picks a few of them per round). */
  readonly bonusSpots: { x: number; y: number; z: number }[] = [];
  readonly rng: Rng;
  /** Portal pairs in build order (their index is how the server names them to clients). */
  readonly portalPairs: PortalPair[] = [];
  /** Server: somebody went through portal pair `pair` (the arena tells the clients). */
  onPortal: ((pair: number, from: number, t: number) => void) | null = null;
  /** Look of the map (client only): the pattern its palette materials use by default. */
  readonly style: { pattern: PatternKind } = { pattern: 'stripes' };
  /** This round's look (sim/looks.ts): colours, light, sky, scenery. Visual only. */
  look: ResolvedLook = CLASSIC;

  constructor(
    readonly seed: number,
    readonly view: View | null,
  ) {
    this.rng = mulberry32(seed);
  }

  get server() {
    return this.view === null;
  }

  /** Sets the round's look (before the build): the palettes are repainted, the pattern is its pick. */
  setLook(look: ResolvedLook) {
    this.look = look;
    this.style.pattern = look.pattern;
  }

  /** A palette as this round's look paints it (hex colours are kept as they are). */
  pal<P extends Palette | string>(p: P): P | Palette {
    if (typeof p === 'string' || this.look.id === 'classic') return p;
    const k = PAL_KEY.get(p as Palette);
    return k ? this.look.palette[k] : p;
  }

  mat(pal: Palette | string, freq?: number): THREE.Material | undefined {
    return this.view?.material(this.pal(pal), freq);
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
    const [a = 1, b = 1, c = 1] = dims;
    const big = kind === 'box' ? a * c >= 30 && Math.min(a, c) >= 3 && b <= 3 : kind === 'cyl' ? a >= 4 && b <= 3 : false;
    const surface = opts.surface ?? (kind === 'sphere' ? 'rubber' : big ? 'padded' : 'plastic');
    return v.prim(
      kind,
      dims,
      opts.material ?? v.material(this.pal(pal), opts.freq ?? freq, surface, opts.pattern ?? this.style.pattern),
    );
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
      this.collider(
        this.anchor(len / 2 + 0.3, 0, 0, pivot),
        { type: 'box', hx: len / 2 - 0.3, hy: 0.36, hz: 0.36 },
        { hit, tag: 'rotor', sweep: true },
      );
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
    this.collider(
      this.anchor(x, y + 0.95 * s, z),
      { type: 'cyl', r: 0.9 * s, hh: 0.9 * s },
      { isStatic: true, bounce: power, tag: 'bumper' },
    );
    return b;
  }

  hammer(x: number, y: number, z: number, speed: number, phase: number, amp = 1.05, withFrame = true) {
    if (withFrame) {
      // Legs in front of and behind the swing plane: the head (1.9 m deep) passes between them.
      for (const sx of [-4.4, 4.4]) for (const sz of [-1.5, 1.5]) this.box(x + sx, y - 3.6, z + sz, 0.8, 8.4, 0.8, PAL.purple);
      for (const sx of [-4.4, 4.4]) this.box(x + sx, y + 0.8, z, 0.8, 0.8, 3.8, PAL.purple);
      this.box(x, y + 0.8, z, 9.6, 0.8, 1.2, PAL.purple);
    }
    const h = this.model('hammer');
    h.position.set(x, y, z);
    this.collider(this.anchor(0, -6, 0, h), { type: 'box', hx: 1.35, hy: 0.95, hz: 0.95 }, { hit: 0.9, tag: 'hammer' });
    this.move((t) => {
      h.rotation.z = Math.sin(t * speed + phase) * amp;
    });
    return h;
  }

  /** A launch pad: throws beans up (power, m/s), and along `launch` (horizontal m/s) when given. */
  pad(x: number, y: number, z: number, r = 1.4, power = 17, launch?: { x: number; z: number }) {
    // The rim stands a little proud of the floor: flush faces would flicker (z-fighting).
    this.cyl(x, y - 0.26, z, r + 0.2, 0.6, '#5a3fb8', { surface: 'rubber' });
    const top = this.cyl(x, y + 0.07, z, r, 0.2, launch ? PAL.orange : PAL.teal, {
      pad: power,
      freq: 1.2,
      surface: 'rubber',
      launch: launch ? new THREE.Vector3(launch.x, 0, launch.z) : null,
    });
    const base = top.obj.position.y;
    this.anim((t) => {
      top.obj.position.y = base + Math.max(0, Math.sin(t * 8)) * 0.04;
    });
    const v = this.view;
    if (v && launch) {
      // Chevrons on the pad: which way it throws.
      const chevron = new THREE.Shape();
      chevron.moveTo(0, 0.35);
      chevron.lineTo(0.55, -0.2);
      chevron.lineTo(0.3, -0.2);
      chevron.lineTo(0, 0.1);
      chevron.lineTo(-0.3, -0.2);
      chevron.lineTo(-0.55, -0.2);
      chevron.closePath();
      const geo = v.own(new THREE.ShapeGeometry(chevron));
      const mat = v.own(new THREE.MeshBasicMaterial({ color: '#fff6d0', toneMapped: false }));
      const holder = this.anchor(x, y + 0.18, z);
      holder.rotation.y = Math.atan2(launch.x, launch.z);
      for (const dz of [-0.35 * r, 0.1 * r]) {
        const m = new THREE.Mesh(geo, mat);
        m.rotation.x = -Math.PI / 2;
        m.position.z = dz;
        m.scale.setScalar(r * 0.7);
        holder.add(m);
      }
      holder.userData.dynamic = true;
      this.anim(() => {
        holder.position.y = top.obj.position.y + 0.11;
      });
    }
    return top;
  }

  /**
   * A bouncy mushroom standing at (x, y, z): its cap (2.1 × scale m wide, 1.9 × scale m up) throws
   * beans up at `power` m/s; the stem is solid.
   */
  mushroom(x: number, y: number, z: number, scale = 1.5, power = 17, tint?: string) {
    const m = this.model('mushroom');
    m.position.set(x, y, z);
    m.scale.setScalar(scale);
    m.rotation.y = (x * 1.7 + z * 0.9) % 6.283;
    this.collider(this.anchor(x, y + 0.6 * scale, z), { type: 'cyl', r: 0.42 * scale, hh: 0.6 * scale }, { isStatic: true });
    const cap = this.collider(
      this.anchor(x, y + 1.66 * scale, z),
      { type: 'cyl', r: 0.98 * scale, hh: 0.26 * scale },
      { isStatic: true, pad: power },
    );
    const v = this.view;
    if (v) {
      const capMesh = m.getObjectByName('MushCap');
      if (tint && capMesh instanceof THREE.Mesh)
        capMesh.material = v.plain(tint, { aoMap: (capMesh.material as THREE.MeshStandardMaterial).aoMap }, 'glossy');
      // The cap squashes a little now and then, like jelly.
      m.userData.dynamic = true;
      const ph = (x * 3.1 + z * 1.3) % 6.283;
      this.anim((t) => {
        const k = Math.max(0, Math.sin(t * 5 + ph)) ** 6;
        m.scale.set(scale * (1 + k * 0.05), scale * (1 - k * 0.06), scale * (1 + k * 0.05));
      });
    }
    return cap;
  }

  /**
   * A ladder up a wall: its foot at (x, y0, z) on the wall's face, up to y1 (the top of what it leans
   * on), the rungs facing `yaw` (away from the wall). Walk into it to climb; jump to leap off.
   */
  ladder(x: number, y0: number, z: number, y1: number, yaw = 0, color = '#ffb347') {
    const h = y1 - y0;
    const holder = this.anchor(x, y0, z);
    holder.rotation.y = yaw;
    this.collider(
      this.anchor(0, h / 2, 0.45, holder),
      { type: 'box', hx: 0.55, hy: h / 2, hz: 0.3 },
      { isStatic: true, ladder: true, navSkip: true },
    );
    if (!this.view) return;
    for (const sx of [-0.45, 0.45])
      this.box(sx, (h + 0.7) / 2, 0.14, 0.11, h + 0.7, 0.11, color, { parent: holder, noCollide: true, surface: 'wood' });
    const rungs = Math.max(2, Math.round(h / 0.38));
    for (let k = 1; k < rungs; k++)
      this.cyl(0, (k / rungs) * h, 0.14, 0.045, 0.9, color, {
        parent: holder,
        noCollide: true,
        surface: 'wood',
        rot: [0, 0, Math.PI / 2],
        seg: 8,
      });
  }

  /** A spot where a bonus may lie (on the ground at y). */
  bonus(x: number, y: number, z: number) {
    this.bonusSpots.push({ x, y, z });
  }

  /** A trampoline: a springy mat on a ring frame that throws beans up (power: m/s upwards). */
  trampoline(x: number, y: number, z: number, r = 1.8, power = 19) {
    const legs = 6;
    for (let k = 0; k < legs; k++) {
      const a = (k / legs) * Math.PI * 2;
      this.cyl(x + Math.cos(a) * (r + 0.1), y - 0.65, z + Math.sin(a) * (r + 0.1), 0.09, 1.2, '#39406b', {
        noCollide: true,
        surface: 'metal',
        seg: 10,
      });
    }
    // The rim stands proud of the floor and of anything painted on it (paths, rings up to 0.1 m).
    this.cyl(x, y - 0.03, z, r + 0.3, 0.3, PAL.orange, { surface: 'rubber' });
    const mat = this.view?.pattern('#2b3a8f', '#3f57c9', 1.4, [1, 0], 0, 'fabric', 'dots');
    const top = this.cyl(x, y + 0.14, z, r, 0.1, PAL.blue, { pad: power, material: mat, surface: 'fabric' });
    const base = top.obj.position.y;
    this.anim((t) => {
      // The mat sags and springs back: never below the rim (top y + 0.12), which would hide it.
      const k = Math.max(0, Math.sin(t * 6 + x)) ** 4;
      top.obj.position.y = base - k * 0.03;
      top.obj.scale.set(1, 1 - k * 0.25, 1);
    });
    return top;
  }

  /**
   * Two linked portals (rings standing up, facing yaw): running into either takes PORTAL_T, out of
   * sight, and comes out in front of the other, facing its way, with at least 6 m/s. Each trip
   * closes both ends (sashes shut, solid) until a second after the traveller is out; the end they
   * went in at flashes as it swallows them, the other as it lets them out. One-way: only `a` takes
   * beans in, `b` (rings flowing out, an arrow on the ground) only lets them out.
   */
  portal(
    a: { x: number; y: number; z: number; yaw: number },
    b: { x: number; y: number; z: number; yaw: number },
    color = '#a66bff',
    o: PortalOpts = {},
  ) {
    const k = this.portalPairs.length;
    const pair: PortalPair = { at: -1e9, from: 0, closedUntil: -1e9, closeFor: o.closed ?? PORTAL_CLOSED };
    this.portalPairs.push(pair);
    // (Not in the tick it closed: prediction may replay that tick, and must go through again.)
    const closed = (t: number) => (t > pair.at && t < pair.closedUntil) || (!!o.open && !o.open(t));
    const ends = [a, b];
    ends.forEach((e, i) => {
      const other = ends[1 - i]!;
      const exitOnly = !!o.oneWay && i === 1;
      const to = new THREE.Vector3(other.x + Math.sin(other.yaw) * 1.9, other.y + 0.05, other.z + Math.cos(other.yaw) * 1.9);
      const ring = this.anchor(e.x, e.y, e.z);
      ring.rotation.y = e.yaw;
      if (!exitOnly) {
        this.collider(
          this.anchor(0, 1.35, 0, ring),
          { type: 'box', hx: 1.05, hy: 1.25, hz: 0.3 },
          {
            isStatic: true,
            trigger: true,
            navSkip: true,
            onTouch: (_c, _n, body) => {
              const t = this.world.t;
              if (body.inPortal || closed(t)) return;
              this.portalUsed(k, i, t);
              body.enterPortal(to, other.yaw, o.speed, o.lift);
              this.onPortal?.(k, i, t);
            },
          },
        );
        // Shut: the sashes are a wall.
        const sash = this.collider(
          this.anchor(0, 1.35, 0, ring),
          { type: 'box', hx: 1.25, hy: 1.3, hz: 0.12 },
          { isStatic: true, navSkip: true },
        );
        sash.enabled = false;
        this.move((t) => {
          sash.enabled = closed(t);
        });
      }
      const v = this.view;
      if (!v) return;
      const frame = new THREE.Mesh(
        v.own(new THREE.TorusGeometry(1.35, 0.18, 14, 48)),
        v.plain(color, { roughness: 0.3, metalness: 0.3 }, 'glossy'),
      );
      frame.position.y = 1.4;
      frame.castShadow = true;
      ring.add(frame);
      const discMat = v.own(
        new THREE.MeshBasicMaterial({
          map: v.own(exitOnly ? ringsTexture(color) : swirlTexture(color)),
          transparent: true,
          opacity: 0.85,
          side: THREE.DoubleSide,
          depthWrite: false,
          toneMapped: false,
        }),
      );
      const disc = new THREE.Mesh(v.own(new THREE.CircleGeometry(1.2, 48)), discMat);
      disc.position.y = 1.4;
      ring.add(disc);
      for (const sx of [-1, 1])
        this.box(e.x + Math.cos(e.yaw) * sx * 1.35, e.y + 0.15, e.z - Math.sin(e.yaw) * sx * 1.35, 0.5, 0.3, 0.5, color, {
          noCollide: true,
        });
      if (exitOnly) {
        // An arrow on the ground: the way out (nobody goes in here).
        const arrow = new THREE.Shape();
        arrow.moveTo(0, 0.75);
        arrow.lineTo(0.6, 0);
        arrow.lineTo(0.22, 0);
        arrow.lineTo(0.22, -0.6);
        arrow.lineTo(-0.22, -0.6);
        arrow.lineTo(-0.22, 0);
        arrow.lineTo(-0.6, 0);
        arrow.closePath();
        const mark = new THREE.Mesh(
          v.own(new THREE.ShapeGeometry(arrow)),
          v.own(new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.85, depthWrite: false, toneMapped: false })),
        );
        mark.rotation.x = -Math.PI / 2;
        mark.position.set(0, 0.06, 1.3);
        ring.add(mark);
      }
      // Two half-disc sashes hinged at the rim, sliding shut towards the middle.
      const sashMat = v.plain(color, { roughness: 0.35, metalness: 0.45, side: THREE.DoubleSide }, 'metal');
      const sashes = [-1, 1].map((side) => {
        const g = v.own(new THREE.CircleGeometry(1.22, 32, side < 0 ? Math.PI / 2 : -Math.PI / 2, Math.PI));
        g.translate(-side * 1.22, 0, 0);
        const m = new THREE.Mesh(g, sashMat);
        m.position.set(side * 1.22, 1.4, 0);
        m.visible = false;
        m.userData.dynamic = true;
        ring.add(m);
        return m;
      });
      // Light: a flash that swallows (way in) or bursts out (way out), and a ring of light going out.
      const flashMat = v.own(
        new THREE.MeshBasicMaterial({
          color: '#fff6d0',
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
          toneMapped: false,
        }),
      );
      const flash = new THREE.Mesh(v.own(new THREE.SphereGeometry(1, 24, 16)), flashMat);
      flash.position.y = 1.4;
      flash.visible = false;
      flash.userData.dynamic = true;
      ring.add(flash);
      const waveMat = v.own(
        new THREE.MeshBasicMaterial({
          color,
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
          toneMapped: false,
        }),
      );
      const wave = new THREE.Mesh(v.own(new THREE.TorusGeometry(1.35, 0.1, 10, 48)), waveMat);
      wave.position.y = 1.4;
      wave.visible = false;
      wave.userData.dynamic = true;
      ring.add(wave);
      this.anim((t) => {
        disc.rotation.z = t * (i ? -2.2 : 2.2);
        const used = t < pair.at || t >= pair.closedUntil ? 0 : Math.min(1, (t - pair.at) / 0.15, (pair.closedUntil - t) / 0.2);
        const shut = exitOnly ? 0 : o.open && !o.open(t) ? 1 : used;
        for (const m of sashes) {
          m.visible = shut > 0.001;
          m.scale.x = Math.max(0.001, shut);
        }
        // How long ago somebody went in here, or came out here.
        const age = pair.from === i ? t - pair.at : t - pair.at - PORTAL_T;
        const into = pair.from === i;
        const span = into ? 0.4 : 0.6;
        const f = age >= 0 && age < span ? age / span : -1;
        flash.visible = f >= 0;
        wave.visible = f >= 0 && !into;
        if (f >= 0) {
          const out = 1 - (1 - f) ** 3;
          flash.scale.setScalar(into ? 0.2 + 2.4 * (1 - f) : 0.3 + 3.2 * out);
          flashMat.opacity = into ? 0.95 * (1 - f * f) : 1 - f;
          wave.scale.setScalar(1 + 1.4 * out);
          waveMat.opacity = 1 - f;
        }
        discMat.color.setScalar(1 + (f >= 0 ? 1.5 * (1 - f) : 0));
        disc.scale.setScalar((1 + Math.sin(t * 4 + i) * 0.03) * (1 - shut * 0.6));
      });
    });
    return pair;
  }

  /** A trip through portal pair `pair`, in at end `from` at time t: both ends shut till it is over. */
  portalUsed(pair: number, from: number, t: number) {
    const p = this.portalPairs[pair];
    if (!p || t < p.at) return;
    p.at = t;
    p.from = from;
    p.closedUntil = t + PORTAL_T + p.closeFor;
  }

  finish(x: number, y: number, z: number) {
    const f = this.model('finish');
    f.position.set(x, y, z);
    for (const sx of [-8.5, 8.5])
      this.collider(this.anchor(x + sx, y + 3, z), { type: 'cyl', r: 0.6, hh: 3 }, { isStatic: true });
    // Stars twirling over the arch; flags just past the line (finished beans have left the course).
    for (const sx of [-5, 0, 5]) this.prop('star', x + sx, y + 8.2 + (sx ? 0 : 0.6), z, { scale: sx ? 1.1 : 1.5 });
    for (const sx of [-7, 7])
      this.prop('flag', x + sx, y, z + 3, { tint: sx < 0 ? '#ffd23f' : '#4fdc6a', yaw: sx < 0 ? Math.PI : 0 });
  }

  /**
   * A decorative model (client only, no collision): trees, flags (their pennant sways, `tint` colours
   * it), cones, stars (spin and bob), fans (blades turn), mushrooms, islands.
   */
  prop(name: ModelName, x: number, y: number, z: number, o: { yaw?: number; scale?: number; tint?: string } = {}) {
    const v = this.view;
    if (!v) return null;
    const m = this.model(name);
    m.position.set(x, y, z);
    m.rotation.y = o.yaw ?? 0;
    m.scale.setScalar(o.scale ?? 1);
    m.userData.cat = 'decor';
    const ph = (x * 12.9898 + z * 78.233) % 6.283;
    if (name === 'flag') {
      const cloth = m.getObjectByName('Pennant');
      // The tinted pennant keeps the model's baked AO.
      if (cloth instanceof THREE.Mesh && o.tint)
        cloth.material = v.plain(o.tint, { aoMap: (cloth.material as THREE.MeshStandardMaterial).aoMap }, 'cloth');
      if (cloth)
        this.anim((t) => {
          cloth.rotation.y = Math.sin(t * 2.2 + ph) * 0.22 + Math.sin(t * 5.1 + ph * 2) * 0.05;
        });
    } else if (name === 'fan') {
      const blades = m.getObjectByName('FanBlades');
      if (blades)
        this.anim((t) => {
          blades.rotation.z = t * 6 + ph;
        });
    } else if (name === 'star') {
      this.anim((t) => {
        m.rotation.y = (o.yaw ?? 0) + t * 1.4 + ph;
        m.position.y = y + Math.sin(t * 1.8 + ph) * 0.2;
      });
    }
    return m;
  }

  /** Decorative clouds around the course (client only; placed clear of the course after the build). */
  clouds(cx: number, cz: number, spread: number, n = 26, yMin = -30, yMax = -4) {
    if (this.view) this.scenery.push({ cx, cz, spread, clouds: n, yMin, yMax });
  }

  /** Start pen with a gate that opens at t = 0; returns 8 spawn points. */
  startArea(z0 = 0): THREE.Vector3[] {
    this.box(0, -1, z0, 18, 2, 14, PAL.purple);
    this.box(-9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(0, 0.6, z0 - 7.4, 19.6, 1.2, 0.8, PAL.pink);
    const gateMat = this.view?.own(new THREE.MeshStandardMaterial({ color: '#ff5fa2', transparent: true, opacity: 0.35 }));
    const gate = this.box(0, 1.8, z0 + 7.1, 18, 3.6, 0.4, PAL.pink, { material: gateMat, castShadow: false });
    // Flags and cones by the gate (on the rails: nothing to trip over, out of the camera's way).
    this.prop('flag', -9.4, 1.2, z0 + 6.4, { tint: '#ff5fa2', yaw: Math.PI });
    this.prop('flag', 9.4, 1.2, z0 + 6.4, { tint: '#3fa9ff' });
    for (const sx of [-1, 1]) this.prop('cone', sx * 9.4, 1.2, z0 + 4.2, { scale: 0.9 });
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
