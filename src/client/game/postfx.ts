import * as THREE from 'three';
import { FullScreenQuad, Pass } from 'three/addons/postprocessing/Pass.js';

const VERT = 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }';

const copyMat = () =>
  new THREE.ShaderMaterial({
    uniforms: { tDiffuse: { value: null } },
    vertexShader: VERT,
    fragmentShader: 'uniform sampler2D tDiffuse; varying vec2 vUv; void main(){ gl_FragColor = texture2D(tDiffuse, vUv); }',
    depthTest: false,
    depthWrite: false,
  });

/**
 * The scene into its own multisampled target (the spatial part of SMAA 4x) with a depth texture
 * for the passes after it (temporal reprojection); the resolved colour is copied on.
 */
export class ScenePass extends Pass {
  readonly target: THREE.WebGLRenderTarget;
  private readonly quad = new FullScreenQuad(copyMat());

  constructor(
    private readonly scene: THREE.Scene,
    private readonly camera: THREE.Camera,
    w: number,
    h: number,
    samples: number,
  ) {
    super();
    this.target = new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType, samples });
    this.target.depthTexture = new THREE.DepthTexture(w, h, THREE.UnsignedIntType);
  }

  get depth(): THREE.DepthTexture {
    return this.target.depthTexture!;
  }

  override setSize(w: number, h: number) {
    this.target.setSize(w, h);
  }

  override render(renderer: THREE.WebGLRenderer, writeBuffer: THREE.WebGLRenderTarget) {
    renderer.setRenderTarget(this.target);
    renderer.clear();
    renderer.render(this.scene, this.camera);
    (this.quad.material as THREE.ShaderMaterial).uniforms.tDiffuse!.value = this.target.texture;
    renderer.setRenderTarget(this.renderToScreen ? null : writeBuffer);
    this.quad.render(renderer);
  }

  override dispose() {
    this.target.dispose();
    this.quad.dispose();
  }
}

/** Sub-pixel camera offsets (pixels) cycled by the temporal pass: SMAA T2x's two positions. */
const JITTER: readonly [number, number][] = [
  [0.25, -0.25],
  [-0.25, 0.25],
];

/**
 * The temporal part of SMAA 4x: the camera is jittered by a sub-pixel offset that alternates every
 * frame, and each frame is blended with the previous result reprojected by depth and the camera
 * motion, clamped to the current neighbourhood (no ghosts behind moving things).
 */
export class TemporalPass extends Pass {
  private readonly history: [THREE.WebGLRenderTarget, THREE.WebGLRenderTarget];
  private readonly quad: FullScreenQuad;
  private readonly copy = new FullScreenQuad(copyMat());
  private readonly prevViewProj = new THREE.Matrix4();
  private readonly viewProj = new THREE.Matrix4();
  private readonly invViewProj = new THREE.Matrix4();
  private cur = 0;
  private index = 0;
  private valid = false;
  private w = 1;
  private h = 1;
  /** The unjittered projection while a frame renders. */
  private readonly proj = new THREE.Matrix4();
  private readonly projInv = new THREE.Matrix4();

  constructor(
    private readonly camera: THREE.PerspectiveCamera,
    private readonly depth: () => THREE.DepthTexture,
    w: number,
    h: number,
  ) {
    super();
    const rt = () => new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType });
    this.history = [rt(), rt()];
    this.w = w;
    this.h = h;
    this.quad = new FullScreenQuad(
      new THREE.ShaderMaterial({
        uniforms: {
          tDiffuse: { value: null },
          tHistory: { value: null },
          tDepth: { value: null },
          uInvViewProj: { value: new THREE.Matrix4() },
          uPrevViewProj: { value: new THREE.Matrix4() },
          uTexel: { value: new THREE.Vector2(1 / w, 1 / h) },
          uJitter: { value: new THREE.Vector2() },
          uValid: { value: 0 },
        },
        vertexShader: VERT,
        fragmentShader: `
          uniform sampler2D tDiffuse; uniform sampler2D tHistory; uniform sampler2D tDepth;
          uniform mat4 uInvViewProj; uniform mat4 uPrevViewProj; uniform vec2 uTexel; uniform vec2 uJitter; uniform float uValid;
          varying vec2 vUv;
          vec3 toYCoCg(vec3 c){ return vec3(dot(c, vec3(0.25, 0.5, 0.25)), dot(c, vec3(0.5, 0.0, -0.5)), dot(c, vec3(-0.25, 0.5, -0.25))); }
          vec3 fromYCoCg(vec3 c){ return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z); }
          void main(){
            vec3 c = texture2D(tDiffuse, vUv).rgb;
            // A NaN would spread through the history for good.
            if (any(isnan(c))) c = vec3(0.0);
            if (uValid < 0.5) { gl_FragColor = vec4(c, 1.0); return; }
            vec3 cy = toYCoCg(c);
            vec3 mn = cy, mx = cy, m1 = cy, m2 = cy * cy;
            for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
              if (x == 0 && y == 0) continue;
              vec3 s = toYCoCg(texture2D(tDiffuse, vUv + vec2(float(x), float(y)) * uTexel).rgb);
              mn = min(mn, s); mx = max(mx, s); m1 += s; m2 += s * s;
            }
            // Variance clipping box, no wider than the min/max box.
            m1 /= 9.0; m2 /= 9.0;
            vec3 sd = sqrt(max(m2 - m1 * m1, 0.0));
            mn = max(mn, m1 - sd * 1.25); mx = min(mx, m1 + sd * 1.25);
            // Reproject with the closest depth around the pixel (edges of moving things).
            float d = 1.0;
            for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) d = min(d, texture2D(tDepth, vUv + vec2(float(x), float(y)) * uTexel).x);
            vec4 w = uInvViewProj * vec4((vUv - uJitter) * 2.0 - 1.0, d * 2.0 - 1.0, 1.0);
            w /= w.w;
            vec4 p = uPrevViewProj * w;
            vec2 puv = p.xy / p.w * 0.5 + 0.5;
            if (any(lessThan(puv, vec2(0.0))) || any(greaterThan(puv, vec2(1.0)))) { gl_FragColor = vec4(c, 1.0); return; }
            vec3 hs = texture2D(tHistory, puv).rgb;
            if (any(isnan(hs))) hs = c;
            vec3 h = toYCoCg(hs);
            h = clamp(h, mn, mx);
            // Fast motion: trust the current frame more.
            float motion = length((puv - vUv) / uTexel);
            float k = mix(0.5, 0.25, clamp(motion / 12.0, 0.0, 1.0));
            gl_FragColor = vec4(fromYCoCg(mix(cy, h, k)), 1.0);
          }`,
        depthTest: false,
        depthWrite: false,
      }),
    );
  }

  /** Offsets the camera for this frame (call before the scene renders; unjitter() after all passes). */
  jitter() {
    const cam = this.camera;
    this.proj.copy(cam.projectionMatrix);
    this.projInv.copy(cam.projectionMatrixInverse);
    this.viewProj.multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse);
    this.invViewProj.copy(this.viewProj).invert();
    if (!this.enabled) return;
    const [jx, jy] = JITTER[this.index % JITTER.length]!;
    this.index++;
    const ox = (jx * 2) / this.w;
    const oy = (jy * 2) / this.h;
    const e = cam.projectionMatrix.elements;
    // Perspective: shifting the third column moves every point by the same amount in NDC.
    e[8]! += ox;
    e[9]! += oy;
    cam.projectionMatrixInverse.copy(cam.projectionMatrix).invert();
    (this.quad.material as THREE.ShaderMaterial).uniforms.uJitter!.value.set(ox / 2, oy / 2);
  }

  unjitter() {
    this.camera.projectionMatrix.copy(this.proj);
    this.camera.projectionMatrixInverse.copy(this.projInv);
  }

  /** Forget the history (camera cut, resize). */
  reset() {
    this.valid = false;
  }

  override setSize(w: number, h: number) {
    this.w = w;
    this.h = h;
    for (const r of this.history) r.setSize(w, h);
    (this.quad.material as THREE.ShaderMaterial).uniforms.uTexel!.value.set(1 / w, 1 / h);
    this.valid = false;
  }

  override render(renderer: THREE.WebGLRenderer, writeBuffer: THREE.WebGLRenderTarget, readBuffer: THREE.WebGLRenderTarget) {
    const u = (this.quad.material as THREE.ShaderMaterial).uniforms;
    const prev = this.history[this.cur]!;
    const next = this.history[1 - this.cur]!;
    u.tDiffuse!.value = readBuffer.texture;
    u.tHistory!.value = prev.texture;
    u.tDepth!.value = this.depth();
    u.uInvViewProj!.value.copy(this.invViewProj);
    u.uPrevViewProj!.value.copy(this.prevViewProj);
    u.uValid!.value = this.valid ? 1 : 0;
    renderer.setRenderTarget(next);
    this.quad.render(renderer);
    (this.copy.material as THREE.ShaderMaterial).uniforms.tDiffuse!.value = next.texture;
    renderer.setRenderTarget(this.renderToScreen ? null : writeBuffer);
    this.copy.render(renderer);
    this.cur = 1 - this.cur;
    this.prevViewProj.copy(this.viewProj);
    this.valid = true;
  }

  override dispose() {
    for (const r of this.history) r.dispose();
    this.quad.dispose();
    this.copy.dispose();
  }
}
