import * as THREE from 'three';
import type { Collider, CollisionWorld, Contact } from '../../sim/physics';

const hit: Contact = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
const probe = new THREE.Vector3();
const near: Collider[] = [];

/** Third-person orbit camera that pulls in instead of clipping through walls. */
export class CameraRig {
  yaw = 0;
  pitch = 0.32;
  distance = 8.5;
  private readonly target = new THREE.Vector3();
  private readonly smoothTarget = new THREE.Vector3();
  private arm = 8.5;
  private first = true;
  /** Shake energy (0…1), decays; the offset grows with its square. */
  private trauma = 0;
  private shakeT = 0;

  constructor(readonly camera: THREE.PerspectiveCamera) {}

  look(dx: number, dy: number) {
    this.yaw -= dx;
    this.pitch = THREE.MathUtils.clamp(this.pitch + dy, -0.55, 1.25);
  }

  /** Faces the camera along a yaw (e.g. down the course at the start of a round). */
  face(yaw: number) {
    this.yaw = yaw + Math.PI;
    this.pitch = 0.32;
  }

  snap() {
    this.first = true;
  }

  shake(amount: number) {
    this.trauma = Math.min(1, this.trauma + amount);
  }

  private applyShake(dt: number) {
    if (this.trauma <= 0) return;
    this.shakeT += dt;
    const k = this.trauma * this.trauma * 0.35;
    const t = this.shakeT * 31;
    this.camera.position.x += Math.sin(t * 1.1) * Math.sin(t * 0.37 + 1) * k;
    this.camera.position.y += Math.sin(t * 1.3 + 2) * Math.sin(t * 0.41) * k;
    this.camera.position.z += Math.sin(t * 0.9 + 4) * Math.sin(t * 0.53 + 3) * k;
    this.trauma = Math.max(0, this.trauma - dt * 1.8);
  }

  update(focus: THREE.Vector3, dt: number, world: CollisionWorld | null) {
    this.target.set(focus.x, focus.y + 1.4, focus.z);
    if (this.first) {
      this.smoothTarget.copy(this.target);
      this.first = false;
    } else this.smoothTarget.lerp(this.target, 1 - Math.exp(-dt * 14));
    const cp = Math.cos(this.pitch);
    const dir = new THREE.Vector3(Math.sin(this.yaw) * cp, Math.sin(this.pitch), Math.cos(this.yaw) * cp);
    let want = this.distance;
    if (world) {
      // March along the arm; stop just before the first solid collider.
      const steps = 12;
      for (let i = 1; i <= steps; i++) {
        const d = (this.distance * i) / steps;
        probe.copy(this.smoothTarget).addScaledVector(dir, d);
        const cols = world.query(probe.x, probe.z, 0.6, near);
        if (cols.some((c) => c.enabled && !c.hit && c.contact(probe, 0.3, hit))) {
          want = Math.max(1.2, d - this.distance / steps);
          break;
        }
      }
    }
    this.arm = want < this.arm ? want : this.arm + (want - this.arm) * (1 - Math.exp(-dt * 4));
    this.camera.position.copy(this.smoothTarget).addScaledVector(dir, this.arm);
    this.camera.lookAt(this.smoothTarget);
    this.applyShake(dt);
  }

  /** Scripted shot (intro fly-over, podium): smoothly moves the camera to eye, looking at look. */
  cinematic(eye: THREE.Vector3, look: THREE.Vector3, dt: number, snap = false) {
    const k = snap ? 1 : 1 - Math.exp(-dt * 3);
    this.camera.position.lerp(eye, k);
    this.lookAt.lerp(look, k);
    this.camera.lookAt(this.lookAt);
    // Hand over to the follow camera without a jump.
    this.first = true;
  }

  private readonly lookAt = new THREE.Vector3();

  /** World-space movement for camera-relative stick input. */
  toWorld(moveX: number, moveY: number): [number, number] {
    const fx = -Math.sin(this.yaw);
    const fz = -Math.cos(this.yaw);
    // Right vector: forward rotated by −90° around y.
    const rx = -fz;
    const rz = fx;
    return [fx * moveY + rx * moveX, fz * moveY + rz * moveX];
  }
}
