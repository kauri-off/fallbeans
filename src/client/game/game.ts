import { effect } from '@preact/signals';
import * as THREE from 'three';
import { decodeSnapshot } from '../../shared/codec';
import { ANIM, BASE_PATH, DT, INTRO_MS } from '../../shared/consts';
import { Sections } from '../../shared/prof';
import type { DevCmd, LobbyPlayer, ServerMsg } from '../../shared/protocol';
import { BONUS_KINDS } from '../../sim/bonus';
import { GIANT_SIZE, POWER, pushOut } from '../../sim/physics';
import { report } from '../debug/capture';
import { Connection } from '../net/connection';
import { identity, settings, updateSettings } from '../settings';
import {
  arenaInfo,
  chatOpen,
  clearChat,
  conn,
  denied,
  devMode,
  feed,
  gameEnd,
  type Hud,
  hud,
  lobby,
  menuOpen,
  myId,
  needClick,
  note,
  ownRoom,
  type PlayStatus,
  practiceGame,
  pushChat,
  pushFeed,
  results,
  room,
  roomList,
  shotMode,
} from '../state';
import { fmtSec, ordinal } from '../ui/labels';
import { ClientArena } from './arena';
import { setVolume, sfx } from './audio';
import { Bean, tickRainbow } from './bean';
import { CameraRig } from './camera';
import { Input } from './input';
import { type Quality, Renderer } from './renderer';

const QUALITY_ORDER: Quality[] = ['medium', 'high'];
/** The intro fly-over ends this long before the start; then the camera sits behind the bean. */
const INTRO_HANDOVER = 1.4;

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
  /** Spectated player id; -1 = overview camera. */
  private spectate = -1;
  private spectateAuto = true;
  private spectateIds: number[] = [];
  private lastSpectate = -2;
  private last = performance.now();
  private hudAt = 0;
  private lastCount = 99;
  private frameMs = 16;
  private slowFor = 0;
  private fpsFrames = 0;
  private fpsAt = 0;
  private fps = 0;
  private started = false;
  private readonly tmp = new THREE.Vector3();
  private lastGrab = -1;
  /** Smoothed velocities of the rendered beans (animation). */
  private readonly vels = new Map<number, THREE.Vector3>();
  private readonly eye = new THREE.Vector3();
  private readonly look = new THREE.Vector3();
  /** Arena whose scripted shot is running (the first frame of a shot cuts instead of easing). */
  private cineArena = -1;
  /** Debug: a fixed camera (eye, look) instead of the follow/intro camera. */
  cameraOverride: { eye: THREE.Vector3; look: THREE.Vector3 } | null = null;
  /** Debug: called after every frame with its real duration and the CPU time spent in it (ms). */
  onFrame: ((frameMs: number, cpuMs: number) => void) | null = null;
  /** Lower the quality by itself on slow frames (off while the profiler runs). */
  autoQuality = true;
  /** Debug: sees every control message from the server. */
  onServerMessage: ((m: ServerMsg) => void) | null = null;
  /** CPU time of the frame by section (predict, events, animate, beans, camera, hud, render). */
  readonly prof = new Sections(2000);
  private devSeq = 1;
  private readonly devWait = new Map<number, (r: { ok: boolean; msg: string }) => void>();
  /** The room to be in: from the link (`?room=`), then whichever the player entered; null at the room list. */
  private wantRoom: string | null;

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new Renderer(canvas);
    this.input = new Input(canvas);
    this.rig = new CameraRig(this.renderer.camera);
    const query = new URLSearchParams(location.search);
    const practice = query.get('practice');
    practiceGame.value = practice;
    const linked = query.get('room')?.toLowerCase() ?? '';
    this.wantRoom = /^[a-z0-9]{2,8}$/.test(linked) ? linked : null;
    this.net = new Connection({
      name: () => settings.value.name,
      token: () => identity.get(),
      room: () => this.wantRoom,
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
      this.renderer.setEffects(s.gfx);
      this.renderer.setUpscale(s.upscale);
      if (this.renderer.quality !== s.quality || !this.started) this.renderer.setQuality(s.quality);
      this.renderer.camera.fov = s.fov;
      this.renderer.camera.updateProjectionMatrix();
      this.input.sensitivity = s.sensitivity;
      this.input.invertY = s.invertY;
      setVolume(s.volume);
    });
    window.addEventListener('resize', () => this.renderer.resize());
    canvas.addEventListener('webglcontextlost', () => report('webgl', 'WebGL context lost'));
    // Any click on the field captures the mouse again (after Alt+Tab, a refused lock, closing the menu…).
    canvas.addEventListener('click', () => {
      if (!this.input.locked) this.resume();
    });
    let blurredAt = -1e9;
    window.addEventListener('blur', () => {
      blurredAt = performance.now();
    });
    window.addEventListener('focus', () => {
      // Back from another window: try to take the mouse again; if the browser wants a click, ask for one.
      if (!menuOpen.value && this.arena) void this.input.lock();
    });
    let wasLocked = false;
    document.addEventListener('pointerlockchange', () => {
      const locked = this.input.locked;
      if (locked) {
        menuOpen.value = false;
        needClick.value = false;
      } else if (wasLocked) {
        // Released by Esc: that opens the menu (or just closes the chat line). Released because the
        // window lost focus (Alt+Tab): no menu, just a prompt to click back in.
        const focusLoss = !document.hasFocus() || document.hidden || performance.now() - blurredAt < 400;
        if (focusLoss || chatOpen.value) needClick.value = true;
        else menuOpen.value = true;
        chatOpen.value = false;
      }
      wasLocked = locked;
    });
    document.addEventListener('pointerlockerror', () => {
      if (!menuOpen.value) needClick.value = true;
    });
    this.input.onEscape = () => {
      // With the mouse captured the browser handles Esc (and the lock change opens the menu).
      if (this.input.locked || !this.arena) return;
      if (needClick.value && !menuOpen.value) {
        needClick.value = false;
        menuOpen.value = true;
        return;
      }
      if (menuOpen.value) {
        // Esc is not a gesture that may capture the mouse: try, and prompt for a click if refused.
        menuOpen.value = false;
        void this.input.lock().then(() => {
          if (!this.input.locked && !menuOpen.value) needClick.value = true;
        });
      } else menuOpen.value = true;
    };
    this.input.onEmote = (e) => {
      if (!this.arena?.body) return;
      this.net.send({ t: 'emote', e });
    };
    this.input.onCycle = (dir) => this.cycleSpectate(dir);
    this.input.onChat = () => {
      if (!this.arena || practiceGame.value) return;
      this.input.release();
      chatOpen.value = true;
    };
    canvas.addEventListener('mousedown', (e) => {
      // Spectating: the mouse buttons switch between players.
      if (menuOpen.value || this.arena?.body || this.arena?.kind !== 'round') return;
      if (e.button === 0) this.cycleSpectate(1);
      if (e.button === 2) this.cycleSpectate(-1);
    });
  }

  /** Closes the menu and captures the mouse. */
  resume() {
    if (!this.arena) return;
    menuOpen.value = false;
    needClick.value = false;
    void this.input.lock().then(() => {
      if (!this.input.locked && !menuOpen.value) needClick.value = true;
    });
  }

  start() {
    this.started = true;
    this.renderer.setQuality(settings.value.quality);
    // (Connection.start handles its own failures: it retries.)
    void this.net.start();
    const loop = () => {
      this.frame();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  /** Runs a dev command on the server (only a server started with --dev accepts them). */
  dev(cmd: DevCmd): Promise<{ ok: boolean; msg: string }> {
    const q = this.devSeq++;
    return new Promise((resolve) => {
      this.devWait.set(q, resolve);
      this.net.send({ t: 'dev', q, cmd });
      setTimeout(() => {
        if (this.devWait.delete(q)) resolve({ ok: false, msg: 'no answer from the server' });
      }, 10_000);
    });
  }

  // ------------------------------------------------------------------ rooms and chat

  /** Opens a room of one's own (or returns to it) and enters it. */
  createRoom(title: string, isPrivate: boolean) {
    denied.value = null;
    this.net.send({ t: 'create', title, private: isPrivate });
  }

  joinRoom(id: string, pin?: string) {
    // (With a PIN the form stays up until the server answers: wrong PIN or welcome.)
    if (!pin) denied.value = null;
    this.net.send({ t: 'join', room: id, ...(pin ? { pin } : {}) });
  }

  /** Back to the room list (the server answers with `home`). */
  leaveRoom() {
    this.net.send({ t: 'leave' });
  }

  sendChat(text: string) {
    const line = text.trim();
    if (line) this.net.send({ t: 'chat', text: line });
  }

  /** The room (or none) shows in the address, so a reload comes back to it and the link can be shared. */
  private setRoom(id: string | null) {
    this.wantRoom = id;
    if (practiceGame.value) return;
    const url = new URL(location.href);
    if (id) url.searchParams.set('room', id);
    else url.searchParams.delete('room');
    try {
      history.replaceState(null, '', url.search ? url : BASE_PATH);
    } catch {}
  }

  /** Out of the room: nothing to simulate or draw but the sky behind the room list. */
  private exitRoom() {
    this.arena?.dispose();
    this.arena = null;
    this.renderer.statics = null;
    for (const id of [...this.beans.keys()]) this.removeBean(id);
    this.decor.clear();
    this.players = new Map();
    this.input.unlock();
    this.input.script = null;
    arenaInfo.value = null;
    lobby.value = null;
    room.value = null;
    results.value = null;
    gameEnd.value = null;
    feed.value = [];
    myId.value = -1;
    clearChat();
    menuOpen.value = true;
    needClick.value = false;
    this.setRoom(null);
  }

  /** Internals for the debug probe (read-only use). */
  inspect() {
    return {
      beans: this.beans,
      players: this.players,
      spectate: this.spectate,
      fps: this.fps,
      frameMs: this.frameMs,
    };
  }

  nameOf(id: number) {
    return this.players.get(id)?.name ?? `#${id}`;
  }

  // ------------------------------------------------------------------ messages

  private onMessage(m: ServerMsg) {
    this.onServerMessage?.(m);
    switch (m.t) {
      case 'ready':
        devMode.value = m.dev;
        identity.set(m.token);
        return;
      case 'rooms':
        roomList.value = m.rooms;
        ownRoom.value = m.mine;
        return;
      case 'denied':
        // The room from the link or the one we were in is not to be had: the room list it is.
        if (this.arena || (this.wantRoom && m.reason !== 'pin')) this.exitRoom();
        denied.value = m;
        return;
      case 'home':
        this.exitRoom();
        denied.value = m.msg ? { t: 'denied', room: null, reason: 'gone', msg: m.msg } : null;
        return;
      case 'welcome':
        myId.value = m.id;
        denied.value = null;
        if (!m.resumed) {
          clearChat();
          menuOpen.value = true;
        }
        if (m.room) this.setRoom(m.room);
        return;
      case 'chat': {
        const p = this.players.get(m.id);
        pushChat({ name: m.name, color: p?.color ?? '#ffffff', text: m.text, mine: m.id === myId.value });
        if (m.id !== myId.value) sfx('click', 0.5);
        return;
      }
      case 'lobby': {
        lobby.value = m;
        room.value = m.room;
        this.players = new Map(m.players.map((p) => [p.id, p]));
        const me = this.players.get(myId.value);
        if (me && !settings.value.name) updateSettings({ name: me.name });
        for (const [id, bean] of this.beans) {
          const p = this.players.get(id);
          if (!p) continue;
          if (bean.color !== p.color) bean.setColor(p.color);
          if (bean.name !== p.name) bean.name = p.name;
          bean.setCrown(p.crowns > 0);
        }
        if (m.phase !== 'results') results.value = null;
        if (m.phase !== 'podium') gameEnd.value = null;
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
          this.spectateAuto = true;
        }
        this.beans.get(m.id)?.react('laugh', 3);
        note(`🏁 ${this.nameOf(m.id)}: финиш, ${ordinal(m.place)} место (${fmtSec(m.time)})`);
        return;
      case 'ko':
        if (m.out) this.arena?.out.add(m.id);
        this.beans.get(m.id)?.react(m.out ? 'cry' : 'scared', m.out ? 3 : 1.2);
        // Out of the round: the bean (and its name tag) leaves the map right away.
        if (m.out && m.id !== myId.value) this.lastSeen.set(m.id, 0);
        if (m.id === myId.value) {
          sfx(m.out ? 'out' : 'fall');
          if (m.out) {
            this.arena?.stopPlaying();
            this.spectateAuto = true;
          }
        }
        if (this.arena?.kind === 'round')
          pushFeed({ victim: m.id, by: m.by, cause: m.cause, out: m.out, shortcut: !!m.shortcut });
        return;
      case 'ev':
        this.arena?.onEvent(m.n, m.d);
        return;
      case 'scores':
        for (const [id, v] of m.s) {
          const was = this.arena?.scores.get(id) ?? 0;
          this.arena?.scores.set(id, v);
          if (this.arena?.kind === 'lobby' && v > was) this.bell(id, v);
        }
        return;
      case 'emote':
        this.beans.get(m.id)?.playEmote(m.e);
        return;
      case 'left':
        this.removeBean(m.id);
        return;
      case 'roundEnd':
        results.value = m;
        sfx('results');
        return;
      case 'clock':
        this.net.clock.setRate(m.rate, m.s);
        return;
      case 'devAck': {
        const wait = m.q !== null ? this.devWait.get(m.q) : undefined;
        if (m.q !== null) this.devWait.delete(m.q);
        if (wait) wait({ ok: m.ok, msg: m.msg });
        note(`🛠 ${m.ok ? '' : '✖ '}${m.msg}`);
        return;
      }
      case 'gameEnd':
        gameEnd.value = m;
        results.value = null;
        sfx('win');
        return;
      default:
        return;
    }
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
    this.renderer.statics = arena.statics;
    this.renderer.applyLook(arena.builder.look);
    arena.onBonus = (b) => {
      const info = BONUS_KINDS[b.kind];
      if (!info || b.takenBy === null) return;
      sfx('boing');
      this.beans.get(b.takenBy)?.react('grin', 1.5);
      note(`${info.icon} ${b.takenBy === myId.value ? 'Вы' : this.nameOf(b.takenBy)}: ${info.title}!`);
    };
    const me = myId.value;
    const inPlay =
      info.kind === 'lobby' ||
      ((info.kind === 'podium' || info.kind === 'round') &&
        info.participants.includes(me) &&
        !info.finished.includes(me) &&
        !info.out.includes(me));
    if (inPlay) arena.play(me);
    this.spectate = -1;
    this.spectateAuto = true;
    this.lastCount = 99;
    feed.value = [];
    // Camera: down the course in races, towards the middle in arenas.
    const i = Math.max(0, info.participants.indexOf(me));
    const { pos, yaw } = arena.spawn(i);
    this.rig.face(arena.spec.finish ? 0 : yaw);
    this.rig.update(pos, 1, null);
    this.rig.snap();
    this.renderer.cut();
    if (info.kind !== 'lobby') {
      results.value = null;
      menuOpen.value = false;
    }
    if (info.kind === 'round' && !info.late)
      void this.input.lock().then(() => {
        if (!this.input.locked && !menuOpen.value) needClick.value = true;
      });
  }

  /** Lobby: somebody climbed the tower and rang the bell (the lobby map counts it as a score). */
  private bell(id: number, times: number) {
    const mine = id === myId.value;
    if (this.players.get(id)?.bot) return;
    sfx(mine ? 'win' : 'boing', mine ? 1 : 0.5);
    this.beans.get(id)?.react('laugh', 2);
    const again = times > 1 ? ` (в ${times}-й раз)` : '';
    note(`🔔 ${mine ? 'Вы звоните' : `${this.nameOf(id)} звонит`} в колокол на башне${again}!`);
  }

  private removeBean(id: number) {
    this.beans.get(id)?.dispose();
    this.beans.delete(id);
    this.vels.delete(id);
  }

  private bean(id: number): Bean {
    let b = this.beans.get(id);
    if (!b) {
      const p = this.players.get(id);
      b = new Bean(p?.color ?? '#ffffff', p?.name ?? '');
      b.setCrown((p?.crowns ?? 0) > 0);
      b.setTail(!!this.decor.get(id)?.tail);
      this.renderer.scene.add(b.root);
      this.beans.set(id, b);
    }
    return b;
  }

  // ------------------------------------------------------------------ spectating

  private cycleSpectate(dir: number) {
    if (this.arena?.body) return;
    const ids = [-1, ...this.spectateIds];
    const i = Math.max(0, ids.indexOf(this.spectate));
    this.spectate = ids[(i + dir + ids.length) % ids.length]!;
    this.spectateAuto = false;
    sfx('click');
  }

  // ------------------------------------------------------------------ frame

  private frame() {
    const now = performance.now();
    const raw = (now - this.last) / 1000;
    const dt = Math.min(0.1, raw);
    this.last = now;
    // Real frame time (not capped): on a slow machine the fallback must kick in after seconds, not minutes.
    this.measure(now, Math.min(1, raw));
    const arena = this.arena;
    const prof = this.prof;
    this.renderer.tick(shotMode.value ? 0 : now / 1000);
    tickRainbow(shotMode.value ? 0 : now / 1000);
    this.renderer.motes.visible = !shotMode.value;
    if (arena) {
      prof.start('predict');
      this.input.enabled = !menuOpen.value && !chatOpen.value;
      this.input.spectating = !arena.body && arena.kind === 'round';
      this.rig.look(this.input.lookX, this.input.lookY);
      this.input.lookX = 0;
      this.input.lookY = 0;
      arena.predict(() => {
        const s = this.input.sample(DT);
        const [mx, mz] = s.world ?? this.rig.toWorld(s.moveX, s.moveY);
        return { mx, mz, jump: s.jump, dive: s.dive, grab: s.grab };
      });
      prof.start('events');
      this.playEvents(arena);
      // (Screenshots: server time, which does not depend on the connection like the prediction lead.)
      const t = (shotMode.value ? arena.tickAt(this.net.clock.serverNow()) : arena.renderTick()) * DT;
      prof.start('animate');
      arena.animate(t, dt);
      prof.start('beans');
      const focus = this.updateBeans(arena, dt, t);
      prof.start('camera');
      const cam = this.cameraOverride;
      if (cam) this.rig.cinematic(cam.eye, cam.look, dt, true);
      else if (!this.cinematic(arena, t, dt)) this.rig.update(focus, dt, arena.builder.world);
      if (arena.teleported) {
        // Respawned (or first placed by the server): look the way the bean faces.
        if (arena.body) this.rig.face(arena.body.yaw);
        this.rig.snap();
        this.renderer.cut();
        arena.teleported = false;
      }
      this.renderer.focus(focus);
      prof.start('hud');
      this.countdown(arena, t);
      if (now - this.hudAt > 100) {
        this.hudAt = now;
        this.updateHud(arena, t);
      }
    }
    prof.start('render');
    this.renderer.render();
    prof.stop();
    prof.frame();
    this.onFrame?.(raw * 1000, performance.now() - now);
  }

  /** Scripted camera: fly over the course during the intro, orbit the podium. Returns false when not active. */
  private cinematic(arena: ClientArena, t: number, dt: number): boolean {
    const spec = arena.spec;
    if (arena.kind === 'podium') {
      const a = Math.sin(t * 0.25) * 0.55;
      this.eye.set(Math.sin(a) * 12.5, 4.6, Math.cos(a) * 12.5);
      this.look.set(0, 2.6, 0);
      this.rig.cinematic(this.eye, this.look, dt, this.cut(arena));
      return true;
    }
    if (arena.kind !== 'round' || arena.info.late || t > -INTRO_HANDOVER) return false;
    const intro = INTRO_MS / 1000;
    const f = THREE.MathUtils.clamp((t + intro) / (intro - INTRO_HANDOVER), 0, 1);
    const e = f * f * (3 - 2 * f);
    const start = arena.spawn(Math.max(0, arena.info.participants.indexOf(myId.value))).pos;
    if (spec.finish) {
      // From above the finish back to the start line.
      const fz = spec.finish.z;
      const fy = spec.finish.y;
      const ez = THREE.MathUtils.lerp(fz + 14, start.z - 9, e);
      // Stay well above the course (it climbs on some maps) until the last moment.
      const courseY = THREE.MathUtils.lerp(start.y, fy, THREE.MathUtils.clamp((ez - start.z) / (fz - start.z), 0, 1));
      const above = courseY + 5 + 8 * (1 - THREE.MathUtils.smoothstep(e, 0.75, 1));
      this.eye.set(THREE.MathUtils.lerp(10, 0, e), Math.max(THREE.MathUtils.lerp(fy + 16, start.y + 5, e), above), ez);
      this.look.set(0, THREE.MathUtils.lerp(fy, start.y + 1, e), THREE.MathUtils.lerp(fz - 20, start.z + 6, e));
    } else {
      const c = spec.view ?? new THREE.Vector3();
      const a = t * 0.35;
      const r = THREE.MathUtils.lerp(30, 20, e);
      this.eye.set(c.x + Math.sin(a) * r, c.y + THREE.MathUtils.lerp(18, 11, e), c.z + Math.cos(a) * r);
      this.look.copy(c);
    }
    this.rig.cinematic(this.eye, this.look, dt, this.cut(arena));
    return true;
  }

  private cut(arena: ClientArena) {
    if (this.cineArena === arena.info.id) return false;
    this.cineArena = arena.info.id;
    this.renderer.cut();
    return true;
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
    if (this.autoQuality && document.visibilityState === 'visible' && this.frameMs > 24) this.slowFor += dt;
    else this.slowFor = 0;
    if (this.slowFor > 6) {
      this.slowFor = 0;
      const i = QUALITY_ORDER.indexOf(settings.value.quality);
      if (i > 0) {
        updateSettings({ quality: QUALITY_ORDER[i - 1]! });
        note('Качество графики снижено, чтобы игра шла плавнее');
      }
    }
  }

  private playEvents(arena: ClientArena) {
    const e = arena.takeEvents();
    if (e.jumped) sfx('jump');
    if (e.dived) sfx('dive');
    if (e.knocked) {
      sfx('tackle');
      this.rig.shake(0.55);
    } else if (e.hit) {
      sfx('hit');
      this.rig.shake(0.3);
    } else if (e.bumped > 5) {
      sfx('hit', Math.min(1, e.bumped / 12) * 0.6);
      this.rig.shake(Math.min(0.25, e.bumped * 0.02));
    }
    if (e.landed > 0.55) this.rig.shake((e.landed - 0.5) * 0.4);
    if (e.bounced) sfx('boing');
    if (arena.ownGrab !== this.lastGrab) {
      if (arena.ownGrab >= 0) sfx('grab');
      this.lastGrab = arena.ownGrab;
    }
  }

  private updateBeans(arena: ClientArena, dt: number, t: number): THREE.Vector3 {
    const me = myId.value;
    const now = performance.now();
    const podium = arena.kind === 'podium';
    let focus: THREE.Vector3 | null = null;
    if (arena.body) {
      const b = this.bean(me);
      const yaw = arena.ownPose(dt, this.tmp);
      // Inside a portal: out of sight, gliding to the other end (the camera follows it there).
      const hidden = arena.body.inPortal;
      if (!hidden) pushOut(arena.builder.world, this.tmp, arena.body.tilt, arena.body.tiltDir, arena.body.size);
      b.root.position.copy(this.tmp);
      b.root.rotation.y = yaw;
      b.root.visible = !hidden;
      if (podium) b.pose = this.podiumPose(me);
      const held = arena.ownGrab >= 0 ? this.beans.get(arena.ownGrab) : undefined;
      b.animate(dt, {
        vel: arena.body.vel,
        anim: arena.ownAnim(),
        t,
        landImpact: arena.events.landed,
        tilt: arena.body.tilt,
        tiltDir: arena.body.tiltDir,
        grabAt: held?.root.visible ? held.root.position : null,
        grabSize: held?.root.scale.x ?? 1,
        size: arena.body.size,
        power: arena.body.power,
      });
      this.lastSeen.set(me, now);
      focus = b.root.position;
    }
    const poses = arena.remotePoses();
    for (const [id, p] of poses) {
      if (id === me) continue;
      const b = this.bean(id);
      const fresh = !b.root.visible;
      let v = this.vels.get(id);
      if (!v) {
        v = new THREE.Vector3();
        this.vels.set(id, v);
      }
      const inPortal = p.anim === ANIM.portal;
      if (!inPortal) pushOut(arena.builder.world, p.pos, p.tilt, p.tiltDir, p.power === POWER.giant ? GIANT_SIZE : 1);
      // Velocity from the interpolated path, smoothed: steady cycles instead of per-frame jitter.
      if (!fresh && dt > 0) {
        this.tmp.subVectors(p.pos, b.root.position).divideScalar(dt);
        if (this.tmp.length() > 40) this.tmp.set(0, 0, 0);
        v.lerp(this.tmp, 1 - Math.exp(-dt * 14));
      } else v.set(0, 0, 0);
      b.root.position.copy(p.pos);
      b.root.rotation.y = p.yaw;
      b.root.visible = true;
      if (podium) b.pose = this.podiumPose(id);
      const held = p.grab >= 0 ? (p.grab === me ? this.beans.get(me) : this.beans.get(p.grab)) : undefined;
      b.animate(dt, {
        vel: v,
        anim: p.anim,
        t,
        tilt: p.tilt,
        tiltDir: p.tiltDir,
        grabAt: held?.root.visible ? held.root.position : null,
        grabSize: held?.root.scale.x ?? 1,
        size: p.power === POWER.giant ? GIANT_SIZE : 1,
        power: p.power,
      });
      // (A bean knocked out just now stays hidden: the last snapshots may still carry it.)
      if (arena.out.has(id) || arena.finished.has(id)) b.root.visible = false;
      else if (inPortal) {
        b.root.visible = false;
        this.lastSeen.set(id, now);
      } else this.lastSeen.set(id, now);
    }
    for (const [id, b] of this.beans) {
      if (now - (this.lastSeen.get(id) ?? 0) > 400) b.root.visible = false;
    }
    if (!focus) {
      // Spectating: follow someone still in play, or look over the whole map.
      this.spectateIds = [...poses.keys()].filter((id) => id !== me).sort((a, b) => a - b);
      if (this.spectate !== -1 && !this.spectateIds.includes(this.spectate)) this.spectateAuto = true;
      if (this.spectateAuto) this.spectate = this.spectateIds[0] ?? -1;
      // A new target: cut to it instead of sliding the camera across the map.
      if (this.spectate !== this.lastSpectate) {
        this.lastSpectate = this.spectate;
        this.rig.snap();
        this.renderer.cut();
      }
      focus = this.beans.get(this.spectate)?.root.position ?? arena.spec.view ?? new THREE.Vector3();
    }
    return focus;
  }

  /** Podium: the top three celebrate, the rest are sad (or just clap). */
  private podiumPose(id: number): 'cheer' | 'clap' | 'sad' {
    const place = (gameEnd.value?.standings.find((s) => s.id === id)?.place ?? 99) - 1;
    const n = gameEnd.value?.standings.length ?? 1;
    if (place === 0) return 'cheer';
    if (place < 3 && place < n - 1) return 'clap';
    return place >= n - 2 && n > 3 ? 'sad' : 'clap';
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
    let status: Hud['status'] = info.kind === 'lobby' ? 'lobby' : info.kind === 'podium' ? 'podium' : 'spectating';
    const roster: Hud['roster'] = {};
    const fin = [...arena.finished];
    for (const id of info.participants) {
      const st: PlayStatus = arena.finished.has(id) ? 'finished' : arena.out.has(id) ? 'out' : 'play';
      roster[id] = { status: st, place: fin.indexOf(id) + 1 };
    }
    if (info.kind === 'round') {
      if (arena.body) status = 'play';
      else if (arena.finished.has(me)) status = 'finished';
      else if (arena.out.has(me)) status = 'out';
    }
    let bonus: string | null = null;
    const body = arena.body;
    if (body && body.power !== POWER.none) {
      const info = BONUS_KINDS[body.power];
      const left = Math.max(0, Math.ceil(body.powerUntil - arena.predTick * DT));
      if (info && left > 0) bonus = `${info.icon} ${info.title} · ${left} с`;
    }
    let mapText: string | null = null;
    try {
      mapText = info.kind === 'round' && t >= 0 ? (arena.spec.hud?.() ?? null) : null;
    } catch {}
    const next = lobby.value?.next ?? null;
    hud.value = {
      t,
      timeLeft: Math.max(0, (info.endAt - this.net.clock.serverNow()) / 1000),
      nextIn: next === null ? -1 : Math.max(0, (next - this.net.clock.serverNow()) / 1000),
      status,
      place: fin.indexOf(me) + 1,
      spectating:
        status === 'play' || status === 'lobby' || status === 'podium' || this.spectate < 0 ? '' : this.nameOf(this.spectate),
      mapText,
      bonus,
      roster,
      roundScores: Object.fromEntries(arena.scores),
      fps: this.fps,
      drawCalls: this.renderer.info.calls,
    };
    conn.value = conn.value.transport === this.net.kind ? conn.value : { ...conn.value, transport: this.net.kind };
  }

  /** Screen position of a bean's head for HTML name tags (null when off screen). */
  tagPositions(out: Map<number, { x: number; y: number; d: number; name: string; color: string }>) {
    out.clear();
    const cam = this.renderer.camera;
    const w = this.renderer.canvas.clientWidth;
    const h = this.renderer.canvas.clientHeight;
    const me = myId.value;
    for (const [id, b] of this.beans) {
      if (!b.root.visible || id === me) continue;
      this.tmp.copy(b.root.position);
      this.tmp.y += 0.65 + 1.6 * b.root.scale.y;
      const d = this.tmp.distanceTo(cam.position);
      this.tmp.project(cam);
      if (this.tmp.z > 1 || Math.abs(this.tmp.x) > 1.1 || Math.abs(this.tmp.y) > 1.1) continue;
      out.set(id, { x: (this.tmp.x * 0.5 + 0.5) * w, y: (-this.tmp.y * 0.5 + 0.5) * h, d, name: b.name, color: b.color });
    }
  }
}
