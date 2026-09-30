import * as THREE from 'three';
import { levels } from './lod';
import { BAKE_LAYER, type Statics } from './statics';

/**
 * Baked shadows of the static parts of a map. Their depth, as the sun sees it, is rendered into a
 * window around the live shadow map, on exactly its texel grid (same size and alignment), and redone
 * only when the live map nears the window's edge or something static starts moving. Every frame the
 * live shadow map starts from a copy of it (one full-screen pass, texel for texel: no resampling) and
 * only the moving things (beans, movers) are drawn into it, instead of the whole course.
 *
 * (A single bake of the whole course was coarser than the live map on long courses — 8 to 13 cm a
 * texel against 3.3 — and its texels showed as steps along every static shadow.)
 */

/** Extra room around the live shadow map (m): the window is redone after moving this far. */
const MARGIN = 16;
/** Bakes for changed statics at most this often (hex tiles dropping one after another). */
const MIN_INTERVAL_MS = 300;

const COPY_VERT = /* glsl */ `
varying vec2 vNdc;
void main() {
  vNdc = position.xy;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}`;

/**
 * For each texel of the live shadow map: the static occluder on its ray (from the bake), as a depth
 * of the live shadow camera. Both are orthographic along the sun on the same texel grid, so every
 * live texel reads exactly one bake texel.
 */
const COPY_FRAG = /* glsl */ `
uniform sampler2D uBake;
uniform mat4 uRtToWorld;
uniform mat4 uRtViewProj;
uniform mat4 uBakeViewProj;
uniform mat4 uBakeToWorld;
varying vec2 vNdc;
void main() {
  vec4 w = uRtToWorld * vec4(vNdc, -1.0, 1.0);
  vec4 b = uBakeViewProj * vec4(w.xyz / w.w, 1.0);
  vec2 uv = b.xy * 0.5 + 0.5;
  if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) discard;
  float d = texture2D(uBake, uv).r;
  if (d > 0.99999) discard;
  vec4 o = uBakeToWorld * vec4(b.xy, d * 2.0 - 1.0, 1.0);
  vec4 r = uRtViewProj * vec4(o.xyz / o.w, 1.0);
  gl_FragDepth = clamp(r.z / r.w * 0.5 + 0.5, 0.0, 1.0);
  gl_FragColor = vec4(1.0);
}`;

/** The sun's light space: x and y across its rays (as a shadow camera looking along them sees it), z towards it. */
export function sunBasis(sunDir: THREE.Vector3, x: THREE.Vector3, y: THREE.Vector3, z: THREE.Vector3) {
  z.copy(sunDir).normalize();
  // As Matrix4.lookAt builds it for a camera with up = +y.
  x.set(0, 1, 0).cross(z).normalize();
  y.crossVectors(z, x);
}

export class ShadowBake {
  private target: THREE.WebGLRenderTarget | null = null;
  private readonly cam = new THREE.OrthographicCamera();
  private readonly depth = new THREE.MeshBasicMaterial({ colorWrite: false, side: THREE.BackSide });
  private readonly copy: THREE.ShaderMaterial;
  /** Draws the bake into the live shadow map (and nothing into the picture). */
  private readonly quad: THREE.Mesh;
  /** What the current bake is of: statics, their version, texel size, window centre (light space). */
  private baked: { statics: Statics; version: number; texel: number; cx: number; cy: number; half: number } | null = null;
  private at = -Infinity;
  private readonly bx = new THREE.Vector3();
  private readonly by = new THREE.Vector3();
  private readonly bz = new THREE.Vector3();
  /** Size of the last bake, for the debug overlay. */
  size = { w: 0, h: 0, ms: 0, casters: 0 };

  constructor(
    private readonly renderer: THREE.WebGLRenderer,
    /** Towards the sun (the live shadow camera looks along it too). */
    private readonly sunDir: THREE.Vector3,
    private readonly scene: THREE.Scene,
  ) {
    const u = {
      uBake: { value: null as THREE.Texture | null },
      uRtToWorld: { value: new THREE.Matrix4() },
      uRtViewProj: { value: new THREE.Matrix4() },
      uBakeViewProj: { value: new THREE.Matrix4() },
      uBakeToWorld: { value: new THREE.Matrix4() },
    };
    this.copy = new THREE.ShaderMaterial({ uniforms: u, vertexShader: COPY_VERT, fragmentShader: COPY_FRAG });
    // The picture: a triangle-free draw (every vertex outside the clip volume).
    const hidden = new THREE.ShaderMaterial({
      vertexShader: 'void main() { gl_Position = vec4(0.0, 0.0, 2.0, 1.0); }',
      fragmentShader: 'void main() { gl_FragColor = vec4(0.0); }',
      side: THREE.DoubleSide,
      depthWrite: false,
      depthTest: false,
    });
    this.quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), hidden);
    this.quad.name = 'baked shadows';
    this.quad.frustumCulled = false;
    // Far out of every view: passes that draw the scene with a material of their own (ambient
    // occlusion) must not see a plane at the origin. (The copy ignores the transform.)
    this.quad.position.set(0, -1e6, 0);
    this.quad.castShadow = true;
    this.quad.receiveShadow = false;
    this.quad.customDepthMaterial = this.copy;
    this.quad.userData.noLod = true;
    this.quad.onBeforeShadow = (_r, _o, _c, shadowCamera) => {
      u.uRtToWorld.value.multiplyMatrices(shadowCamera.matrixWorld, shadowCamera.projectionMatrixInverse);
      u.uRtViewProj.value.multiplyMatrices(shadowCamera.projectionMatrix, shadowCamera.matrixWorldInverse);
      this.copy.uniformsNeedUpdate = true;
    };
  }

  /**
   * Keeps the bake ready for the live shadow map: `texel` its texel size (m), (`lx`, `ly`) its centre
   * in the sun's light space (on the texel grid: see Renderer.focus), `range` its half width (m).
   */
  update(statics: Statics | null, texel: number, lx: number, ly: number, range: number) {
    if (!statics) {
      this.clear();
      return;
    }
    const b = this.baked;
    const same = !!b && b.statics === statics && b.texel === texel;
    const inside = !!b && Math.abs(lx - b.cx) <= b.half - range && Math.abs(ly - b.cy) <= b.half - range;
    const changed = !same || b?.version !== statics.version;
    if (!changed && inside) return;
    // Statics that changed after the first bake wait a little (several at once make one bake);
    // the window following the player never waits.
    const now = performance.now();
    if (same && inside && now - this.at < MIN_INTERVAL_MS) return;
    this.at = now;
    this.bake(statics, texel, lx, ly, range);
  }

  private bake(statics: Statics, texel: number, cx: number, cy: number, range: number) {
    const t0 = performance.now();
    const r = this.renderer;
    const casters = statics.casters();
    const { bx, by, bz } = this;
    sunBasis(this.sunDir, bx, by, bz);
    // An even number of texels across: the window's edges fall on the live map's texel grid.
    const maxTex = Math.min(r.capabilities.maxTextureSize, 8192);
    const size = Math.min(maxTex, 2 * Math.ceil((range + MARGIN) / texel)) & ~1;
    const half = (size * texel) / 2;
    // Depth: everything that casts, in front of the camera.
    let zMin = Number.POSITIVE_INFINITY;
    let zMax = Number.NEGATIVE_INFINITY;
    const box = new THREE.Box3();
    const p = new THREE.Vector3();
    for (const m of casters) {
      const g = m.geometry;
      if (!g.boundingBox) g.computeBoundingBox();
      box.copy(g.boundingBox!).applyMatrix4(m.matrixWorld);
      for (let i = 0; i < 8; i++) {
        const d = p.set(i & 1 ? box.max.x : box.min.x, i & 2 ? box.max.y : box.min.y, i & 4 ? box.max.z : box.min.z).dot(bz);
        zMin = Math.min(zMin, d);
        zMax = Math.max(zMax, d);
      }
    }
    if (!Number.isFinite(zMin)) zMin = zMax = 0;
    const cam = this.cam;
    cam.position
      .copy(bx)
      .multiplyScalar(cx)
      .addScaledVector(by, cy)
      .addScaledVector(bz, zMax + 10);
    cam.up.set(0, 1, 0);
    cam.lookAt(p.copy(cam.position).sub(bz));
    cam.updateMatrixWorld();
    cam.left = -half;
    cam.right = half;
    cam.bottom = -half;
    cam.top = half;
    cam.near = 1;
    cam.far = zMax - zMin + 20;
    cam.updateProjectionMatrix();
    cam.layers.set(BAKE_LAYER);

    if (!this.target || this.target.width !== size) {
      this.target?.dispose();
      this.target?.depthTexture?.dispose();
      // Depth only (a minimal colour attachment, never written): 16 bits, as precise as the live map.
      this.target = new THREE.WebGLRenderTarget(size, size, {
        format: THREE.RedFormat,
        minFilter: THREE.NearestFilter,
        magFilter: THREE.NearestFilter,
        generateMipmaps: false,
      });
      this.target.depthTexture = new THREE.DepthTexture(size, size, THREE.UnsignedShortType);
    }
    // Full detail for the bake, whatever level of detail they are drawn at now.
    const geo = casters.map((m) => m.geometry);
    for (const m of casters) {
      m.layers.enable(BAKE_LAYER);
      m.geometry = levels(m.geometry)?.[0] ?? m.geometry;
    }
    const prev = { target: r.getRenderTarget(), override: this.scene.overrideMaterial, shadows: r.shadowMap.autoUpdate };
    r.shadowMap.autoUpdate = false;
    this.scene.overrideMaterial = this.depth;
    r.setRenderTarget(this.target);
    r.clear(false, true, false);
    r.render(this.scene, cam);
    r.setRenderTarget(prev.target);
    this.scene.overrideMaterial = prev.override;
    r.shadowMap.autoUpdate = prev.shadows;
    casters.forEach((m, i) => {
      m.layers.disable(BAKE_LAYER);
      m.geometry = geo[i]!;
    });

    const u = this.copy.uniforms;
    u.uBake!.value = this.target.depthTexture;
    u.uBakeViewProj!.value.multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse);
    u.uBakeToWorld!.value.copy(u.uBakeViewProj!.value).invert();
    if (!this.quad.parent) this.scene.add(this.quad);
    this.baked = { statics, version: statics.version, texel, cx, cy, half };
    this.size = { w: size, h: size, ms: Math.round((performance.now() - t0) * 10) / 10, casters: casters.length };
  }

  clear() {
    this.target?.dispose();
    this.target?.depthTexture?.dispose();
    this.target = null;
    this.quad.removeFromParent();
    this.baked = null;
  }
}
