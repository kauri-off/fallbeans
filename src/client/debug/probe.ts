import * as THREE from 'three';
import { DT, PROTOCOL_VERSION, TICK_MS } from '../../shared/consts';
import type { DevCmd, ServerMsg } from '../../shared/protocol';
import { type Collider, R, SPHERES } from '../../sim/physics';
import type { Expr } from '../game/face';
import type { Game } from '../game/game';
import { lod } from '../game/lod';
import type { Effects, Quality } from '../game/renderer';
import { settings, updateSettings } from '../settings';
import {
  arenaInfo,
  conn,
  debugOverlay,
  devMode,
  feed,
  gameEnd,
  hud,
  lobby,
  menuOpen,
  myId,
  results,
  shotMode,
  uiHidden,
} from '../state';
import { log } from './capture';
import { createProfiler } from './profiler';

/**
 * window.__fallbeans: read the game's state and drive it from tests, the browser console or an
 * automation tool. Everything returned is plain JSON (numbers rounded). See CLAUDE.md for recipes.
 */

type V3 = [number, number, number];
const r3 = (v: number) => Math.round(v * 1000) / 1000;
const r1 = (v: number) => Math.round(v * 10) / 10;
const vec = (v: THREE.Vector3): V3 => [r3(v.x), r3(v.y), r3(v.z)];

function percentiles(xs: readonly number[]) {
  if (!xs.length) return { n: 0, avg: 0, p50: 0, p95: 0, p99: 0, max: 0 };
  const s = [...xs].sort((a, b) => a - b);
  const at = (q: number) => s[Math.min(s.length - 1, Math.floor(q * s.length))]!;
  return {
    n: s.length,
    avg: r1(s.reduce((a, b) => a + b, 0) / s.length),
    p50: r1(at(0.5)),
    p95: r1(at(0.95)),
    p99: r1(at(0.99)),
    max: r1(s.at(-1)!),
  };
}

function shapeInfo(c: Collider) {
  const sh = c.shape;
  if (sh.type === 'box') return { type: 'box', half: [r3(sh.hx), r3(sh.hy), r3(sh.hz)] };
  if (sh.type === 'cyl') return { type: 'cyl', r: r3(sh.r), hh: r3(sh.hh) };
  return { type: 'sphere', r: r3(sh.r) };
}

export function createProbe(game: Game) {
  const frameMs: number[] = [];
  const cpuMs: number[] = [];
  let frameCount = 0;
  const frameWaiters: { at: number; resolve: () => void }[] = [];
  game.onFrame = (ms, cpu) => {
    frameMs.push(ms);
    cpuMs.push(cpu);
    if (frameMs.length > 600) frameMs.shift();
    if (cpuMs.length > 600) cpuMs.shift();
    frameCount++;
    for (let i = frameWaiters.length - 1; i >= 0; i--) {
      const w = frameWaiters[i]!;
      if (frameCount >= w.at) {
        frameWaiters.splice(i, 1);
        w.resolve();
      }
    }
  };
  const msgs: { at: number; m: ServerMsg }[] = [];
  game.onServerMessage = (m) => {
    if (m.t === 'pong') return;
    msgs.push({ at: Math.round(performance.now()), m });
    if (msgs.length > 300) msgs.shift();
  };
  const tmp = new THREE.Vector3();
  let steer: ReturnType<typeof setInterval> | null = null;
  const stopSteer = () => {
    if (steer) clearInterval(steer);
    steer = null;
  };

  const time = () => {
    const a = game.arena;
    const serverNow = game.net.clock.serverNow();
    const info = arenaInfo.value;
    return {
      serverNow: Math.round(serverNow),
      rate: game.net.clock.rate,
      phase: lobby.value?.phase ?? null,
      arena: info?.game ?? null,
      kind: info?.kind ?? null,
      index: info?.index ?? 0,
      total: info?.total ?? 0,
      /** Round time (s): negative during the intro. */
      t: a ? r3(a.tickAt(serverNow) * DT) : null,
      renderT: a ? r3(a.renderTick() * DT) : null,
      tick: a ? Math.floor(a.tickAt(serverNow)) : null,
      timeLeft: info ? r1(Math.max(0, (info.endAt - serverNow) / 1000)) : null,
      startsIn: info ? r1(Math.max(0, (info.startAt - serverNow) / 1000)) : null,
    };
  };

  const body = () => {
    const b = game.arena?.body;
    if (!b) return null;
    return {
      id: b.actor,
      pos: vec(b.pos),
      vel: vec(b.vel),
      speed: r3(Math.hypot(b.vel.x, b.vel.z)),
      yaw: r3(b.yaw),
      state: b.state,
      stateT: r3(b.stateT),
      grounded: b.grounded,
      ground: b.groundCol ? { index: b.groundCol.index, tag: b.groundCol.tag, slip: b.groundCol.slip } : null,
      down: b.down,
      tilt: r3(b.tilt),
      tiltDir: r3(b.tiltDir),
      slowed: b.slowUntil > (game.arena?.builder.world.t ?? 0) ? r3(b.slowK) : 1,
      holding: game.arena?.ownGrab ?? -1,
      hazard: b.hazard,
    };
  };

  const beans = () => {
    const a = game.arena;
    if (!a) return [];
    const me = a.body?.pos;
    const poses = a.remotePoses();
    const { beans, players } = game.inspect();
    return [...beans].map(([id, bean]) => {
      const p = poses.get(id);
      const pos = id === myId.value && me ? me : (p?.pos ?? bean.root.position);
      return {
        id,
        name: players.get(id)?.name ?? '',
        bot: players.get(id)?.bot ?? false,
        visible: bean.root.visible,
        pos: vec(pos),
        dist: me ? r1(pos.distanceTo(me)) : null,
        anim: p?.anim ?? null,
        grab: p?.grab ?? -1,
        status: hud.value.roster[id]?.status ?? null,
      };
    });
  };

  const colliders = (radius = 6, at?: V3) => {
    const a = game.arena;
    if (!a) return [];
    const c = at ? tmp.set(...at) : (a.body?.pos ?? a.spec.view ?? new THREE.Vector3());
    const out: Collider[] = [];
    a.builder.world.query(c.x, c.z, radius, out);
    const b = a.body;
    const hit = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
    const sphere = new THREE.Vector3();
    return out
      .map((col) => {
        let depth = 0;
        if (b)
          for (const h of SPHERES) {
            sphere.set(b.pos.x, b.pos.y + h, b.pos.z);
            if (col.contact(sphere, R, hit)) depth = Math.max(depth, hit.depth);
          }
        return {
          index: col.index,
          ...shapeInfo(col),
          center: vec(col.center),
          dist: r1(Math.hypot(col.center.x - c.x, col.center.z - c.z)),
          static: col.isStatic,
          enabled: col.enabled,
          tag: col.tag,
          hit: col.hit || undefined,
          bounce: col.bounce || undefined,
          pad: col.pad || undefined,
          slip: col.slip || undefined,
          conveyor: col.conveyor ? vec(col.conveyor) : undefined,
          /** Penetration into the local bean right now (0 = not touching). */
          touch: r3(depth),
        };
      })
      .sort((x, y) => x.dist - y.dist);
  };

  const net = () => {
    const st = game.arena?.netStats();
    return {
      transport: game.net.kind,
      status: conn.value.status,
      rtt: r1(game.net.clock.rtt),
      clockOffset: r1(game.net.clock.offset),
      rate: game.net.clock.rate,
      /** Prediction and interpolation of the current arena. */
      arena: st ? (Object.fromEntries(Object.entries(st).map(([k, v]) => [k, r1(v)])) as typeof st) : null,
    };
  };

  const render = () => {
    const rd = game.renderer;
    const i = rd.info;
    const size = rd.renderer.getSize(new THREE.Vector2());
    return {
      quality: rd.quality,
      size: [size.x, size.y],
      pixelRatio: rd.renderer.getPixelRatio(),
      calls: i.calls,
      triangles: i.triangles,
      points: i.points,
      lines: i.lines,
      ...rd.memory,
      fps: game.inspect().fps,
      frame: percentiles(frameMs),
      cpu: percentiles(cpuMs),
      gpu: percentiles(rd.gpuMs),
      sceneObjects: countObjects(rd.scene),
    };
  };

  const memory = () => {
    const m = (
      performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number; jsHeapSizeLimit: number } }
    ).memory;
    return m
      ? {
          heapMB: r1(m.usedJSHeapSize / 1048576),
          totalMB: r1(m.totalJSHeapSize / 1048576),
          limitMB: r1(m.jsHeapSizeLimit / 1048576),
        }
      : null;
  };

  const world = () => {
    const a = game.arena;
    if (!a) return null;
    const w = a.builder.world;
    return {
      game: a.info.game,
      seed: a.info.seed,
      colliders: w.colliders.length,
      dynamic: w.dynamic.length,
      movers: w.movers.length,
      anims: a.builder.anims.length,
      spawns: a.spec.spawns.length,
      killY: a.spec.killY,
      finish: a.spec.finish ?? null,
      checkpoints: a.spec.checkpoints?.map((c) => ({ z: c.z, p: vec(c.p) })) ?? [],
      view: a.spec.view ? vec(a.spec.view) : null,
    };
  };

  /** Scene checks: broken transforms, missing materials, oversized things, shadow cost. */
  const sceneAudit = () => {
    const problems: string[] = [];
    let meshes = 0;
    let casters = 0;
    let transparent = 0;
    let vertices = 0;
    const materials = new Set<THREE.Material>();
    const e = new THREE.Vector3();
    game.renderer.scene.traverse((o) => {
      o.getWorldPosition(e);
      if (!Number.isFinite(e.x + e.y + e.z)) problems.push(`NaN position: ${o.name || o.type}`);
      if (o instanceof THREE.Mesh) {
        meshes++;
        if (o.castShadow && o.visible) casters++;
        const mats = Array.isArray(o.material) ? o.material : [o.material];
        for (const m of mats) {
          if (!m) problems.push(`mesh without material: ${o.name || '?'}`);
          else {
            materials.add(m);
            if (m.transparent) transparent++;
          }
        }
        vertices += o.geometry.attributes.position?.count ?? 0;
        if (!o.geometry.attributes.normal && !(o.material instanceof THREE.MeshBasicMaterial))
          problems.push(`lit mesh without normals: ${o.name || '?'}`);
      }
    });
    return { meshes, shadowCasters: casters, transparent, materials: materials.size, vertices, problems: problems.slice(0, 50) };
  };

  const snapshot = () => ({
    time: time(),
    me: myId.value,
    menu: menuOpen.value,
    devMode: devMode.value,
    body: body(),
    beans: beans(),
    hud: { ...hud.value, roster: undefined },
    net: net(),
    render: render(),
    memory: memory(),
    errors: log.filter((l) => l.level === 'error').length,
    warnings: log.filter((l) => l.level === 'warn').length,
  });

  const api = {
    version: PROTOCOL_VERSION,
    build: __BUILD__,
    /** Short state (kept compatible with the e2e tests). */
    state: () => ({
      id: game.arena?.body?.actor ?? null,
      pos: game.arena?.body?.pos.toArray() ?? null,
      arena: game.arena?.info.game ?? null,
      kind: game.arena?.kind ?? null,
      transport: game.net.kind,
      corrections: game.arena?.corrections ?? 0,
      lead: Math.round(game.arena?.inputLead ?? 0),
      rtt: Math.round(game.net.clock.rtt),
      drawCalls: game.renderer.info.calls,
      input: game.input.enabled,
      menu: menuOpen.value,
    }),
    snapshot,
    time,
    body,
    beans,
    colliders,
    world,
    net,
    render,
    memory,
    hud: () => ({ hud: hud.value, results: results.value, gameEnd: gameEnd.value, feed: feed.value }),
    lobby: () => lobby.value,
    logs: (level?: 'error' | 'warn' | 'info') => log.filter((l) => !level || l.level === level),
    errors: () => log.filter((l) => l.level === 'error'),
    msgs: (n = 50, type?: ServerMsg['t']) => msgs.filter((x) => !type || x.m.t === type).slice(-n),
    audit: { scene: sceneAudit },
    /** What costs the most: scene breakdown, GPU per pass, CPU per section, on/off experiments. */
    profile: createProfiler(game),
    /** CPU time per frame section (predict, beans, render…). */
    sections: () => game.prof.report(),
    input: {
      /** Holds a direction for `ms`: camera-relative {f, r} (−1…1) or world {x, z}; optional grab. */
      hold(dir: { f?: number; r?: number; x?: number; z?: number; grab?: boolean }, ms = 1000) {
        stopSteer();
        const world: [number, number] | undefined =
          dir.x !== undefined || dir.z !== undefined ? [dir.x ?? 0, dir.z ?? 0] : undefined;
        game.input.script = {
          moveX: dir.r ?? 0,
          moveY: dir.f ?? 0,
          ...(world ? { world } : {}),
          grab: !!dir.grab,
          until: performance.now() + ms,
        };
        return new Promise<void>((resolve) => setTimeout(resolve, ms));
      },
      press(button: 'jump' | 'dive') {
        game.input.press(button);
      },
      stop() {
        stopSteer();
        game.input.script = null;
      },
      /** Walks (world space) to x, z; resolves with the distance left when there or after `timeoutMs`. */
      walkTo(x: number, z: number, timeoutMs = 15_000) {
        stopSteer();
        const until = performance.now() + timeoutMs;
        return new Promise<number>((resolve) => {
          steer = setInterval(() => {
            const b = game.arena?.body;
            const d = b ? Math.hypot(x - b.pos.x, z - b.pos.z) : 0;
            if (!b || d < 0.6 || performance.now() > until) {
              stopSteer();
              game.input.script = null;
              resolve(r1(d));
              return;
            }
            game.input.script = { moveX: 0, moveY: 0, world: [(x - b.pos.x) / d, (z - b.pos.z) / d], grab: false, until: until };
          }, 30);
        });
      },
    },
    camera: {
      /** Fixed camera at `eye` looking at `look` until camera.free(). */
      set(eye: V3, look: V3) {
        game.cameraOverride = { eye: new THREE.Vector3(...eye), look: new THREE.Vector3(...look) };
      },
      free() {
        game.cameraOverride = null;
      },
      get: () => ({ pos: vec(game.renderer.camera.position), fov: game.renderer.camera.fov }),
    },
    /** Puts a crown or a tail on a bean (default: the local one) until the next lobby update. */
    decorate(d: { crown?: boolean; tail?: boolean }, id = myId.value) {
      const b = game.inspect().beans.get(id);
      if (!b) return false;
      if (d.crown !== undefined) b.setCrown(d.crown);
      if (d.tail !== undefined) b.setTail(d.tail);
      return true;
    },
    /** Shows a facial expression on a bean for `s` seconds (see game/face.ts Expr). */
    face(e: Expr, s = 5, id = myId.value) {
      const b = game.inspect().beans.get(id);
      b?.react(e, s);
      return !!b;
    },
    /** Bonuses of the round: kind, position, when they show up, who took them. */
    bonuses: () =>
      game.arena?.bonuses?.list.map((x) => ({ kind: x.kind, pos: [x.x, x.y, x.z], appearAt: x.appearAt, takenBy: x.takenBy })) ??
      [],
    /** Plays an emote (1–5) as if the key had been pressed. */
    emote(e: number) {
      game.net.send({ t: 'emote', e });
    },
    ui: (hidden: boolean) => {
      uiHidden.value = hidden;
    },
    /** Screenshot mode: no interface, still sky and effects, no blinking (see e2e/visual.spec.ts). */
    shot: (on: boolean) => {
      uiHidden.value = on;
      shotMode.value = on;
    },
    overlay: (on: boolean) => {
      debugOverlay.value = on;
    },
    quality: (q: Quality) => updateSettings({ quality: q }),
    /** Post effects on/off (god rays, SMAA, temporal AA) for benchmarks; returns the current set. */
    fx: (patch: Partial<Effects> = {}) => {
      if (patch.godrays !== undefined) updateSettings({ godrays: patch.godrays });
      game.renderer.setEffects(patch);
      return { ...game.renderer.fx };
    },
    /** Levels of detail: stats, or switch off / force a level (null: automatic). */
    lod: (o: { enabled?: boolean; force?: number | null; bias?: number } = {}) => {
      if (o.enabled !== undefined) lod.enabled = o.enabled;
      if (o.force !== undefined) lod.force = o.force;
      if (o.bias !== undefined) lod.bias = o.bias;
      return { enabled: lod.enabled, force: lod.force, bias: lod.bias, ...lod.stats, levels: [...lod.stats.levels] };
    },
    settings: () => settings.value,
    /** GPU timing: 'frame', 'passes' (per render pass) or 'off'; returns false if unsupported. */
    gpu: (mode: 'off' | 'frame' | 'passes' | boolean = 'frame') => game.renderer.measureGpu(mode),
    gpuReport: () => game.renderer.gpuReport(),
    dev: (cmd: DevCmd) => game.dev(cmd),
    /** Resolves after `n` rendered frames. */
    frames: (n = 1) => new Promise<void>((resolve) => frameWaiters.push({ at: frameCount + n, resolve })),
    /** Polls `pred` every 50 ms; resolves with its first truthy value, or rejects after `timeoutMs`. */
    waitFor<T>(pred: () => T, timeoutMs = 10_000): Promise<T> {
      const until = performance.now() + timeoutMs;
      return new Promise((resolve, reject) => {
        const poll = () => {
          let v: T;
          try {
            v = pred();
          } catch (e) {
            reject(e);
            return;
          }
          if (v) resolve(v);
          else if (performance.now() > until) reject(new Error('waitFor: timed out'));
          else setTimeout(poll, 50);
        };
        poll();
      });
    },
    /** Game time in ms per real ms (dev slow motion). */
    tickMs: TICK_MS,
  };
  return api;
}

export type Probe = ReturnType<typeof createProbe>;

function countObjects(scene: THREE.Object3D) {
  let n = 0;
  scene.traverse(() => {
    n++;
  });
  return n;
}
