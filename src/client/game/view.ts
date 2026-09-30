import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import type { ModelName, Palette, PatternKind, PrimKind, SurfaceKind, View } from '../../sim/builder';
import { clone } from './assets';
import { LOD_LEVELS, setLevels } from './lod';
import { applySurface } from './materials';

export { timeUniform } from './materials';

const geoCache = new Map<string, THREE.BufferGeometry>();
const matCache = new Map<string, THREE.Material>();
/** Overlapping primitives often share a top face: each gets one of a few millimetre lifts so they never z-fight. */
const LIFTS = 6;
const LIFT = 0.0025;

/** Segment counts per LOD level (M0…M6). */
const SPHERE_SEG = [
  [32, 20],
  [26, 16],
  [20, 12],
  [16, 10],
  [12, 8],
  [10, 7],
  [8, 6],
] as const;
const CYL_SEG = [48, 36, 24, 18, 14, 10, 8] as const;
const BOX_SEG = [2, 1, 1, 1, 0, 0, 0] as const;
/** Rounding radius per level: flatter bevels before the plain box. */
const BOX_ROUND = [1, 1, 0.75, 0.5, 0, 0, 0] as const;

function cylSeg(dims: readonly number[], level: number): number {
  const seg = dims[2] ?? 48;
  return Math.min(seg, Math.max(Math.min(seg, 8), CYL_SEG[level]!));
}

function build(kind: PrimKind, dims: readonly number[], level: number): THREE.BufferGeometry {
  if (kind === 'box') {
    const [sx = 1, sy = 1, sz = 1] = dims;
    const seg = BOX_SEG[level]!;
    const r = Math.min(0.25, sx / 4, sy / 4, sz / 4);
    return seg ? new RoundedBoxGeometry(sx, sy, sz, seg, r * BOX_ROUND[level]!) : new THREE.BoxGeometry(sx, sy, sz);
  }
  if (kind === 'cyl') {
    const [r = 1, h = 1] = dims;
    return new THREE.CylinderGeometry(r, r, h, cylSeg(dims, level));
  }
  const [r = 1] = dims;
  const [w, hs] = SPHERE_SEG[level]!;
  return new THREE.SphereGeometry(r, w, hs);
}

function geometry(kind: PrimKind, dims: readonly number[], lift: number): THREE.BufferGeometry {
  const key = `${kind}|${dims.map((d) => d.toFixed(3)).join('|')}|${lift}`;
  let g = geoCache.get(key);
  if (!g) {
    const built = new Map<string, THREE.BufferGeometry>();
    const lv = Array.from({ length: LOD_LEVELS }, (_, l) => {
      // Levels that come out the same share one geometry (no pointless cross-fades).
      const id = kind === 'box' ? `${BOX_SEG[l]}|${BOX_ROUND[l]}` : kind === 'cyl' ? `${cylSeg(dims, l)}` : `${SPHERE_SEG[l]}`;
      const known = built.get(id);
      if (known) return known;
      const x = build(kind, dims, l);
      built.set(id, x);
      if (lift) x.translate(0, lift * LIFT, 0);
      x.computeBoundingSphere();
      return x;
    });
    g = lv[0]!;
    setLevels(g, lv);
    geoCache.set(key, g);
  }
  return g;
}

export function patternMaterial(
  c1: string,
  c2: string,
  freq = 0.25,
  dir: [number, number] = [1, 1],
  speed = 0,
  surface: SurfaceKind = 'plastic',
  kind: PatternKind = 'stripes',
): THREE.Material {
  const key = ['pat', c1, c2, freq, dir, speed, surface, kind].join('|');
  let m = matCache.get(key);
  if (!m) {
    m = applySurface(new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.5, metalness: 0 }), surface, {
      pattern: { c1, c2, freq, dir, speed, kind },
    });
    matCache.set(key, m);
  }
  return m;
}

export function plainMaterial(
  color: string,
  opts: THREE.MeshStandardMaterialParameters = {},
  surface: SurfaceKind = 'plastic',
): THREE.Material {
  const key = `plain|${color}|${JSON.stringify(opts)}|${surface}`;
  let m = matCache.get(key);
  if (!m) {
    m = applySurface(new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.5, ...opts }), surface, {
      keepRoughness: opts.roughness !== undefined,
    });
    matCache.set(key, m);
  }
  return m;
}

export function emojiTexture(emoji: string, bg: string, size = 256): THREE.CanvasTexture {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d')!;
  g.fillStyle = bg;
  g.fillRect(0, 0, size, size);
  g.strokeStyle = 'rgba(255,255,255,0.85)';
  g.lineWidth = size * 0.04;
  g.strokeRect(size * 0.04, size * 0.04, size * 0.92, size * 0.92);
  if (emoji) {
    g.font = `${Math.floor(size * 0.62)}px "Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji", sans-serif`;
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    g.fillText(emoji, size / 2, size * 0.54);
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 8;
  tex.generateMipmaps = true;
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  return tex;
}

/** The client's rendering side of map building (see sim/builder.ts View). */
export class ClientView implements View {
  private owned: { dispose(): void }[] = [];
  private seq = 0;

  prim(kind: PrimKind, dims: readonly number[], material: THREE.Material): THREE.Mesh {
    return new THREE.Mesh(geometry(kind, dims, 1 + (this.seq++ % LIFTS)), material);
  }

  instanced(kind: PrimKind, dims: readonly number[], material: THREE.Material, count: number): THREE.InstancedMesh {
    const m = new THREE.InstancedMesh(geometry(kind, dims, 0), material, count);
    m.castShadow = true;
    m.receiveShadow = true;
    m.frustumCulled = false;
    m.setColorAt(0, new THREE.Color('#ffffff'));
    this.owned.push({ dispose: () => m.dispose() });
    return m;
  }

  material(pal: Palette | string, freq?: number, surface?: SurfaceKind, kind?: PatternKind): THREE.Material {
    return typeof pal === 'string'
      ? plainMaterial(pal, {}, surface)
      : patternMaterial(pal[0], pal[1], freq, undefined, 0, surface, kind);
  }

  pattern(
    c1: string,
    c2: string,
    freq?: number,
    dir?: [number, number],
    speed?: number,
    surface?: SurfaceKind,
    kind?: PatternKind,
  ): THREE.Material {
    return patternMaterial(c1, c2, freq, dir, speed, surface, kind);
  }

  plain(color: string, opts?: THREE.MeshStandardMaterialParameters, surface?: SurfaceKind): THREE.Material {
    return plainMaterial(color, opts, surface);
  }

  model(name: ModelName): THREE.Object3D {
    return clone(name);
  }

  emojiTexture(emoji: string, bg: string): THREE.Texture {
    return emojiTexture(emoji, bg);
  }

  own<T extends { dispose(): void }>(x: T): T {
    this.owned.push(x);
    return x;
  }

  disposeOwned() {
    for (const o of this.owned) o.dispose();
    this.owned = [];
  }
}
