import * as THREE from 'three';
import { ANIM } from '../../shared/consts';
import { clone } from './assets';

const bodyMats = new Map<string, THREE.MeshStandardMaterial>();
function bodyMaterial(color: string) {
  let m = bodyMats.get(color);
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.45 });
    bodyMats.set(color, m);
  }
  return m;
}

function makeTag(text: string, color: string) {
  const c = document.createElement('canvas');
  c.width = 256;
  c.height = 64;
  const g = c.getContext('2d')!;
  g.font = 'bold 30px Trebuchet MS, sans-serif';
  const w = Math.min(248, g.measureText(text).width + 28);
  g.fillStyle = 'rgba(255,255,255,0.9)';
  g.beginPath();
  g.roundRect((256 - w) / 2, 8, w, 46, 18);
  g.fill();
  g.fillStyle = color;
  g.fillRect((256 - w) / 2 + 10, 24, 12, 12);
  g.fillStyle = '#3a2372';
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.fillText(text, 128 + 8, 32);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true }));
  s.scale.set(2.4, 0.6, 1);
  s.renderOrder = 10;
  return s;
}

let tailProto: THREE.Object3D | null = null;
function makeTail(): THREE.Object3D {
  if (!tailProto) {
    const g = new THREE.Group();
    const mat = new THREE.MeshStandardMaterial({ color: '#ff9f1c', roughness: 0.7 });
    const tip = new THREE.MeshStandardMaterial({ color: '#fff4d6', roughness: 0.9 });
    const segs = 5;
    for (let i = 0; i < segs; i++) {
      const r = 0.2 - i * 0.025;
      const m = new THREE.Mesh(new THREE.SphereGeometry(r, 12, 8), i === segs - 1 ? tip : mat);
      m.position.set(0, 0.15 + i * 0.12, -0.15 - i * 0.16);
      m.castShadow = true;
      g.add(m);
    }
    tailProto = g;
  }
  return tailProto.clone(true);
}

interface Part {
  o: THREE.Object3D;
  base: THREE.Euler;
}

export class Bean {
  readonly root = new THREE.Group();
  readonly model: THREE.Object3D;
  private readonly parts: Record<'ArmL' | 'ArmR' | 'LegL' | 'LegR', Part>;
  color = '';
  name = '';
  private tag: THREE.Sprite | null = null;
  private crown: THREE.Object3D | null = null;
  private tail: THREE.Object3D | null = null;
  private phase = 0;
  private squash = 0;
  private pitch = 0;
  private roll = 0;
  private emote = 0;
  emoteT = 0;

  constructor(color: string, name: string, showTag = true) {
    this.model = clone('bean');
    this.root.add(this.model);
    const part = (n: string): Part => {
      const o = this.model.getObjectByName(n);
      if (!o) throw new Error(`bean model is missing ${n}`);
      return { o, base: o.rotation.clone() };
    };
    this.parts = { ArmL: part('ArmL'), ArmR: part('ArmR'), LegL: part('LegL'), LegR: part('LegR') };
    this.setColor(color);
    if (showTag) this.setName(name, color);
    else this.name = name;
  }

  setColor(color: string) {
    this.color = color;
    const m = bodyMaterial(color);
    this.model.traverse((o) => {
      if (o instanceof THREE.Mesh && ((o.material as THREE.Material).name === 'Body' || o.userData.body)) {
        o.material = m;
        o.userData.body = true;
      }
    });
    if (this.tag) this.setName(this.name, color);
  }

  setName(name: string, color = this.color) {
    this.name = name;
    if (this.tag) {
      this.root.remove(this.tag);
      this.tag.material.map?.dispose();
      this.tag.material.dispose();
    }
    this.tag = makeTag(name, color);
    this.tag.position.y = 2.3;
    this.root.add(this.tag);
  }

  setCrown(on: boolean) {
    if (on && !this.crown) {
      this.crown = clone('crown');
      this.crown.scale.setScalar(0.7);
      this.crown.position.set(0, 1.5, 0);
      this.crown.rotation.x = -0.15;
      this.model.add(this.crown);
    } else if (!on && this.crown) {
      this.model.remove(this.crown);
      this.crown = null;
    }
  }

  setTail(on: boolean) {
    if (on && !this.tail) {
      this.tail = makeTail();
      this.tail.position.set(0, 0.35, -0.35);
      this.model.add(this.tail);
    } else if (!on && this.tail) {
      this.model.remove(this.tail);
      this.tail = null;
    }
  }

  get hasTail() {
    return !!this.tail;
  }

  playEmote(e: number) {
    this.emote = e;
    this.emoteT = 2.2;
  }

  dispose() {
    this.root.removeFromParent();
    this.tag?.material.map?.dispose();
    this.tag?.material.dispose();
  }

  animate(dt: number, speed: number, a: number, time: number, landImpact = 0) {
    const P = this.parts;
    const k = Math.min(1, speed / 7);
    const lerp = (x: number, y: number, f: number) => x + (y - x) * Math.min(1, f * dt);
    let armX = 0;
    let legX = 0;
    let armZ = 0;
    let targetPitch = 0;
    let targetRoll = 0;
    let lift = 0;
    let mirror = false;
    this.emoteT -= dt;
    if (landImpact > 0.15) this.squash = Math.max(this.squash, landImpact * 0.35);
    this.squash = lerp(this.squash, 0, 10);

    if (a === ANIM.dive || a === ANIM.slide) {
      targetPitch = 1.35;
      armX = -2.9;
      legX = 0.3;
      lift = a === ANIM.dive ? 0.45 : 0.35;
    } else if (a === ANIM.stun) {
      targetPitch = Math.sin(time * 11) * 0.6;
      targetRoll = Math.cos(time * 9) * 0.6;
      armX = Math.sin(time * 20) * 1.5;
      armZ = 1.2;
      legX = Math.cos(time * 20) * 0.8;
    } else if (a === ANIM.air) {
      armX = -2.4;
      armZ = 0.5;
      legX = 0.5 * Math.sin(time * 6);
    } else if (a === ANIM.grab) {
      armX = -1.6;
      targetPitch = 0.2;
      this.phase += dt * speed * 1.8;
      legX = Math.sin(this.phase) * 0.8 * k;
    } else {
      this.phase += dt * Math.max(speed, 0.001) * 1.8;
      legX = Math.sin(this.phase) * 0.9 * k;
      armX = -Math.sin(this.phase) * 0.8 * k;
      mirror = true;
      targetPitch = 0.12 * k;
      lift = Math.abs(Math.sin(this.phase)) * 0.08 * k;
      if (this.emoteT > 0 && k < 0.2) {
        mirror = false;
        if (this.emote === 1) {
          armX = -2.8 + Math.sin(time * 14) * 0.4;
          armZ = 0.4;
          lift = Math.abs(Math.sin(time * 8)) * 0.4;
        } else if (this.emote === 2) {
          armX = -2.2;
          armZ = Math.sin(time * 10) * 0.8;
          targetRoll = Math.sin(time * 5) * 0.25;
        } else if (this.emote === 3) {
          targetPitch = -0.3;
          armX = -0.6;
          armZ = 1.3 + Math.sin(time * 16) * 0.2;
          lift = Math.abs(Math.sin(time * 6)) * 0.2;
        }
      }
    }
    if (this.tail) this.tail.rotation.y = Math.sin(time * 9) * 0.35 * (0.3 + k);
    this.pitch = lerp(this.pitch, targetPitch, 12);
    this.roll = lerp(this.roll, targetRoll, 12);
    this.model.rotation.set(this.pitch, 0, this.roll);
    this.model.position.y = lerp(this.model.position.y, lift, 14);
    const sq = this.squash;
    const breathe = k < 0.2 && this.emoteT <= 0 ? Math.sin(time * 3) * 0.015 * (1 - k) : 0;
    this.model.scale.set(1 + sq * 0.6, 1 - sq + breathe, 1 + sq * 0.6);

    const set = (p: Part, x: number, z: number) => {
      p.o.rotation.x = lerp(p.o.rotation.x, p.base.x + x, 18);
      p.o.rotation.z = lerp(p.o.rotation.z, p.base.z + z, 18);
    };
    set(P.ArmL, armX, -armZ);
    set(P.ArmR, mirror ? -armX : armX, armZ);
    set(P.LegL, legX, 0);
    set(P.LegR, -legX, 0);
  }
}
