import { MeshoptSimplifier } from 'meshoptimizer/simplifier';
import * as THREE from 'three';
import { canFade, fadeMaterial, lodFrame } from './materials';

/**
 * Client-only levels of detail M0 (full) … M3 (coarsest) for every mesh that has them. The level
 * follows the projected size on screen; near each switch both levels are drawn with complementary
 * dither masks (see materials.ts), which the temporal anti-aliasing blends into a smooth cross-fade.
 */

export const LOD_LEVELS = 4;
/** Screen-size thresholds (bounding radius / (distance · tan(fov/2))) where M0→M1, M1→M2, M2→M3. */
const THRESHOLDS = [0.14, 0.055, 0.022] as const;
/** Half width of each cross-fade band, as a ratio of the size (log scale). */
const BAND = 1.18;
/** Target share of the triangles kept at M1, M2, M3 (model meshes). */
const KEEP = [1, 0.45, 0.18, 0.06] as const;

const levelsOf = new WeakMap<THREE.BufferGeometry, THREE.BufferGeometry[]>();

/** Registers pre-built levels for a geometry (primitives build their own with fewer segments). */
export function setLevels(g: THREE.BufferGeometry, levels: THREE.BufferGeometry[]) {
  levelsOf.set(g, levels);
  for (const l of levels) levelsOf.set(l, levels);
}

export function levels(g: THREE.BufferGeometry): THREE.BufferGeometry[] | undefined {
  return levelsOf.get(g);
}

/** Geometry of a level (the full one when there are no levels). */
export function atLevel(g: THREE.BufferGeometry, level: number): THREE.BufferGeometry {
  return levelsOf.get(g)?.[level] ?? g;
}

export const simplifierReady = MeshoptSimplifier.ready;

/**
 * Simplified versions of an indexed mesh: new index buffers over the same vertex buffers (shared on
 * the GPU), so a level costs only its indices.
 */
export function simplifyLevels(g: THREE.BufferGeometry): THREE.BufferGeometry[] {
  const known = levelsOf.get(g);
  if (known) return known;
  const pos = g.attributes.position;
  const idx = g.index;
  const out: THREE.BufferGeometry[] = [g];
  if (!pos || !idx || idx.count < 240 || !MeshoptSimplifier.supported) {
    setLevels(g, [g, g, g, g]);
    return levelsOf.get(g)!;
  }
  const n = pos.count;
  const positions = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    positions[i * 3] = pos.getX(i);
    positions[i * 3 + 1] = pos.getY(i);
    positions[i * 3 + 2] = pos.getZ(i);
  }
  const nrm = g.attributes.normal;
  const normals = new Float32Array(n * 3);
  if (nrm) for (let i = 0; i < n; i++) normals.set([nrm.getX(i), nrm.getY(i), nrm.getZ(i)], i * 3);
  const indices = new Uint32Array(idx.count);
  for (let i = 0; i < idx.count; i++) indices[i] = idx.getX(i);
  let prev: Uint32Array = indices;
  for (let l = 1; l < LOD_LEVELS; l++) {
    const target = Math.max(36, Math.floor((indices.length * KEEP[l]!) / 3) * 3);
    let [res] = MeshoptSimplifier.simplifyWithAttributes(
      prev,
      positions,
      3,
      normals,
      3,
      [0.5, 0.5, 0.5],
      null,
      target,
      0.05 * l,
      ['Prune'],
    );
    // Some shapes stop short of the target (seams, small parts): the coarsest level may be sloppy.
    if (l === LOD_LEVELS - 1 && res.length > target * 1.6)
      [res] = MeshoptSimplifier.simplifySloppy(prev, positions, 3, null, target, 0.08);
    if (!res.length) res = prev;
    const lg = new THREE.BufferGeometry();
    for (const [name, attr] of Object.entries(g.attributes)) lg.setAttribute(name, attr);
    lg.setIndex(new THREE.BufferAttribute(res, 1));
    lg.boundingSphere = g.boundingSphere;
    lg.boundingBox = g.boundingBox;
    out.push(res === prev && l > 1 ? out[l - 1]! : lg);
    prev = res;
  }
  setLevels(g, out);
  return out;
}

interface Entry {
  mesh: THREE.Mesh;
  levels: THREE.BufferGeometry[];
  /** Draws the other level while cross-fading. */
  ghost: THREE.Mesh | null;
  base: THREE.Material | null;
  mainFade: { value: number };
  ghostFade: { value: number };
  mainMat: THREE.Material | null;
  ghostMat: THREE.Material | null;
  /** Fade copies per base material (the game may alternate materials, e.g. a warning blink). */
  pairs: Map<THREE.Material, [THREE.Material, THREE.Material]>;
  cast: boolean;
  level: number;
  /** Levels chosen for a whole model at once: its root and bounding radius (see register). */
  group: { root: THREE.Object3D; radius: number } | null;
}

const _s = new THREE.Sphere();
const _gp = new THREE.Vector3();
const _gs = new THREE.Vector3();

export interface LodStats {
  meshes: number;
  levels: number[];
  fading: number;
}

/** Keeps every registered mesh at the level its size on screen calls for. */
export class LodSystem {
  private readonly entries = new Map<THREE.Mesh, Entry>();
  private readonly owners = new Map<object, THREE.Mesh[]>();
  /** Multiplies the thresholds' distances (quality presets: further detail on high). */
  bias = 1;
  /** Debug: every mesh at this level (no fading), or null. */
  force: number | null = null;
  enabled = true;
  private frame = 0;
  readonly stats: LodStats = { meshes: 0, levels: [0, 0, 0, 0], fading: 0 };

  /**
   * Every eligible mesh under root, until drop(owner). With `groupRadius` the whole model changes
   * level together, by the size of a sphere of that radius around its middle (characters: small
   * parts like hands and feet would otherwise drop to coarse levels long before the body).
   */
  register(root: THREE.Object3D, owner: object = root, groupRadius?: number) {
    const group = groupRadius ? { root, radius: groupRadius } : null;
    const list = this.owners.get(owner) ?? [];
    root.traverse((o) => {
      if (!(o instanceof THREE.Mesh) || o instanceof THREE.InstancedMesh || o.userData.lodGhost || o.userData.noLod) return;
      if (this.entries.has(o) || Array.isArray(o.material)) return;
      const lv = levelsOf.get(o.geometry);
      if (!lv || lv[1] === lv[0]) return;
      this.entries.set(o, {
        mesh: o,
        levels: lv,
        ghost: null,
        base: null,
        mainFade: { value: 0 },
        ghostFade: { value: 0 },
        mainMat: null,
        ghostMat: null,
        pairs: new Map(),
        cast: o.castShadow,
        level: 0,
        group,
      });
      list.push(o);
    });
    this.owners.set(owner, list);
  }

  drop(owner: object) {
    for (const m of this.owners.get(owner) ?? []) {
      const e = this.entries.get(m);
      if (!e) continue;
      this.reset(e);
      e.ghost?.removeFromParent();
      for (const pair of e.pairs.values()) for (const x of pair) x.dispose();
      this.entries.delete(m);
    }
    this.owners.delete(owner);
  }

  /**
   * Level for something of bounding sphere (center, radius) seen from the camera, without a cross-fade
   * band (instances of a batch switch level whole: see statics.ts).
   */
  levelOf(center: THREE.Vector3, radius: number, camera: THREE.PerspectiveCamera): number {
    if (!this.enabled) return 0;
    if (this.force !== null) return this.force;
    const k = 1 / (Math.tan(THREE.MathUtils.degToRad(camera.fov) / 2) * this.bias);
    const size = (radius * k) / Math.max(0.01, center.distanceTo(camera.position));
    let lvl = 0;
    for (const t of THRESHOLDS) if (size < t) lvl++;
    return lvl;
  }

  /** Whether a mesh casts a (live) shadow; kept through level changes. */
  setCast(mesh: THREE.Mesh, cast: boolean) {
    const e = this.entries.get(mesh);
    if (e) e.cast = cast;
    mesh.castShadow = cast;
  }

  /** Back to the full mesh (LOD switched off). */
  private reset(e: Entry) {
    e.mesh.geometry = e.levels[0]!;
    if (e.base) e.mesh.material = e.base;
    e.mesh.castShadow = e.cast;
    if (e.ghost) e.ghost.visible = false;
    e.level = 0;
  }

  update(camera: THREE.PerspectiveCamera) {
    this.frame = (this.frame + 1) % 64;
    lodFrame.value = this.frame;
    const st = this.stats;
    st.meshes = this.entries.size;
    st.levels.fill(0);
    st.fading = 0;
    const k = 1 / (Math.tan(THREE.MathUtils.degToRad(camera.fov) / 2) * this.bias);
    const cam = camera.position;
    for (const e of this.entries.values()) {
      const m = e.mesh;
      // The game may swap materials (warning blinks): follow it.
      if (m.material !== e.mainMat && m.material !== e.base) e.base = m.material as THREE.Material;
      e.base ??= m.material as THREE.Material;
      if (!this.enabled) {
        if (e.level !== 0 || e.ghost?.visible) this.reset(e);
        continue;
      }
      let lvl: number;
      let fade = 0;
      if (this.force !== null) lvl = this.force;
      else {
        if (e.group) {
          const w = e.group.root.matrixWorld;
          _gs.setFromMatrixScale(w);
          _s.center.copy(_gp.set(0, e.group.radius, 0).applyMatrix4(w));
          _s.radius = e.group.radius * Math.max(_gs.x, _gs.y, _gs.z);
        } else {
          const g = e.levels[0]!;
          if (!g.boundingSphere) g.computeBoundingSphere();
          _s.copy(g.boundingSphere!).applyMatrix4(m.matrixWorld);
        }
        const size = (_s.radius * k) / Math.max(0.01, _s.center.distanceTo(cam));
        lvl = 0;
        for (const t of THRESHOLDS) if (size < t / BAND) lvl++;
        // Inside a band: fading from level lvl−1 (bigger) to lvl.
        for (let i = 0; i < THRESHOLDS.length; i++) {
          const t = THRESHOLDS[i]!;
          if (size < t * BAND && size >= t / BAND) {
            lvl = i + 1;
            fade = Math.log((t * BAND) / size) / Math.log(BAND * BAND);
          }
        }
      }
      const fadable = fade > 0.02 && fade < 0.98 && canFade(e.base);
      st.levels[lvl]!++;
      if (!fadable) {
        const g = e.levels[Math.min(LOD_LEVELS - 1, fade > 0 && fade < 0.5 ? lvl - 1 : lvl)]!;
        if (m.geometry !== g) m.geometry = g;
        if (m.material !== e.base) m.material = e.base;
        m.castShadow = e.cast;
        if (e.ghost?.visible) e.ghost.visible = false;
        continue;
      }
      st.fading++;
      // Main mesh: the bigger level fading out; ghost: the smaller one fading in.
      this.materials(e);
      m.geometry = e.levels[lvl - 1]!;
      m.material = e.mainMat!;
      e.mainFade.value = fade;
      const ghost = this.ghost(e);
      ghost.geometry = e.levels[lvl]!;
      ghost.material = e.ghostMat!;
      e.ghostFade.value = -fade;
      ghost.visible = true;
      // One shadow is enough.
      m.castShadow = e.cast && fade < 0.5;
      ghost.castShadow = e.cast && fade >= 0.5;
      ghost.receiveShadow = m.receiveShadow;
    }
  }

  private materials(e: Entry) {
    const base = e.base!;
    let pair = e.pairs.get(base);
    if (!pair || pair[0].userData.srcVersion !== base.version) {
      if (pair) for (const x of pair) x.dispose();
      pair = [fadeMaterial(base, e.mainFade), fadeMaterial(base, e.ghostFade)];
      e.pairs.set(base, pair);
    }
    // Colours that change after the copy was made (the rainbow bean) follow the original.
    if (base instanceof THREE.MeshStandardMaterial)
      for (const m of pair) if (m instanceof THREE.MeshStandardMaterial) m.color.copy(base.color);
    [e.mainMat, e.ghostMat] = pair;
  }

  private ghost(e: Entry): THREE.Mesh {
    if (!e.ghost) {
      const g = new THREE.Mesh(e.levels[1], e.ghostMat!);
      g.name = '__lod';
      g.userData.lodGhost = true;
      g.userData.cat = e.mesh.userData.cat;
      g.matrixAutoUpdate = false;
      e.mesh.add(g);
      e.ghost = g;
    }
    return e.ghost;
  }
}

/** The one LOD system (updated by the renderer every frame). */
export const lod = new LodSystem();
