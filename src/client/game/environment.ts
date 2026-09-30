import * as THREE from 'three';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';

/**
 * Ambient light with no light source (the sun is the only light): an environment map prefiltered
 * for image-based lighting, rebuilt for each look from
 *   - three's RoomEnvironment (soft studio reflections), scaled by the look's `env`;
 *   - the look's sky and ground colours as a vertical gradient (what a hemisphere light gave).
 * The gradient's radiance is the hemisphere light's colour × intensity / π, 1.5× steeper around
 * the middle: cosine-weighted over a hemisphere, a gradient flattens to 2/3 of its slope.
 */
export class Ambient {
  private readonly room: THREE.WebGLCubeRenderTarget;
  private readonly scene = new THREE.Scene();
  private readonly material: THREE.ShaderMaterial;
  private readonly pmrem: THREE.PMREMGenerator;
  private target: THREE.WebGLRenderTarget | null = null;

  constructor(private readonly renderer: THREE.WebGLRenderer) {
    this.room = new THREE.WebGLCubeRenderTarget(256, { type: THREE.HalfFloatType, generateMipmaps: false });
    const room = new RoomEnvironment();
    const cam = new THREE.CubeCamera(0.1, 100, this.room);
    const autoClear = renderer.autoClear;
    renderer.autoClear = true;
    cam.update(renderer, room);
    renderer.autoClear = autoClear;
    room.dispose();
    this.material = new THREE.ShaderMaterial({
      side: THREE.BackSide,
      depthTest: false,
      depthWrite: false,
      uniforms: {
        tRoom: { value: this.room.texture },
        uRoom: { value: 0.3 },
        uSky: { value: new THREE.Color() },
        uGround: { value: new THREE.Color() },
      },
      vertexShader:
        'varying vec3 vDir; void main(){ vDir = position; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
      fragmentShader: `
        uniform samplerCube tRoom; uniform float uRoom; uniform vec3 uSky; uniform vec3 uGround; varying vec3 vDir;
        void main() {
          vec3 d = normalize(vDir);
          vec3 mid = (uSky + uGround) * 0.5;
          vec3 grad = max(mid + (uSky - uGround) * 0.75 * d.y, 0.0);
          gl_FragColor = vec4(textureCube(tRoom, d).rgb * uRoom + grad, 1.0);
        }`,
    });
    const sphere = new THREE.Mesh(new THREE.SphereGeometry(10, 32, 16), this.material);
    this.scene.add(sphere);
    this.pmrem = new THREE.PMREMGenerator(renderer);
  }

  /** The prefiltered environment for a look (the previous one is released). */
  build(env: number, hemi: { sky: string; ground: string; intensity: number }): THREE.Texture {
    const u = this.material.uniforms;
    u.uRoom!.value = env;
    const k = hemi.intensity / Math.PI;
    (u.uSky!.value as THREE.Color).set(hemi.sky).multiplyScalar(k);
    (u.uGround!.value as THREE.Color).set(hemi.ground).multiplyScalar(k);
    const autoClear = this.renderer.autoClear;
    this.renderer.autoClear = true;
    const next = this.pmrem.fromScene(this.scene, 0.04);
    this.renderer.autoClear = autoClear;
    this.target?.dispose();
    this.target = next;
    return next.texture;
  }
}
