import * as THREE from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { GTAOPass } from 'three/addons/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';

export type Quality = 'medium' | 'high' | 'ultra';

interface Preset {
  pixelRatio: number;
  shadowMap: number;
  shadowRange: number;
  msaa: number;
  bloom: boolean;
  ao: boolean;
}

const PRESETS: Record<Quality, Preset> = {
  medium: { pixelRatio: 1, shadowMap: 1024, shadowRange: 28, msaa: 0, bloom: false, ao: false },
  high: { pixelRatio: 1.5, shadowMap: 2048, shadowRange: 34, msaa: 4, bloom: true, ao: false },
  ultra: { pixelRatio: 2, shadowMap: 4096, shadowRange: 40, msaa: 4, bloom: true, ao: true },
};

const SKY_TOP = new THREE.Color('#6fb8ff');
const SKY_HORIZON = new THREE.Color('#ffd9f2');

function skyDome(): THREE.Mesh {
  const geo = new THREE.SphereGeometry(900, 32, 16);
  const mat = new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    fog: false,
    uniforms: { top: { value: SKY_TOP }, horizon: { value: SKY_HORIZON } },
    vertexShader:
      'varying vec3 vP; void main(){ vP = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position,1.0); }',
    fragmentShader:
      'uniform vec3 top; uniform vec3 horizon; varying vec3 vP; void main(){ float h = clamp(vP.y * 1.6 + 0.12, 0.0, 1.0); gl_FragColor = vec4(mix(horizon, top, pow(h, 0.8)), 1.0); }',
  });
  const m = new THREE.Mesh(geo, mat);
  m.renderOrder = -10;
  m.frustumCulled = false;
  return m;
}

/**
 * WebGL2 renderer with the high-quality preset (RTX 3060 class): MSAA through the composer,
 * soft shadows following the camera, image-based lighting, neutral tone mapping, bloom and optional GTAO.
 */
export class Renderer {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(70, 1, 0.1, 1200);
  readonly sun = new THREE.DirectionalLight('#fff1dc', 2.2);
  private composer: EffectComposer | null = null;
  private preset: Preset = PRESETS.high;
  quality: Quality = 'high';
  private readonly sunOffset = new THREE.Vector3(18, 32, 12);
  private readonly sky = skyDome();

  constructor(readonly canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance', stencil: false });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = 0.95;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFShadowMap;

    const pmrem = new THREE.PMREMGenerator(this.renderer);
    this.scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environmentIntensity = 0.3;
    pmrem.dispose();
    this.scene.fog = new THREE.Fog(SKY_HORIZON.clone().lerp(SKY_TOP, 0.25), 120, 520);
    this.scene.add(this.sky);
    this.scene.add(new THREE.HemisphereLight('#cfe8ff', '#b99be0', 0.9));
    this.sun.castShadow = true;
    this.sun.shadow.bias = -0.0004;
    this.sun.shadow.normalBias = 0.03;
    this.sun.shadow.radius = 3;
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
      ao.blendIntensity = 0.7;
      c.addPass(ao);
    }
    if (p.bloom) c.addPass(new UnrealBloomPass(new THREE.Vector2(w, h), 0.22, 0.45, 0.92));
    c.addPass(new OutputPass());
    this.composer = c;
  }

  /** Keeps the shadow frustum centred on the action, snapped to texels to avoid shimmering. */
  focus(p: THREE.Vector3) {
    const texel = (this.preset.shadowRange * 2) / this.preset.shadowMap;
    const x = Math.round(p.x / texel) * texel;
    const z = Math.round(p.z / texel) * texel;
    this.sun.target.position.set(x, p.y, z);
    this.sun.position.set(x + this.sunOffset.x, p.y + this.sunOffset.y, z + this.sunOffset.z);
    this.sky.position.copy(this.camera.position);
  }

  render() {
    if (this.composer) this.composer.render();
    else this.renderer.render(this.scene, this.camera);
  }

  get info() {
    return this.renderer.info.render;
  }
}
