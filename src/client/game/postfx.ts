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
 * for the passes after it (god rays, temporal reprojection); the resolved colour is copied on.
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

const _sun = new THREE.Vector3();

/** Interleaved gradient noise, shifted per frame (the temporal pass averages it out). */
const IGN = `float ign(vec2 p, float f){ p += vec2(5.588238, 3.1178) * f; return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))); }`;

/**
 * Sun shafts, at half resolution: light scattered in the air towards the camera, raymarched through
 * the sun's shadow map (air behind hammers, frames and platforms stays darker) with a forward-
 * scattering phase, plus streaks radiating from the sun's disc where the sky is not blocked, added
 * to the HDR image.
 */
export class GodRaysPass extends Pass {
  intensity = 0.55;
  /** Radial streaks from the sun (0 = volumetric part only). */
  streaks = 1.3;
  private readonly half: THREE.WebGLRenderTarget;
  private readonly halfB: THREE.WebGLRenderTarget;
  private readonly march: FullScreenQuad;
  private readonly radial: FullScreenQuad;
  private readonly comp: FullScreenQuad;
  private frame = 0;

  constructor(
    private readonly camera: THREE.PerspectiveCamera,
    private readonly sun: THREE.DirectionalLight,
    private readonly depth: () => THREE.DepthTexture,
    w: number,
    h: number,
  ) {
    super();
    this.half = new THREE.WebGLRenderTarget(Math.max(1, w >> 1), Math.max(1, h >> 1), { type: THREE.HalfFloatType });
    this.halfB = this.half.clone();
    this.march = new FullScreenQuad(
      new THREE.ShaderMaterial({
        uniforms: {
          tDepth: { value: null },
          tShadow: { value: null },
          uShadowMatrix: { value: new THREE.Matrix4() },
          uInvProj: { value: new THREE.Matrix4() },
          uCamWorld: { value: new THREE.Matrix4() },
          uCamPos: { value: new THREE.Vector3() },
          uSunDir: { value: new THREE.Vector3() },
          uFrame: { value: 0 },
          uMaxDist: { value: 70 },
          uHasShadow: { value: 0 },
        },
        vertexShader: VERT,
        fragmentShader: `
          precision highp sampler2DShadow;
          uniform sampler2D tDepth; uniform sampler2DShadow tShadow;
          uniform mat4 uShadowMatrix; uniform mat4 uInvProj; uniform mat4 uCamWorld;
          uniform vec3 uCamPos; uniform vec3 uSunDir; uniform float uFrame; uniform float uMaxDist; uniform float uHasShadow;
          varying vec2 vUv;
          ${IGN}
          #define STEPS 20
          void main(){
            float d = texture2D(tDepth, vUv).x;
            vec4 v = uInvProj * vec4(vUv * 2.0 - 1.0, d * 2.0 - 1.0, 1.0);
            v /= v.w;
            vec3 wp = (uCamWorld * vec4(v.xyz, 1.0)).xyz;
            vec3 rd = wp - uCamPos;
            float len = length(rd);
            rd /= max(len, 1e-4);
            len = min(len, uMaxDist);
            float stepLen = len / float(STEPS);
            float j = ign(gl_FragCoord.xy, uFrame);
            float lit = 0.0;
            for (int i = 0; i < STEPS; i++) {
              vec3 p = uCamPos + rd * (float(i) + j) * stepLen;
              vec4 sc = uShadowMatrix * vec4(p, 1.0);
              sc.xyz /= sc.w;
              float s = 1.0;
              if (uHasShadow > 0.5 && all(greaterThan(sc.xyz, vec3(0.0))) && all(lessThan(sc.xyz, vec3(1.0))))
                s = texture(tShadow, vec3(sc.xy, sc.z - 0.001));
              lit += s;
            }
            // Shadowed air along the view ray, as a share (shafts), and the path length it scatters over.
            float share = lit / float(STEPS);
            float cosT = dot(rd, uSunDir);
            float g = 0.6;
            float hg = (1.0 - g * g) / pow(1.0 + g * g - 2.0 * g * cosT, 1.5) / 12.566;
            float amount = share * (1.0 - exp(-len * 0.012)) * (0.04 + hg * 2.5);
            // Open sky around the sun: the source of the streaks.
            float sky = step(0.99999, d) * pow(max(cosT, 0.0), 16.0);
            gl_FragColor = vec4(amount, sky, 0.0, 1.0);
          }`,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.radial = new FullScreenQuad(
      new THREE.ShaderMaterial({
        uniforms: { tRays: { value: this.half.texture }, uSun: { value: new THREE.Vector2() }, uFrame: { value: 0 } },
        vertexShader: VERT,
        fragmentShader: `
          uniform sampler2D tRays; uniform vec2 uSun; uniform float uFrame; varying vec2 vUv;
          ${IGN}
          #define SAMPLES 36
          void main(){
            vec4 c = texture2D(tRays, vUv);
            vec2 dv = (uSun - vUv) / float(SAMPLES) * 0.9;
            vec2 uv = vUv + dv * ign(gl_FragCoord.xy, uFrame);
            float sum = 0.0, w = 1.0;
            for (int i = 0; i < SAMPLES; i++) {
              sum += texture2D(tRays, uv).g * w;
              w *= 0.955;
              uv += dv;
            }
            gl_FragColor = vec4(c.r, sum / float(SAMPLES) * 3.0, 0.0, 1.0);
          }`,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.comp = new FullScreenQuad(
      new THREE.ShaderMaterial({
        uniforms: {
          tDiffuse: { value: null },
          tRays: { value: this.halfB.texture },
          uTexel: { value: new THREE.Vector2() },
          uColor: { value: new THREE.Color() },
          uIntensity: { value: 1 },
          uStreaks: { value: 1 },
        },
        vertexShader: VERT,
        fragmentShader: `
          uniform sampler2D tDiffuse; uniform sampler2D tRays; uniform vec2 uTexel; uniform vec3 uColor; uniform float uIntensity; uniform float uStreaks;
          varying vec2 vUv;
          void main(){
            vec4 c = texture2D(tDiffuse, vUv);
            vec2 r = texture2D(tRays, vUv).rg * 0.4;
            r += texture2D(tRays, vUv + vec2(uTexel.x, 0.0)).rg * 0.15;
            r += texture2D(tRays, vUv - vec2(uTexel.x, 0.0)).rg * 0.15;
            r += texture2D(tRays, vUv + vec2(0.0, uTexel.y)).rg * 0.15;
            r += texture2D(tRays, vUv - vec2(0.0, uTexel.y)).rg * 0.15;
            gl_FragColor = vec4(c.rgb + uColor * (r.x * uIntensity + r.y * uStreaks), c.a);
          }`,
        depthTest: false,
        depthWrite: false,
      }),
    );
  }

  override setSize(w: number, h: number) {
    this.half.setSize(Math.max(1, w >> 1), Math.max(1, h >> 1));
    this.halfB.setSize(Math.max(1, w >> 1), Math.max(1, h >> 1));
    (this.comp.material as THREE.ShaderMaterial).uniforms.uTexel!.value.set(2 / w, 2 / h);
  }

  override render(renderer: THREE.WebGLRenderer, writeBuffer: THREE.WebGLRenderTarget, readBuffer: THREE.WebGLRenderTarget) {
    const cam = this.camera;
    const sun = this.sun;
    const mu = (this.march.material as THREE.ShaderMaterial).uniforms;
    this.frame = (this.frame + 1) % 64;
    mu.tDepth!.value = this.depth();
    const map = sun.castShadow ? sun.shadow.map?.depthTexture : null;
    mu.tShadow!.value = map ?? null;
    mu.uHasShadow!.value = map ? 1 : 0;
    mu.uShadowMatrix!.value.copy(sun.shadow.matrix);
    mu.uInvProj!.value.copy(cam.projectionMatrixInverse);
    mu.uCamWorld!.value.copy(cam.matrixWorld);
    mu.uCamPos!.value.setFromMatrixPosition(cam.matrixWorld);
    mu.uSunDir!.value.subVectors(sun.position, sun.target.position).normalize();
    mu.uFrame!.value = this.frame;
    renderer.setRenderTarget(this.half);
    this.march.render(renderer);
    // The sun on screen (or off its edge): streaks fade out as it leaves the view.
    const ru = (this.radial.material as THREE.ShaderMaterial).uniforms;
    _sun.copy(mu.uSunDir!.value).multiplyScalar(500).add(mu.uCamPos!.value).applyMatrix4(cam.matrixWorldInverse);
    const behind = _sun.z > 0;
    _sun.applyMatrix4(cam.projectionMatrix);
    ru.uSun!.value.set(_sun.x * 0.5 + 0.5, _sun.y * 0.5 + 0.5);
    ru.uFrame!.value = this.frame;
    const off = Math.max(0, Math.max(Math.abs(_sun.x), Math.abs(_sun.y)) - 1);
    const streaks = behind ? 0 : this.streaks * Math.max(0, 1 - off / 0.8);
    renderer.setRenderTarget(this.halfB);
    this.radial.render(renderer);
    const cu = (this.comp.material as THREE.ShaderMaterial).uniforms;
    cu.tDiffuse!.value = readBuffer.texture;
    cu.uColor!.value.copy(sun.color).multiplyScalar(sun.intensity * 0.35);
    cu.uIntensity!.value = this.intensity;
    cu.uStreaks!.value = streaks;
    renderer.setRenderTarget(this.renderToScreen ? null : writeBuffer);
    this.comp.render(renderer);
  }

  override dispose() {
    this.half.dispose();
    this.halfB.dispose();
    this.march.dispose();
    this.radial.dispose();
    this.comp.dispose();
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
            vec3 h = toYCoCg(texture2D(tHistory, puv).rgb);
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
