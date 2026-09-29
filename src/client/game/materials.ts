import * as THREE from 'three';
import type { SurfaceKind } from '../../sim/builder';

/**
 * Surface detail for every material: a procedural, tileable texture per surface kind holding a
 * normal (RG, derived from a height map), the height itself (B, cavity shading) and a roughness
 * mask (A). It is applied by object-space triplanar mapping (the models have no UVs), so detail
 * keeps its size on any primitive and stays glued to moving parts.
 */

/** Shared by every pattern material: drives animated stripes (conveyors). */
export const timeUniform = { value: 0 };

interface SurfaceDef {
  /** Texture tiles per metre. */
  scale: number;
  /** Normal perturbation strength. */
  normal: number;
  /** Roughness modulation (± fraction) by the mask. */
  roughVar: number;
  /** Darkening of the height map's low points. */
  cavity: number;
  roughness?: number;
  metalness?: number;
  build: (u: number, v: number) => { h: number; r: number };
}

const SIZE = 256;

function hash(x: number, y: number, seed: number) {
  let h = (x * 374761393 + y * 668265263 + seed * 2147483647) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

/** Periodic value noise: period cells across the unit square. */
function vnoise(u: number, v: number, period: number, seed: number) {
  const x = u * period;
  const y = v * period;
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const fx = x - xi;
  const fy = y - yi;
  const sx = fx * fx * (3 - 2 * fx);
  const sy = fy * fy * (3 - 2 * fy);
  const w = (i: number) => ((i % period) + period) % period;
  const a = hash(w(xi), w(yi), seed);
  const b = hash(w(xi + 1), w(yi), seed);
  const c = hash(w(xi), w(yi + 1), seed);
  const d = hash(w(xi + 1), w(yi + 1), seed);
  return a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy;
}

function fbm(u: number, v: number, period: number, octaves: number, seed: number) {
  let sum = 0;
  let amp = 0.5;
  let norm = 0;
  for (let o = 0; o < octaves; o++) {
    sum += vnoise(u, v, period << o, seed + o * 17) * amp;
    norm += amp;
    amp *= 0.5;
  }
  return sum / norm;
}

/** Periodic cellular noise: distances to the nearest two feature points (in cell units). */
function cells(u: number, v: number, period: number, seed: number): [number, number] {
  const x = u * period;
  const y = v * period;
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  let f1 = 9;
  let f2 = 9;
  for (let j = -1; j <= 1; j++)
    for (let i = -1; i <= 1; i++) {
      const cx = xi + i;
      const cy = yi + j;
      const wx = ((cx % period) + period) % period;
      const wy = ((cy % period) + period) % period;
      const px = cx + hash(wx, wy, seed);
      const py = cy + hash(wx, wy, seed + 1);
      const d = Math.hypot(px - x, py - y);
      if (d < f1) {
        f2 = f1;
        f1 = d;
      } else if (d < f2) f2 = d;
    }
  return [f1, f2];
}

/** Thin random scratches: 0 on a scratch, 1 elsewhere. */
function scratches(u: number, v: number, count: number, seed: number) {
  let s = 1;
  for (let k = 0; k < count; k++) {
    const x0 = hash(k, 1, seed);
    const y0 = hash(k, 2, seed);
    const a = hash(k, 3, seed) * Math.PI;
    const len = 0.05 + hash(k, 4, seed) * 0.2;
    let dx = u - x0;
    let dy = v - y0;
    dx -= Math.round(dx);
    dy -= Math.round(dy);
    const along = dx * Math.cos(a) + dy * Math.sin(a);
    const across = -dx * Math.sin(a) + dy * Math.cos(a);
    if (Math.abs(along) > len) continue;
    s = Math.min(s, THREE.MathUtils.smoothstep(Math.abs(across), 0, 0.0035));
  }
  return s;
}

const smooth = THREE.MathUtils.smoothstep;

const SURFACES: Record<SurfaceKind, SurfaceDef> = {
  // Glossy painted plastic: a little orange peel, mottled roughness, the odd scratch.
  plastic: {
    scale: 0.5,
    normal: 0.35,
    roughVar: 0.35,
    cavity: 0.06,
    build: (u, v) => {
      const peel = fbm(u, v, 24, 2, 1);
      const sc = scratches(u, v, 18, 7);
      return { h: 0.55 + (peel - 0.5) * 0.35 - (1 - sc) * 0.25, r: 0.35 + fbm(u, v, 4, 3, 2) * 0.45 + (1 - sc) * 0.3 };
    },
  },
  // Big floors: quilted foam pads with a fine grain.
  padded: {
    scale: 0.25,
    normal: 0.9,
    roughVar: 0.25,
    cavity: 0.14,
    roughness: 0.62,
    build: (u, v) => {
      const n = 4;
      const cu = (u * n) % 1;
      const cv = (v * n) % 1;
      const pillow = (Math.sin(Math.PI * cu) * Math.sin(Math.PI * cv)) ** 0.35;
      const grain = fbm(u, v, 64, 2, 3);
      return { h: pillow * 0.85 + grain * 0.15, r: 0.4 + grain * 0.4 + fbm(u, v, 8, 2, 4) * 0.2 };
    },
  },
  // Rubber: dense round dimples.
  rubber: {
    scale: 1.2,
    normal: 0.6,
    roughVar: 0.2,
    cavity: 0.1,
    roughness: 0.72,
    build: (u, v) => {
      const [f1] = cells(u, v, 22, 5);
      const dot = 1 - smooth(f1, 0.18, 0.42);
      return { h: 0.35 + dot * 0.5 + fbm(u, v, 32, 2, 6) * 0.15, r: 0.5 + fbm(u, v, 16, 2, 7) * 0.5 };
    },
  },
  // Brushed metal: fine streaks in one direction and a few scratches.
  metal: {
    scale: 0.8,
    normal: 0.3,
    roughVar: 0.45,
    cavity: 0.04,
    roughness: 0.4,
    metalness: 0.6,
    build: (u, v) => {
      const streak = vnoise(u * 0.25, v, 180, 8) * 0.6 + vnoise(u, v, 90, 9) * 0.4;
      const sc = scratches(u, v, 30, 10);
      return { h: 0.5 + (streak - 0.5) * 0.5 - (1 - sc) * 0.3, r: 0.25 + streak * 0.5 + (1 - sc) * 0.25 };
    },
  },
  // The beans' suits: a knitted weave with fuzz.
  fabric: {
    scale: 2.2,
    normal: 0.55,
    roughVar: 0.18,
    cavity: 0.12,
    build: (u, v) => {
      const n = 28;
      const x = u * n;
      const y = v * n;
      const over = (Math.floor(x) + Math.floor(y)) % 2 === 0;
      const t = over ? Math.sin(Math.PI * (y % 1)) : Math.sin(Math.PI * (x % 1));
      const fuzz = fbm(u, v, 64, 2, 11);
      return { h: t ** 0.6 * 0.8 + fuzz * 0.2, r: 0.55 + fuzz * 0.45 };
    },
  },
  // Ice: gentle undulations, hairline cracks and frosty patches.
  ice: {
    scale: 0.3,
    normal: 0.5,
    roughVar: 0.9,
    cavity: 0.05,
    roughness: 0.1,
    build: (u, v) => {
      const [f1, f2] = cells(u, v, 6, 12);
      const crack = smooth(f2 - f1, 0, 0.05);
      const frost = smooth(fbm(u, v, 4, 4, 13), 0.5, 0.75);
      return { h: 0.5 + (fbm(u, v, 4, 3, 14) - 0.5) * 0.4 - (1 - crack) * 0.35, r: frost * 0.9 + (1 - crack) * 0.6 };
    },
  },
  // Clouds: soft billows.
  cloud: {
    scale: 0.35,
    normal: 1,
    roughVar: 0,
    cavity: 0.18,
    roughness: 1,
    build: (u, v) => {
      const b = 1 - Math.abs(fbm(u, v, 4, 5, 15) * 2 - 1);
      return { h: b, r: 1 };
    },
  },
  // Gold: hammered dents.
  gold: {
    scale: 1.6,
    normal: 0.55,
    roughVar: 0.35,
    cavity: 0.05,
    roughness: 0.26,
    metalness: 1,
    build: (u, v) => {
      const [f1] = cells(u, v, 10, 16);
      return { h: 1 - Math.min(1, f1 * f1 * 1.6), r: 0.3 + fbm(u, v, 16, 2, 17) * 0.7 };
    },
  },
  // Painted wood (doors): grain along one axis.
  wood: {
    scale: 0.35,
    normal: 0.18,
    roughVar: 0.35,
    cavity: 0.1,
    roughness: 0.55,
    build: (u, v) => {
      const warp = fbm(u, v, 4, 3, 18);
      const ring = 0.5 + 0.5 * Math.sin((u * 26 + warp * 6) * Math.PI);
      const fine = vnoise(u * 0.1, v, 120, 19);
      return { h: ring * 0.7 + fine * 0.3, r: 0.4 + ring * 0.3 + fine * 0.3 };
    },
  },
  // Eyes, gems, visors: nearly flat, very fine imperfections.
  glossy: {
    scale: 1,
    normal: 0.08,
    roughVar: 0.5,
    cavity: 0,
    build: (u, v) => ({ h: 0.5 + (fbm(u, v, 16, 3, 20) - 0.5) * 0.2, r: fbm(u, v, 8, 3, 21) }),
  },
  // Ceramic tiles (hexes, plates): speckle and a slight wave.
  tile: {
    scale: 0.7,
    normal: 0.3,
    roughVar: 0.3,
    cavity: 0.08,
    roughness: 0.38,
    build: (u, v) => {
      const speck = hash(Math.floor(u * 180), Math.floor(v * 180), 22) > 0.93 ? 1 : 0;
      return { h: 0.5 + (fbm(u, v, 6, 3, 23) - 0.5) * 0.4 - speck * 0.15, r: 0.35 + fbm(u, v, 12, 2, 24) * 0.4 + speck * 0.25 };
    },
  },
};

const textures = new Map<SurfaceKind, THREE.DataTexture>();
let anisotropy = 8;

export function setMaxAnisotropy(a: number) {
  anisotropy = Math.max(1, Math.min(16, a));
  for (const t of textures.values()) {
    t.anisotropy = anisotropy;
    t.needsUpdate = true;
  }
}

function detailTexture(kind: SurfaceKind): THREE.DataTexture {
  let tex = textures.get(kind);
  if (tex) return tex;
  const def = SURFACES[kind];
  const h = new Float32Array(SIZE * SIZE);
  const r = new Float32Array(SIZE * SIZE);
  for (let y = 0; y < SIZE; y++)
    for (let x = 0; x < SIZE; x++) {
      const s = def.build(x / SIZE, y / SIZE);
      h[y * SIZE + x] = THREE.MathUtils.clamp(s.h, 0, 1);
      r[y * SIZE + x] = THREE.MathUtils.clamp(s.r, 0, 1);
    }
  const data = new Uint8Array(SIZE * SIZE * 4);
  const at = (x: number, y: number) => h[((y + SIZE) % SIZE) * SIZE + ((x + SIZE) % SIZE)]!;
  // Height gradient over a texel, scaled so a full 0→1 rise across ~6 texels tilts about 45°.
  const k = 6;
  for (let y = 0; y < SIZE; y++)
    for (let x = 0; x < SIZE; x++) {
      const dx = (at(x + 1, y) - at(x - 1, y)) * 0.5 * k;
      const dy = (at(x, y + 1) - at(x, y - 1)) * 0.5 * k;
      const l = Math.hypot(dx, dy, 1);
      const i = (y * SIZE + x) * 4;
      data[i] = Math.round((-dx / l) * 127.5 + 127.5);
      data[i + 1] = Math.round((-dy / l) * 127.5 + 127.5);
      data[i + 2] = Math.round(at(x, y) * 255);
      data[i + 3] = Math.round(r[y * SIZE + x]! * 255);
    }
  tex = new THREE.DataTexture(data, SIZE, SIZE, THREE.RGBAFormat, THREE.UnsignedByteType);
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  tex.generateMipmaps = true;
  tex.anisotropy = anisotropy;
  tex.colorSpace = THREE.NoColorSpace;
  tex.needsUpdate = true;
  textures.set(kind, tex);
  return tex;
}

export interface Pattern {
  c1: string;
  c2: string;
  freq: number;
  dir: [number, number];
  speed: number;
}

interface Patched {
  surface: SurfaceKind;
  pattern: Pattern | null;
  strength: number;
}

const VERT_COMMON = `#include <common>
varying vec3 vDetailPos;
varying vec3 vDetailNrm;
varying vec3 vDetailAx;
varying vec3 vDetailAy;
varying vec3 vDetailAz;`;

const VERT_MAIN = `#include <project_vertex>
{
  mat3 im = mat3(1.0);
  #ifdef USE_INSTANCING
    im = mat3(instanceMatrix);
  #endif
  vec3 sc = vec3(length(modelMatrix[0].xyz), length(modelMatrix[1].xyz), length(modelMatrix[2].xyz));
  sc *= vec3(length(im[0]), length(im[1]), length(im[2]));
  vDetailPos = transformed * sc;
  vDetailNrm = normalize(objectNormal / sc);
  vDetailAx = normalize(normalMatrix * (im * vec3(1.0, 0.0, 0.0)));
  vDetailAy = normalize(normalMatrix * (im * vec3(0.0, 1.0, 0.0)));
  vDetailAz = normalize(normalMatrix * (im * vec3(0.0, 0.0, 1.0)));
}`;

const FRAG_COMMON = `#include <common>
varying vec3 vDetailPos;
varying vec3 vDetailNrm;
varying vec3 vDetailAx;
varying vec3 vDetailAy;
varying vec3 vDetailAz;
uniform sampler2D uDetail;
uniform vec4 uDetailP;
#ifdef DETAIL_PATTERN
uniform vec3 uC1; uniform vec3 uC2; uniform float uF; uniform vec2 uDir; uniform float uSpeed; uniform float uTime;
#endif`;

const FRAG_COLOR = `#include <color_fragment>
vec3 dW = pow(abs(normalize(vDetailNrm)), vec3(4.0));
dW /= dW.x + dW.y + dW.z;
vec3 dP = vDetailPos * uDetailP.x;
vec4 dTx = texture2D(uDetail, dP.zy);
vec4 dTy = texture2D(uDetail, dP.xz);
vec4 dTz = texture2D(uDetail, dP.xy);
float dHeight = dTx.b * dW.x + dTy.b * dW.y + dTz.b * dW.z;
float dRough = dTx.a * dW.x + dTy.a * dW.y + dTz.a * dW.z;
#ifdef DETAIL_PATTERN
{
  float sx = dot(vDetailPos.xz, uDir) * uF + uTime * uSpeed;
  float fw = min(0.5, fwidth(sx));
  float tri = abs(fract(sx) - 0.5);
  float stp = smoothstep(0.25 - fw, 0.25 + fw, tri);
  diffuseColor.rgb *= mix(uC1, uC2, stp);
}
#endif
diffuseColor.rgb *= mix(1.0 - uDetailP.w, 1.0, smoothstep(0.1, 0.7, dHeight));`;

const FRAG_ROUGH = `#include <roughnessmap_fragment>
roughnessFactor = clamp(roughnessFactor * mix(1.0 - uDetailP.z, 1.0 + uDetailP.z, dRough), 0.04, 1.0);`;

const FRAG_NORMAL = `#include <normal_fragment_maps>
{
  vec3 nX = vec3(0.0, dTx.y * 2.0 - 1.0, dTx.x * 2.0 - 1.0);
  vec3 nY = vec3(dTy.x * 2.0 - 1.0, 0.0, dTy.y * 2.0 - 1.0);
  vec3 nZ = vec3(dTz.x * 2.0 - 1.0, dTz.y * 2.0 - 1.0, 0.0);
  vec3 dObj = (nX * dW.x + nY * dW.y + nZ * dW.z) * uDetailP.y;
  normal = normalize(normal + dObj.x * vDetailAx + dObj.y * vDetailAy + dObj.z * vDetailAz);
}`;

/**
 * Gives a standard material its surface: detail maps, base roughness/metalness for the kind and
 * (optionally) animated stripes. Idempotent; returns the material.
 */
export function applySurface<T extends THREE.Material>(
  mat: T,
  surface: SurfaceKind,
  opts: { pattern?: Pattern | null; strength?: number; keepRoughness?: boolean } = {},
): T {
  if (!(mat instanceof THREE.MeshStandardMaterial)) return mat;
  if ((mat.userData.detail as Patched | undefined)?.surface) return mat;
  const def = SURFACES[surface];
  if (!opts.keepRoughness && def.roughness !== undefined) mat.roughness = def.roughness;
  if (def.metalness !== undefined) mat.metalness = def.metalness;
  const patched: Patched = { surface, pattern: opts.pattern ?? null, strength: opts.strength ?? 1 };
  mat.userData.detail = patched;
  const tex = detailTexture(surface);
  const p = patched.pattern;
  mat.onBeforeCompile = (sh) => {
    sh.uniforms.uDetail = { value: tex };
    sh.uniforms.uDetailP = {
      value: new THREE.Vector4(def.scale, def.normal * patched.strength, def.roughVar, def.cavity * patched.strength),
    };
    if (p) {
      sh.defines = { ...sh.defines, DETAIL_PATTERN: '' };
      sh.uniforms.uC1 = { value: new THREE.Color(p.c1) };
      sh.uniforms.uC2 = { value: new THREE.Color(p.c2) };
      sh.uniforms.uF = { value: p.freq };
      sh.uniforms.uDir = { value: new THREE.Vector2(p.dir[0], p.dir[1]).normalize() };
      sh.uniforms.uSpeed = { value: p.speed };
      sh.uniforms.uTime = timeUniform;
    }
    sh.vertexShader = sh.vertexShader.replace('#include <common>', VERT_COMMON).replace('#include <project_vertex>', VERT_MAIN);
    sh.fragmentShader = sh.fragmentShader
      .replace('#include <common>', FRAG_COMMON)
      .replace('#include <color_fragment>', FRAG_COLOR)
      .replace('#include <roughnessmap_fragment>', FRAG_ROUGH)
      .replace('#include <normal_fragment_maps>', FRAG_NORMAL);
  };
  mat.customProgramCacheKey = () => `detail|${p ? 'pat' : ''}`;
  mat.needsUpdate = true;
  return mat;
}

/** Surface for a material of the asset pack, by material name. */
export function surfaceForModelMaterial(name: string): SurfaceKind {
  switch (name) {
    case 'Body':
    case 'Belly':
    case 'Blush':
      return 'fabric';
    case 'Shoe':
    case 'Bumper':
      return 'rubber';
    case 'Metal':
      return 'metal';
    case 'Gold':
      return 'gold';
    case 'Gem':
    case 'Eye':
    case 'Sclera':
    case 'Glint':
    case 'Visor':
      return 'glossy';
    case 'Cloud':
      return 'cloud';
    case 'Door':
      return 'wood';
    case 'Top':
    case 'Side':
      return 'tile';
    default:
      return 'plastic';
  }
}

/** Every standard material under root that has no surface yet gets `fallback`. */
export function applySurfaces(root: THREE.Object3D, fallback: SurfaceKind = 'plastic') {
  root.traverse((o) => {
    if (!(o instanceof THREE.Mesh)) return;
    const mats = Array.isArray(o.material) ? o.material : [o.material];
    for (const m of mats) if (m instanceof THREE.MeshStandardMaterial && !m.userData.detail) applySurface(m, fallback);
  });
}
