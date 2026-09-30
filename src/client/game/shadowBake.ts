import * as THREE from 'three';
import { levels } from './lod';
import { BAKE_LAYER, type Statics } from './statics';

/**
 * Baked shadows of the static parts of a map. Their depth, as the sun sees it, is rendered once for
 * the whole course into a texture (redone only when something static starts moving). Every frame
 * the shadow map that follows the camera then starts from a copy of it (one full-screen pass) and
 * only the moving things (beans, movers) are drawn into it, instead of the whole course.
 */

/** Largest bake (texels): 32 MB. Longer courses get a coarser bake (still softened by the shadow filter). */
const MAX_TEXELS = 8 * 1024 * 1024;
/** Bakes at most this often (hex tiles dropping one after another). */
const MIN_INTERVAL_MS = 300;

/** Depth (0…1) in 24 bits of an RGBA8 texel. */
const BAKE_VERT = 'void main() { gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }';
const BAKE_FRAG = /* glsl */ `
void main() {
  vec3 e = fract(gl_FragCoord.z * vec3(1.0, 255.0, 65025.0));
  e -= e.yzz * vec3(1.0 / 255.0, 1.0 / 255.0, 0.0);
  gl_FragColor = vec4(e, 1.0);
}`;

const COPY_VERT = /* glsl */ `
varying vec2 vNdc;
void main() {
  vNdc = position.xy;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}`;

/**
 * For each texel of the live shadow map: the static occluder on its ray (from the bake), as a depth
 * of the live shadow camera. Both are orthographic along the sun, so the mapping is exact.
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
  float d = dot(texture2D(uBake, uv).rgb, vec3(1.0, 1.0 / 255.0, 1.0 / 65025.0));
  if (d > 0.9999) discard;
  vec4 o = uBakeToWorld * vec4(b.xy, d * 2.0 - 1.0, 1.0);
  vec4 r = uRtViewProj * vec4(o.xyz / o.w, 1.0);
  gl_FragDepth = clamp(r.z / r.w * 0.5 + 0.5, 0.0, 1.0);
  gl_FragColor = vec4(1.0);
}`;

export class ShadowBake {
  private tex: THREE.Texture | null = null;
  private target: THREE.WebGLRenderTarget | null = null;
  private readonly cam = new THREE.OrthographicCamera();
  private readonly depth = new THREE.ShaderMaterial({ vertexShader: BAKE_VERT, fragmentShader: BAKE_FRAG, side: THREE.BackSide });
  private readonly copy: THREE.ShaderMaterial;
  /** Draws the bake into the live shadow map (and nothing into the picture). */
  private readonly quad: THREE.Mesh;
  private baked: { statics: Statics; version: number; texel: number } | null = null;
  private at = -Infinity;
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
   * Bakes when the statics changed (or the texel size of the live shadow map did); `texel` is the
   * live shadow map's texel size (m), which the bake matches where it can.
   */
  update(statics: Statics | null, texel: number) {
    if (!statics) {
      this.clear();
      return;
    }
    const b = this.baked;
    const fresh = b && b.statics === statics && b.version === statics.version && b.texel === texel;
    if (fresh) return;
    // Changes after the first bake wait a little (several at once make one bake).
    const now = performance.now();
    if (b && b.statics === statics && now - this.at < MIN_INTERVAL_MS) return;
    this.at = now;
    this.bake(statics, texel);
    this.baked = { statics, version: statics.version, texel };
  }

  private bake(statics: Statics, texel: number) {
    const t0 = performance.now();
    const r = this.renderer;
    const casters = statics.casters();
    const cam = this.cam;
    // Looking down the sun's rays at the course.
    const dir = this.sunDir.clone().normalize();
    const center = statics.region.getCenter(new THREE.Vector3());
    cam.position.copy(center).addScaledVector(dir, 500);
    cam.up.set(0, 1, 0);
    cam.lookAt(center);
    cam.updateMatrixWorld();
    const view = cam.matrixWorldInverse;
    // Width and height from the course (where shadows are seen), depth from everything that casts.
    const xy = new THREE.Box3();
    const z = new THREE.Box3();
    const corner = new THREE.Vector3();
    const reg = statics.region;
    for (let i = 0; i < 8; i++) {
      corner.set(i & 1 ? reg.max.x : reg.min.x, i & 2 ? reg.max.y : reg.min.y, i & 4 ? reg.max.z : reg.min.z).applyMatrix4(view);
      xy.expandByPoint(corner);
      z.expandByPoint(corner);
    }
    const box = new THREE.Box3();
    for (const m of casters) {
      const g = m.geometry;
      if (!g.boundingBox) g.computeBoundingBox();
      box.copy(g.boundingBox!).applyMatrix4(m.matrixWorld).applyMatrix4(view);
      z.union(box);
    }
    const maxTex = Math.min(r.capabilities.maxTextureSize, 16384);
    const sx = xy.max.x - xy.min.x;
    const sy = xy.max.y - xy.min.y;
    const tx = Math.max(texel, Math.sqrt((sx * sy) / MAX_TEXELS), sx / maxTex, sy / maxTex);
    const w = Math.max(1, Math.ceil(sx / tx));
    const h = Math.max(1, Math.ceil(sy / tx));
    cam.left = xy.min.x;
    cam.right = xy.min.x + w * tx;
    cam.bottom = xy.min.y;
    cam.top = xy.min.y + h * tx;
    cam.near = -z.max.z - 1;
    cam.far = -z.min.z + 1;
    cam.updateProjectionMatrix();
    cam.layers.set(BAKE_LAYER);

    const target = new THREE.WebGLRenderTarget(w, h, {
      minFilter: THREE.NearestFilter,
      magFilter: THREE.NearestFilter,
      generateMipmaps: false,
      depthBuffer: true,
    });
    // Full detail for the bake, whatever level of detail they are drawn at now.
    const geo = casters.map((m) => m.geometry);
    for (const m of casters) {
      m.layers.enable(BAKE_LAYER);
      m.geometry = levels(m.geometry)?.[0] ?? m.geometry;
    }
    const prev = {
      target: r.getRenderTarget(),
      clear: r.getClearColor(new THREE.Color()),
      alpha: r.getClearAlpha(),
      override: this.scene.overrideMaterial,
      shadows: r.shadowMap.autoUpdate,
    };
    r.shadowMap.autoUpdate = false;
    this.scene.overrideMaterial = this.depth;
    r.setRenderTarget(target);
    r.setClearColor(0xffffff, 1);
    r.clear();
    r.render(this.scene, cam);
    r.setRenderTarget(prev.target);
    r.setClearColor(prev.clear, prev.alpha);
    this.scene.overrideMaterial = prev.override;
    r.shadowMap.autoUpdate = prev.shadows;
    casters.forEach((m, i) => {
      m.layers.disable(BAKE_LAYER);
      m.geometry = geo[i]!;
    });

    // Keep only the colour (the packed depth): a plain texture instead of the whole render target.
    this.release();
    let tex: THREE.Texture = target.texture;
    try {
      const copy = new THREE.FramebufferTexture(w, h);
      r.copyTextureToTexture(target.texture, copy);
      target.dispose();
      tex = copy;
    } catch {
      this.target = target;
    }
    this.tex = tex;
    const u = this.copy.uniforms;
    u.uBake!.value = tex;
    u.uBakeViewProj!.value.multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse);
    u.uBakeToWorld!.value.copy(u.uBakeViewProj!.value).invert();
    if (!this.quad.parent) this.scene.add(this.quad);
    this.size = { w, h, ms: Math.round((performance.now() - t0) * 10) / 10, casters: casters.length };
  }

  private release() {
    if (this.target) this.target.dispose();
    else this.tex?.dispose();
    this.target = null;
    this.tex = null;
  }

  /** Whether the bake is copied into shadow maps being drawn (only the sun's matches it). */
  set active(on: boolean) {
    this.quad.castShadow = on;
  }

  clear() {
    this.release();
    this.quad.removeFromParent();
    this.baked = null;
  }
}
