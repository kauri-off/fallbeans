import * as THREE from 'three';
import { clone } from './assets.js';

const bodyMats = new Map();
function bodyMaterial(color) {
  if (!bodyMats.has(color)) bodyMats.set(color, new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.45 }));
  return bodyMats.get(color);
}

function makeTag(text, color) {
  const c = document.createElement('canvas');
  c.width = 256; c.height = 64;
  const g = c.getContext('2d');
  g.font = 'bold 30px Trebuchet MS, sans-serif';
  const w = Math.min(248, g.measureText(text).width + 28);
  g.fillStyle = 'rgba(255,255,255,0.9)';
  g.beginPath(); g.roundRect((256 - w) / 2, 8, w, 46, 18); g.fill();
  g.fillStyle = color; g.fillRect((256 - w) / 2 + 10, 24, 12, 12);
  g.fillStyle = '#3a2372'; g.textAlign = 'center'; g.textBaseline = 'middle';
  g.fillText(text, 128 + 8, 32);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true }));
  s.scale.set(2.4, 0.6, 1);
  s.renderOrder = 10;
  return s;
}

export class Bean {
  constructor(color, name, showTag = true) {
    this.root = new THREE.Group();
    this.model = clone('bean');
    this.root.add(this.model);
    this.parts = {};
    for (const n of ['ArmL', 'ArmR', 'LegL', 'LegR']) {
      const o = this.model.getObjectByName(n);
      this.parts[n] = { o, base: o.rotation.clone() };
    }
    this.setColor(color);
    this.tag = null;
    if (showTag) this.setName(name, color);
    this.phase = 0;
    this.squash = 0;
    this.pitch = 0;
    this.roll = 0;
    this.emote = 0;
    this.emoteT = 0;
    this.crown = null;
  }

  setColor(color) {
    this.color = color;
    const m = bodyMaterial(color);
    this.model.traverse((o) => { if (o.isMesh && (o.material.name === 'Body' || o.userData.body)) { o.material = m; o.userData.body = true; } });
    if (this.tag && this.name) this.setName(this.name, color);
  }

  setName(name, color = this.color) {
    this.name = name;
    if (this.tag) { this.root.remove(this.tag); this.tag.material.map.dispose(); this.tag.material.dispose(); }
    this.tag = makeTag(name, color);
    this.tag.position.y = 2.3;
    this.root.add(this.tag);
  }

  setCrown(on) {
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

  playEmote(e) { this.emote = e; this.emoteT = 2.2; }

  animate(dt, speed, a, time, landImpact = 0) {
    const P = this.parts;
    const k = Math.min(1, speed / 7);
    const lerp = (x, y, f) => x + (y - x) * Math.min(1, f * dt);
    let armX = 0, legX = 0, armZ = 0, targetPitch = 0, targetRoll = 0, lift = 0, mirror = false;
    this.emoteT -= dt;
    if (landImpact > 0.15) this.squash = Math.max(this.squash, landImpact * 0.35);
    this.squash = lerp(this.squash, 0, 10);

    if (a === 2 || a === 5) {
      targetPitch = 1.35; armX = -2.9; legX = 0.3; lift = a === 2 ? 0.45 : 0.35;
    } else if (a === 3) {
      targetPitch = Math.sin(time * 11) * 0.6; targetRoll = Math.cos(time * 9) * 0.6;
      armX = Math.sin(time * 20) * 1.5; armZ = 1.2; legX = Math.cos(time * 20) * 0.8;
    } else if (a === 1) {
      armX = -2.4; armZ = 0.5; legX = 0.5 * Math.sin(time * 6);
    } else if (a === 4) {
      armX = -1.6; targetPitch = 0.2;
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
        if (this.emote === 1) { armX = -2.8 + Math.sin(time * 14) * 0.4; armZ = 0.4; lift = Math.abs(Math.sin(time * 8)) * 0.4; }
        else if (this.emote === 2) { armX = -2.2; armZ = Math.sin(time * 10) * 0.8; targetRoll = Math.sin(time * 5) * 0.25; }
        else if (this.emote === 3) { targetPitch = -0.3; armX = -0.6; armZ = 1.3 + Math.sin(time * 16) * 0.2; lift = Math.abs(Math.sin(time * 6)) * 0.2; }
      } else {
        this.model.scale.y = 1 + Math.sin(time * 3) * 0.015 * (1 - k);
      }
    }
    this.pitch = lerp(this.pitch, targetPitch, 12);
    this.roll = lerp(this.roll, targetRoll, 12);
    this.model.rotation.set(this.pitch, 0, this.roll);
    this.model.position.y = lerp(this.model.position.y, lift, 14);
    const sq = this.squash;
    this.model.scale.set(1 + sq * 0.6, 1 - sq, 1 + sq * 0.6);

    const set = (p, x, z, sign) => {
      p.o.rotation.x = lerp(p.o.rotation.x, p.base.x + x, 18);
      p.o.rotation.z = lerp(p.o.rotation.z, p.base.z + z * sign, 18);
    };
    set(P.ArmL, armX, -armZ, 1);
    set(P.ArmR, mirror ? -armX : armX, armZ, 1);
    set(P.LegL, legX, 0, 1);
    set(P.LegR, -legX, 0, 1);
  }
}
