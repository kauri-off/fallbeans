import * as THREE from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { GTAOPass } from 'three/addons/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { ShaderPass } from 'three/addons/postprocessing/ShaderPass.js';
import { SMAAPass } from 'three/addons/postprocessing/SMAAPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
import { setMaxAnisotropy } from './materials';

export type Quality = 'medium' | 'high' | 'ultra';

interface Preset {
  pixelRatio: number;
  shadowMap: number;
  shadowRange: number;
  msaa: number;
  /** Post-process edge smoothing (for presets without MSAA, or on top of it). */
  smaa: boolean;
  /** A barely-there glow on bright highlights. */
  bloom: boolean;
  ao: boolean;
}

const PRESETS: Record<Quality, Preset> = {
  medium: { pixelRatio: 1, shadowMap: 1024, shadowRange: 28, msaa: 0, smaa: true, bloom: false, ao: false },
  high: { pixelRatio: 1.5, shadowMap: 2048, shadowRange: 34, msaa: 4, smaa: false, bloom: false, ao: false },
  ultra: { pixelRatio: 2, shadowMap: 4096, shadowRange: 40, msaa: 4, smaa: true, bloom: true, ao: true },
};

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
/** High sun: shadows fall close under the beans, which helps judging jumps. */
const SUN_OFFSET = new THREE.Vector3(9, 40, 7);

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
      sunDir: { value: SUN_OFFSET.clone().normalize() },
      time,
    },
    vertexShader:
      'varying vec3 vP; void main(){ vP = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }',
    fragmentShader: `
      uniform vec3 top; uniform vec3 horizon; uniform vec3 sunDir; uniform float time; varying vec3 vP;
      float h2(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
      float n2(vec2 p){ vec2 i = floor(p), f = fract(p); vec2 u = f * f * (3.0 - 2.0 * f);
        return mix(mix(h2(i), h2(i + vec2(1.0, 0.0)), u.x), mix(h2(i + vec2(0.0, 1.0)), h2(i + vec2(1.0, 1.0)), u.x), u.y); }
      float fbm(vec2 p){ float s = 0.0, a = 0.5; for (int i = 0; i < 5; i++) { s += a * n2(p); p = p * 2.03 + vec2(1.7, 9.2); a *= 0.5; } return s; }
      void main(){
        vec3 d = normalize(vP);
        float h = clamp(d.y * 1.6 + 0.12, 0.0, 1.0);
        vec3 col = mix(horizon, top, pow(h, 0.8));
        float sd = max(dot(d, sunDir), 0.0);
        col += vec3(1.0, 0.93, 0.8) * (pow(sd, 700.0) * 2.0 + pow(sd, 10.0) * 0.18);
        if (d.y > 0.0) {
          vec2 uv = d.xz / (d.y + 0.18) * 1.3;
          vec2 wind = vec2(time * 0.010, time * 0.004);
          float c = fbm(uv + wind) + 0.25 * fbm(uv * 3.1 - wind * 2.5);
          c = smoothstep(0.62, 0.95, c);
          float fade = smoothstep(0.02, 0.3, d.y);
          vec3 cc = mix(vec3(1.0), horizon, 0.18) + vec3(1.0, 0.95, 0.85) * pow(sd, 4.0) * 0.25;
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
    },
    vertexShader: `
      attribute float seed; uniform float time; uniform vec3 box; uniform vec3 center; uniform float px; varying float vA;
      void main(){
        vec3 drift = vec3(sin(time * 0.3 + seed * 40.0) * 0.8 + time * 0.35, sin(time * 0.5 + seed * 17.0) * 0.6 + time * 0.12, cos(time * 0.27 + seed * 23.0) * 0.8);
        vec3 p = mod(position + drift - center + box * 0.5, box) - box * 0.5 + center;
        vec4 mv = modelViewMatrix * vec4(p, 1.0);
        gl_Position = projectionMatrix * mv;
        float d = -mv.z;
        vA = smoothstep(1.5, 4.0, d) * (1.0 - smoothstep(14.0, 22.0, d)) * (0.55 + 0.45 * sin(time * (1.5 + seed * 2.0) + seed * 60.0));
        gl_PointSize = px * (0.8 + seed * 1.6) * 36.0 / max(d, 0.5);
      }`,
    fragmentShader: `
      varying float vA;
      void main(){ vec2 c = gl_PointCoord - 0.5; float r = dot(c, c); float a = smoothstep(0.25, 0.0, r) * vA; gl_FragColor = vec4(vec3(1.0, 0.97, 0.88) * a * 0.55, a); }`,
  });
  const p = new THREE.Points(g, mat);
  p.frustumCulled = false;
  p.renderOrder = 5;
  return p;
}

/**
 * WebGL2 renderer with the high-quality preset (RTX 3060 class): MSAA through the composer,
 * soft shadows following the camera, image-based lighting, neutral tone mapping, a light grade, SMAA
 * and (ultra) GTAO with a barely-there bloom.
 */
export class Renderer {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(70, 1, 0.25, 1100);
  readonly sun = new THREE.DirectionalLight('#fff1dc', 2.2);
  private composer: EffectComposer | null = null;
  private preset: Preset = PRESETS.high;
  quality: Quality = 'high';
  private readonly time = { value: 0 };
  private readonly sky = skyDome(this.time);
  private readonly motes = motes(this.time);

  constructor(readonly canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance', stencil: false });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.95;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFShadowMap;
    setMaxAnisotropy(this.renderer.capabilities.getMaxAnisotropy());

    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environmentIntensity = 0.3;
    pmrem.dispose();
    this.scene.fog = new THREE.Fog(SKY_HORIZON.clone().lerp(SKY_TOP, 0.25), 120, 520);
    this.scene.add(this.sky, this.motes);
    this.scene.add(new THREE.HemisphereLight('#cfe8ff', '#b99be0', 0.9));
    this.sun.castShadow = true;
    this.sun.shadow.bias = -0.0003;
    this.sun.shadow.normalBias = 0.035;
    this.sun.shadow.radius = 4;
    this.sun.shadow.blurSamples = 12;
    this.scene.add(this.sun, this.sun.target);
  }

  setQuality(q: Quality) {
    this.quality = q;
    this.preset = PRESETS[q];
    const p = this.preset;
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

  resize() {
    const w = this.canvas.clientWidth || window.innerWidth;
    const h = this.canvas.clientHeight || window.innerHeight;
    const pr = Math.min(window.devicePixelRatio || 1, this.preset.pixelRatio);
    this.renderer.setPixelRatio(pr);
    this.renderer.setSize(w, h, false);
    (this.motes.material as THREE.ShaderMaterial).uniforms.px!.value = pr * (h / 900);
    this.camera.aspect = w / Math.max(1, h);
    this.camera.updateProjectionMatrix();
    this.buildComposer(w, h, pr);
  }

  private buildComposer(w: number, h: number, pr: number) {
    this.composer?.dispose();
    const p = this.preset;
    const target = new THREE.WebGLRenderTarget(w * pr, h * pr, { type: THREE.HalfFloatType, samples: p.msaa });
    const c = new EffectComposer(this.renderer, target);
    c.setPixelRatio(pr);
    c.setSize(w, h);
    c.addPass(new RenderPass(this.scene, this.camera));
    if (p.ao) {
      const ao = new GTAOPass(this.scene, this.camera, w, h);
      ao.blendIntensity = 0.55;
      c.addPass(ao);
    }
    if (p.bloom) c.addPass(new UnrealBloomPass(new THREE.Vector2(w, h), 0.06, 0.3, 0.97));
    c.addPass(new OutputPass());
    c.addPass(new ShaderPass(GradeShader));
    if (p.smaa) c.addPass(new SMAAPass());
    this.composer = c;
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
    if (this.composer) this.composer.render();
    else this.renderer.render(this.scene, this.camera);
  }

  get info() {
    return this.renderer.info.render;
  }
}
