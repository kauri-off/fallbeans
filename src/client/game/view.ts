import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import type { ModelName, Palette, PrimKind, View } from '../../sim/builder';
import { clone, meshParts } from './assets';

/** Shared by every pattern material: drives animated stripes (conveyors). */
export const timeUniform = { value: 0 };

const geoCache = new Map<string, THREE.BufferGeometry>();
const matCache = new Map<string, THREE.Material>();

function geometry(kind: PrimKind, dims: readonly number[]): THREE.BufferGeometry {
  const key = `${kind}|${dims.map((d) => d.toFixed(3)).join('|')}`;
  let g = geoCache.get(key);
  if (!g) {
    if (kind === 'box') {
      const [sx = 1, sy = 1, sz = 1] = dims;
      g = new RoundedBoxGeometry(sx, sy, sz, 2, Math.min(0.25, sx / 4, sy / 4, sz / 4));
    } else if (kind === 'cyl') {
      const [r = 1, h = 1, seg = 48] = dims;
      g = new THREE.CylinderGeometry(r, r, h, seg);
    } else {
      const [r = 1] = dims;
      g = new THREE.SphereGeometry(r, 32, 20);
    }
    geoCache.set(key, g);
  }
  return g;
}

export function patternMaterial(c1: string, c2: string, freq = 0.25, dir: [number, number] = [1, 1], speed = 0): THREE.Material {
  const key = ['pat', c1, c2, freq, dir, speed].join('|');
  const cached = matCache.get(key);
  if (cached) return cached;
  const m = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.55, metalness: 0 });
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
  const key = `plain|${color}|${JSON.stringify(opts)}`;
  let m = matCache.get(key);
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.5, ...opts });
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
  return tex;
}

/** The client's rendering side of map building (see sim/builder.ts View). */
export class ClientView implements View {
  private owned: { dispose(): void }[] = [];

  prim(kind: PrimKind, dims: readonly number[], material: THREE.Material): THREE.Mesh {
    return new THREE.Mesh(geometry(kind, dims), material);
  }

  instanced(kind: PrimKind, dims: readonly number[], material: THREE.Material, count: number): THREE.InstancedMesh {
    const m = new THREE.InstancedMesh(geometry(kind, dims), material, count);
    m.castShadow = true;
    m.receiveShadow = true;
    m.frustumCulled = false;
    m.setColorAt(0, new THREE.Color('#ffffff'));
    this.owned.push({ dispose: () => m.dispose() });
    return m;
  }

  material(pal: Palette | string, freq?: number): THREE.Material {
    return typeof pal === 'string' ? plainMaterial(pal) : patternMaterial(pal[0], pal[1], freq);
  }

  pattern(c1: string, c2: string, freq?: number, dir?: [number, number], speed?: number): THREE.Material {
    return patternMaterial(c1, c2, freq, dir, speed);
  }

  plain(color: string, opts?: THREE.MeshStandardMaterialParameters): THREE.Material {
    return plainMaterial(color, opts);
  }

  model(name: ModelName): THREE.Object3D {
    return clone(name);
  }

  scatter(name: ModelName, matrices: readonly THREE.Matrix4[], shadows = true): THREE.Object3D {
    const group = new THREE.Group();
    const m = new THREE.Matrix4();
    for (const { mesh, local } of meshParts(name)) {
      const inst = new THREE.InstancedMesh(mesh.geometry, mesh.material, matrices.length);
      matrices.forEach((mat, i) => {
        inst.setMatrixAt(i, m.multiplyMatrices(mat, local));
      });
      inst.castShadow = shadows;
      inst.receiveShadow = shadows;
      inst.computeBoundingSphere();
      group.add(inst);
      this.owned.push({ dispose: () => inst.dispose() });
    }
    return group;
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
