import * as THREE from 'three';
import type { Builder } from '../../sim/builder';
import type { Collider } from '../../sim/physics';
import { clone, meshParts } from './assets';
import { cloudTint, decorate, island } from './decor';
import { atLevel } from './lod';
import { applySurface } from './materials';
import { patternMaterial, plainMaterial } from './view';

/** Half size of the cloud model at scale 1 (x/z and y). */
const CLOUD_R = 3.9;
const CLOUD_H = 2.1;
/** How far a cloud drifts from its home (m). */
const DRIFT = 2.5;

interface Box {
  min: THREE.Vector3;
  max: THREE.Vector3;
}

function colliderBox(c: Collider): Box {
  const e = c.cur.elements;
  const s = c.shape;
  const [hx, hy, hz] = s.type === 'box' ? [s.hx, s.hy, s.hz] : s.type === 'cyl' ? [s.r, s.hh, s.r] : [s.r, s.r, s.r];
  const ext = new THREE.Vector3(
    Math.abs(e[0]!) * hx + Math.abs(e[4]!) * hy + Math.abs(e[8]!) * hz,
    Math.abs(e[1]!) * hx + Math.abs(e[5]!) * hy + Math.abs(e[9]!) * hz,
    Math.abs(e[2]!) * hx + Math.abs(e[6]!) * hy + Math.abs(e[10]!) * hz,
  );
  return { min: c.center.clone().sub(ext), max: c.center.clone().add(ext) };
}

/** Is a sphere-ish volume (horizontal radius r, half height h) too close to anything solid? */
function blocked(boxes: readonly Box[], p: THREE.Vector3, r: number, h: number, margin: number) {
  for (const b of boxes) {
    const dx = Math.max(b.min.x - p.x, 0, p.x - b.max.x);
    const dz = Math.max(b.min.z - p.z, 0, p.z - b.max.z);
    if (Math.hypot(dx, dz) > r + margin) continue;
    // Far below the course is fine (you look down on them); above it they would hide the action.
    if (p.y + h < b.min.y - 16) continue;
    if (p.y - h > b.max.y + 32) continue;
    return true;
  }
  return false;
}

function bounds(boxes: readonly Box[]): Box {
  const min = new THREE.Vector3(Infinity, Infinity, Infinity);
  const max = new THREE.Vector3(-Infinity, -Infinity, -Infinity);
  for (const b of boxes) {
    min.min(b.min);
    max.max(b.max);
  }
  if (!boxes.length) return { min: new THREE.Vector3(-10, -1, -10), max: new THREE.Vector3(10, 1, 10) };
  return { min, max };
}

/**
 * Client-only scenery after a map is built and its colliders placed: drifting clouds kept clear
 * of the course, birds circling far out and hot-air balloons bobbing on the horizon, floating
 * islands, and the set pieces and the land below of the round's look (decor.ts).
 */
export function placeScenery(b: Builder) {
  if (!b.view) return;
  const boxes = b.world.colliders.map(colliderBox);
  const all = bounds(boxes);
  // Scenery randomness is visual only: its own generator, so map layouts stay as they are.
  let seed = b.seed ^ 0x5eed;
  const rnd = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  for (const req of b.scenery) clouds(b, boxes, req, rnd);
  if (b.look.birds) birds(b, all, rnd);
  if (b.look.balloons) balloons(b, boxes, all, rnd);
  islands(b, boxes, all, rnd);
  decorate(b, all, (p, r, h, margin) => blocked(boxes, p, r, h, margin), rnd);
}

/** Looks whose islands grow trees. */
const LEAFY = new Set(['classic', 'meadow', 'castle', 'circus', 'royal', 'jungle', 'ocean']);

/** Floating islands below and around the course, with trees, pines and mushrooms on top. */
function islands(b: Builder, boxes: readonly Box[], all: Box, rnd: () => number) {
  const cx = (all.min.x + all.max.x) / 2;
  const cz = (all.min.z + all.max.z) / 2;
  const reach = Math.hypot(all.max.x - all.min.x, all.max.z - all.min.z) / 2;
  const count = 7;
  const flora = ['tree', 'pine', 'mushroom', 'tree', 'pine'] as const;
  for (let k = 0; k < count; k++) {
    let pos: THREE.Vector3 | null = null;
    const scale = 0.8 + rnd() * 0.9;
    for (let tries = 0; tries < 24 && !pos; tries++) {
      const a = (k / count) * Math.PI * 2 + rnd() * 0.8;
      const d = reach * 0.6 + 18 + rnd() * 40;
      const c = new THREE.Vector3(cx + Math.cos(a) * d, all.min.y - 10 - rnd() * 22, cz + Math.sin(a) * d);
      if (!blocked(boxes, c, 4.5 * scale + 2, 7 * scale, 8)) pos = c;
    }
    if (!pos) continue;
    const g = island(b, b.look);
    g.scale.setScalar(scale);
    g.rotation.y = rnd() * 6.3;
    const n = LEAFY.has(b.look.id) ? 1 + Math.floor(rnd() * 3) : 0;
    for (let i = 0; i < n; i++) {
      const f = clone(flora[Math.floor(rnd() * flora.length)]!);
      const a = rnd() * 6.3;
      const r = i === 0 && n === 1 ? 0 : 1 + rnd() * 1.6;
      f.position.set(Math.cos(a) * r, 0.45, Math.sin(a) * r);
      f.rotation.y = rnd() * 6.3;
      f.scale.setScalar(0.55 + rnd() * 0.35);
      g.add(f);
    }
    g.position.copy(pos);
    g.userData.cat = 'islands';
    b.group.add(g);
    const home = pos.clone();
    const ph = rnd() * 50;
    const yaw = g.rotation.y;
    b.anim((t) => {
      g.position.y = home.y + Math.sin(t * 0.25 + ph) * 0.7;
      g.rotation.y = yaw + Math.sin(t * 0.05 + ph) * 0.15;
    });
  }
}

interface Cloud {
  home: THREE.Vector3;
  scale: number;
  yaw: number;
  spin: number;
  ph: number;
  w: number;
}

function clouds(b: Builder, boxes: readonly Box[], req: Builder['scenery'][number], rnd: () => number) {
  const list: Cloud[] = [];
  for (let i = 0; i < req.clouds; i++) {
    for (let tries = 0; tries < 40; tries++) {
      const a = rnd() * Math.PI * 2;
      const d = req.spread * (0.55 + rnd() * 0.9);
      const home = new THREE.Vector3(
        req.cx + Math.cos(a) * d,
        req.yMin + rnd() * (req.yMax - req.yMin),
        req.cz + Math.sin(a) * d * 1.2,
      );
      const scale = 1.4 + rnd() * 2.8;
      if (blocked(boxes, home, CLOUD_R * scale + DRIFT, CLOUD_H * scale, 10)) continue;
      list.push({ home, scale, yaw: rnd() * 6.3, spin: (rnd() - 0.5) * 0.04, ph: rnd() * 100, w: 0.05 + rnd() * 0.07 });
      break;
    }
  }
  if (!list.length) return;
  const parts = meshParts('cloud').map(({ mesh, local }) => {
    const mat = Array.isArray(mesh.material) ? mesh.material : cloudTint(b, b.look, mesh.material);
    const inst = new THREE.InstancedMesh(atLevel(mesh.geometry, 2), mat, list.length);
    inst.frustumCulled = false;
    inst.castShadow = false;
    inst.receiveShadow = false;
    inst.userData.cat = 'clouds';
    b.group.add(inst);
    b.view!.own({ dispose: () => inst.dispose() });
    return { inst, local };
  });
  const m = new THREE.Matrix4();
  const tmp = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const p = new THREE.Vector3();
  const s = new THREE.Vector3();
  const up = new THREE.Vector3(0, 1, 0);
  const place = (t: number) => {
    list.forEach((c, i) => {
      p.set(
        c.home.x + Math.sin(t * c.w + c.ph) * DRIFT,
        c.home.y + Math.sin(t * c.w * 2.3 + c.ph * 1.7) * 0.6,
        c.home.z + Math.cos(t * c.w * 0.8 + c.ph) * DRIFT,
      );
      q.setFromAxisAngle(up, c.yaw + t * c.spin);
      const breathe = 1 + Math.sin(t * 0.35 + c.ph) * 0.035;
      s.set(c.scale * breathe, c.scale * (2 - breathe), c.scale * breathe);
      m.compose(p, q, s);
      for (const part of parts) part.inst.setMatrixAt(i, tmp.multiplyMatrices(m, part.local));
    });
    for (const part of parts) part.inst.instanceMatrix.needsUpdate = true;
  };
  place(0);
  for (const part of parts) part.inst.computeBoundingSphere();
  b.anim((t) => place(t));
}

/** A few small flocks circling well outside the course. */
function birds(b: Builder, all: Box, rnd: () => number) {
  const flocks = 2;
  const per = 5;
  const n = flocks * per;
  const mat = applySurface(new THREE.MeshStandardMaterial({ color: '#4b3d7a', roughness: 0.7 }), 'fabric');
  b.view!.own(mat);
  const body = b.view!.own(new THREE.SphereGeometry(0.22, 10, 8).scale(0.8, 0.7, 1.6));
  const wing = b.view!.own(new THREE.BoxGeometry(0.9, 0.04, 0.34).translate(0.45, 0, 0));
  const mk = (g: THREE.BufferGeometry) => {
    const inst = new THREE.InstancedMesh(g, mat, n);
    inst.frustumCulled = false;
    inst.userData.cat = 'birds';
    b.group.add(inst);
    b.view!.own({ dispose: () => inst.dispose() });
    return inst;
  };
  const bodies = mk(body);
  const left = mk(wing);
  const right = mk(wing);
  const cx = (all.min.x + all.max.x) / 2;
  const cz = (all.min.z + all.max.z) / 2;
  const reach = Math.hypot(all.max.x - all.min.x, all.max.z - all.min.z) / 2;
  const fl = Array.from({ length: flocks }, (_, k) => ({
    r: reach + 25 + rnd() * 30,
    y: all.max.y + 10 + rnd() * 14,
    speed: (0.06 + rnd() * 0.05) * (k % 2 ? -1 : 1),
    a0: rnd() * 6.3,
  }));
  const offs = Array.from({ length: per }, (_, i) => ({
    back: i * 1.3,
    side: (i % 2 ? 1 : -1) * Math.ceil(i / 2) * 1.1,
    ph: rnd() * 6,
  }));
  const m = new THREE.Matrix4();
  const w = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const e = new THREE.Euler();
  const p = new THREE.Vector3();
  const one = new THREE.Vector3(1, 1, 1);
  const flip = new THREE.Matrix4().makeRotationY(Math.PI);
  b.anim((t) => {
    let i = 0;
    for (const f of fl) {
      const a = f.a0 + t * f.speed;
      const dir = Math.sign(f.speed);
      for (const o of offs) {
        const aa = a - (o.back / f.r) * dir;
        const rr = f.r + o.side;
        p.set(cx + Math.cos(aa) * rr, f.y + Math.sin(t * 0.7 + o.ph) * 0.8 + o.back * 0.15, cz + Math.sin(aa) * rr);
        // Heading along the circle.
        const yaw = Math.atan2(-Math.sin(aa) * dir, Math.cos(aa) * dir);
        q.setFromEuler(e.set(0, yaw, Math.sin(t * 0.5 + o.ph) * 0.15 - dir * 0.25));
        m.compose(p, q, one);
        bodies.setMatrixAt(i, m);
        const flap = Math.sin(t * 9 + o.ph) * 0.7 + 0.1;
        w.makeRotationZ(flap);
        left.setMatrixAt(i, w.premultiply(m));
        w.makeRotationZ(flap).premultiply(flip);
        right.setMatrixAt(i, w.premultiply(m));
        i++;
      }
    }
    bodies.instanceMatrix.needsUpdate = true;
    left.instanceMatrix.needsUpdate = true;
    right.instanceMatrix.needsUpdate = true;
  });
}

/** Striped hot-air balloons far out, slowly rising, sinking and turning. */
function balloons(b: Builder, boxes: readonly Box[], all: Box, rnd: () => number) {
  const pals: [string, string][] = [
    ['#ff8cc8', '#ffffff'],
    ['#ffd84a', '#ff9f4a'],
    ['#7ccfff', '#ffffff'],
    ['#a98bff', '#ffd84a'],
    ['#6fe08a', '#ffffff'],
  ];
  const cx = (all.min.x + all.max.x) / 2;
  const cz = (all.min.z + all.max.z) / 2;
  const reach = Math.hypot(all.max.x - all.min.x, all.max.z - all.min.z) / 2;
  const basket = plainMaterial('#b07a4a', {}, 'wood');
  const rope = plainMaterial('#6b4f3a', {}, 'fabric');
  const count = 4;
  for (let k = 0; k < count; k++) {
    let pos: THREE.Vector3 | null = null;
    for (let tries = 0; tries < 20 && !pos; tries++) {
      const a = (k / count) * Math.PI * 2 + rnd() * 1.2;
      const d = reach + 45 + rnd() * 50;
      const c = new THREE.Vector3(cx + Math.cos(a) * d, all.max.y + 4 + rnd() * 22, cz + Math.sin(a) * d);
      if (!blocked(boxes, c, 6, 6, 10)) pos = c;
    }
    if (!pos) continue;
    const [c1, c2] = pals[Math.floor(rnd() * pals.length)]!;
    const g = new THREE.Group();
    const env = new THREE.Mesh(
      b.view!.own(new THREE.SphereGeometry(3.2, 28, 18).scale(1, 1.15, 1)),
      patternMaterial(c1, c2, 0.9, [1, 0], 0, 'fabric'),
    );
    env.position.y = 5.2;
    const skirt = new THREE.Mesh(b.view!.own(new THREE.CylinderGeometry(1.1, 0.7, 1.2, 18)), plainMaterial(c2, {}, 'fabric'));
    skirt.position.y = 1.9;
    const bk = new THREE.Mesh(b.view!.own(new THREE.CylinderGeometry(0.75, 0.6, 0.8, 12)), basket);
    bk.position.y = 0;
    g.add(env, skirt, bk);
    for (let i = 0; i < 4; i++) {
      const a = (i / 4) * Math.PI * 2 + Math.PI / 4;
      const r = new THREE.Mesh(b.view!.own(new THREE.CylinderGeometry(0.03, 0.03, 1.4, 4)), rope);
      r.position.set(Math.cos(a) * 0.65, 0.95, Math.sin(a) * 0.65);
      g.add(r);
    }
    g.position.copy(pos);
    g.userData.cat = 'balloons';
    b.group.add(g);
    const home = pos.clone();
    const ph = rnd() * 50;
    const spin = (rnd() - 0.5) * 0.08;
    b.anim((t) => {
      g.position.set(
        home.x + Math.sin(t * 0.03 + ph) * 4,
        home.y + Math.sin(t * 0.11 + ph) * 2.5,
        home.z + Math.cos(t * 0.025 + ph) * 4,
      );
      g.rotation.set(Math.sin(t * 0.4 + ph) * 0.03, t * spin, Math.cos(t * 0.33 + ph) * 0.03);
    });
  }
}
