import * as THREE from 'three';
import { loadAssets } from './assets.js';
import { Bean } from './bean.js';
import { PlayerBody } from './physics.js';
import { buildMap, MAP_META, setMapTime } from './maps.js';

const $ = (id) => document.getElementById(id);
const COLORS = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a', '#a66bff', '#ff8a3d', '#39e0d0', '#ffffff'];

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.setSize(innerWidth, innerHeight);
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFShadowMap;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 1.05;
document.body.prepend(renderer.domElement);

const scene = new THREE.Scene();
scene.fog = new THREE.Fog('#bfe3ff', 70, 260);
const camera = new THREE.PerspectiveCamera(65, innerWidth / innerHeight, 0.1, 700);

const sky = new THREE.Mesh(
  new THREE.SphereGeometry(600, 32, 16),
  new THREE.ShaderMaterial({
    side: THREE.BackSide, depthWrite: false, fog: false,
    uniforms: { top: { value: new THREE.Color('#2f7dff') }, mid: { value: new THREE.Color('#aee0ff') }, bot: { value: new THREE.Color('#ffc2ea') } },
    vertexShader: 'varying vec3 vP; void main(){ vP = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
    fragmentShader: 'uniform vec3 top; uniform vec3 mid; uniform vec3 bot; varying vec3 vP; void main(){ float h = vP.y; vec3 c = h > 0.0 ? mix(mid, top, pow(h, 0.7)) : mix(mid, bot, pow(-h, 0.5)); gl_FragColor = vec4(c, 1.0); \n#include <colorspace_fragment>\n}',
  }),
);
sky.renderOrder = -1;
scene.add(sky);

scene.add(new THREE.HemisphereLight('#ffffff', '#b89cff', 1.3));
const sun = new THREE.DirectionalLight('#fff3dd', 2.4);
sun.castShadow = true;
sun.shadow.mapSize.set(2048, 2048);
Object.assign(sun.shadow.camera, { left: -32, right: 32, top: 32, bottom: -32, near: 1, far: 140 });
sun.shadow.bias = -0.0005;
sun.shadow.normalBias = 0.03;
scene.add(sun, sun.target);

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});

let actx = null;
function sfx(type) {
  try {
    actx ||= new AudioContext();
    const t0 = actx.currentTime;
    const tone = (f1, f2, dur, wave = 'sine', vol = 0.12, delay = 0) => {
      const o = actx.createOscillator(), g = actx.createGain();
      o.type = wave;
      o.frequency.setValueAtTime(f1, t0 + delay);
      o.frequency.exponentialRampToValueAtTime(f2, t0 + delay + dur);
      g.gain.setValueAtTime(vol, t0 + delay);
      g.gain.exponentialRampToValueAtTime(0.001, t0 + delay + dur);
      o.connect(g).connect(actx.destination);
      o.start(t0 + delay); o.stop(t0 + delay + dur + 0.02);
    };
    if (type === 'jump') tone(330, 620, 0.14, 'triangle', 0.1);
    else if (type === 'dive') tone(500, 180, 0.2, 'triangle', 0.1);
    else if (type === 'hit') tone(180, 60, 0.25, 'sawtooth', 0.09);
    else if (type === 'boing') tone(200, 700, 0.22, 'sine', 0.14);
    else if (type === 'break') tone(140, 50, 0.3, 'square', 0.08);
    else if (type === 'count') tone(520, 520, 0.18, 'square', 0.07);
    else if (type === 'go') tone(880, 880, 0.45, 'square', 0.08);
    else if (type === 'qualify') [523, 659, 784, 1046].forEach((f, i) => tone(f, f, 0.18, 'triangle', 0.12, i * 0.09));
    else if (type === 'out') [440, 330, 220].forEach((f, i) => tone(f, f * 0.95, 0.22, 'sawtooth', 0.07, i * 0.14));
    else if (type === 'win') [523, 659, 784, 1046, 784, 1046].forEach((f, i) => tone(f, f, 0.25, 'triangle', 0.12, i * 0.12));
  } catch {}
}

let ws = null;
const clock = { offset: 0, samples: [] };
const serverNow = () => Date.now() + clock.offset;
const practice = new URLSearchParams(location.search).get('practice');
function send(m) {
  if (practice) {
    if (m.t === 'tile') G.map?.onTile?.(m.i, serverNow() + 450);
    if (m.t === 'finish' || m.t === 'out') setTimeout(startPractice, 2500);
    return;
  }
  if (ws && ws.readyState === 1) ws.send(JSON.stringify(m));
}

const G = {
  myId: null, host: null, phase: 'menu', players: new Map(), min: 2,
  map: null, round: null, active: false, eliminated: false,
  body: new PlayerBody(), myBean: null, remotes: new Map(),
  finished: new Set(), out: new Set(), cp: 0, specIdx: 0, winner: null, podium: null,
  lastSend: 0, grabbing: false, lastGrab: 0, lastBump: 0, lastCount: null,
};

const cam = { yaw: Math.PI, pitch: 0.35, dist: 8, target: new THREE.Vector3(0, 2, 0) };

const keys = new Set();
const input = { mx: 0, mz: 0, jump: false, dive: false };
let jumpQueued = false, diveQueued = false, grabMouse = false;
const pad = { jumpPrev: false, divePrev: false, grab: false, mx: 0, mz: 0 };

function typing() { return document.activeElement && document.activeElement.tagName === 'INPUT'; }
addEventListener('keydown', (e) => {
  if (typing()) { if (e.code === 'Enter' && G.phase === 'menu') $('playBtn').click(); return; }
  if (['Space', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.code)) e.preventDefault();
  if (e.repeat) return;
  keys.add(e.code);
  if (e.code === 'Space') { jumpQueued = true; if (!G.active) G.specIdx++; }
  if (e.code === 'KeyE') diveQueued = true;
  if (e.code === 'KeyH') $('help').classList.toggle('hidden');
  if (['Digit1', 'Digit2', 'Digit3'].includes(e.code) && G.active) {
    const em = Number(e.code.slice(5));
    G.myBean?.playEmote(em);
    send({ t: 'emote', e: em });
  }
});
addEventListener('keyup', (e) => keys.delete(e.code));
addEventListener('blur', () => keys.clear());

const canvas = renderer.domElement;
canvas.addEventListener('mousedown', (e) => {
  if (G.phase === 'menu') return;
  if (document.pointerLockElement !== canvas) { canvas.requestPointerLock?.(); return; }
  if (e.button === 0) { if (G.active) diveQueued = true; else G.specIdx++; }
  if (e.button === 2) grabMouse = true;
});
addEventListener('mouseup', (e) => { if (e.button === 2) grabMouse = false; });
canvas.addEventListener('contextmenu', (e) => e.preventDefault());
addEventListener('mousemove', (e) => {
  if (document.pointerLockElement !== canvas) return;
  cam.yaw -= e.movementX * 0.0025;
  cam.pitch = THREE.MathUtils.clamp(cam.pitch + e.movementY * 0.0025, -0.25, 1.25);
});
addEventListener('wheel', (e) => { cam.dist = THREE.MathUtils.clamp(cam.dist + Math.sign(e.deltaY) * 0.8, 4, 16); });

function pollGamepad(dt) {
  const gp = [...(navigator.getGamepads?.() || [])].find((g) => g);
  pad.mx = 0; pad.mz = 0; pad.grab = false;
  if (!gp) return;
  const dz = (v) => (Math.abs(v) < 0.18 ? 0 : v);
  pad.mx = dz(gp.axes[0] || 0); pad.mz = dz(gp.axes[1] || 0);
  cam.yaw -= dz(gp.axes[2] || 0) * 2.6 * dt;
  cam.pitch = THREE.MathUtils.clamp(cam.pitch + dz(gp.axes[3] || 0) * 1.8 * dt, -0.25, 1.25);
  const b = (i) => !!gp.buttons[i]?.pressed;
  const j = b(0), d = b(2) || b(1);
  if (j && !pad.jumpPrev) { jumpQueued = true; if (!G.active) G.specIdx++; }
  if (d && !pad.divePrev) diveQueued = true;
  pad.jumpPrev = j; pad.divePrev = d;
  pad.grab = b(5) || b(7);
}

function readInput() {
  let f = 0, s = 0;
  if (keys.has('KeyW') || keys.has('ArrowUp')) f += 1;
  if (keys.has('KeyS') || keys.has('ArrowDown')) f -= 1;
  if (keys.has('KeyD') || keys.has('ArrowRight')) s += 1;
  if (keys.has('KeyA') || keys.has('ArrowLeft')) s -= 1;
  f -= pad.mz; s += pad.mx;
  const l = Math.hypot(f, s);
  if (l > 1) { f /= l; s /= l; }
  const fx = -Math.sin(cam.yaw), fz = -Math.cos(cam.yaw);
  input.mx = fx * f + -fz * s;
  input.mz = fz * f + fx * s;
  input.jump = jumpQueued; input.dive = diveQueued;
  jumpQueued = false; diveQueued = false;
  G.grabbing = keys.has('KeyQ') || grabMouse || pad.grab;
}

function feed(text) {
  const d = document.createElement('div');
  d.textContent = text;
  $('feed').appendChild(d);
  setTimeout(() => d.remove(), 4500);
}
let bigTimer = null;
function bigText(text, sub = '', ms = 1800) {
  const b = $('big');
  b.textContent = text; $('sub').textContent = sub;
  b.classList.remove('pop'); void b.offsetWidth; b.classList.add('pop');
  clearTimeout(bigTimer);
  if (ms) bigTimer = setTimeout(() => { b.textContent = ''; $('sub').textContent = ''; }, ms);
}
const pname = (id) => G.players.get(id)?.name || `Боб ${id}`;
const pcolor = (id) => G.players.get(id)?.color || '#ffffff';

function loadMap(name, seed) {
  if (G.map) G.map.dispose();
  G.map = buildMap(name, scene, seed, { send, serverNow, sfx });
  G.map.update(0, 0);
}

function clearPodium() {
  if (G.podium) { G.podium.removeFromParent(); G.podium = null; }
  G.winner = null;
}

function spawnYaw(p) {
  return G.map.kind === 'race' ? 0 : Math.atan2(-p.x, -p.z);
}

function placeBody(p) {
  const yaw = spawnYaw(p);
  G.body.reset(p.clone(), yaw);
  cam.yaw = yaw + Math.PI;
  cam.target.copy(p).add(new THREE.Vector3(0, 1.3, 0));
}

function enterLobby() {
  clearPodium();
  G.round = null; G.eliminated = false;
  G.finished.clear(); G.out.clear();
  loadMap('lobby', 1);
  G.active = true;
  placeBody(G.map.respawn());
  for (const r of G.remotes.values()) r.snaps.length = 0;
  $('lobby').classList.remove('hidden');
  $('hud').classList.add('hidden');
  $('intro').classList.add('hidden');
  $('results').classList.add('hidden');
  bigText('', '', 1);
}

function onRound(m) {
  clearPodium();
  G.round = m;
  G.phase = 'round';
  G.finished = new Set(m.finished || []);
  G.out = new Set(m.out || []);
  loadMap(m.map, m.seed);
  for (const [i, at] of m.tiles || []) G.map.onTile?.(i, at);
  for (const i of m.doors || []) G.map.onDoor?.(i);
  const idx = m.participants.indexOf(G.myId);
  G.active = idx >= 0 && !G.finished.has(G.myId) && !G.out.has(G.myId);
  G.cp = 0;
  if (idx >= 0) placeBody(G.map.spawns[idx % G.map.spawns.length]);
  else cam.target.copy(G.map.spawns[0]);
  for (const r of G.remotes.values()) r.snaps.length = 0;
  G.lastCount = null;
  $('lobby').classList.add('hidden');
  $('results').classList.add('hidden');
  $('hud').classList.remove('hidden');
  const meta = MAP_META[m.map];
  const intro = $('intro');
  intro.querySelector('.kind').textContent = `Раунд ${m.index} из ${m.total} · ${meta.kind}`;
  intro.querySelector('h2').textContent = meta.title;
  intro.querySelector('.desc').textContent = meta.desc;
  let goal = meta.goal;
  if (m.kind === 'race') goal += m.eliminate > 0 ? ` · пройдут ${m.qualify} из ${m.participants.length}` : ' · разминка, выбывших нет';
  if (m.kind === 'survival') goal += m.eliminate > 0 ? ` · вылетит ${m.eliminate}` : ' · разминка, выбывших нет';
  intro.querySelector('.goal').textContent = goal;
  intro.classList.toggle('hidden', !!m.late);
  if (!G.active && G.eliminated) bigText('Вы наблюдаете', 'Клик или пробел — сменить игрока', 2500);
}

function onRoundEnd(m) {
  G.phase = 'results';
  G.active = false;
  const el = $('results');
  el.querySelector('h2').textContent = `${MAP_META[m.map].title}: итоги`;
  const list = el.querySelector('.list');
  list.innerHTML = '';
  for (const [ids, ok] of [[m.qualified, true], [m.eliminated, false]]) {
    for (const id of ids) {
      const row = document.createElement('div');
      row.className = 'row ' + (ok ? 'ok' : 'bad');
      row.innerHTML = `<span class="dot" style="width:18px;height:18px;border-radius:50%;background:${pcolor(id)}"></span><span></span><span>${ok ? 'ДАЛЬШЕ ✔' : 'ВЫБЫВАЕТ ✖'}</span>`;
      row.children[1].textContent = pname(id) + (id === G.myId ? ' (вы)' : '');
      list.appendChild(row);
    }
  }
  el.classList.remove('hidden');
  if (m.eliminated.includes(G.myId)) { G.eliminated = true; bigText('ВЫБЫЛИ!', 'Можно досмотреть шоу', 3000); sfx('out'); }
  else if (m.qualified.includes(G.myId)) { bigText('ПРОШЛИ ДАЛЬШЕ!', '', 2500); sfx('qualify'); }
}

function onWinner(m) {
  G.phase = 'winner';
  G.active = false;
  $('results').classList.add('hidden');
  $('intro').classList.add('hidden');
  loadMap('lobby', 1);
  clearPodium();
  const g = new THREE.Group();
  const ped = new THREE.Mesh(new THREE.CylinderGeometry(2.2, 2.6, 4, 48), new THREE.MeshStandardMaterial({ color: '#ffd23f', roughness: 0.4 }));
  ped.position.y = 2; ped.castShadow = ped.receiveShadow = true;
  g.add(ped);
  const bean = new Bean(pcolor(m.id), m.name, true);
  bean.root.position.set(0, 4, 0);
  bean.setCrown(true);
  g.add(bean.root);
  g.position.set(0, 0, 9);
  scene.add(g);
  G.podium = g;
  G.winner = { bean, id: m.id, t: 0 };
  const me = m.id === G.myId;
  bigText(me ? 'ПОБЕДА!' : `${m.name}`, me ? 'Корона ваша!' : 'забирает корону!', 0);
  sfx(me ? 'win' : 'qualify');
}

function getRemote(id) {
  let r = G.remotes.get(id);
  if (!r) {
    const bean = new Bean(pcolor(id), pname(id), true);
    scene.add(bean.root);
    r = { id, bean, snaps: [], pos: new THREE.Vector3(), yaw: 0, a: 0, speed: 0, lastSeen: 0, visible: false, color: pcolor(id), name: pname(id) };
    G.remotes.set(id, r);
  }
  return r;
}

function removeRemote(id) {
  const r = G.remotes.get(id);
  if (r) { r.bean.root.removeFromParent(); G.remotes.delete(id); }
}

function renderLobbyUI() {
  const list = $('plist');
  list.innerHTML = '';
  for (const p of G.players.values()) {
    const li = document.createElement('li');
    li.innerHTML = `<span class="dot" style="background:${p.color}"></span><span class="n"></span><span class="tag"></span>`;
    li.querySelector('.n').textContent = p.name + (p.crowns ? ` 👑${p.crowns}` : '');
    li.querySelector('.tag').textContent = [p.id === G.host ? 'хост' : '', p.id === G.myId ? 'вы' : ''].filter(Boolean).join(' · ');
    list.appendChild(li);
  }
  const sw = $('swatches');
  sw.innerHTML = '';
  const taken = new Set([...G.players.values()].filter((p) => p.id !== G.myId).map((p) => p.color));
  const mine = G.players.get(G.myId)?.color;
  for (const c of COLORS) {
    const d = document.createElement('div');
    d.style.background = c;
    if (taken.has(c)) d.className = 'taken';
    if (c === mine) d.className = 'mine';
    d.onclick = () => { if (!taken.has(c)) send({ t: 'color', c }); };
    sw.appendChild(d);
  }
  const isHost = G.myId === G.host;
  const n = G.players.size;
  const btn = $('startBtn');
  btn.classList.toggle('hidden', !isHost);
  btn.disabled = n < G.min;
  $('startHint').textContent = isHost
    ? (n < G.min ? `Нужно минимум ${G.min} игрока (сейчас ${n}/5)` : `Игроков: ${n}/5 — можно начинать!`)
    : `Ждём, пока хост начнёт шоу (${n}/5)`;
}

function handle(m) {
  switch (m.t) {
    case 'welcome':
      G.myId = m.id;
      break;
    case 'full':
      $('menuErr').textContent = `Сервер заполнен (максимум ${m.max} игроков)`;
      break;
    case 'pong': {
      const rtt = performance.now() - m.c;
      clock.samples.push({ rtt, off: m.s + rtt / 2 - Date.now() });
      if (clock.samples.length > 12) clock.samples.shift();
      clock.offset = clock.samples.reduce((a, b) => (b.rtt < a.rtt ? b : a)).off;
      break;
    }
    case 'lobby': {
      const prevPhase = G.phase;
      G.host = m.host; G.min = m.min;
      G.players = new Map(m.players.map((p) => [p.id, p]));
      for (const p of m.players) {
        if (p.id === G.myId) {
          if (!G.myBean) { G.myBean = new Bean(p.color, p.name, false); scene.add(G.myBean.root); }
          if (G.myBean.color !== p.color) G.myBean.setColor(p.color);
          G.myBean.setCrown(p.crowns > 0);
        } else {
          const r = getRemote(p.id);
          if (r.color !== p.color) { r.color = p.color; r.bean.setColor(p.color); }
          if (r.name !== p.name) { r.name = p.name; r.bean.setName(p.name, p.color); }
          r.bean.setCrown(p.crowns > 0);
        }
      }
      renderLobbyUI();
      if (m.phase === 'lobby' && prevPhase !== 'lobby') { G.phase = 'lobby'; enterLobby(); }
      break;
    }
    case 'round': onRound(m); break;
    case 'roundEnd': onRoundEnd(m); break;
    case 'winner': onWinner(m); break;
    case 'S':
      for (const [id, x, y, z, r, a] of m.l) {
        if (id === G.myId) continue;
        const rem = getRemote(id);
        rem.snaps.push({ t: m.s, x, y, z, r, a });
        if (rem.snaps.length > 30) rem.snaps.shift();
        rem.lastSeen = performance.now();
      }
      break;
    case 'fin':
      G.finished.add(m.id);
      feed(`🏁 ${pname(m.id)} — финиш #${m.place}`);
      if (m.id !== G.myId) sfx('count');
      break;
    case 'out':
      G.out.add(m.id);
      feed(`💥 ${pname(m.id)} — за бортом!`);
      break;
    case 'tile': G.map?.onTile?.(m.i, m.at); break;
    case 'door': G.map?.onDoor?.(m.i); break;
    case 'bump':
      if (G.active) { G.body.vel.set(m.v[0], m.v[1], m.v[2]); G.body.stun(0.9); sfx('hit'); }
      break;
    case 'grab':
      G.body.slowUntil = performance.now() + 260;
      break;
    case 'emote': if (m.id !== G.myId) G.remotes.get(m.id)?.bean.playEmote(m.e); break;
    case 'left':
      feed(`👋 ${pname(m.id)} — отключение`);
      removeRemote(m.id);
      break;
  }
}

function connect(name) {
  ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`);
  ws.onopen = () => {
    send({ t: 'name', name });
    const ping = () => send({ t: 'ping', c: performance.now() });
    ping();
    let n = 0;
    const iv = setInterval(() => { if (ws.readyState !== 1) return clearInterval(iv); ping(); if (++n > 8) { clearInterval(iv); setInterval(ping, 3000); } }, 400);
    $('menu').classList.add('hidden');
    $('help').classList.remove('hidden');
    G.phase = 'connecting';
  };
  ws.onmessage = (e) => handle(JSON.parse(e.data));
  ws.onclose = () => {
    if (G.phase === 'menu') return;
    G.phase = 'menu';
    $('lobby').classList.add('hidden');
    $('hud').classList.add('hidden');
    $('menu').classList.remove('hidden');
    $('menuErr').textContent = $('menuErr').textContent || 'Соединение с сервером потеряно';
    $('playBtn').textContent = 'ПЕРЕПОДКЛЮЧИТЬСЯ';
    $('playBtn').onclick = () => location.reload();
  };
}

function remotesForPhysics() {
  const list = [];
  for (const r of G.remotes.values()) if (r.visible) list.push({ id: r.id, x: r.pos.x, y: r.pos.y, z: r.pos.z, touching: false });
  return list;
}

function roundTime(sNow) {
  if (G.round && G.phase !== 'lobby') return (sNow - G.round.startAt) / 1000;
  return (sNow % 1e6) / 1000;
}

function simulate(dt, t, nowP) {
  const locked = G.round && G.phase === 'round' && t < 0;
  readInput();
  if (locked || G.phase === 'results') { input.mx = 0; input.mz = 0; input.jump = false; input.dive = false; }
  const others = remotesForPhysics();
  const wasDive = G.body.state === 'dive';
  const n = Math.max(1, Math.ceil(dt / (1 / 90)));
  const sub = dt / n;
  const stateBefore = G.body.state;
  for (let i = 0; i < n; i++) {
    G.body.step(sub, input, G.map.colliders, nowP, others, dt);
    if (i === 0) { input.jump = false; input.dive = false; }
  }
  const b = G.body;
  if (b.jumped) sfx('jump');
  if (!wasDive && b.state === 'dive') sfx('dive');
  if (b.hitSomething) sfx(stateBefore !== 'stun' && b.state === 'stun' ? 'hit' : 'boing');

  if (b.state === 'dive' && nowP - G.lastBump > 500) {
    for (const o of others) {
      if (!o.touching) continue;
      const l = Math.hypot(b.vel.x, b.vel.z) || 1;
      send({ t: 'bump', to: o.id, v: [b.vel.x / l * 10, 5.5, b.vel.z / l * 10] });
      b.vel.x *= 0.3; b.vel.z *= 0.3;
      G.lastBump = nowP;
      sfx('hit');
      break;
    }
  }
  if (G.grabbing && b.state === 'normal' && nowP - G.lastGrab > 150) {
    const fx = Math.sin(b.yaw), fz = Math.cos(b.yaw);
    for (const o of others) {
      const dx = o.x - b.pos.x, dz = o.z - b.pos.z, d = Math.hypot(dx, dz);
      if (d < 1.6 && Math.abs(o.y - b.pos.y) < 1.2 && (dx * fx + dz * fz) / (d || 1) > 0.3) {
        send({ t: 'grab', to: o.id });
        b.slowUntil = nowP + 200;
        G.lastGrab = nowP;
        break;
      }
    }
  }

  const map = G.map;
  if (map.kind === 'lobby') {
    if (b.pos.y < map.killY) placeBody(map.respawn());
  } else if (map.kind === 'race') {
    const cps = map.checkpoints;
    for (let i = G.cp + 1; i < cps.length; i++) if (b.grounded && b.pos.z > cps[i].z) G.cp = i;
    if (b.pos.y < map.killY) {
      const p = cps[G.cp].p.clone();
      p.x += (Math.random() - 0.5) * 3;
      b.reset(p, 0);
      cam.yaw = Math.PI;
      sfx('out');
    }
    if (t >= 0 && b.pos.z > map.finishZ && b.pos.y > map.finishY && !G.finished.has(G.myId)) {
      G.finished.add(G.myId);
      G.active = false;
      send({ t: 'finish' });
      bigText('ФИНИШ!', 'Вы прошли в следующий раунд', 2500);
      sfx('qualify');
    }
  } else if (b.pos.y < map.killY && !G.out.has(G.myId)) {
    G.out.add(G.myId);
    G.active = false;
    send({ t: 'out' });
    bigText('УПАЛИ!', map.kind === 'final' ? 'Корона достанется другому…' : 'Ждём итогов раунда', 2500);
    sfx('out');
  }
}

function animCode() {
  const b = G.body;
  if (b.state === 'stun') return 3;
  if (b.state === 'dive') return 2;
  if (b.state === 'slide') return 5;
  if (G.grabbing) return 4;
  return b.grounded ? 0 : 1;
}

const _v = new THREE.Vector3();
function updateRemotes(dt, sNow, time) {
  const renderT = sNow - 130;
  const nowP = performance.now();
  for (const r of G.remotes.values()) {
    const inPlay = !G.round || G.phase === 'lobby' || (G.round.participants.includes(r.id) && !G.finished.has(r.id) && !G.out.has(r.id));
    const vis = r.snaps.length > 0 && nowP - r.lastSeen < 600 && inPlay && G.phase !== 'winner' && G.phase !== 'results';
    r.visible = vis;
    r.bean.root.visible = vis;
    if (!vis) continue;
    const s = r.snaps;
    let a = s[s.length - 1], b2 = a, f = 0;
    for (let i = s.length - 2; i >= 0; i--) {
      if (s[i].t <= renderT) { a = s[i]; b2 = s[i + 1]; f = THREE.MathUtils.clamp((renderT - a.t) / Math.max(1, b2.t - a.t), 0, 1); break; }
    }
    _v.set(a.x + (b2.x - a.x) * f, a.y + (b2.y - a.y) * f, a.z + (b2.z - a.z) * f);
    const moved = Math.hypot(_v.x - r.pos.x, _v.z - r.pos.z);
    const sp = moved > 8 || dt <= 0 ? 0 : moved / dt;
    r.speed += (sp - r.speed) * Math.min(1, 10 * dt);
    r.pos.copy(_v);
    let dy = b2.r - a.r; dy = Math.atan2(Math.sin(dy), Math.cos(dy));
    r.yaw = a.r + dy * f;
    r.a = (f < 0.5 ? a : b2).a;
    r.bean.root.position.copy(r.pos);
    r.bean.root.rotation.y = r.yaw;
    r.bean.animate(dt, r.speed, r.a, time);
  }
}

function spectateTarget() {
  const vis = [...G.remotes.values()].filter((r) => r.visible);
  if (!vis.length) return null;
  return vis[((G.specIdx % vis.length) + vis.length) % vis.length];
}

const fmt = (s) => { s = Math.max(0, Math.ceil(s)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

function updateHud(t, sNow) {
  if (!G.round || G.phase === 'lobby' || G.phase === 'winner') { $('spec').classList.add('hidden'); return; }
  const m = G.round;
  $('intro').classList.toggle('hidden', !(G.phase === 'round' && t < -3 && !m.late));
  if (G.phase === 'round') {
    if (t < 0 && t >= -3) {
      const c = Math.ceil(-t);
      if (G.lastCount !== c) { G.lastCount = c; bigText(String(c), '', 900); sfx('count'); }
    } else if (t >= 0 && G.lastCount !== 0 && G.lastCount !== null) {
      G.lastCount = 0; bigText('ВПЕРЁД!', '', 900); sfx('go');
    }
  }
  const meta = MAP_META[m.map];
  $('roundInfo').querySelector('.name').textContent = meta.title;
  const parts = m.participants.filter((id) => G.players.has(id));
  let stat = '';
  if (m.kind === 'race') stat = m.eliminate > 0 ? `Финиш: ${[...G.finished].length} / ${m.qualify} мест` : `Финиш: ${[...G.finished].length} / ${parts.length} · разминка`;
  else if (m.kind === 'survival') stat = m.eliminate > 0 ? `Вылетело: ${G.out.size} / ${m.eliminate}` : `На арене: ${parts.length - G.out.size} · разминка`;
  else stat = `Осталось: ${parts.filter((id) => !G.out.has(id)).length}`;
  $('roundInfo').querySelector('.stat').textContent = stat;
  $('timer').textContent = m.kind === 'final' || t < 0 ? '' : fmt((m.endAt - sNow) / 1000);
  const spec = $('spec');
  if (!G.active && G.phase === 'round') {
    const tg = spectateTarget();
    spec.textContent = tg ? `Наблюдение: ${pname(tg.id)} · клик/пробел — следующий` : 'Ждём остальных…';
    spec.classList.remove('hidden');
  } else spec.classList.add('hidden');
}

const _focus = new THREE.Vector3();
function updateCamera(dt, time) {
  if (G.winner) {
    const p = G.podium.position;
    G.winner.t += dt;
    const a = G.winner.t * 0.4;
    camera.position.set(p.x + Math.sin(a) * 9, p.y + 7, p.z + Math.cos(a) * 9);
    camera.lookAt(p.x, p.y + 5, p.z);
    G.winner.bean.root.rotation.y = a;
    if (G.winner.bean.emoteT <= 0) G.winner.bean.playEmote(1 + (Math.floor(G.winner.t / 2.2) % 3));
    G.winner.bean.animate(dt, 0, 0, time);
    _focus.copy(p);
  } else {
    if (G.active) _focus.copy(G.body.pos).add(_v.set(0, 1.3, 0));
    else { const tg = spectateTarget(); if (tg) _focus.copy(tg.pos).add(_v.set(0, 1.3, 0)); else if (G.map?.view) _focus.copy(G.map.view); else _focus.copy(cam.target); }
    cam.target.lerp(_focus, 1 - Math.exp(-10 * dt));
    const cp = Math.cos(cam.pitch);
    camera.position.set(
      cam.target.x + Math.sin(cam.yaw) * cp * cam.dist,
      cam.target.y + Math.sin(cam.pitch) * cam.dist + 0.8,
      cam.target.z + Math.cos(cam.yaw) * cp * cam.dist);
    camera.lookAt(cam.target.x, cam.target.y + 0.6, cam.target.z);
  }
  sky.position.copy(camera.position);
  sun.position.copy(_focus).add(_v.set(25, 45, 18));
  sun.target.position.copy(_focus);
}

let last = performance.now();
function frame() {
  requestAnimationFrame(frame);
  const nowP = performance.now();
  const dt = Math.min(0.05, (nowP - last) / 1000);
  last = nowP;
  const sNow = serverNow();
  const t = roundTime(sNow);
  const time = nowP / 1000;
  setMapTime(Math.max(0, t) % 1000);
  pollGamepad(dt);
  if (G.map) {
    if (G.active) G.body.beforeWorldUpdate();
    G.map.update(t, dt);
    if (G.active) { G.body.afterWorldUpdate(); simulate(dt, t, nowP); }
    else readInput();
  }
  updateRemotes(dt, sNow, time);
  if (G.myBean) {
    G.myBean.root.visible = G.active;
    if (G.active) {
      G.myBean.root.position.copy(G.body.pos);
      G.myBean.root.rotation.y = G.body.yaw;
      G.myBean.animate(dt, Math.hypot(G.body.vel.x, G.body.vel.z), animCode(), time, G.body.landImpact);
      G.body.landImpact = 0;
    }
  }
  updateCamera(dt, time);
  updateHud(t, sNow);
  if (G.active && nowP - G.lastSend > 50) {
    G.lastSend = nowP;
    const p = G.body.pos;
    send({ t: 's', p: [p.x, p.y, p.z], r: G.body.yaw, a: animCode() });
  }
  renderer.render(scene, camera);
}

window.__fb = G;
function startPractice() {
  const map = MAP_META[practice] && practice !== 'lobby' ? practice : 'race1';
  const kind = map === 'hex' ? 'final' : map === 'jumpclub' ? 'survival' : 'race';
  G.myId = 0;
  G.players = new Map([[0, { id: 0, name: 'Вы', color: '#ff5fa2' }]]);
  if (!G.myBean) { G.myBean = new Bean('#ff5fa2', 'Вы', false); scene.add(G.myBean.root); }
  onRound({ map, kind, eliminate: 0, qualify: 1, participants: [0], startAt: serverNow() + 4000, endAt: serverNow() + 604000, seed: Math.floor(Math.random() * 1e9), index: 1, total: 1 });
}
async function main() {
  await loadAssets();
  $('loading').classList.add('hidden');
  $('menu').classList.remove('hidden');
  const inp = $('nameInput');
  inp.value = localStorage.getItem('fb-name') || '';
  inp.focus();
  $('playBtn').onclick = () => {
    const name = inp.value.trim() || `Боб${Math.floor(Math.random() * 900 + 100)}`;
    localStorage.setItem('fb-name', name);
    inp.blur();
    sfx('count');
    connect(name);
  };
  $('startBtn').onclick = () => send({ t: 'start' });
  loadMap('lobby', 1);
  cam.target.set(0, 1, 0);
  if (practice) { inp.blur(); $('menu').classList.add('hidden'); $('help').classList.remove('hidden'); startPractice(); }
  frame();
}

main().catch((e) => { $('loading').textContent = 'Ошибка загрузки: ' + e.message; console.error(e); });
