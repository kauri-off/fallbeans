import * as THREE from 'three';
import { levels, lod } from './lod';

/**
 * The parts of a map that never move, found once when it is built: drawn as instances (identical
 * meshes of one area in one draw call, at their own level of detail) and casting a shadow that is
 * baked once (see shadowBake.ts) instead of being drawn into the shadow map every frame.
 *
 * Maps change some "static" things on events (a door bursts open, a tile drops, a pane shatters):
 * every other frame each one is checked against how it was, and one that moved, disappeared or
 * changed material goes back to being an ordinary mesh with a live shadow (and the bake is redone).
 */

/** Layer of meshes a batch draws for them (nothing renders it). */
const HIDDEN = 31;
/** Layer the shadow bake renders. */
export const BAKE_LAYER = 30;
/** Batches are per area (so that frustum culling still leaves out what is off screen). */
const CELL = 32;
/** Sample times (s) for finding what never moves. */
const SAMPLES = [0.37, 1.13, 2.71, 4.29, 6.83, 11.9, 17.3, 26.1, 41.7];

interface Member {
  mesh: THREE.Mesh;
  /** Transform relative to the map's root. */
  rel: THREE.Matrix4;
  center: THREE.Vector3;
  radius: number;
  level: number;
}

interface Batch {
  members: Member[];
  /** One instanced mesh per distinct level geometry, and the index of it for each LOD level. */
  meshes: THREE.InstancedMesh[];
  byLevel: number[];
  stale: boolean;
}

interface Tracked {
  mesh: THREE.Mesh;
  world: Float32Array;
  material: THREE.Material | THREE.Material[];
  cast: boolean;
  layers: number;
  batch: Batch | null;
}

const _s = new THREE.Sphere();

function shown(o: THREE.Object3D, root: THREE.Object3D): boolean {
  for (let x: THREE.Object3D | null = o; x; x = x.parent) {
    if (!x.visible) return false;
    if (x === root) return true;
  }
  return false;
}

function same(a: ArrayLike<number>, b: ArrayLike<number>) {
  for (let i = 0; i < 16; i++) if (Math.abs(a[i]! - b[i]!) > 1e-4) return false;
  return true;
}

export class Statics {
  private readonly tracked = new Map<THREE.Mesh, Tracked>();
  private batches: Batch[] = [];
  private readonly batchRoot = new THREE.Group();
  private frame = 0;
  /** Bumped whenever the set of static shadow casters changes (the bake is redone). */
  version = 0;

  constructor(private readonly root: THREE.Object3D) {
    this.batchRoot.name = 'static batches';
    this.batchRoot.userData.noLod = true;
  }

  /**
   * Finds the meshes that stay put at every sample time (`pose(t)` places everything for time t;
   * `restore()` puts it back), batches the identical ones, and takes all of them out of the live
   * shadow map. Call before registering the map's meshes for LOD.
   */
  prepare(pose: (t: number) => void, restore: () => void) {
    const root = this.root;
    const candidates: THREE.Mesh[] = [];
    root.updateMatrixWorld(true);
    root.traverse((o) => {
      if (!(o instanceof THREE.Mesh) || o instanceof THREE.InstancedMesh || o instanceof THREE.SkinnedMesh) return;
      if (o.userData.lodGhost || o.userData.dynamic || !shown(o, root)) return;
      candidates.push(o);
    });
    const base = new Map(candidates.map((m) => [m, Float32Array.from(m.matrixWorld.elements)]));
    const moving = new Set<THREE.Mesh>();
    for (const t of SAMPLES) {
      pose(t);
      root.updateMatrixWorld(true);
      for (const m of candidates)
        if (!moving.has(m) && (!shown(m, root) || !same(m.matrixWorld.elements, base.get(m)!))) moving.add(m);
    }
    restore();
    root.updateMatrixWorld(true);

    const groups = new Map<string, THREE.Mesh[]>();
    for (const m of candidates) {
      if (moving.has(m)) continue;
      const t: Tracked = {
        mesh: m,
        world: Float32Array.from(m.matrixWorld.elements),
        material: m.material,
        cast: m.castShadow,
        layers: m.layers.mask,
        batch: null,
      };
      this.tracked.set(m, t);
      m.castShadow = false;
      const mat = m.material;
      if (Array.isArray(mat) || mat.transparent || m.onBeforeRender !== THREE.Object3D.prototype.onBeforeRender) continue;
      const g = m.geometry;
      if (!g.boundingSphere) g.computeBoundingSphere();
      _s.copy(g.boundingSphere!).applyMatrix4(m.matrixWorld);
      const key = [
        g.id,
        mat.uuid,
        m.receiveShadow,
        m.renderOrder,
        m.frustumCulled,
        Math.floor(_s.center.x / CELL),
        Math.floor(_s.center.z / CELL),
      ].join('|');
      const list = groups.get(key) ?? [];
      list.push(m);
      groups.set(key, list);
    }
    const inv = new THREE.Matrix4().copy(root.matrixWorld).invert();
    for (const list of groups.values()) if (list.length >= 2) this.batches.push(this.batch(list, inv));
    if (this.batches.length) root.add(this.batchRoot);
    this.version++;
  }

  private batch(list: THREE.Mesh[], inv: THREE.Matrix4): Batch {
    const first = list[0]!;
    const g = first.geometry;
    const lv = levels(g) ?? [g];
    const distinct = [...new Set(lv)];
    const members: Member[] = list.map((mesh) => {
      _s.copy(g.boundingSphere!).applyMatrix4(mesh.matrixWorld);
      mesh.layers.set(HIDDEN);
      mesh.userData.noLod = true;
      return {
        mesh,
        rel: new THREE.Matrix4().multiplyMatrices(inv, mesh.matrixWorld),
        center: _s.center.clone(),
        radius: _s.radius,
        level: -1,
      };
    });
    const meshes = distinct.map((geo) => {
      const im = new THREE.InstancedMesh(geo, first.material, list.length);
      im.castShadow = false;
      im.receiveShadow = first.receiveShadow;
      im.renderOrder = first.renderOrder;
      im.frustumCulled = first.frustumCulled;
      im.userData.cat = first.userData.cat;
      im.userData.noLod = true;
      im.count = 0;
      this.batchRoot.add(im);
      return im;
    });
    const b: Batch = { members, meshes, byLevel: lv.map((x) => distinct.indexOf(x)), stale: true };
    for (const t of list) this.tracked.get(t)!.batch = b;
    return b;
  }

  /** Meshes casting a baked shadow now (visible, still static). */
  casters(): THREE.Mesh[] {
    const out: THREE.Mesh[] = [];
    for (const t of this.tracked.values()) if (t.cast) out.push(t.mesh);
    return out;
  }

  /** Every frame: picks up static things that moved after all, and keeps batches at their levels of detail. */
  update(camera: THREE.PerspectiveCamera) {
    this.frame++;
    if (this.frame % 2 === 0) this.check();
    const relevel = this.frame % 8 === 0;
    for (const b of this.batches) {
      if (relevel || b.stale)
        for (const m of b.members) {
          const l = lod.levelOf(m.center, m.radius, camera);
          if (l !== m.level) {
            m.level = l;
            b.stale = true;
          }
        }
      if (b.stale) this.fill(b);
    }
  }

  private fill(b: Batch) {
    b.stale = false;
    const counts = b.meshes.map(() => 0);
    for (const m of b.members) {
      const i = b.byLevel[Math.max(0, m.level)] ?? 0;
      b.meshes[i]!.setMatrixAt(counts[i]!++, m.rel);
    }
    b.meshes.forEach((im, i) => {
      im.count = counts[i]!;
      im.visible = im.count > 0;
      im.instanceMatrix.needsUpdate = true;
      if (im.count) im.computeBoundingSphere();
    });
  }

  private check() {
    const root = this.root;
    for (const t of this.tracked.values()) {
      const m = t.mesh;
      const gone = !m.parent || !shown(m, root);
      const moved = !gone && !same(m.matrixWorld.elements, t.world);
      const changed = !!t.batch && m.material !== t.material;
      if (gone || moved || changed) this.release(t, gone);
    }
  }

  /** Back to an ordinary mesh: drawn by itself, with a live shadow. */
  private release(t: Tracked, gone: boolean) {
    const m = t.mesh;
    this.tracked.delete(m);
    const b = t.batch;
    if (b) {
      b.members = b.members.filter((x) => x.mesh !== m);
      b.stale = true;
      m.layers.mask = t.layers;
      m.userData.noLod = false;
      if (!gone) lod.register(m, this.root);
    }
    lod.setCast(m, t.cast);
    if (t.cast) this.version++;
  }

  /** Numbers for the debug overlay and profiler. */
  get stats() {
    let instances = 0;
    let draws = 0;
    for (const b of this.batches) {
      instances += b.members.length;
      for (const im of b.meshes) if (im.count) draws++;
    }
    return { static: this.tracked.size, batches: this.batches.length, instances, draws };
  }

  dispose() {
    for (const b of this.batches) for (const im of b.meshes) im.dispose();
    this.batchRoot.removeFromParent();
    for (const t of this.tracked.values()) if (t.batch) t.mesh.layers.mask = t.layers;
    this.tracked.clear();
    this.batches = [];
  }
}
