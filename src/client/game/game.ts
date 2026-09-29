import { effect } from '@preact/signals';
import * as THREE from 'three';
import { decodeSnapshot } from '../../shared/codec';
import { ANIM, DT } from '../../shared/consts';
import type { LobbyPlayer, ServerMsg } from '../../shared/protocol';
import { Connection } from '../net/connection';
import { session, settings, updateSettings } from '../settings';
import {
  arenaInfo,
  conn,
  hud,
  lobby,
  myId,
  panelOpen,
  paused,
  practiceGame,
  pushFeed,
  results,
  scoreboard,
  settingsOpen,
  winner,
} from '../state';
import { ClientArena } from './arena';
import { setVolume, sfx } from './audio';
import { Bean } from './bean';
import { CameraRig } from './camera';
import { Input } from './input';
import { type Quality, Renderer } from './renderer';

const QUALITY_ORDER: Quality[] = ['medium', 'high', 'ultra'];

/** Ties everything together: connection, the current arena, beans, camera, render loop and HUD. */
export class Game {
  readonly renderer: Renderer;
  readonly input: Input;
  readonly rig: CameraRig;
  readonly net: Connection;
  arena: ClientArena | null = null;
  private readonly beans = new Map<number, Bean>();
  private readonly lastSeen = new Map<number, number>();
  private readonly decor = new Map<number, { tail?: boolean }>();
  private players = new Map<number, LobbyPlayer>();
  private spectate = -1;
  private last = performance.now();
  private hudAt = 0;
  private lastCount = 99;
  private frameMs = 16;
  private slowFor = 0;
  private fpsFrames = 0;
  private fpsAt = 0;
  private fps = 0;
  private readonly tmp = new THREE.Vector3();

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new Renderer(canvas);
    this.input = new Input(canvas);
    this.rig = new CameraRig(this.renderer.camera);
    const practice = new URLSearchParams(location.search).get('practice');
    practiceGame.value = practice;
    this.net = new Connection({
      name: () => settings.value.name,
      token: () => (practice ? null : session.get('fb_token')),
      practice,
      onMessage: (m) => this.onMessage(m),
      onDatagram: (d) => {
        const s = decodeSnapshot(d);
        if (s) this.arena?.onSnapshot(s);
      },
      onDisconnect: () => {},
    });

    effect(() => {
      const s = settings.value;
      if (this.renderer.quality !== s.quality || !this.started) this.renderer.setQuality(s.quality);
      this.renderer.camera.fov = s.fov;
      this.renderer.camera.updateProjectionMatrix();
      this.input.sensitivity = s.sensitivity;
      this.input.invertY = s.invertY;
      setVolume(s.volume);
    });
    window.addEventListener('resize', () => this.renderer.resize());
    canvas.addEventListener('click', () => {
      if (!settingsOpen.value) this.capture();
    });
    let wasLocked = false;
    document.addEventListener('pointerlockchange', () => {
      // Esc releases the mouse: that is the pause. A browser that refuses the lock never pauses.
      if (this.input.locked) {
        panelOpen.value = false;
        paused.value = false;
      } else if (wasLocked) paused.value = true;
      wasLocked = this.input.locked;
    });
    this.input.onEscape = () => {
      if (settingsOpen.value) settingsOpen.value = false;
    };
    this.input.onScoreboard = (show) => {
      scoreboard.value = show;
    };
    this.input.onEmote = (e) => {
      if (!this.arena?.body) return;
      this.net.send({ t: 'emote', e });
    };
    canvas.addEventListener('mousedown', (e) => {
      // Spectating: click to watch someone else.
      if (this.input.locked && !this.arena?.body && e.button === 0) this.spectate = -1;
    });
  }

  private started = false;

  capture() {
    paused.value = false;
    this.input.lock();
  }

  start() {
    this.started = true;
    this.renderer.setQuality(settings.value.quality);
    this.net.start();
    const loop = () => {
      this.frame();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  // ------------------------------------------------------------------ messages

  private onMessage(m: ServerMsg) {
    switch (m.t) {
      case 'welcome':
        myId.value = m.id;
        if (!m.practice) session.set('fb_token', m.token);
        return;
      case 'lobby': {
        lobby.value = m;
        this.players = new Map(m.players.map((p) => [p.id, p]));
        const me = this.players.get(myId.value);
        if (me && !settings.value.name) updateSettings({ name: me.name });
        for (const [id, bean] of this.beans) {
          const p = this.players.get(id);
          if (!p) continue;
          if (bean.color !== p.color) bean.setColor(p.color);
          if (bean.name !== p.name && id !== myId.value) bean.setName(p.name);
          bean.setCrown(p.crowns > 0);
        }
        if (m.phase !== 'results') results.value = null;
        if (m.phase !== 'winner') winner.value = null;
        return;
      }
      case 'arena':
        this.enterArena(m);
        return;
      case 'fin':
        this.arena?.finished.add(m.id);
        if (m.id === myId.value) {
          sfx('qualify');
          this.arena?.stopPlaying();
          this.spectate = -1;
        } else pushFeed(`${this.nameOf(m.id)} финишировал(а) #${m.place}`);
        return;
      case 'out':
        this.arena?.out.add(m.id);
        if (m.id === myId.value) {
          sfx('out');
          this.arena?.stopPlaying();
          this.spectate = -1;
        } else pushFeed(`${this.nameOf(m.id)} выбыл(а)`);
        return;
      case 'ev':
        this.arena?.spec.onEvent?.(m.n, m.d);
        return;
      case 'scores':
        for (const [id, v] of m.s) this.arena?.scores.set(id, v);
        return;
      case 'emote':
        this.beans.get(m.id)?.playEmote(m.e);
        return;
      case 'left':
        this.removeBean(m.id);
        return;
      case 'roundEnd':
        results.value = m;
        if (!m.practice) {
          const mine = m.ranking.find((e) => e.id === myId.value);
          if (mine) sfx(mine.ok ? 'qualify' : 'out');
        }
        return;
      case 'winner':
        winner.value = m;
        sfx('win');
        return;
      default:
        return;
    }
  }

  private nameOf(id: number) {
    return this.players.get(id)?.name ?? `#${id}`;
  }

  private enterArena(info: Extract<ServerMsg, { t: 'arena' }>) {
    this.arena?.dispose();
    for (const id of [...this.beans.keys()]) this.removeBean(id);
    this.decor.clear();
    arenaInfo.value = info;
    const arena = new ClientArena(info, this.renderer.scene, {
      myId: myId.value,
      sfx: (s) => sfx(s),
      decorate: (id, d) => {
        this.decor.set(id, { ...this.decor.get(id), ...d });
        this.beans.get(id)?.setTail(!!d.tail);
      },
      send: (d) => this.net.datagram(d),
      serverNow: () => this.net.clock.serverNow(),
      rtt: () => this.net.clock.rtt,
    });
    this.arena = arena;
    const me = myId.value;
    const inPlay =
      info.kind === 'lobby' || (info.participants.includes(me) && !info.finished.includes(me) && !info.out.includes(me));
    if (inPlay) arena.play(me);
    this.spectate = -1;
    this.lastCount = 99;
    // Camera: down the course in races, towards the middle in arenas.
    const i = Math.max(0, info.participants.indexOf(me));
    const { pos, yaw } = arena.spawn(i);
    this.rig.face(arena.spec.finish ? 0 : yaw);
    this.rig.update(pos, 1, null);
    this.rig.snap();
    if (info.kind === 'round') {
      results.value = null;
      panelOpen.value = false;
    }
  }

  private removeBean(id: number) {
    this.beans.get(id)?.dispose();
    this.beans.delete(id);
  }

  private bean(id: number): Bean {
    let b = this.beans.get(id);
    if (!b) {
      const p = this.players.get(id);
      b = new Bean(p?.color ?? '#ffffff', p?.name ?? '', id !== myId.value);
      b.setCrown((p?.crowns ?? 0) > 0);
      b.setTail(!!this.decor.get(id)?.tail);
      this.renderer.scene.add(b.root);
      this.beans.set(id, b);
    }
    return b;
  }

  // ------------------------------------------------------------------ frame

  private frame() {
    const now = performance.now();
    const dt = Math.min(0.1, (now - this.last) / 1000);
    this.last = now;
    this.measure(now, dt);
    const arena = this.arena;
    if (arena) {
      const inLobby = arena.kind === 'lobby';
      this.input.enabled = !settingsOpen.value && !paused.value && !(inLobby && panelOpen.value);
      this.rig.look(this.input.lookX, -this.input.lookY);
      this.input.lookX = 0;
      this.input.lookY = 0;
      arena.predict(() => {
        const s = this.input.sample(DT);
        const [mx, mz] = this.rig.toWorld(s.moveX, s.moveY);
        return { mx, mz, jump: s.jump, dive: s.dive, grab: s.grab };
      });
      this.playEvents(arena);
      const t = arena.renderTick() * DT;
      arena.animate(t, dt);
      const focus = this.updateBeans(arena, dt, t);
      this.rig.update(focus, dt, arena.builder.world);
      if (arena.teleported) {
        // Respawned (or first placed by the server): look the way the bean faces.
        if (arena.body) this.rig.face(arena.body.yaw);
        this.rig.snap();
        arena.teleported = false;
      }
      this.renderer.focus(focus);
      this.countdown(arena, t);
      if (now - this.hudAt > 100) {
        this.hudAt = now;
        this.updateHud(arena, t);
      }
    }
    this.renderer.render();
  }

  private measure(now: number, dt: number) {
    this.fpsFrames++;
    if (now - this.fpsAt > 1000) {
      this.fps = Math.round((this.fpsFrames * 1000) / (now - this.fpsAt));
      this.fpsFrames = 0;
      this.fpsAt = now;
    }
    // Automatic fallback when the machine cannot keep up (never upwards: that is the player's call).
    this.frameMs += (dt * 1000 - this.frameMs) * 0.05;
    if (document.visibilityState === 'visible' && this.frameMs > 24) this.slowFor += dt;
    else this.slowFor = 0;
    if (this.slowFor > 6) {
      this.slowFor = 0;
      const i = QUALITY_ORDER.indexOf(settings.value.quality);
      if (i > 0) {
        updateSettings({ quality: QUALITY_ORDER[i - 1]! });
        pushFeed('Качество графики снижено для плавности');
      }
    }
  }

  private playEvents(arena: ClientArena) {
    const e = arena.takeEvents();
    if (e.jumped) sfx('jump');
    if (e.dived) sfx('dive');
    if (e.hit) sfx('hit');
    if (e.bounced) sfx('boing');
  }

  private updateBeans(arena: ClientArena, dt: number, t: number): THREE.Vector3 {
    const me = myId.value;
    const now = performance.now();
    let focus: THREE.Vector3 | null = null;
    if (arena.body) {
      const b = this.bean(me);
      const yaw = arena.ownPose(dt, this.tmp);
      b.root.position.copy(this.tmp);
      b.root.rotation.y = yaw;
      b.root.visible = true;
      const speed = Math.hypot(arena.body.vel.x, arena.body.vel.z);
      b.animate(dt, speed, arena.ownAnim(false), t, arena.events.landed);
      this.lastSeen.set(me, now);
      focus = b.root.position;
    }
    const poses = arena.remotePoses();
    const ids = [...poses.keys()];
    for (const [id, p] of poses) {
      if (id === me) continue;
      const b = this.bean(id);
      const prev = b.root.position.clone();
      b.root.position.copy(p.pos);
      b.root.rotation.y = p.yaw;
      b.root.visible = true;
      const speed = dt > 0 ? Math.hypot(p.pos.x - prev.x, p.pos.z - prev.z) / dt : 0;
      b.animate(dt, Math.min(speed, 12), p.anim === ANIM.grab ? ANIM.grab : p.anim, t);
      this.lastSeen.set(id, now);
    }
    for (const [id, b] of this.beans) {
      if (now - (this.lastSeen.get(id) ?? 0) > 400) b.root.visible = false;
    }
    if (!focus) {
      // Spectating: follow someone still in play.
      const others = ids.filter((id) => id !== me);
      if (!others.includes(this.spectate)) this.spectate = others[0] ?? -1;
      focus = this.beans.get(this.spectate)?.root.position ?? arena.spec.view ?? new THREE.Vector3();
    }
    return focus;
  }

  private countdown(arena: ClientArena, t: number) {
    if (arena.kind !== 'round') return;
    const n = Math.ceil(-t);
    if (n !== this.lastCount) {
      if (n >= 1 && n <= 3) sfx('count');
      if (n === 0 && this.lastCount === 1) sfx('go');
      this.lastCount = n;
    }
  }

  private updateHud(arena: ClientArena, t: number) {
    const me = myId.value;
    const info = arena.info;
    let status: ReturnType<typeof hud.peek>['status'] = 'lobby';
    if (info.kind === 'round') {
      if (arena.body) status = 'play';
      else if (arena.finished.has(me)) status = 'finished';
      else if (arena.out.has(me)) status = 'out';
      else status = 'spectating';
    }
    let mapText: string | null = null;
    try {
      mapText = info.kind === 'round' && t >= 0 ? (arena.spec.hud?.() ?? null) : null;
    } catch {}
    hud.value = {
      t,
      timeLeft: Math.max(0, (info.endAt - this.net.clock.serverNow()) / 1000),
      status,
      place: [...arena.finished].indexOf(me) + 1,
      spectating: status === 'play' || status === 'lobby' ? '' : this.nameOf(this.spectate),
      mapText,
      finished: arena.finished.size,
      out: arena.out.size,
      fps: this.fps,
      drawCalls: this.renderer.info.calls,
    };
    conn.value = conn.value.transport === this.net.kind ? conn.value : { ...conn.value, transport: this.net.kind };
  }
}
