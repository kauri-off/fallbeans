import * as THREE from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { GTAOPass } from 'three/addons/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { ShaderPass } from 'three/addons/postprocessing/ShaderPass.js';
import { SMAAPass } from 'three/addons/postprocessing/SMAAPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import type { ResolvedLook } from '../../sim/looks';
import { GpuTimer } from '../debug/gpuTimer';
import { lod } from './lod';
import { setMaxAnisotropy } from './materials';
import { ScenePass, TemporalPass } from './postfx';
import { ShadowBake } from './shadowBake';
import type { Statics } from './statics';

export type Quality = 'medium' | 'high' | 'ultra';

interface Preset {
  pixelRatio: number;
  shadowMap: number;
  shadowRange: number;
  /**
   * Anti-aliasing is SMAA 4x on every preset: SMAA 1x (edges) + 2x MSAA (spatial multisampling)
   * + 2x temporal supersampling (jittered frames blended with the reprojected previous one).
   */
  msaa: number;
  /** A barely-there glow on bright highlights. */
  bloom: boolean;
  ao: boolean;
  /** LOD distances multiplier (further detail on better presets). */
  lodBias: number;
}

const PRESETS: Record<Quality, Preset> = {
  medium: { pixelRatio: 1, shadowMap: 1024, shadowRange: 28, msaa: 2, bloom: false, ao: false, lodBias: 0.75 },
  high: { pixelRatio: 1.5, shadowMap: 2048, shadowRange: 34, msaa: 2, bloom: false, ao: false, lodBias: 1 },
  ultra: { pixelRatio: 2, shadowMap: 4096, shadowRange: 40, msaa: 2, bloom: true, ao: true, lodBias: 1.4 },
};

/** Post effects that can be switched on and off (settings, profiler, benchmarks). */
export interface Effects {
  smaa: boolean;
  temporal: boolean;
}

/** Final grade in display space: a touch of saturation and contrast, and a soft vignette. */
const GradeShader = {
  uniforms: { tDiffuse: { value: null }, uVignette: { value: 0.22 }, uSat: { value: 1.06 } },
  vertexShader:
    'varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
  fragmentShader: `
    uniform sampler2D tDiffuse; uniform float uVignette; uniform float uSat; varying vec2 vUv;
    void main() {
      vec4 c = texture2D(tDiffuse, vUv);
      float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
      c.rgb = mix(vec3(l), c.rgb, uSat);
      c.rgb = (c.rgb - 0.5) * 1.03 + 0.5;
      vec2 d = vUv - 0.5;
      c.rgb *= 1.0 - uVignette * smoothstep(0.35, 0.85, length(d * vec2(1.0, 0.8)) * 1.25);
      gl_FragColor = c;
    }`,
};

const SKY_TOP = new THREE.Color('#6fb8ff');
const SKY_HORIZON = new THREE.Color('#ffd9f2');
const SKY_CLOUD = new THREE.Color('#ffffff');
const SUN_COLOR = new THREE.Color('#fff1dc');
/**
 * Towards the sun from what it lights (the map's look turns it; the length stays). High: shadows
 * fall close under the beans, which helps judging jumps.
 */
export const SUN_OFFSET = new THREE.Vector3(9, 40, 7);
const SUN_DISTANCE = SUN_OFFSET.length();

/** Gradient sky with a slowly drifting layer of clouds and a soft sun glow. */
function skyDome(time: { value: number }): THREE.Mesh {
  const geo = new THREE.SphereGeometry(900, 48, 24);
  const mat = new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    fog: false,
    uniforms: {
      top: { value: SKY_TOP },
      horizon: { value: SKY_HORIZON },
      cloudCol: { value: SKY_CLOUD },
      sunCol: { value: SUN_COLOR },
      stars: { value: 0 },
      sunDir: { value: SUN_OFFSET.clone().normalize() },
      time,
    },
    vertexShader:
      'varying vec3 vP; void main(){ vP = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }',
    fragmentShader: `
      uniform vec3 top; uniform vec3 horizon; uniform vec3 cloudCol; uniform vec3 sunCol; uniform float stars;
      uniform vec3 sunDir; uniform float time; varying vec3 vP;
      float h2(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
      float n2(vec2 p){ vec2 i = floor(p), f = fract(p); vec2 u = f * f * (3.0 - 2.0 * f);
        return mix(mix(h2(i), h2(i + vec2(1.0, 0.0)), u.x), mix(h2(i + vec2(0.0, 1.0)), h2(i + vec2(1.0, 1.0)), u.x), u.y); }
      float fbm(vec2 p){ float s = 0.0, a = 0.5; for (int i = 0; i < 5; i++) { s += a * n2(p); p = p * 2.03 + vec2(1.7, 9.2); a *= 0.5; } return s; }
      void main(){
        vec3 d = normalize(vP);
        float h = clamp(d.y * 1.6 + 0.12, 0.0, 1.0);
        vec3 col = mix(horizon, top, pow(h, 0.8));
        float sd = max(dot(d, sunDir), 0.0);
        col += sunCol * (pow(sd, 700.0) * 2.0 + pow(sd, 10.0) * 0.18);
        if (stars > 0.0 && d.y > 0.0) {
          // A still field of twinkling stars (night looks), fading towards the horizon.
          vec3 q = d * 220.0;
          vec3 cell = floor(q);
          float s = h2(cell.xy + cell.z * 17.13);
          float star = step(0.9965, s) * smoothstep(0.35, 0.05, length(fract(q) - 0.5));
          star *= 0.6 + 0.4 * sin(time * (1.0 + s * 3.0) + s * 90.0);
          col += vec3(1.0, 0.96, 0.9) * star * stars * smoothstep(0.0, 0.25, d.y);
        }
        if (d.y > 0.0) {
          vec2 uv = d.xz / (d.y + 0.18) * 1.3;
          vec2 wind = vec2(time * 0.010, time * 0.004);
          float c = fbm(uv + wind) + 0.25 * fbm(uv * 3.1 - wind * 2.5);
          c = smoothstep(0.62, 0.95, c);
          float fade = smoothstep(0.02, 0.3, d.y);
          vec3 cc = mix(cloudCol, horizon, 0.18) + sunCol * pow(sd, 4.0) * 0.25;
          col = mix(col, cc, c * fade * 0.8);
        }
        gl_FragColor = vec4(col, 1.0);
      }`,
  });
  const m = new THREE.Mesh(geo, mat);
  m.renderOrder = -10;
  m.frustumCulled = false;
  return m;
}

/** Soft specks of pollen and sparkle drifting around the camera (wrapped in a box that follows it). */
function motes(time: { value: number }): THREE.Points {
  const n = 420;
  const box = 44;
  const pos = new Float32Array(n * 3);
  const seed = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    pos[i * 3] = Math.random() * box;
    pos[i * 3 + 1] = Math.random() * box * 0.5;
    pos[i * 3 + 2] = Math.random() * box;
    seed[i] = Math.random();
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  g.setAttribute('seed', new THREE.BufferAttribute(seed, 1));
  const mat = new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: {
      time,
      box: { value: new THREE.Vector3(box, box * 0.5, box) },
      center: { value: new THREE.Vector3() },
      px: { value: 1 },
      rise: { value: 0.12 },
      tint: { value: new THREE.Color('#fff7e0') },
    },
    vertexShader: `
      attribute float seed; uniform float time; uniform vec3 box; uniform vec3 center; uniform float px; uniform float rise; varying float vA;
      void main(){
        vec3 drift = vec3(sin(time * 0.3 + seed * 40.0) * 0.8 + time * 0.35, sin(time * 0.5 + seed * 17.0) * 0.6 + time * rise * (0.7 + seed * 0.6), cos(time * 0.27 + seed * 23.0) * 0.8);
        vec3 p = mod(position + drift - center + box * 0.5, box) - box * 0.5 + center;
        vec4 mv = modelViewMatrix * vec4(p, 1.0);
        gl_Position = projectionMatrix * mv;
        float d = -mv.z;
        vA = smoothstep(1.5, 4.0, d) * (1.0 - smoothstep(14.0, 22.0, d)) * (0.55 + 0.45 * sin(time * (1.5 + seed * 2.0) + seed * 60.0));
        gl_PointSize = px * (0.8 + seed * 1.6) * 36.0 / max(d, 0.5);
      }`,
    fragmentShader: `
      uniform vec3 tint; varying float vA;
      void main(){ vec2 c = gl_PointCoord - 0.5; float r = dot(c, c); float a = smoothstep(0.25, 0.0, r) * vA; gl_FragColor = vec4(tint * a * 0.55, a); }`,
  });
  const p = new THREE.Points(g, mat);
  p.frustumCulled = false;
  p.renderOrder = 5;
  return p;
}

/**
 * WebGL2 renderer: soft shadows following the camera, image-based lighting, neutral tone
 * mapping, a light grade, SMAA 4x anti-aliasing, levels of detail with cross-fades, and (ultra) GTAO
 * with a barely-there bloom.
 */
export class Renderer {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(70, 1, 0.25, 1100);
  readonly sun = new THREE.DirectionalLight('#fff1dc', 2.2);
  readonly hemi = new THREE.HemisphereLight('#cfe8ff', '#b99be0', 0.9);
  /** Saturation of the final grade (the look sets it). */
  private saturation = 1.06;
  private grade: ShaderPass | null = null;
  private composer: EffectComposer | null = null;
  private preset: Preset = PRESETS.high;
  quality: Quality = 'high';
  private readonly time = { value: 0 };
  readonly sky = skyDome(this.time);
  readonly motes = motes(this.time);
  /** GPU timing (debug): off, the whole frame, or each pass (scene, shadows, AO, bloom…). */
  private gpu: GpuTimer | null = null;
  private gpuMode: 'off' | 'frame' | 'passes' = 'off';
  private readonly wrapped = new WeakSet<object>();
  private temporal: TemporalPass | null = null;
  /** Switchable effects (see Effects); the composer is rebuilt when they change. */
  readonly fx: Effects = { smaa: true, temporal: true };
  /** Static meshes of the current map (their shadows are baked once, not drawn every frame). */
  statics: Statics | null = null;
  readonly bake: ShadowBake;

  constructor(readonly canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance', stencil: false });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.95;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFShadowMap;
    // Totals per frame over all composer passes (reset in render()).
    this.renderer.info.autoReset = false;
    setMaxAnisotropy(this.renderer.capabilities.getMaxAnisotropy());

    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environmentIntensity = 0.3;
    pmrem.dispose();
    this.scene.fog = new THREE.Fog(SKY_HORIZON.clone().lerp(SKY_TOP, 0.25), 120, 520);
    this.scene.add(this.sky, this.motes);
    this.scene.add(this.hemi);
    this.sun.castShadow = true;
    this.sun.shadow.bias = -0.0003;
    this.sun.shadow.normalBias = 0.035;
    this.sun.shadow.radius = 4;
    this.sun.shadow.blurSamples = 12;
    this.scene.add(this.sun, this.sun.target);
    this.bake = new ShadowBake(this.renderer, SUN_OFFSET, this.scene);
  }

  setQuality(q: Quality) {
    this.quality = q;
    this.preset = PRESETS[q];
    const p = this.preset;
    lod.bias = p.lodBias;
    this.sun.shadow.mapSize.set(p.shadowMap, p.shadowMap);
    this.sun.shadow.map?.dispose();
    this.sun.shadow.map = null;
    const cam = this.sun.shadow.camera;
    cam.left = cam.bottom = -p.shadowRange;
    cam.right = cam.top = p.shadowRange;
    cam.near = 1;
    cam.far = 140;
    cam.updateProjectionMatrix();
    this.resize();
  }

  /** Debug (profiler experiments): render resolution scale and MSAA override (null: the preset's). */
  debugScale = 1;
  /** Profiling: renders per frame (GPU times are then divided by it). */
  debugRepeat = 1;
  debugMsaa: number | null = null;

  resize() {
    const w = this.canvas.clientWidth || window.innerWidth;
    const h = this.canvas.clientHeight || window.innerHeight;
    const pr = Math.min(window.devicePixelRatio || 1, this.preset.pixelRatio) * this.debugScale;
    this.renderer.setPixelRatio(pr);
    this.renderer.setSize(w, h, false);
    (this.motes.material as THREE.ShaderMaterial).uniforms.px!.value = pr * (h / 900);
    this.camera.aspect = w / Math.max(1, h);
    this.camera.updateProjectionMatrix();
    this.buildComposer(w, h, pr);
  }

  /** Switches effects on or off (rebuilds the post-processing chain). */
  setEffects(patch: Partial<Effects>) {
    const before = JSON.stringify(this.fx);
    Object.assign(this.fx, patch);
    if (JSON.stringify(this.fx) !== before) this.resize();
  }

  private buildComposer(w: number, h: number, pr: number) {
    this.composer?.dispose();
    const p = this.preset;
    const fx = this.fx;
    // Everything after the scene pass is single-sampled; the scene pass resolves its own MSAA target.
    const target = new THREE.WebGLRenderTarget(w * pr, h * pr, { type: THREE.HalfFloatType });
    const c = new EffectComposer(this.renderer, target);
    c.setPixelRatio(pr);
    c.setSize(w, h);
    const named = <T extends object>(p: T, name: string) => Object.assign(p, { fbName: name });
    const scene = new ScenePass(this.scene, this.camera, w * pr, h * pr, this.debugMsaa ?? p.msaa);
    c.addPass(named(scene, 'scene'));
    if (p.ao) {
      const ao = new GTAOPass(this.scene, this.camera, w, h);
      ao.blendIntensity = 0.55;
      c.addPass(named(ao, 'gtao'));
    }
    if (p.bloom) c.addPass(named(new UnrealBloomPass(new THREE.Vector2(w, h), 0.06, 0.3, 0.97), 'bloom'));
    c.addPass(named(new OutputPass(), 'output'));
    this.grade = named(new ShaderPass(GradeShader), 'grade');
    this.grade.uniforms.uSat!.value = this.saturation;
    c.addPass(this.grade);
    if (fx.smaa) c.addPass(named(new SMAAPass(), 'smaa'));
    this.temporal = fx.temporal ? named(new TemporalPass(this.camera, () => scene.depth, w * pr, h * pr), 'temporal') : null;
    if (this.temporal) c.addPass(this.temporal);
    this.composer = c;
  }

  /**
   * Lights and sky for a map's look: sky colours and stars, the sun's colour, strength and
   * direction (the baked shadows follow it), ambient light, fog, exposure, the grade, the specks in the air.
   */
  applyLook(look: ResolvedLook) {
    const sky = (this.sky.material as THREE.ShaderMaterial).uniforms;
    SKY_TOP.set(look.sky.top);
    SKY_HORIZON.set(look.sky.horizon);
    SKY_CLOUD.set(look.sky.cloud);
    SUN_COLOR.set(look.sun.color);
    sky.stars!.value = look.sky.stars;
    document.documentElement.classList.toggle('night', look.sky.stars > 0);
    const az = THREE.MathUtils.degToRad(look.sun.azimuth);
    const el = THREE.MathUtils.degToRad(look.sun.elevation);
    SUN_OFFSET.set(Math.cos(el) * Math.cos(az), Math.sin(el), Math.cos(el) * Math.sin(az)).multiplyScalar(SUN_DISTANCE);
    (sky.sunDir!.value as THREE.Vector3).copy(SUN_OFFSET).normalize();
    this.sun.color.set(look.sun.color);
    this.sun.intensity = look.sun.intensity;
    this.hemi.color.set(look.hemi.sky);
    this.hemi.groundColor.set(look.hemi.ground);
    this.hemi.intensity = look.hemi.intensity;
    const fog = this.scene.fog as THREE.Fog;
    fog.color.set(look.fog.color);
    fog.near = look.fog.near;
    fog.far = look.fog.far;
    this.renderer.toneMappingExposure = look.exposure;
    this.scene.environmentIntensity = look.env;
    this.saturation = look.saturation;
    if (this.grade) this.grade.uniforms.uSat!.value = look.saturation;
    const motes = (this.motes.material as THREE.ShaderMaterial).uniforms;
    (motes.tint!.value as THREE.Color).set(look.motes.color);
    motes.rise!.value = look.motes.rise;
  }

  /** A camera cut: the previous frame is not reused by the temporal anti-aliasing. */
  cut() {
    this.temporal?.reset();
  }

  /** Keeps the shadow frustum centred on the action, snapped to texels to avoid shimmering. */
  focus(p: THREE.Vector3) {
    const texel = (this.preset.shadowRange * 2) / this.preset.shadowMap;
    const x = Math.round(p.x / texel) * texel;
    const z = Math.round(p.z / texel) * texel;
    this.sun.target.position.set(x, p.y, z);
    this.sun.position.set(x + SUN_OFFSET.x, p.y + SUN_OFFSET.y, z + SUN_OFFSET.z);
    this.sky.position.copy(this.camera.position);
    (this.motes.material as THREE.ShaderMaterial).uniforms.center!.value.copy(this.camera.position);
  }

  /** Advances the sky and ambient effects (seconds since start). */
  tick(t: number) {
    this.time.value = t;
  }

  render() {
    this.renderer.info.reset();
    const gpu = this.gpuMode === 'off' ? null : this.gpu;
    if (gpu && this.gpuMode === 'passes') this.wrapPasses();
    if (gpu && this.gpuMode === 'frame') gpu.begin('frame');
    // Profiling: the same frame several times back to back keeps the GPU busy, so timer queries
    // measure work rather than the gaps while it waits for the CPU (see GpuTimer).
    this.camera.updateMatrixWorld();
    this.statics?.update(this.camera);
    lod.update(this.camera);
    // Static shadows at the live shadow map's texel size (the bake is redone when that or the statics change).
    this.bake.update(this.statics, (this.preset.shadowRange * 2) / this.preset.shadowMap);
    for (let i = 0; i < this.debugRepeat; i++) {
      if (i) this.renderer.info.reset();
      const taa = this.temporal?.enabled ? this.temporal : null;
      taa?.jitter();
      if (this.composer) this.composer.render();
      else this.renderer.render(this.scene, this.camera);
      taa?.unjitter();
    }
    gpu?.endFrame();
  }

  /** GPU time of recent frames (ms), while measuring. */
  get gpuMs(): readonly number[] {
    return this.gpu?.frameMs ?? [];
  }

  /**
   * Starts measuring GPU time ('frame': per frame; 'passes': per render pass, with the shadow maps
   * separately), or stops ('off'). Returns false when the browser has no GPU timer queries.
   */
  measureGpu(mode: 'off' | 'frame' | 'passes' | boolean): boolean {
    const m = mode === true ? 'frame' : mode === false ? 'off' : mode;
    this.gpu ??= new GpuTimer(this.renderer.getContext() as WebGL2RenderingContext);
    if (!this.gpu.supported) return false;
    if (m !== this.gpuMode) this.gpu.reset();
    this.gpuMode = m;
    return m !== 'off';
  }

  /** Per-label GPU averages since measuring started (see GpuTimer.report). */
  gpuReport() {
    return this.gpu?.report() ?? null;
  }

  resetGpu() {
    this.gpu?.reset();
  }

  /** Composer passes (and the shadow map render inside the scene pass) time themselves on the GPU. */
  private wrapPasses() {
    const shadow = this.renderer.shadowMap as unknown as { render: (...a: unknown[]) => void };
    if (!this.wrapped.has(shadow)) {
      this.wrapped.add(shadow);
      const orig = shadow.render.bind(shadow);
      shadow.render = (...a: unknown[]) => {
        const g = this.gpuMode === 'passes' ? this.gpu : null;
        g?.begin('shadows');
        orig(...a);
        g?.end();
      };
    }
    for (const p of this.composer?.passes ?? []) {
      if (this.wrapped.has(p)) continue;
      this.wrapped.add(p);
      const label = (p as { fbName?: string }).fbName ?? 'pass';
      const orig = p.render.bind(p);
      p.render = (...a: Parameters<typeof p.render>) => {
        const g = this.gpuMode === 'passes' ? this.gpu : null;
        g?.begin(label);
        orig(...a);
        g?.end();
      };
    }
  }

  /** Composer passes by name (profiler experiments switch them off). */
  get passes(): { name: string; pass: { enabled: boolean } }[] {
    return (this.composer?.passes ?? []).map((p) => ({ name: (p as { fbName?: string }).fbName ?? 'pass', pass: p }));
  }

  /** Draw calls, triangles… of the last frame, summed over all passes. */
  get info() {
    return this.renderer.info.render;
  }

  /** GPU resources held (geometries, textures, shader programs). */
  get memory() {
    return { ...this.renderer.info.memory, programs: this.renderer.info.programs?.length ?? 0 };
  }
}
