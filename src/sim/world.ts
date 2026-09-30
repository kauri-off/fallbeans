import type * as THREE from 'three';
import { type Collider, ColliderGrid, type CollisionWorld } from './physics';

/** Pure function of sim time: positions the moving parts of a map. Runs on server and client. */
export type Mover = (t: number) => void;

/**
 * The collision side of a built map. Static colliders live in a grid; moving ones are checked
 * individually. Its state is a pure function of time (plus authoritative map events), so the
 * server and every client agree on it without syncing geometry.
 */
export class World implements CollisionWorld {
  readonly colliders: Collider[] = [];
  readonly dynamic: Collider[] = [];
  readonly movers: Mover[] = [];
  readonly grid = new ColliderGrid(4);
  t = Number.NEGATIVE_INFINITY;
  private stamp = 1;
  private finalized = false;

  constructor(readonly root: THREE.Object3D) {}

  add(col: Collider): Collider {
    if (this.finalized) throw new Error('world is finalized');
    col.index = this.colliders.length;
    this.colliders.push(col);
    return col;
  }

  /** Call once after building: positions everything for time t and indexes static colliders. */
  finalize(t: number) {
    for (const m of this.movers) m(t);
    this.root.updateMatrixWorld(true);
    for (const c of this.colliders) {
      c.sync();
      if (c.isStatic) this.grid.insert(c);
      else this.dynamic.push(c);
    }
    this.t = t;
    this.finalized = true;
  }

  /** Moves the world to time t (previous matrices become the ones from the last call). */
  setTime(t: number) {
    for (const m of this.movers) m(t);
    for (const c of this.dynamic) c.sync();
    this.t = t;
  }

  /**
   * Fingerprint of the collision geometry (shapes, transforms, flags) at the current time. Server and
   * clients build maps separately; equal hashes at the same time mean they agree.
   */
  hash(staticOnly = false): string {
    let h = 2166136261;
    const mix = (v: number) => {
      h = Math.imul(h ^ (Math.round(v * 1e3) | 0), 16777619);
    };
    const list = staticOnly ? this.colliders.filter((c) => c.isStatic) : this.colliders;
    for (const c of list) {
      for (const e of c.cur.elements) mix(e);
      const s = c.shape;
      if (s.type === 'box') mix(s.hx + s.hy * 3 + s.hz * 7);
      else if (s.type === 'cyl') mix(s.r + s.hh * 3);
      else mix(s.r);
      mix(c.enabled ? 1 : 0);
      mix(c.hit + c.bounce * 3 + c.pad * 7 + c.slip * 11);
    }
    return `${list.length}:${(h >>> 0).toString(16).padStart(8, '0')}`;
  }

  query(x: number, z: number, r: number, out: Collider[]): Collider[] {
    out.length = 0;
    const stamp = ++this.stamp;
    this.grid.query(x, z, r, stamp, out);
    for (const c of this.dynamic) {
      if (c.stamp === stamp) continue;
      const dx = c.center.x - x;
      const dz = c.center.z - z;
      const reach = c.radius + r;
      if (dx * dx + dz * dz <= reach * reach) out.push(c);
    }
    return out;
  }
}
