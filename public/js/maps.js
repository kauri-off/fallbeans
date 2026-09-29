import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { Collider } from './physics.js';
import { clone, meshParts, sharedGeometries } from './assets.js';

export const MAP_META = {
  lobby: { title: 'Лобби', kind: 'Разминка', desc: '', goal: '' },
  race1: { title: 'Дверной переполох', kind: 'Гонка', desc: 'Ломайте фальшивые двери, перепрыгивайте вертушки и не упадите с движущихся платформ!', goal: 'Добегите до финиша' },
  race2: { title: 'Молоты и качели', kind: 'Гонка', desc: 'Уворачивайтесь от маятников, удержитесь на качелях и пробегите против ленты!', goal: 'Добегите до финиша' },
  jumpclub: { title: 'Прыг-клуб', kind: 'Выживание', desc: 'Перепрыгивайте нижнюю балку и не попадите под верхнюю. Со временем они ускоряются!', goal: 'Не упадите' },
  hex: { title: 'Хекс-а-гон', kind: 'Финал', desc: 'Плитки исчезают под ногами. Три этажа. Кто продержится последним — забирает корону!', goal: 'Останьтесь последним' },
};

export function mulberry32(a) {
  return function () {
    a |= 0; a = (a + 0x6D2B79F5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const timeUniform = { value: 0 };
const matCache = new Map();

function patternMaterial(c1, c2, freq = 0.25, dir = [1, 1], speed = 0) {
  const key = [c1, c2, freq, dir, speed].join('|');
  if (matCache.has(key)) return matCache.get(key);
  const m = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.6 });
  m.onBeforeCompile = (sh) => {
    sh.uniforms.uC1 = { value: new THREE.Color(c1) };
    sh.uniforms.uC2 = { value: new THREE.Color(c2) };
    sh.uniforms.uF = { value: freq };
    sh.uniforms.uDir = { value: new THREE.Vector2(dir[0], dir[1]).normalize() };
    sh.uniforms.uSpeed = { value: speed };
    sh.uniforms.uTime = timeUniform;
    sh.vertexShader = sh.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vWP;')
      .replace('#include <project_vertex>', '#include <project_vertex>\nvWP = (modelMatrix * vec4(transformed, 1.0)).xyz;');
    sh.fragmentShader = sh.fragmentShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vWP; uniform vec3 uC1; uniform vec3 uC2; uniform float uF; uniform vec2 uDir; uniform float uSpeed; uniform float uTime;')
      .replace('#include <color_fragment>', '#include <color_fragment>\nfloat stp = smoothstep(0.46, 0.54, fract(dot(vWP.xz, uDir) * uF + uTime * uSpeed));\ndiffuseColor.rgb *= mix(uC1, uC2, stp);');
  };
  m.customProgramCacheKey = () => key;
  matCache.set(key, m);
  return m;
}

function plainMaterial(color, opts = {}) {
  const key = 'p|' + color + JSON.stringify(opts);
  if (!matCache.has(key)) matCache.set(key, new THREE.MeshStandardMaterial({ color: new THREE.Color(color), roughness: 0.55, ...opts }));
  return matCache.get(key);
}

const PAL = {
  blue: ['#7ccfff', '#9bdcff'], purple: ['#a98bff', '#bca4ff'], pink: ['#ff8cc8', '#ffa6d6'],
  yellow: ['#ffd84a', '#ffe47a'], green: ['#6fe08a', '#8ceaa2'], white: ['#f4f1ff', '#ffffff'], orange: ['#ff9f4a', '#ffb673'],
};

class Builder {
  constructor(scene, seed, ctx) {
    this.group = new THREE.Group();
    scene.add(this.group);
    this.colliders = [];
    this.updaters = [];
    this.rng = mulberry32(seed);
    this.ctx = ctx;
  }

  mat(pal, freq) { return Array.isArray(pal) ? patternMaterial(pal[0], pal[1], freq) : plainMaterial(pal); }

  box(x, y, z, sx, sy, sz, pal = PAL.blue, opts = {}) {
    const r = Math.min(0.25, sx / 4, sy / 4, sz / 4);
    const geo = new RoundedBoxGeometry(sx, sy, sz, 2, r);
    const mesh = new THREE.Mesh(geo, opts.material || this.mat(pal, opts.freq));
    mesh.position.set(x, y, z);
    if (opts.rot) mesh.rotation.set(...opts.rot);
    mesh.castShadow = opts.castShadow ?? true;
    mesh.receiveShadow = true;
    (opts.parent || this.group).add(mesh);
    if (opts.noCollide) return { mesh };
    const col = new Collider(mesh, { type: 'box', hx: sx / 2, hy: sy / 2, hz: sz / 2 }, { isStatic: !opts.dynamic, ...opts });
    this.colliders.push(col);
    return { mesh, col };
  }

  cyl(x, y, z, r, h, pal = PAL.purple, opts = {}) {
    const geo = new THREE.CylinderGeometry(r, r, h, opts.seg || 48);
    const mesh = new THREE.Mesh(geo, opts.material || this.mat(pal, opts.freq));
    mesh.position.set(x, y, z);
    mesh.castShadow = true; mesh.receiveShadow = true;
    (opts.parent || this.group).add(mesh);
    if (opts.noCollide) return { mesh };
    const col = new Collider(mesh, { type: 'cyl', r, hh: h / 2 }, { isStatic: !opts.dynamic, ...opts });
    this.colliders.push(col);
    return { mesh, col };
  }

  collider(obj, shape, opts = {}) {
    const c = new Collider(obj, shape, opts);
    this.colliders.push(c);
    return c;
  }

  hub(x, y, z, scale = 1) {
    const h = clone('hub');
    h.position.set(x, y, z);
    h.scale.setScalar(scale);
    this.group.add(h);
    const o = new THREE.Object3D();
    o.position.set(x, y + 1.6 * scale, z);
    this.group.add(o);
    this.collider(o, { type: 'cyl', r: 1.1 * scale, hh: 1.6 * scale }, { isStatic: true });
  }

  rotor(x, y, z, len, count, angleFn, hit = 1) {
    const rotor = new THREE.Object3D();
    rotor.position.set(x, y, z);
    this.group.add(rotor);
    for (let k = 0; k < count; k++) {
      const pivot = new THREE.Object3D();
      pivot.rotation.y = (k / count) * Math.PI * 2;
      rotor.add(pivot);
      const arm = clone('arm');
      arm.scale.set(len, 1, 1);
      pivot.add(arm);
      const co = new THREE.Object3D();
      co.position.x = len / 2 + 0.3;
      pivot.add(co);
      this.collider(co, { type: 'box', hx: len / 2 - 0.3, hy: 0.36, hz: 0.36 }, { hit });
    }
    this.updaters.push((t) => { rotor.rotation.y = angleFn(t); });
    return rotor;
  }

  bumper(x, y, z, s = 1, power = 13) {
    const b = clone('bumper');
    b.position.set(x, y, z);
    b.scale.setScalar(s);
    this.group.add(b);
    const o = new THREE.Object3D();
    o.position.set(x, y + 0.95 * s, z);
    this.group.add(o);
    this.collider(o, { type: 'cyl', r: 0.9 * s, hh: 0.9 * s }, { isStatic: true, bounce: power });
    return b;
  }

  finish(x, y, z) {
    const f = clone('finish');
    f.position.set(x, y, z);
    this.group.add(f);
    for (const sx of [-8.5, 8.5]) {
      const o = new THREE.Object3D();
      o.position.set(x + sx, y + 3, z);
      this.group.add(o);
      this.collider(o, { type: 'cyl', r: 0.6, hh: 3 }, { isStatic: true });
    }
  }

  clouds(cx, cz, spread, n = 26, yMin = -30, yMax = -4) {
    for (let i = 0; i < n; i++) {
      const c = clone('cloud');
      const a = this.rng() * Math.PI * 2, d = spread * (0.55 + this.rng() * 0.8);
      c.position.set(cx + Math.cos(a) * d, yMin + this.rng() * (yMax - yMin), cz + Math.sin(a) * d * 1.2);
      c.scale.setScalar(1.5 + this.rng() * 3);
      c.rotation.y = this.rng() * 6;
      c.traverse((o) => { if (o.isMesh) { o.castShadow = false; o.receiveShadow = false; } });
      this.group.add(c);
    }
  }

  startArea(z0 = 0) {
    this.box(0, -1, z0, 18, 2, 14, PAL.purple);
    this.box(-9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(9.4, 0.6, z0, 0.8, 1.2, 14, PAL.pink);
    this.box(0, 0.6, z0 - 7.4, 19.6, 1.2, 0.8, PAL.pink);
    const gate = this.box(0, 1.8, z0 + 7.1, 18, 3.6, 0.4, null, {
      material: new THREE.MeshStandardMaterial({ color: '#ff5fa2', transparent: true, opacity: 0.35 }), dynamic: false, castShadow: false,
    });
    this.updaters.push((t) => { gate.col.enabled = t < 0; gate.mesh.visible = t < 0; });
    return [-6, -3, 0, 3, 6].map((x) => new THREE.Vector3(x, 0.05, z0 - 2));
  }

  dispose() {
    this.group.traverse((o) => {
      if (o.isMesh && !sharedGeometries.has(o.geometry)) o.geometry.dispose();
    });
    this.group.removeFromParent();
  }
}

export function setMapTime(t) { timeUniform.value = t; }

function buildLobby(b) {
  b.cyl(0, -1, 0, 15, 2, PAL.blue, { freq: 0.3 });
  b.cyl(0, 0.05, 0, 4, 0.2, PAL.yellow, { noCollide: true });
  b.hub(0, 0, 0, 1);
  b.rotor(0, 0.6, 0, 9, 1, (t) => t * 0.6, 0.5);
  b.bumper(10, 0, 4); b.bumper(-9, 0, -6); b.bumper(4, 0, -11);
  for (let i = 0; i < 5; i++) {
    const a = -0.5 + i * 0.3, h = 0.7 * (i + 1);
    b.box(Math.cos(a) * 12, h / 2, Math.sin(a) * 12, 2.6, h, 2.6, PAL.pink, { rot: [0, -a, 0] });
  }
  b.clouds(0, 0, 40);
  const spawns = [];
  for (let i = 0; i < 8; i++) { const a = i / 8 * Math.PI * 2; spawns.push(new THREE.Vector3(Math.cos(a) * 7, 0.05, Math.sin(a) * 7)); }
  return { kind: 'lobby', spawns, killY: -15, respawn: () => spawns[Math.floor(Math.random() * spawns.length)] };
}

function buildRace1(b) {
  const spawns = b.startArea(0);
  b.box(0, -1, 26, 18, 2, 38, PAL.blue);
  b.box(-9.4, 0.6, 26, 0.8, 1.2, 38, PAL.pink);
  b.box(9.4, 0.6, 26, 0.8, 1.2, 38, PAL.pink);

  const doors = [];
  const rows = [[14, 3], [24, 2], [34, 2]];
  for (const [z, nBreak] of rows) {
    const idx = [0, 1, 2, 3, 4].sort(() => b.rng() - 0.5).slice(0, nBreak);
    for (let i = 0; i < 5; i++) {
      const x = -6.8 + i * 3.4;
      const obj = clone('door');
      obj.position.set(x, 0, z);
      obj.scale.x = 3.4 / 3.1;
      obj.rotation.y = Math.PI;
      b.group.add(obj);
      const co = new THREE.Object3D();
      co.position.set(x, 1.6, z);
      b.group.add(co);
      const d = { id: doors.length, obj, breakable: idx.includes(i), broken: false, t: 0 };
      d.col = b.collider(co, { type: 'box', hx: 1.7, hy: 1.6, hz: 0.3 }, { isStatic: true });
      if (d.breakable) d.col.onTouch = () => breakDoor(d.id, true);
      doors.push(d);
    }
    b.box(0, 3.6, z, 18.4, 0.8, 1.0, PAL.yellow);
    b.box(-9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
    b.box(9.4, 1.6, z, 0.8, 3.2, 1.0, PAL.yellow);
  }
  function breakDoor(id, local) {
    const d = doors[id];
    if (!d || d.broken) return;
    d.broken = true; d.col.enabled = false; d.t = 0;
    if (local) b.ctx.send({ t: 'door', i: id });
    b.ctx.sfx?.('break');
  }
  b.updaters.push((t, dt) => {
    for (const d of doors) {
      if (!d.broken || !d.obj.visible) continue;
      d.t += dt;
      d.obj.rotation.x = Math.min(Math.PI / 2, d.t * d.t * 7);
      if (d.t > 1.4) d.obj.visible = false;
    }
  });

  b.box(0, -1, 46.5, 3.6, 2, 5, PAL.yellow);
  const plats = [[54, 2, 1.5, 0], [70, 3, -1.7, 0], [86, 2, 1.9, 1]];
  plats.forEach(([z, n, sp, high], i) => {
    b.cyl(0, -1, z, 6, 2, i % 2 ? PAL.pink : PAL.purple);
    b.hub(0, 0, z, 1);
    b.rotor(0, 0.6, z, 5.7, n, (t) => t * sp + i, 0.75);
    if (high) b.rotor(0, 2.45, z, 5.7, 1, (t) => -t * 1.1 + 1.5, 0.75);
    if (i < 2) b.box(0, -1, z + 8, 3.6, 2, 6, PAL.yellow);
  });
  b.box(0, -1, 93.5, 3.6, 2, 5, PAL.yellow);

  b.box(0, -1, 97, 10, 2, 6, PAL.purple);
  const movers = [[104, 0.9, 0], [110.5, 1.1, 2], [117, 0, 0], [123.5, 1.3, 4], [130, 1.0, 1]];
  movers.forEach(([z, sp, ph], i) => {
    const m = b.box(0, -0.5, z, 4.5, 1, 4.5, i % 2 ? PAL.orange : PAL.green, { dynamic: true });
    if (sp === 0) b.updaters.push((t) => { m.mesh.rotation.y = t * 0.9; });
    else b.updaters.push((t) => { m.mesh.position.x = Math.sin(t * sp + ph) * 4; });
  });
  b.box(0, -1, 139, 12, 2, 10, PAL.purple);

  const ang = Math.atan2(4, 20);
  b.box(0, 2 - 0.5 / Math.cos(ang), 154, 12, 1, Math.hypot(20, 4), PAL.blue, { rot: [-ang, 0, 0] });
  b.box(-6.3, 2.6, 154, 0.8, 1.2, 20.4, PAL.pink, { rot: [-ang, 0, 0] });
  b.box(6.3, 2.6, 154, 0.8, 1.2, 20.4, PAL.pink, { rot: [-ang, 0, 0] });
  for (const [x, z] of [[-3, 148], [3, 152], [-1, 157], [4, 160], [-4, 161]]) b.bumper(x, (z - 144) / 20 * 4 - 0.1, z, 0.9, 11);
  b.box(0, 3, 172, 18, 2, 16, PAL.yellow);
  b.finish(0, 4, 170);
  b.clouds(0, 90, 70, 40);

  const cps = [
    { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
    { z: 45, p: new THREE.Vector3(0, 0.1, 46) },
    { z: 94, p: new THREE.Vector3(0, 0.1, 97) },
    { z: 134, p: new THREE.Vector3(0, 0.1, 138) },
  ];
  return {
    kind: 'race', spawns, killY: -14, finishZ: 170, finishY: 3, checkpoints: cps, length: 170,
    onDoor: (id) => breakDoor(id, false),
  };
}

function buildRace2(b) {
  const spawns = b.startArea(0);
  b.box(0, -1, 30, 7, 2, 46, PAL.blue);
  const hammers = [[14, 1.9, 0], [22, 2.2, 1.6], [30, 1.7, 3.1], [38, 2.4, 0.8], [46, 2.0, 2.4]];
  for (const [z, w, ph] of hammers) {
    for (const sx of [-4.4, 4.4]) b.box(sx, 3.8, z, 0.8, 8.4, 0.8, PAL.purple);
    b.box(0, 8.2, z, 9.6, 0.8, 1.2, PAL.purple);
    const h = clone('hammer');
    h.position.set(0, 7.4, z);
    b.group.add(h);
    const co = new THREE.Object3D();
    co.position.set(0, -6, 0);
    h.add(co);
    b.collider(co, { type: 'box', hx: 1.35, hy: 0.95, hz: 0.95 }, { hit: 0.9 });
    b.updaters.push((t) => { h.rotation.z = Math.sin(t * w + ph) * 1.05; });
  }

  b.box(0, -1, 57, 10, 2, 8, PAL.purple);
  [67, 78, 89].forEach((z, i) => {
    const s = b.box(0, -0.5, z, 9, 1, 9, i % 2 ? PAL.pink : PAL.green, { dynamic: true });
    b.updaters.push((t) => {
      s.mesh.rotation.z = Math.sin(t * 1.1 + i * 2) * 0.32;
      s.mesh.rotation.x = Math.sin(t * 0.7 + i) * 0.12;
    });
  });
  b.box(0, -1, 98, 10, 2, 8, PAL.purple);

  const conv = patternMaterial('#8a8f9e', '#c7ccd8', 0.9, [0, 1], 3.5 * 0.9);
  b.box(0, -1, 118, 8, 2, 32, null, { material: conv, conveyor: new THREE.Vector3(0, 0, -3.5) });
  b.box(-4.4, 0.6, 118, 0.8, 1.2, 32, PAL.yellow);
  b.box(4.4, 0.6, 118, 0.8, 1.2, 32, PAL.yellow);
  for (const [x, z] of [[-2, 106], [2.2, 111], [-1, 121], [2.5, 127], [-2.5, 131]]) b.bumper(x, 0, z, 0.8, 10);
  [[109, 1.8, 0], [116, 2.1, 2], [124, 1.6, 4]].forEach(([z, w, ph]) => {
    const p = b.box(0, 0.8, z, 3, 1.6, 1, PAL.orange, { dynamic: true, hit: 0.8 });
    b.updaters.push((t) => { p.mesh.position.x = Math.sin(t * w + ph) * 2.4; });
  });

  b.box(0, -1, 142, 18, 2, 16, PAL.yellow);
  b.finish(0, 0, 142);
  b.clouds(0, 70, 60, 36);
  const cps = [
    { z: -100, p: new THREE.Vector3(0, 0.1, 2) },
    { z: 54, p: new THREE.Vector3(0, 0.1, 57) },
    { z: 95, p: new THREE.Vector3(0, 0.1, 98) },
  ];
  return { kind: 'race', spawns, killY: -14, finishZ: 142, finishY: -1, checkpoints: cps, length: 142 };
}

function buildJumpClub(b) {
  b.cyl(0, -1, 0, 13, 2, PAL.blue, { freq: 0.35 });
  b.cyl(0, 0.03, 0, 13.05, 0.1, PAL.yellow, { noCollide: true });
  b.cyl(0, 0.06, 0, 11.5, 0.1, PAL.blue, { noCollide: true, freq: 0.35 });
  b.hub(0, 0, 0, 1.2);
  const lowAng = (t) => (t <= 0 ? 0 : 1.15 * t + 0.009 * t * t);
  const highAng = (t) => Math.PI / 2 - (t <= 0 ? 0 : 0.7 * t + 0.006 * t * t);
  b.rotor(0, 0.6, 0, 12.6, 2, lowAng, 0.6);
  b.rotor(0, 2.45, 0, 12.6, 2, highAng, 0.6);
  b.clouds(0, 0, 40);
  const spawns = [25, 65, 115, 155, 205, 245, 295, 335].map((d) => {
    const a = d * Math.PI / 180;
    return new THREE.Vector3(Math.cos(a) * 6, 0.1, Math.sin(a) * 6);
  });
  return { kind: 'survival', spawns, killY: -6, view: new THREE.Vector3(0, 3, 0) };
}

function buildHex(b) {
  const parts = meshParts('hex');
  const layers = [
    { y: 0, color: new THREE.Color('#ff7ac2') },
    { y: -9, color: new THREE.Color('#7ccfff') },
    { y: -18, color: new THREE.Color('#8ceaa2') },
  ];
  const N = 6, S = 1.03;
  const cells = [];
  for (let q = -N; q <= N; q++)
    for (let r = Math.max(-N, -q - N); r <= Math.min(N, -q + N); r++) cells.push([S * 1.5 * q, S * Math.sqrt(3) * (r + q / 2)]);
  const total = cells.length * layers.length;
  const meshes = parts.map((p) => {
    const m = p.material.clone();
    if (m.name === 'Top') m.color.set('#ffffff');
    const im = new THREE.InstancedMesh(p.geometry, m, total);
    im.castShadow = true; im.receiveShadow = true;
    im.userData.top = m.name === 'Top';
    b.group.add(im);
    return im;
  });
  const tiles = [];
  const mtx = new THREE.Matrix4();
  const warn = new THREE.Color('#fff4a8');
  layers.forEach((L) => {
    for (const [x, z] of cells) {
      const i = tiles.length;
      const o = new THREE.Object3D();
      o.position.set(x, L.y - 0.25, z);
      b.group.add(o);
      const tile = { i, x, y: L.y, z, color: L.color.clone(), base: L.color, touched: false, fallAt: null, gone: false };
      tile.col = b.collider(o, { type: 'cyl', r: 0.97, hh: 0.25 }, { isStatic: true, onGround: () => touch(i) });
      mtx.makeTranslation(x, L.y, z);
      for (const im of meshes) { im.setMatrixAt(i, mtx); if (im.userData.top) im.setColorAt(i, tile.color); }
      tiles.push(tile);
    }
  });
  const active = new Set();
  let curT = -1;
  function touch(i) {
    const t = tiles[i];
    if (t.touched || curT < 0) return;
    t.touched = true; active.add(t);
    b.ctx.send({ t: 'tile', i });
  }
  function setFall(i, at) {
    const t = tiles[i];
    if (!t) return;
    t.touched = true; t.fallAt = at; active.add(t);
  }
  b.updaters.push((time) => {
    curT = time;
    if (!active.size) return;
    const now = b.ctx.serverNow();
    const top = meshes.find((m) => m.userData.top);
    for (const t of active) {
      let y = t.y, sx = 0, sz = 0;
      const left = t.fallAt ? t.fallAt - now : 450;
      if (left > 0) {
        const k = 1 - left / 450;
        t.color.copy(t.base).lerp(warn, 0.5 + 0.5 * k);
        sx = Math.sin(now * 0.08 + t.i) * 0.05 * k; sz = Math.cos(now * 0.07 + t.i) * 0.05 * k;
      } else {
        t.col.enabled = false;
        const f = -left / 1000;
        y -= 0.5 * 25 * f * f;
        t.color.copy(warn);
        if (f > 1.6) { t.gone = true; active.delete(t); mtx.makeScale(0, 0, 0); for (const im of meshes) { im.setMatrixAt(t.i, mtx); im.instanceMatrix.needsUpdate = true; } continue; }
      }
      mtx.makeTranslation(t.x + sx, y, t.z + sz);
      for (const im of meshes) { im.setMatrixAt(t.i, mtx); im.instanceMatrix.needsUpdate = true; }
      top.setColorAt(t.i, t.color);
      top.instanceColor.needsUpdate = true;
    }
  });
  b.clouds(0, 0, 38, 30, -45, -10);
  const spawns = [0, 1, 2, 3, 4, 5, 6, 7].map((k) => {
    const a = k / 8 * Math.PI * 2 + 0.3;
    return new THREE.Vector3(Math.cos(a) * 4.5, 0.1, Math.sin(a) * 4.5);
  });
  return { kind: 'final', spawns, killY: -26, onTile: setFall, view: new THREE.Vector3(0, 4, 0) };
}

const BUILDERS = { lobby: buildLobby, race1: buildRace1, race2: buildRace2, jumpclub: buildJumpClub, hex: buildHex };

export function buildMap(name, scene, seed, ctx) {
  const b = new Builder(scene, seed, ctx);
  const m = BUILDERS[name](b);
  m.name = name;
  m.meta = MAP_META[name];
  m.colliders = b.colliders;
  m.update = (t, dt) => { for (const u of b.updaters) u(t, dt); for (const c of b.colliders) c.sync(); };
  m.dispose = () => b.dispose();
  return m;
}
