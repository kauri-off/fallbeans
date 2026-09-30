import * as THREE from 'three';
import type { Collider } from './physics';
import type { World } from './world';

/**
 * Navigation for bots: a grid over the static part of a map with up to a few walkable layers per
 * cell (a bridge under a hammer frame, floors of a tower), linked by walking, stepping, jumping up,
 * dropping down and jumping gaps. A* finds routes on it; moving parts are left to the map's bot
 * logic (they are not in the grid).
 */

export const NAV_CELL = 0.5;
const LAYERS = 4;
const CLEAR = 1.7;
const STEP = 0.55;
const JUMP_UP = 1.85;
const DROP = 6;
/** Longest gap a running jump clears (m). */
const GAP = 3.2;

export interface NavPoint {
  x: number;
  y: number;
  z: number;
  /** Jump on the way to this point (a climb or a gap). */
  jump: boolean;
}

export interface PathOpts {
  /** Extra cost of standing at (x, z) (e.g. other beans in the way). */
  cost?: ((x: number, z: number) => number) | undefined;
  /** Goal radius (m). */
  radius?: number;
  maxNodes?: number;
}

interface Span {
  lo: number;
  hi: number;
  ny: number;
  col: number;
}

const _o = new THREE.Vector3();
const _d = new THREE.Vector3();
const _n = new THREE.Vector3();
const DOWN = new THREE.Vector3(0, -1, 0);
const near: Collider[] = [];

/** Where a downward ray enters and leaves a collider (world y), with the entry normal's y. */
function rayDown(c: Collider, x: number, z: number, top: number): Span | null {
  _o.set(x, top, z).applyMatrix4(c.inv);
  _d.copy(DOWN).transformDirection(c.inv);
  const s = c.shape;
  let t0 = -Infinity;
  let t1 = Infinity;
  _n.set(0, 0, 0);
  if (s.type === 'box') {
    const half = [s.hx, s.hy, s.hz];
    const o = [_o.x, _o.y, _o.z];
    const d = [_d.x, _d.y, _d.z];
    let axis = -1;
    let sign = 0;
    for (let i = 0; i < 3; i++) {
      const di = d[i]!;
      const oi = o[i]!;
      const h = half[i]!;
      if (Math.abs(di) < 1e-9) {
        if (oi < -h || oi > h) return null;
        continue;
      }
      let a = (-h - oi) / di;
      let b = (h - oi) / di;
      let sg = -1;
      if (a > b) {
        [a, b] = [b, a];
        sg = 1;
      }
      if (a > t0) {
        t0 = a;
        axis = i;
        sign = sg;
      }
      t1 = Math.min(t1, b);
      if (t0 > t1) return null;
    }
    if (axis < 0) return null;
    if (axis === 0) _n.set(sign, 0, 0);
    else if (axis === 1) _n.set(0, sign, 0);
    else _n.set(0, 0, sign);
  } else if (s.type === 'cyl') {
    // Caps (local y) and the side.
    let capA = -Infinity;
    let capB = Infinity;
    if (Math.abs(_d.y) < 1e-9) {
      if (Math.abs(_o.y) > s.hh) return null;
    } else {
      capA = (-s.hh - _o.y) / _d.y;
      capB = (s.hh - _o.y) / _d.y;
      if (capA > capB) [capA, capB] = [capB, capA];
    }
    const a = _d.x * _d.x + _d.z * _d.z;
    const b = 2 * (_o.x * _d.x + _o.z * _d.z);
    const cc = _o.x * _o.x + _o.z * _o.z - s.r * s.r;
    let sideA = -Infinity;
    let sideB = Infinity;
    if (a < 1e-9) {
      if (cc > 0) return null;
    } else {
      const disc = b * b - 4 * a * cc;
      if (disc < 0) return null;
      const q = Math.sqrt(disc);
      sideA = (-b - q) / (2 * a);
      sideB = (-b + q) / (2 * a);
    }
    t0 = Math.max(capA, sideA);
    t1 = Math.min(capB, sideB);
    if (t0 > t1) return null;
    if (capA >= sideA) _n.set(0, -Math.sign(_d.y) || 1, 0);
    else _n.set(_o.x + _d.x * t0, 0, _o.z + _d.z * t0).normalize();
  } else {
    const b = _o.dot(_d);
    const cc = _o.lengthSq() - s.r * s.r;
    const disc = b * b - cc;
    if (disc < 0) return null;
    const q = Math.sqrt(disc);
    t0 = -b - q;
    t1 = -b + q;
    _n.copy(_d).multiplyScalar(t0).add(_o).normalize();
  }
  if (t1 < 0) return null;
  _n.transformDirection(c.cur);
  return { hi: top - Math.max(0, t0), lo: top - t1, ny: _n.y, col: c.index };
}

class Heap {
  private ids: number[] = [];
  private keys: number[] = [];
  get size() {
    return this.ids.length;
  }
  clear() {
    this.ids.length = 0;
    this.keys.length = 0;
  }
  push(id: number, key: number) {
    const ids = this.ids;
    const keys = this.keys;
    let i = ids.length;
    ids.push(id);
    keys.push(key);
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (keys[p]! <= key) break;
      ids[i] = ids[p]!;
      keys[i] = keys[p]!;
      i = p;
    }
    ids[i] = id;
    keys[i] = key;
  }
  pop(): number {
    const ids = this.ids;
    const keys = this.keys;
    const top = ids[0]!;
    const lastId = ids.pop()!;
    const lastKey = keys.pop()!;
    const n = ids.length;
    if (n) {
      let i = 0;
      for (;;) {
        const l = i * 2 + 1;
        if (l >= n) break;
        const r = l + 1;
        const c = r < n && keys[r]! < keys[l]! ? r : l;
        if (keys[c]! >= lastKey) break;
        ids[i] = ids[c]!;
        keys[i] = keys[c]!;
        i = c;
      }
      ids[i] = lastId;
      keys[i] = lastKey;
    }
    return top;
  }
}

const DIRS: readonly [number, number][] = [
  [1, 0],
  [-1, 0],
  [0, 1],
  [0, -1],
  [1, 1],
  [1, -1],
  [-1, 1],
  [-1, -1],
];

export class NavGrid {
  readonly x0: number;
  readonly z0: number;
  readonly nx: number;
  readonly nz: number;
  /** Surface height per node (cell · LAYERS + layer), NaN where there is none. */
  private readonly ys: Float32Array;
  /** Collider carrying the node (it may disappear: hex tiles, fake bridge tiles). */
  private readonly cols: Int32Array;
  /** Extra cost of a node: walls and edges close by. */
  private readonly pen: Float32Array;
  private readonly g: Float32Array;
  private readonly from: Int32Array;
  private readonly how: Uint8Array;
  private readonly seen: Uint32Array;
  private gen = 0;
  private readonly heap = new Heap();

  private constructor(
    private readonly world: World,
    x0: number,
    z0: number,
    nx: number,
    nz: number,
  ) {
    this.x0 = x0;
    this.z0 = z0;
    this.nx = nx;
    this.nz = nz;
    const n = nx * nz * LAYERS;
    this.ys = new Float32Array(n).fill(Number.NaN);
    this.cols = new Int32Array(n).fill(-1);
    this.pen = new Float32Array(n);
    this.g = new Float32Array(n);
    this.from = new Int32Array(n);
    this.how = new Uint8Array(n);
    this.seen = new Uint32Array(n);
  }

  /** Builds the grid from the world's static, solid colliders as they are now. */
  static build(world: World, forbidden?: (p: THREE.Vector3) => boolean): NavGrid {
    const solid = world.colliders.filter((c) => c.isStatic && c.enabled && !c.navSkip && !c.trigger);
    let x0 = Infinity;
    let x1 = -Infinity;
    let z0 = Infinity;
    let z1 = -Infinity;
    let top = -Infinity;
    for (const c of solid) {
      const [ex, ez] = c.extentXZ();
      x0 = Math.min(x0, c.center.x - ex);
      x1 = Math.max(x1, c.center.x + ex);
      z0 = Math.min(z0, c.center.z - ez);
      z1 = Math.max(z1, c.center.z + ez);
      top = Math.max(top, c.center.y + c.radius);
    }
    if (!solid.length) {
      x0 = z0 = -1;
      x1 = z1 = 1;
      top = 1;
    }
    const nx = Math.max(1, Math.ceil((x1 - x0) / NAV_CELL) + 2);
    const nz = Math.max(1, Math.ceil((z1 - z0) / NAV_CELL) + 2);
    const nav = new NavGrid(world, x0 - NAV_CELL, z0 - NAV_CELL, nx, nz);
    const spans: Span[][] = new Array(nx * nz);
    const probe = new THREE.Vector3();
    for (let iz = 0; iz < nz; iz++)
      for (let ix = 0; ix < nx; ix++) {
        const x = nav.cx(ix);
        const z = nav.cz(iz);
        const list: Span[] = [];
        for (const c of world.query(x, z, NAV_CELL * 0.5, near)) {
          if (!c.isStatic || !c.enabled || c.navSkip || c.trigger) continue;
          const sp = rayDown(c, x, z, top + 5);
          if (sp) list.push(sp);
        }
        list.sort((a, b) => b.hi - a.hi);
        const cell = iz * nx + ix;
        spans[cell] = list;
        let layer = 0;
        for (const s of list) {
          if (layer >= LAYERS) break;
          // Too steep, bouncy or hazardous to stand on.
          const col = world.colliders[s.col]!;
          if (s.ny < 0.6 || col.bounce || col.hit) continue;
          const y = s.hi;
          // Buried inside, or no head room under something else.
          if (list.some((o) => o !== s && o.lo < y + CLEAR && o.hi > y + 0.05)) continue;
          const prev = layer > 0 ? nav.ys[cell * LAYERS + layer - 1]! : Number.NaN;
          if (!Number.isNaN(prev) && Math.abs(prev - y) < 0.3) continue;
          if (forbidden?.(probe.set(x, y + 0.05, z))) continue;
          nav.ys[cell * LAYERS + layer] = y;
          nav.cols[cell * LAYERS + layer] = s.col;
          layer++;
        }
      }
    // Penalties: walls within a bean's radius, and edges (a drop right next to it).
    for (let iz = 0; iz < nz; iz++)
      for (let ix = 0; ix < nx; ix++) {
        const cell = iz * nx + ix;
        for (let l = 0; l < LAYERS; l++) {
          const id = cell * LAYERS + l;
          const y = nav.ys[id]!;
          if (Number.isNaN(y)) break;
          let wall = 0;
          let edge = 0;
          for (const [dx, dz] of DIRS) {
            const jx = ix + dx;
            const jz = iz + dz;
            if (jx < 0 || jz < 0 || jx >= nx || jz >= nz) {
              edge++;
              continue;
            }
            const other = jz * nx + jx;
            if (spans[other]!.some((s) => s.lo < y + 1.4 && s.hi > y + 0.3)) wall++;
            if (nav.layerNear(other, y, STEP) < 0) edge++;
          }
          nav.pen[id] = (wall ? 1.5 : 0) + (edge ? 1 + edge * 0.25 : 0);
        }
      }
    // Second ring of edge caution: one cell further in.
    const pen2 = nav.pen.slice();
    for (let iz = 1; iz < nz - 1; iz++)
      for (let ix = 1; ix < nx - 1; ix++)
        for (let l = 0; l < LAYERS; l++) {
          const id = (iz * nx + ix) * LAYERS + l;
          const y = nav.ys[id]!;
          if (Number.isNaN(y) || pen2[id]! > 0) continue;
          for (const [dx, dz] of DIRS) {
            const o = nav.layerNear((iz + dz) * nx + ix + dx, y, STEP);
            if (o >= 0 && pen2[o]! >= 1) {
              nav.pen[id] = 0.4;
              break;
            }
          }
        }
    return nav;
  }

  cx(ix: number) {
    return this.x0 + (ix + 0.5) * NAV_CELL;
  }

  cz(iz: number) {
    return this.z0 + (iz + 0.5) * NAV_CELL;
  }

  private cellOf(x: number, z: number): number {
    const ix = Math.floor((x - this.x0) / NAV_CELL);
    const iz = Math.floor((z - this.z0) / NAV_CELL);
    if (ix < 0 || iz < 0 || ix >= this.nx || iz >= this.nz) return -1;
    return iz * this.nx + ix;
  }

  /** Node of `cell` whose surface is within `tol` of y (the closest), or −1. */
  private layerNear(cell: number, y: number, tol: number): number {
    if (cell < 0 || cell >= this.nx * this.nz) return -1;
    let best = -1;
    let bd = tol;
    for (let l = 0; l < LAYERS; l++) {
      const id = cell * LAYERS + l;
      const ly = this.ys[id]!;
      if (Number.isNaN(ly)) break;
      const d = Math.abs(ly - y);
      if (d <= bd && this.alive(id)) {
        bd = d;
        best = id;
      }
    }
    return best;
  }

  private alive(id: number) {
    const c = this.cols[id]!;
    return c >= 0 && (this.world.colliders[c]?.enabled ?? false);
  }

  /** Ground height under (x, z) near height y (within `tol`), or null. */
  groundAt(x: number, z: number, y: number, tol = 1.2): number | null {
    const id = this.layerNear(this.cellOf(x, z), y, tol);
    return id < 0 ? null : this.ys[id]!;
  }

  /** Is (x, z, y) on walkable ground and not right at an edge? */
  safe(x: number, z: number, y: number): boolean {
    const id = this.layerNear(this.cellOf(x, z), y, 0.8);
    return id >= 0 && this.pen[id]! < 1;
  }

  /** The ground a body falling at (x, z) from height y lands on: its height and whether it is safe. */
  floorBelow(x: number, z: number, y: number): { y: number; safe: boolean } | null {
    const id = this.layerBelow(this.cellOf(x, z), y);
    return id < 0 ? null : { y: this.ys[id]!, safe: this.pen[id]! < 1 };
  }

  /** The node a body at p stands on (or the nearest one close by). */
  private nodeAt(x: number, y: number, z: number): number {
    const cell = this.cellOf(x, z);
    let id = this.layerBelow(cell, y);
    if (id >= 0) return id;
    // Standing on something that is not in the grid (a moving platform), or right at an edge.
    const ix = Math.floor((x - this.x0) / NAV_CELL);
    const iz = Math.floor((z - this.z0) / NAV_CELL);
    let best = -1;
    let bd = Infinity;
    for (let r = 1; r <= 4 && best < 0; r++)
      for (let dz = -r; dz <= r; dz++)
        for (let dx = -r; dx <= r; dx++) {
          if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
          const jx = ix + dx;
          const jz = iz + dz;
          if (jx < 0 || jz < 0 || jx >= this.nx || jz >= this.nz) continue;
          id = this.layerBelow(jz * this.nx + jx, y);
          if (id < 0) continue;
          const d = dx * dx + dz * dz + (this.ys[id]! - y) ** 2;
          if (d < bd) {
            bd = d;
            best = id;
          }
        }
    return best;
  }

  /** Highest node of a cell at or a little above y (what a body at y stands on). */
  private layerBelow(cell: number, y: number): number {
    if (cell < 0) return -1;
    for (let l = 0; l < LAYERS; l++) {
      const id = cell * LAYERS + l;
      const ly = this.ys[id]!;
      if (Number.isNaN(ly)) break;
      if (ly <= y + 0.6 && this.alive(id)) return id;
    }
    return -1;
  }

  private point(id: number, jump: boolean): NavPoint {
    const cell = Math.floor(id / LAYERS);
    return { x: this.cx(cell % this.nx), y: this.ys[id]!, z: this.cz(Math.floor(cell / this.nx)), jump };
  }

  /**
   * A* from p to (tx, tz) (any layer; the one nearest ty when given). Returns a smoothed list of
   * points to run through (the first one is where to head now), or null when there is no route.
   */
  path(p: THREE.Vector3, tx: number, tz: number, ty: number | null, opts: PathOpts = {}): NavPoint[] | null {
    const start = this.nodeAt(p.x, p.y, p.z);
    if (start < 0) return null;
    const radius = Math.max(NAV_CELL, opts.radius ?? 0.8);
    const maxNodes = opts.maxNodes ?? 6000;
    const gen = ++this.gen;
    const heap = this.heap;
    heap.clear();
    const h = (id: number) => {
      const cell = Math.floor(id / LAYERS);
      const x = this.cx(cell % this.nx);
      const z = this.cz(Math.floor(cell / this.nx));
      return Math.hypot(x - tx, z - tz);
    };
    const isGoal = (id: number) => {
      if (h(id) > radius) return false;
      return ty === null || Math.abs(this.ys[id]! - ty) < 1.5;
    };
    this.seen[start] = gen;
    this.g[start] = 0;
    this.from[start] = -1;
    this.how[start] = 0;
    heap.push(start, h(start));
    let goal = -1;
    let best = start;
    let bestH = h(start);
    let expanded = 0;
    const tryEdge = (from: number, to: number, cost: number, jump: boolean) => {
      if (to < 0) return;
      let extra = 0;
      if (opts.cost) {
        const cell = Math.floor(to / LAYERS);
        extra = opts.cost(this.cx(cell % this.nx), this.cz(Math.floor(cell / this.nx)));
      }
      const g = this.g[from]! + cost + this.pen[to]! * 0.6 + extra;
      if (this.seen[to] === gen && g >= this.g[to]!) return;
      this.seen[to] = gen;
      this.g[to] = g;
      this.from[to] = from;
      this.how[to] = jump ? 1 : 0;
      heap.push(to, g + h(to));
    };
    while (heap.size && expanded++ < maxNodes) {
      const id = heap.pop();
      if (isGoal(id)) {
        goal = id;
        break;
      }
      const hid = h(id);
      if (hid < bestH) {
        bestH = hid;
        best = id;
      }
      const cell = Math.floor(id / LAYERS);
      const ix = cell % this.nx;
      const iz = Math.floor(cell / this.nx);
      const y = this.ys[id]!;
      let edge = false;
      for (const [dx, dz] of DIRS) {
        const jx = ix + dx;
        const jz = iz + dz;
        if (jx < 0 || jz < 0 || jx >= this.nx || jz >= this.nz) continue;
        const other = jz * this.nx + jx;
        const diag = dx !== 0 && dz !== 0;
        const dist = diag ? NAV_CELL * Math.SQRT2 : NAV_CELL;
        // No cutting corners past walls or holes.
        if (diag && (this.layerNear(iz * this.nx + jx, y, STEP) < 0 || this.layerNear(jz * this.nx + ix, y, STEP) < 0)) continue;
        let linked = false;
        for (let l = 0; l < LAYERS; l++) {
          const to = other * LAYERS + l;
          const ly = this.ys[to]!;
          if (Number.isNaN(ly)) break;
          if (!this.alive(to)) continue;
          const dh = ly - y;
          if (Math.abs(dh) <= STEP) {
            tryEdge(id, to, dist, false);
            linked = true;
          } else if (dh > STEP && dh <= JUMP_UP) tryEdge(id, to, dist + 2.5, true);
          else if (dh < -STEP && dh >= -DROP) tryEdge(id, to, dist + 0.6 - dh * 0.35, false);
        }
        if (!linked && !diag) edge = true;
      }
      // At an edge: jumps across gaps.
      if (edge) {
        for (const [dx, dz] of DIRS) {
          const step = dx !== 0 && dz !== 0 ? Math.SQRT2 : 1;
          for (let k = 2; k * step * NAV_CELL <= GAP; k++) {
            const jx = ix + dx * k;
            const jz = iz + dz * k;
            if (jx < 0 || jz < 0 || jx >= this.nx || jz >= this.nz) break;
            const other = jz * this.nx + jx;
            const land = this.layerNear(other, y - 0.6, 1.8);
            if (land >= 0 && this.ys[land]! <= y + 1.1) {
              if (k > 2) tryEdge(id, land, k * step * NAV_CELL + 3, true);
              break;
            }
            // Something solid in the way at jumping height: no gap jump this way.
            if (this.layerNear(other, y, JUMP_UP) >= 0) break;
          }
        }
      }
    }
    const end = goal >= 0 ? goal : best;
    if (end === start && goal < 0) return null;
    const raw: number[] = [];
    for (let id = end; id >= 0; id = this.from[id]!) raw.push(id);
    raw.reverse();
    return this.smooth(raw);
  }

  /** String pulling: skip nodes while the straight line stays on similar, unbroken ground. */
  private smooth(ids: number[]): NavPoint[] {
    const out: NavPoint[] = [];
    let i = 0;
    while (i < ids.length - 1) {
      let j = i + 1;
      while (j + 1 < ids.length && this.how[ids[j + 1]!] === 0 && this.straight(ids[i]!, ids[j + 1]!)) j++;
      out.push(this.point(ids[j]!, this.how[ids[j]!] === 1));
      i = j;
    }
    if (!out.length && ids.length) out.push(this.point(ids[0]!, false));
    return out;
  }

  private straight(a: number, b: number): boolean {
    const pa = this.point(a, false);
    const pb = this.point(b, false);
    const len = Math.hypot(pb.x - pa.x, pb.z - pa.z);
    const n = Math.ceil(len / (NAV_CELL * 0.5));
    let y = pa.y;
    for (let k = 1; k < n; k++) {
      const f = k / n;
      const id = this.layerNear(this.cellOf(pa.x + (pb.x - pa.x) * f, pa.z + (pb.z - pa.z) * f), y, STEP);
      if (id < 0 || this.pen[id]! >= 1.5) return false;
      y = this.ys[id]!;
    }
    return true;
  }
}
