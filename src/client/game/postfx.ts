import * as THREE from 'three';
import { FullScreenQuad } from 'three/addons/postprocessing/Pass.js';
import { SMAAPass } from 'three/addons/postprocessing/SMAAPass.js';
import { SMAAEdgesShader, SMAAWeightsShader } from 'three/addons/shaders/SMAAShader.js';
import { Fsr } from './fsr';
import { type AoQuality, XeGTAO } from './xegtao';

/**
 * The frame after the scene, as few full-screen passes as it can be:
 *   scene      the scene into a multisampled HDR target (the spatial part of the anti-aliasing),
 *              with a depth texture for the passes after it;
 *   gtao       ambient occlusion at half resolution (xegtao.ts);
 *   composite  one pass for the AO (depth-aware upsample), exposure, tone mapping, sRGB and the
 *              grade (saturation, contrast, vignette), into an 8-bit display-referred target;
 *   smaa       SMAA 1x edge detection and blend weights (weights only where there are edges:
 *              the edge pass marks them in a depth buffer both targets share);
 *   taa        SMAA's neighbourhood blending and the temporal resolve in one pass (SMAA T2x:
 *              jittered frames blended with the reprojected previous one);
 *   fsr        FSR 1 upscaling to the display (when rendering below it), or a copy to the screen.
 * Anti-aliasing is SMAA 4x (SMAA 1x + 2x MSAA + 2x temporal) as before, with 3 passes fewer and
 * half the bandwidth (8-bit targets after tone mapping).
 */

const VERT = 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }';

/** What the pipeline draws; switching any of them is cheap (no targets are reallocated for the toggles). */
export interface PostConfig {
  /** Display (drawing buffer) size. */
  width: number;
  height: number;
  /** Render resolution scale per axis (FSR below 1). */
  scale: number;
  msaa: number;
  ao: AoQuality | null;
  grade: boolean;
  smaa: boolean;
  temporal: boolean;
}

/** A stage the profiler can time and switch off. */
export interface Stage {
  name: string;
  enabled: boolean;
  render: () => void;
}

const COMPOSITE = /* glsl */ `
uniform sampler2D tScene;
uniform sampler2D tDepth;
uniform sampler2D tAO;
uniform sampler2D tAODepth;
uniform vec2 uAOSize;
uniform vec2 uSize;
uniform vec2 uCam;
uniform float uAO;
uniform float uExposure;
uniform float uGrade;
uniform float uSat;
uniform float uVignette;
varying vec2 vUv;

// three.js NeutralToneMapping (Khronos PBR Neutral).
vec3 neutral(vec3 color) {
  const float StartCompression = 0.8 - 0.04;
  const float Desaturation = 0.15;
  float x = min(color.r, min(color.g, color.b));
  float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
  color -= offset;
  float peak = max(color.r, max(color.g, color.b));
  if (peak < StartCompression) return color;
  float d = 1.0 - StartCompression;
  float newPeak = 1.0 - d * d / (peak + d - StartCompression);
  color *= newPeak / peak;
  float g = 1.0 - 1.0 / (Desaturation * (peak - newPeak) + 1.0);
  return mix(color, vec3(newPeak), g);
}

vec3 toSRGB(vec3 c) {
  return mix(pow(c, vec3(0.41666)) * 1.055 - vec3(0.055), c * 12.92, vec3(lessThanEqual(c, vec3(0.0031308))));
}

float linear(float d) { return uCam.x * uCam.y / (uCam.y - d * (uCam.y - uCam.x)); }

// Half-resolution occlusion, upsampled by the 4 nearest texels weighted by how close their depth is.
float occlusion() {
  float z = linear(texture2D(tDepth, vUv).x);
  // AO texel i was computed at full-resolution pixel 2i.
  vec2 p = (vUv * uSize - 0.5) * 0.5;
  vec2 f = fract(p);
  ivec2 i0 = ivec2(floor(p));
  ivec2 mx = ivec2(uAOSize) - 1;
  float sum = 0.0;
  float wsum = 0.0;
  for (int k = 0; k < 4; k++) {
    ivec2 o = ivec2(k & 1, k >> 1);
    ivec2 q = clamp(i0 + o, ivec2(0), mx);
    float zq = texelFetch(tAODepth, q, 0).x;
    float bw = (o.x == 1 ? f.x : 1.0 - f.x) * (o.y == 1 ? f.y : 1.0 - f.y) + 1e-3;
    float w = bw / (abs(z - zq) / z + 1e-3);
    sum += texelFetch(tAO, q, 0).x * w;
    wsum += w;
  }
  return sum / wsum;
}

void main() {
  vec3 c = texture2D(tScene, vUv).rgb;
  // A NaN would spread through the temporal history for good.
  if (any(isnan(c))) c = vec3(0.0);
  if (uAO > 0.0) c *= mix(1.0, occlusion(), uAO);
  c = toSRGB(clamp(neutral(max(c, 0.0) * uExposure), 0.0, 1.0));
  if (uGrade > 0.5) {
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(l), c, uSat);
    c = (c - 0.5) * 1.03 + 0.5;
    vec2 d = vUv - 0.5;
    c *= 1.0 - uVignette * smoothstep(0.35, 0.85, length(d * vec2(1.0, 0.8)) * 1.25);
  }
  gl_FragColor = vec4(clamp(c, 0.0, 1.0), 1.0);
}`;

/** SMAA neighbourhood blending (three's SMAABlendShader) and the temporal resolve, in one pass. */
const RESOLVE = /* glsl */ `
uniform sampler2D tColor;
uniform sampler2D tWeights;
uniform sampler2D tHistory;
uniform sampler2D tDepth;
uniform mat4 uInvViewProj;
uniform mat4 uPrevViewProj;
uniform vec2 uTexel;
uniform vec2 uJitter;
uniform float uSmaa;
uniform float uTemporal;
varying vec2 vUv;

vec3 smaa(vec2 uv) {
  vec4 a;
  a.xz = texture2D(tWeights, uv).xz;
  a.y = texture2D(tWeights, uv + vec2(0.0, -uTexel.y)).g;
  a.w = texture2D(tWeights, uv + vec2(uTexel.x, 0.0)).a;
  vec3 C = texture2D(tColor, uv).rgb;
  if (dot(a, vec4(1.0)) < 1e-5) return C;
  vec2 off;
  off.x = a.a > a.b ? a.a : -a.b;
  off.y = a.g > a.r ? -a.g : a.r;
  if (abs(off.x) > abs(off.y)) off.y = 0.0;
  else off.x = 0.0;
  vec3 Cop = texture2D(tColor, uv + sign(off) * uTexel).rgb;
  float s = max(abs(off.x), abs(off.y));
  return pow(mix(pow(C, vec3(2.2)), pow(Cop, vec3(2.2)), s), vec3(1.0 / 2.2));
}

vec3 toYCoCg(vec3 c) { return vec3(dot(c, vec3(0.25, 0.5, 0.25)), dot(c, vec3(0.5, 0.0, -0.5)), dot(c, vec3(-0.25, 0.5, -0.25))); }
vec3 fromYCoCg(vec3 c) { return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z); }

void main() {
  vec3 c = uSmaa > 0.5 ? smaa(vUv) : texture2D(tColor, vUv).rgb;
  if (uTemporal < 0.5) { gl_FragColor = vec4(c, 1.0); return; }
  // Neighbourhood of the current frame (before SMAA's blend: the blended centre lies within it).
  vec3 cy = toYCoCg(c);
  vec3 mn = cy, mx = cy, m1 = cy, m2 = cy * cy;
  for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
    if (x == 0 && y == 0) continue;
    vec3 s = toYCoCg(texture2D(tColor, vUv + vec2(float(x), float(y)) * uTexel).rgb);
    mn = min(mn, s); mx = max(mx, s); m1 += s; m2 += s * s;
  }
  // Variance clipping box, no wider than the min/max box.
  m1 /= 9.0; m2 /= 9.0;
  vec3 sd = sqrt(max(m2 - m1 * m1, 0.0));
  mn = max(mn, m1 - sd * 1.25); mx = min(mx, m1 + sd * 1.25);
  // Reproject with the closest depth around the pixel (the centre and its diagonals: edges of moving things).
  float d = texture2D(tDepth, vUv).x;
  d = min(d, texture2D(tDepth, vUv + vec2(-1.0, -1.0) * uTexel).x);
  d = min(d, texture2D(tDepth, vUv + vec2(1.0, -1.0) * uTexel).x);
  d = min(d, texture2D(tDepth, vUv + vec2(-1.0, 1.0) * uTexel).x);
  d = min(d, texture2D(tDepth, vUv + vec2(1.0, 1.0) * uTexel).x);
  vec4 w = uInvViewProj * vec4((vUv - uJitter) * 2.0 - 1.0, d * 2.0 - 1.0, 1.0);
  w /= w.w;
  vec4 p = uPrevViewProj * w;
  vec2 puv = p.xy / p.w * 0.5 + 0.5;
  if (any(lessThan(puv, vec2(0.0))) || any(greaterThan(puv, vec2(1.0)))) { gl_FragColor = vec4(c, 1.0); return; }
  vec3 h = clamp(toYCoCg(texture2D(tHistory, puv).rgb), mn, mx);
  // Fast motion: trust the current frame more.
  float motion = length((puv - vUv) / uTexel);
  float k = mix(0.5, 0.25, clamp(motion / 12.0, 0.0, 1.0));
  gl_FragColor = vec4(clamp(fromYCoCg(mix(cy, h, k)), 0.0, 1.0), 1.0);
}`;

const COPY = 'uniform sampler2D tDiffuse; varying vec2 vUv; void main(){ gl_FragColor = texture2D(tDiffuse, vUv); }';

/** Sub-pixel camera offsets (pixels) cycled by the temporal pass: SMAA T2x's two positions. */
const JITTER: readonly [number, number][] = [
  [0.25, -0.25],
  [-0.25, 0.25],
];

const ldr = (w: number, h: number) =>
  new THREE.WebGLRenderTarget(w, h, {
    minFilter: THREE.LinearFilter,
    magFilter: THREE.LinearFilter,
    generateMipmaps: false,
    depthBuffer: false,
  });

/** SMAA's precomputed area and search textures (three ships them inside SMAAPass). */
function smaaTextures(): { area: THREE.Texture; search: THREE.Texture } {
  const src = SMAAPass.prototype as unknown as { _getAreaTexture(): string; _getSearchTexture(): string };
  const load = (url: string, filter: THREE.MagnificationTextureFilter) => {
    const t = new THREE.Texture();
    const img = new Image();
    img.onload = () => {
      t.needsUpdate = true;
    };
    img.src = url;
    t.image = img;
    t.minFilter = filter;
    t.magFilter = filter;
    t.generateMipmaps = false;
    t.flipY = false;
    return t;
  };
  return { area: load(src._getAreaTexture(), THREE.LinearFilter), search: load(src._getSearchTexture(), THREE.NearestFilter) };
}

export class PostPipeline {
  private readonly quad = new FullScreenQuad();
  private cfg: PostConfig | null = null;
  /** Render resolution. */
  readonly size = new THREE.Vector2(1, 1);
  private sceneRT: THREE.WebGLRenderTarget | null = null;
  private ldrRT: THREE.WebGLRenderTarget | null = null;
  private edgesRT: THREE.WebGLRenderTarget | null = null;
  private weightsRT: THREE.WebGLRenderTarget | null = null;
  private history: THREE.WebGLRenderTarget[] = [];
  private readonly ao = new XeGTAO({ slices: 3, steps: 3, denoise: 2 });
  private readonly fsr = new Fsr();
  private smaaTex: { area: THREE.Texture; search: THREE.Texture } | null = null;

  private readonly composite = new THREE.ShaderMaterial({
    uniforms: {
      tScene: { value: null },
      tDepth: { value: null },
      tAO: { value: null },
      tAODepth: { value: null },
      uAOSize: { value: new THREE.Vector2() },
      uSize: { value: new THREE.Vector2() },
      uCam: { value: new THREE.Vector2() },
      uAO: { value: 0 },
      uExposure: { value: 1 },
      uGrade: { value: 1 },
      uSat: { value: 1.06 },
      uVignette: { value: 0.22 },
    },
    vertexShader: VERT,
    fragmentShader: COMPOSITE,
    depthTest: false,
    depthWrite: false,
  });
  private readonly edges = new THREE.ShaderMaterial({
    defines: { ...SMAAEdgesShader.defines },
    uniforms: THREE.UniformsUtils.clone(SMAAEdgesShader.uniforms),
    vertexShader: SMAAEdgesShader.vertexShader,
    fragmentShader: SMAAEdgesShader.fragmentShader,
    // Marks the edge pixels in the shared depth buffer (the edge shader discards the rest).
    depthTest: true,
    depthFunc: THREE.AlwaysDepth,
    depthWrite: true,
  });
  private readonly weights = new THREE.ShaderMaterial({
    defines: { ...SMAAWeightsShader.defines },
    uniforms: THREE.UniformsUtils.clone(SMAAWeightsShader.uniforms),
    vertexShader: SMAAWeightsShader.vertexShader,
    fragmentShader: SMAAWeightsShader.fragmentShader,
    // Only where the edge pass wrote (depth 0 there, cleared to 1 elsewhere): early depth test
    // skips the expensive search on the other pixels, as SMAA's stencil does.
    depthTest: true,
    depthFunc: THREE.GreaterEqualDepth,
    depthWrite: false,
  });
  private readonly resolve = new THREE.ShaderMaterial({
    uniforms: {
      tColor: { value: null },
      tWeights: { value: null },
      tHistory: { value: null },
      tDepth: { value: null },
      uInvViewProj: { value: new THREE.Matrix4() },
      uPrevViewProj: { value: new THREE.Matrix4() },
      uTexel: { value: new THREE.Vector2() },
      uJitter: { value: new THREE.Vector2() },
      uSmaa: { value: 0 },
      uTemporal: { value: 0 },
    },
    vertexShader: VERT,
    fragmentShader: RESOLVE,
    depthTest: false,
    depthWrite: false,
  });
  private readonly copy = new THREE.ShaderMaterial({
    uniforms: { tDiffuse: { value: null } },
    vertexShader: VERT,
    fragmentShader: COPY,
    depthTest: false,
    depthWrite: false,
  });

  // Temporal state.
  private readonly prevViewProj = new THREE.Matrix4();
  private readonly viewProj = new THREE.Matrix4();
  private readonly invViewProj = new THREE.Matrix4();
  private readonly proj = new THREE.Matrix4();
  private readonly projInv = new THREE.Matrix4();
  private cur = 0;
  private index = 0;
  private valid = false;
  private jittered = false;
  /** Frames rendered (the AO noise pattern follows it with the temporal pass on). */
  private frame = 0;

  /** In drawing order; the profiler times them and switches them off. */
  readonly stages: Stage[];
  private readonly stage: Record<'scene' | 'gtao' | 'composite' | 'smaa' | 'taa' | 'fsr', Stage>;

  constructor(
    private readonly renderer: THREE.WebGLRenderer,
    private readonly scene: THREE.Scene,
    private readonly camera: THREE.PerspectiveCamera,
  ) {
    const s = (name: string, render: () => void): Stage => ({ name, enabled: true, render });
    this.stage = {
      scene: s('scene', () => this.renderScene()),
      gtao: s('gtao', () => this.renderAo()),
      composite: s('composite', () => this.renderComposite()),
      smaa: s('smaa', () => this.renderSmaa()),
      taa: s('taa', () => this.renderResolve()),
      fsr: s('fsr', () => this.renderOutput()),
    };
    this.stages = Object.values(this.stage);
  }

  /** The scene's render target (materials compile for it: linear output, no tone mapping). */
  get sceneTarget(): THREE.WebGLRenderTarget | null {
    return this.sceneRT;
  }

  /** Grade and exposure (the map's look). */
  setGrade(saturation: number) {
    this.composite.uniforms.uSat!.value = saturation;
  }

  /** (Re)allocates for a configuration; toggles alone keep the targets. */
  configure(cfg: PostConfig) {
    const prev = this.cfg;
    this.cfg = cfg;
    const w = Math.max(1, Math.round(cfg.width * cfg.scale));
    const h = Math.max(1, Math.round(cfg.height * cfg.scale));
    const resized = !prev || this.size.x !== w || this.size.y !== h;
    this.size.set(w, h);
    if (!prev || resized || prev.msaa !== cfg.msaa || !this.sceneRT) {
      this.sceneRT?.dispose();
      this.sceneRT = new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType, samples: cfg.msaa, generateMipmaps: false });
      this.sceneRT.depthTexture = new THREE.DepthTexture(w, h, THREE.UnsignedIntType);
    }
    if (resized) {
      this.ldrRT?.dispose();
      this.ldrRT = ldr(w, h);
      for (const r of this.history) r.dispose();
      this.history = [ldr(w, h), ldr(w, h)];
      this.valid = false;
      this.edgesRT?.dispose();
      this.weightsRT?.dispose();
      this.edgesRT = this.weightsRT = null;
    }
    if (cfg.smaa && !this.edgesRT) {
      this.smaaTex ??= smaaTextures();
      const depth = new THREE.DepthTexture(w, h, THREE.UnsignedIntType);
      const opts = { minFilter: THREE.LinearFilter, magFilter: THREE.LinearFilter, generateMipmaps: false } as const;
      this.edgesRT = new THREE.WebGLRenderTarget(w, h, opts);
      this.edgesRT.depthTexture = depth;
      this.weightsRT = new THREE.WebGLRenderTarget(w, h, opts);
      this.weightsRT.depthTexture = depth;
      this.edges.uniforms.resolution!.value.set(1 / w, 1 / h);
      const wu = this.weights.uniforms;
      wu.resolution!.value.set(1 / w, 1 / h);
      wu.tDiffuse!.value = this.edgesRT.texture;
      wu.tArea!.value = this.smaaTex.area;
      wu.tSearch!.value = this.smaaTex.search;
    }
    if (cfg.ao) {
      this.ao.setQuality(cfg.ao);
      this.ao.setSize(w, h);
    }
    if (cfg.scale < 1) this.fsr.setSize(cfg.width, cfg.height);
    this.resolve.uniforms.uTexel!.value.set(1 / w, 1 / h);
    this.composite.uniforms.uSize!.value.set(w, h);
  }

  private get temporalOn() {
    return !!this.cfg?.temporal && this.stage.taa.enabled;
  }

  /** Offsets the camera by this frame's sub-pixel jitter (before the scene renders; unjitter() after). */
  jitter() {
    const cam = this.camera;
    this.proj.copy(cam.projectionMatrix);
    this.projInv.copy(cam.projectionMatrixInverse);
    this.viewProj.multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse);
    this.invViewProj.copy(this.viewProj).invert();
    this.jittered = this.temporalOn;
    if (!this.jittered) return;
    const [jx, jy] = JITTER[this.index % JITTER.length]!;
    this.index++;
    const ox = (jx * 2) / this.size.x;
    const oy = (jy * 2) / this.size.y;
    const e = cam.projectionMatrix.elements;
    // Perspective: shifting the third column moves every point by the same amount in NDC.
    e[8]! += ox;
    e[9]! += oy;
    cam.projectionMatrixInverse.copy(cam.projectionMatrix).invert();
    this.resolve.uniforms.uJitter!.value.set(ox / 2, oy / 2);
  }

  unjitter() {
    this.camera.projectionMatrix.copy(this.proj);
    this.camera.projectionMatrixInverse.copy(this.projInv);
  }

  /** Forget the temporal history (camera cut). */
  reset() {
    this.valid = false;
  }

  render() {
    if (!this.cfg) return;
    this.frame++;
    for (const st of this.stages) if (st.enabled || st === this.stage.scene || st === this.stage.composite) st.render();
    if (!this.stage.fsr.enabled) this.renderOutput();
  }

  private draw(m: THREE.Material, target: THREE.WebGLRenderTarget | null) {
    this.quad.material = m;
    this.renderer.setRenderTarget(target);
    this.quad.render(this.renderer);
  }

  private renderScene() {
    const r = this.renderer;
    r.setRenderTarget(this.sceneRT);
    r.clear();
    r.render(this.scene, this.camera);
  }

  private get aoOn() {
    return !!this.cfg?.ao && this.stage.gtao.enabled;
  }

  private renderAo() {
    if (!this.aoOn || !this.sceneRT) return;
    this.ao.render(this.renderer, this.quad, this.sceneRT.depthTexture!, this.camera, this.temporalOn ? this.frame : 0);
  }

  private renderComposite() {
    const u = this.composite.uniforms;
    const cfg = this.cfg!;
    u.tScene!.value = this.sceneRT!.texture;
    u.tDepth!.value = this.sceneRT!.depthTexture;
    const ao = this.aoOn && this.ao.texture;
    u.uAO!.value = ao ? 0.55 : 0;
    if (ao) {
      u.tAO!.value = this.ao.texture;
      u.tAODepth!.value = this.ao.depth;
      u.uAOSize!.value.copy(this.ao.size);
      u.uCam!.value.set(this.camera.near, this.camera.far);
    }
    u.uExposure!.value = this.renderer.toneMappingExposure;
    u.uGrade!.value = cfg.grade ? 1 : 0;
    this.draw(this.composite, this.ldrRT);
  }

  private get smaaOn() {
    return !!this.cfg?.smaa && this.stage.smaa.enabled && !!this.edgesRT;
  }

  private renderSmaa() {
    if (!this.smaaOn) return;
    const r = this.renderer;
    this.edges.uniforms.tDiffuse!.value = this.ldrRT!.texture;
    r.setRenderTarget(this.edgesRT);
    r.setClearColor(0x000000, 0);
    r.clear(true, true, false);
    this.draw(this.edges, this.edgesRT);
    // The weights target shares the edge pass's depth: clear its colour only.
    r.setRenderTarget(this.weightsRT);
    r.clear(true, false, false);
    this.draw(this.weights, this.weightsRT);
  }

  /** Where the frame is after the anti-aliasing (the input of the upscaler or the final copy). */
  private output: THREE.Texture | null = null;

  private renderResolve() {
    this.output = this.ldrRT!.texture;
    const temporal = this.temporalOn;
    const smaa = this.smaaOn;
    if (!temporal && !smaa) return;
    const u = this.resolve.uniforms;
    u.tColor!.value = this.ldrRT!.texture;
    u.tWeights!.value = smaa ? this.weightsRT!.texture : null;
    u.uSmaa!.value = smaa ? 1 : 0;
    u.uTemporal!.value = temporal && this.valid ? 1 : 0;
    const next = this.history[1 - this.cur]!;
    if (temporal) {
      u.tHistory!.value = this.history[this.cur]!.texture;
      u.tDepth!.value = this.sceneRT!.depthTexture;
      u.uInvViewProj!.value.copy(this.invViewProj);
      u.uPrevViewProj!.value.copy(this.prevViewProj);
    }
    this.draw(this.resolve, next);
    this.output = next.texture;
    if (temporal) {
      this.cur = 1 - this.cur;
      this.prevViewProj.copy(this.viewProj);
      this.valid = true;
    } else this.valid = false;
  }

  private renderOutput() {
    const src = this.output ?? this.ldrRT!.texture;
    this.output = null;
    if (this.cfg!.scale < 1 && this.stage.fsr.enabled) this.fsr.render(this.renderer, this.quad, src, null);
    else {
      this.copy.uniforms.tDiffuse!.value = src;
      this.draw(this.copy, null);
    }
  }

  dispose() {
    this.sceneRT?.dispose();
    this.ldrRT?.dispose();
    this.edgesRT?.dispose();
    this.weightsRT?.dispose();
    for (const r of this.history) r.dispose();
    this.ao.dispose();
    this.fsr.dispose();
    this.quad.dispose();
    for (const m of [this.composite, this.edges, this.weights, this.resolve, this.copy]) m.dispose();
  }
}
