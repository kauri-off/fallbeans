import * as THREE from 'three';
import type { ResolvedLook } from '../../sim/looks';
import { GpuTimer } from '../debug/gpuTimer';
import { Ambient } from './environment';
import { FSR_SCALE, type Upscale } from './fsr';
import { lod } from './lod';
import { setMaxAnisotropy } from './materials';
import { PostPipeline } from './postfx';
import { ShadowBake } from './shadowBake';
import type { Statics } from './statics';
import { AO_QUALITY, type AoQuality } from './xegtao';

export type Quality = 'medium' | 'high';

interface Preset {
  pixelRatio: number;
  shadowMap: number;
  shadowRange: number;
  /**
   * Anti-aliasing is SMAA 4x on every preset: SMAA 1x (edges) + 2x MSAA (spatial multisampling)
   * + 2x temporal supersampling (jittered frames blended with the reprojected previous one).
   */
  msaa: number;
  /** Ambient occlusion (XeGTAO at half resolution), or none. */
  ao: AoQuality | null;
  /** LOD distances multiplier (further detail on better presets). */
  lodBias: number;
}

const PRESETS: Record<Quality, Preset> = {
  medium: { pixelRatio: 1, shadowMap: 1024, shadowRange: 28, msaa: 2, ao: null, lodBias: 0.75 },
  high: { pixelRatio: 1.5, shadowMap: 2048, shadowRange: 34, msaa: 2, ao: AO_QUALITY.high, lodBias: 1 },
};

/**
 * Graphics features that can be switched off one by one (settings, profiler, benchmarks): each
 * only ever removes work from the preset (the ambient occlusion stays off on medium).
 */
export interface Effects {
  /** The sun's shadow (the shadow map is not drawn at all). */
  shadows: boolean;
  /** Ambient occlusion. */
  ao: boolean;
  /** Colour grade: saturation, contrast, vignette. */
  grade: boolean;
  /** Multisampling of the scene (the spatial part of the anti-aliasing). */
  msaa: boolean;
  /** SMAA edge anti-aliasing. */
  smaa: boolean;
  /** Temporal anti-aliasing (also smooths the LOD cross-fades and the AO noise). */
  temporal: boolean;
  /** Specks drifting in the air. */
  motes: boolean;
}

export const DEFAULT_EFFECTS: Effects = {
  shadows: true,
  ao: true,
  grade: true,
  msaa: true,
  smaa: true,
  temporal: true,
  motes: true,
};

export type { Upscale };

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
 * The sun's shadow map: one plain map (no cascades), 16-bit depth (plenty for its 140 m range, half
 * the bandwidth of the default) and a minimal colour attachment (never written: see depthOnly).
 */
function shadowTarget(light: THREE.DirectionalLight): THREE.WebGLRenderTarget {
  const { x, y } = light.shadow.mapSize;
  const rt = new THREE.WebGLRenderTarget(x, y, {
    format: THREE.RedFormat,
    minFilter: THREE.NearestFilter,
    magFilter: THREE.NearestFilter,
    generateMipmaps: false,
  });
  rt.texture.name = `${light.name}.shadowMapColor`;
  const d = new THREE.DepthTexture(x, y, THREE.UnsignedShortType);
  d.name = `${light.name}.shadowMap`;
  d.format = THREE.DepthFormat;
  d.compareFunction = THREE.LessEqualCompare;
  d.minFilter = THREE.LinearFilter;
  d.magFilter = THREE.LinearFilter;
  rt.depthTexture = d;
  return rt;
}

/**
 * WebGL2 renderer: one light (the sun) with a soft shadow following the camera, image-based
 * ambient light (see environment.ts), neutral tone
 * mapping, a light grade, SMAA 4x anti-aliasing, levels of detail with cross-fades, (high)
 * half-resolution XeGTAO and FSR 1 upscaling (see postfx.ts).
 */
export class Renderer {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(70, 1, 0.25, 1100);
  readonly sun = new THREE.DirectionalLight('#fff1dc', 2.2);
  private preset: Preset = PRESETS.high;
  quality: Quality = 'high';
  private readonly time = { value: 0 };
  readonly sky = skyDome(this.time);
  readonly motes = motes(this.time);
  /** GPU timing (debug): off, the whole frame, or each pass (scene, shadows, AO…). */
  private gpu: GpuTimer | null = null;
  private gpuMode: 'off' | 'frame' | 'passes' = 'off';
  private readonly wrapped = new WeakSet<object>();
  private readonly post: PostPipeline;
  /** Switchable features (see Effects). */
  readonly fx: Effects = { ...DEFAULT_EFFECTS };
  /** FSR mode: the scene renders below the display resolution and is upscaled. */
  upscale: Upscale = 'off';
  /** Static meshes of the current map (their shadows are baked once, not drawn every frame). */
  statics: Statics | null = null;
  readonly bake: ShadowBake;
  private readonly ambient: Ambient;

  constructor(readonly canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance', stencil: false });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.95;
    // Every pass clears what it needs itself (no clear before each full-screen pass).
    this.renderer.autoClear = false;
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFShadowMap;
    // Totals per frame over all passes (reset in render()).
    this.renderer.info.autoReset = false;
    setMaxAnisotropy(this.renderer.capabilities.getMaxAnisotropy());

    this.ambient = new Ambient(this.renderer);
    this.scene.environment = this.ambient.build(0.3, { sky: '#cfe8ff', ground: '#b99be0', intensity: 0.9 });
    this.scene.fog = new THREE.Fog(SKY_HORIZON.clone().lerp(SKY_TOP, 0.25), 120, 520);
    this.scene.add(this.sky, this.motes);
    this.sun.name = 'sun';
    this.sun.castShadow = true;
    this.sun.shadow.bias = -0.0003;
    this.sun.shadow.normalBias = 0.035;
    this.sun.shadow.radius = 4;
    this.scene.add(this.sun, this.sun.target);
    this.bake = new ShadowBake(this.renderer, SUN_OFFSET, this.scene);
    this.post = new PostPipeline(this.renderer, this.scene, this.camera);
    this.depthOnly();
  }

  /** The shadow map is depth only: colour writes stay off while it renders. */
  private depthOnly() {
    type Render = (lights: THREE.Light[], scene: THREE.Object3D, camera: THREE.Camera) => void;
    const sm = this.renderer.shadowMap as unknown as { render: Render };
    const orig = sm.render.bind(sm);
    const color = this.renderer.state.buffers.color;
    sm.render = (lights, scene, camera) => {
      this.sun.shadow.map ??= shadowTarget(this.sun);
      color.setMask(false);
      color.setLocked(true);
      try {
        orig(lights, scene, camera);
      } finally {
        color.setLocked(false);
        color.setMask(true);
      }
    };
  }

  setQuality(q: Quality) {
    this.quality = q;
    this.preset = PRESETS[q];
    const p = this.preset;
    lod.bias = p.lodBias;
    this.sun.shadow.mapSize.set(p.shadowMap, p.shadowMap);
    this.sun.shadow.map?.depthTexture?.dispose();
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
    const scale = FSR_SCALE[this.upscale];
    (this.motes.material as THREE.ShaderMaterial).uniforms.px!.value = pr * scale * (h / 900);
    this.camera.aspect = w / Math.max(1, h);
    this.camera.updateProjectionMatrix();
    const size = this.renderer.getDrawingBufferSize(new THREE.Vector2());
    const p = this.preset;
    const fx = this.fx;
    this.post.configure({
      width: size.x,
      height: size.y,
      scale,
      msaa: this.debugMsaa ?? (fx.msaa ? p.msaa : 0),
      ao: fx.ao ? p.ao : null,
      grade: fx.grade,
      smaa: fx.smaa,
      temporal: fx.temporal,
    });
  }

  /** Switches features on or off. */
  setEffects(patch: Partial<Effects>) {
    const before = { ...this.fx };
    Object.assign(this.fx, patch);
    this.motes.visible = this.fx.motes;
    if (before.shadows !== this.fx.shadows) this.applyShadows();
    if (JSON.stringify(this.fx) !== JSON.stringify(before)) this.resize();
  }

  /** FSR mode (render scale). */
  setUpscale(u: Upscale) {
    if (u === this.upscale) return;
    this.upscale = u;
    this.resize();
    this.cut();
  }

  /** The sun's shadow on or off (its castShadow changes the shader programs: nothing samples a stale map). */
  private applyShadows() {
    const on = this.fx.shadows;
    this.renderer.shadowMap.enabled = on;
    this.sun.castShadow = on;
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
    const fog = this.scene.fog as THREE.Fog;
    fog.color.set(look.fog.color);
    fog.near = look.fog.near;
    fog.far = look.fog.far;
    this.renderer.toneMappingExposure = look.exposure;
    this.scene.environment = this.ambient.build(look.env, look.hemi);
    this.scene.environmentIntensity = 1;
    this.post.setGrade(look.saturation);
    const motes = (this.motes.material as THREE.ShaderMaterial).uniforms;
    (motes.tint!.value as THREE.Color).set(look.motes.color);
    motes.rise!.value = look.motes.rise;
  }

  /** A camera cut: the previous frame is not reused by the temporal anti-aliasing. */
  cut() {
    this.post.reset();
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
    if (this.fx.shadows) this.bake.update(this.statics, (this.preset.shadowRange * 2) / this.preset.shadowMap);
    for (let i = 0; i < this.debugRepeat; i++) {
      if (i) this.renderer.info.reset();
      this.post.jitter();
      this.post.render();
      this.post.unjitter();
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

  /** Post stages (and the shadow map render inside the scene stage) time themselves on the GPU. */
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
    for (const st of this.post.stages) {
      if (this.wrapped.has(st)) continue;
      this.wrapped.add(st);
      const orig = st.render;
      st.render = () => {
        const g = this.gpuMode === 'passes' ? this.gpu : null;
        g?.begin(st.name);
        orig();
        g?.end();
      };
    }
  }

  /** Post stages by name (profiler experiments switch them off). */
  get passes(): { name: string; pass: { enabled: boolean } }[] {
    return this.post.stages.map((st) => ({ name: st.name, pass: st }));
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
