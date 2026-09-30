import * as THREE from 'three';
import { getGame, getMap, LOBBY, PODIUM } from '../../games';
import { decodeInput, encodeSnapshot } from '../../shared/codec';
import {
  COLORS,
  INTRO_MS,
  MAX_PLAYERS,
  PODIUM_MS,
  PRACTICE_RESULTS_MS,
  RECONNECT_GRACE_MS,
  RESULTS_MS,
  ROOM_PIN_DIGITS,
  SNAPSHOT_EVERY,
  TICK_MS,
} from '../../shared/consts';
import type { GameMeta } from '../../shared/game';
import { Sections } from '../../shared/prof';
import {
  type ArenaInfo,
  type ClientMsg,
  DEFAULT_PLAYLIST,
  type DevCmd,
  type LobbyPlayer,
  type Phase,
  type Playlist,
  type RoomInfo,
  type ServerMsg,
  type Standing,
  sanitizeChat,
  sanitizeName,
} from '../../shared/protocol';
import { mulberry32, type Rng, shuffle } from '../../shared/rng';
import { isRoundOver, type RoundStats, type RoundView, scoreRound } from '../../shared/rules';
import { TickMeter } from '../diag';
import { type Logger, NOOP_LOG } from '../log';
import type { Conn } from '../net/conn';
import { type KoInfo, type Recording, ServerArena } from './arena';
import { computeAwards, emptyGameStats } from './awards';
import { GameClock } from './clock';
import { planGame, validPlaylist } from './director';
import { botName, makePlayer, type Player } from './players';

export interface RoomOptions {
  minPlayers: number;
  maxPlayers?: number;
  practice?: { game: string; bots: number };
  log?: Logger;
  seed?: number;
  introMs?: number;
  /** Server clock in ms (monotonic). */
  clock?: () => number;
  /** Run the simulation on a timer (off in tests, which call update()). */
  autoTick?: boolean;
  /** Accept dev commands (server --dev). */
  dev?: boolean;
  /** The room's code and name in the room list ('' for rooms that are not listed). */
  id?: string;
  title?: string;
  /** Identity of the player who created the room: the host whenever they are in it. */
  owner?: string | null;
  /** PIN of a private room. */
  pin?: string | null;
  /** Kept when nobody is in it (the dev server's room). */
  permanent?: boolean;
  /** Something the room list shows changed (players, host, phase, access). */
  onChange?: () => void;
}

/** One game: a planned series of rounds everyone plays, scored by points. */
interface Session {
  plan: string[];
  /** Rounds started so far. */
  index: number;
  /** Players the game started with (for solo detection). */
  started: number;
}

interface Round {
  game: GameMeta;
  over: boolean;
  index: number;
  total: number;
}

const MSG_RATE = 60;
/** Shortest time between two chat lines of one player. */
const CHAT_GAP_MS = 700;

export function makePin(): string {
  const n = globalThis.crypto.getRandomValues(new Uint32Array(1))[0]!;
  return String(n % 10 ** ROOM_PIN_DIGITS).padStart(ROOM_PIN_DIGITS, '0');
}

/** A lobby and the games played from it: players and bots, the host, rounds, scores, chat. */
export class Room {
  readonly id: string;
  readonly title: string;
  /** Identity of the room's creator (null: nobody's, the dev room). */
  readonly owner: string | null;
  /** PIN of a private room; null when anyone may enter. */
  pin: string | null;
  /** Identities the room let in: they come back without the PIN. */
  admitted = new Set<string>();
  readonly players = new Map<number, Player>();
  /**
   * The acting host. The owner whenever they are in the room; while they are away (or after they
   * handed the role over) another connected player.
   */
  host: number | null = null;
  phase: Phase = 'lobby';
  session: Session | null = null;
  round: Round | null = null;
  arena: ServerArena;
  playlist: Playlist = DEFAULT_PLAYLIST;
  /** The host wants the empty places of the lobby filled with bots. */
  fill = false;
  /** Game time: timers and arenas run on it (dev commands slow, pause or warp it). */
  readonly clock: GameClock;
  private nextId = 1;
  private arenaSeq = 1;
  private timer: { at: number; fn: () => void } | null = null;
  private readonly interval: ReturnType<typeof setInterval> | null;
  private readonly rng: Rng;
  private readonly log: Logger;
  readonly max: number;
  /** Dev: seed for the next round's map. */
  private nextSeed: number | null = null;
  /** Dev: simulating a time warp (no snapshots for the ticks in between). */
  private warping = false;
  private disposed = false;
  /** Simulation cost (debug page). */
  readonly meter = new TickMeter();
  /** Dev: the last rounds as played, for scripts/replay.ts (newest last). */
  readonly replays: Recording[] = [];
  /** Where tick time goes (sections of ServerArena.step). */
  readonly prof = new Sections();

  constructor(private readonly opts: RoomOptions) {
    this.id = opts.id ?? '';
    this.title = opts.title ?? '';
    this.owner = opts.owner ?? null;
    this.pin = opts.pin ?? null;
    this.rng = opts.seed !== undefined ? mulberry32(opts.seed) : Math.random;
    this.log = opts.log ?? NOOP_LOG;
    this.max = Math.min(opts.maxPlayers ?? MAX_PLAYERS, MAX_PLAYERS);
    this.clock = new GameClock(opts.clock ?? (() => performance.now()));
    if (opts.practice && !getGame(opts.practice.game)) throw new Error(`unknown game ${opts.practice.game}`);
    this.arena = this.makeLobbyArena();
    this.interval = opts.autoTick === false ? null : setInterval(() => this.update(), 4);
  }

  get practice() {
    return !!this.opts.practice;
  }

  /** Nobody is in the room (not even someone who may still reconnect). */
  get empty() {
    return this.humans().length === 0;
  }

  get permanent() {
    return !!this.opts.permanent;
  }

  /** Game time (ms). */
  now() {
    return this.clock.now();
  }

  get dev() {
    return !!this.opts.dev;
  }

  /** ms until the pending phase change (results → next round…), or null. */
  get timerIn() {
    return this.timer ? Math.round(this.timer.at - this.now()) : null;
  }

  dispose() {
    this.disposed = true;
    if (this.interval) clearInterval(this.interval);
    this.arena.dispose();
  }

  // ------------------------------------------------------------------ connections

  /**
   * A player enters: someone new, or (same identity) the one who is already here, on a new
   * connection. Returns the player id, or null when the room is full.
   */
  join(conn: Conn, who: { uid: string; name: string }): number | null {
    const resumed = who.uid ? this.playerOf(who.uid) : undefined;
    if (resumed) {
      const old = resumed.conn;
      resumed.conn = conn;
      resumed.disconnectedAt = 0;
      if (old !== conn) old?.close('replaced');
      this.log.info('player resumed', { id: resumed.id, name: resumed.name, via: conn.kind });
      this.seatHost(resumed);
      this.welcome(resumed, true);
      return resumed.id;
    }
    if (this.players.size >= this.max) {
      // A bot gives up its place to a person.
      const bot = this.spareBot();
      if (!bot) return null;
      this.drop(bot);
    }
    const id = this.nextId++;
    const p = makePlayer(id, sanitizeName(who.name) || `Боб ${id}`, this.freeColor(), {
      uid: who.uid,
      conn,
      spectator: this.phase !== 'lobby',
    });
    this.players.set(id, p);
    if (who.uid) this.admitted.add(who.uid);
    this.log.info('player joined', { id, name: p.name, via: conn.kind, practice: this.practice });
    this.seatHost(p);
    if (this.arena.kind === 'lobby') this.arena.addPawn(id, false);
    this.syncBots();
    this.welcome(p, false);
    const pr = this.opts.practice;
    if (pr && !this.session) {
      const need = Math.max(pr.bots, (getGame(pr.game)?.minPlayers ?? 1) - 1);
      for (let i = 0; i < need && this.players.size < this.max; i++) this.addBot();
      this.startGame();
    }
    this.checkRound();
    return id;
  }

  /** The connection of player `id` is gone: out of the lobby at once, out of a game after the grace period. */
  leave(id: number, conn: Conn) {
    const p = this.players.get(id);
    if (!p || p.conn !== conn) return;
    p.conn = null;
    p.disconnectedAt = this.now();
    this.log.info('player disconnected', { id });
    if (this.phase === 'lobby' && !this.practice) {
      this.removePlayer(p);
      return;
    }
    this.updateHost();
    this.sendLobby();
  }

  /** Player `id` leaves for good (back to the room list, or into another room). */
  quit(id: number) {
    const p = this.players.get(id);
    if (!p || p.bot) return;
    p.conn = null;
    this.log.info('player left', { id });
    this.removePlayer(p);
  }

  /** The player with this identity, if they are in the room (connected or not). */
  playerOf(uid: string): Player | undefined {
    for (const p of this.players.values()) if (!p.bot && p.uid === uid) return p;
    return undefined;
  }

  control(id: number, conn: Conn, m: ClientMsg) {
    const p = this.players.get(id);
    if (!p || p.conn !== conn || !this.allowRate(p)) return;
    switch (m.t) {
      // Handled before a message reaches the room (see Gateway).
      case 'hello':
      case 'create':
      case 'join':
      case 'leave':
        return;
      case 'ping':
        if (m.rtt !== undefined) p.rtt = m.rtt;
        return this.sendTo(p, { t: 'pong', c: m.c, s: this.now() });
      case 'name': {
        const name = sanitizeName(m.name);
        if (name) {
          p.name = name;
          this.sendLobby();
        }
        return;
      }
      case 'color':
        if (this.phase === 'lobby' && ![...this.players.values()].some((o) => o !== p && o.color === m.c)) {
          p.color = m.c;
          this.sendLobby();
        }
        return;
      case 'start':
        if (this.isHost(p) && this.phase === 'lobby' && this.roster().length >= this.opts.minPlayers) this.startGame();
        return;
      case 'abort':
        if (this.isHost(p) && this.phase !== 'lobby' && !this.practice) this.backToLobby();
        return;
      case 'playlist':
        if (this.isHost(p) && this.phase === 'lobby') {
          this.playlist = validPlaylist(m.pl);
          this.sendLobby();
        }
        return;
      case 'addBot':
        if (this.isHost(p) && this.phase === 'lobby' && this.players.size < this.max) {
          this.addBot();
          this.sendLobby();
        }
        return;
      case 'removeBot': {
        const b = this.players.get(m.id);
        if (this.isHost(p) && this.phase === 'lobby' && b?.bot) this.removePlayer(b);
        return;
      }
      case 'fill':
        if (this.isHost(p) && this.phase === 'lobby' && !this.practice) {
          this.fill = m.on;
          // Off: the bots that only filled places go; the ones the host added stay.
          if (!m.on) for (const b of [...this.players.values()]) if (b.auto) this.drop(b);
          this.syncBots();
          this.sendLobby();
        }
        return;
      case 'access':
        if (this.isHost(p) && !this.practice && !!this.pin !== m.private) {
          this.pin = m.private ? makePin() : null;
          // A new PIN: whoever is here stays welcome, those who left need it.
          this.admitted = new Set(this.humans().map((h) => h.uid));
          this.log.info(m.private ? 'room made private' : 'room made public', { by: p.id });
          this.sendLobby();
        }
        return;
      case 'host': {
        // The host hands the role over to another connected player (any phase).
        const to = this.players.get(m.id);
        if (this.isHost(p) && to && to !== p && !to.bot && to.conn) {
          this.host = to.id;
          this.sendLobby();
        }
        return;
      }
      case 'emote':
        if (this.arena.pawns.get(p.id)?.status === 'play') this.broadcast({ t: 'emote', id: p.id, e: m.e });
        return;
      case 'chat': {
        const text = sanitizeChat(m.text);
        const now = this.clock.real();
        if (!text || now - p.chatAt < CHAT_GAP_MS) return;
        p.chatAt = now;
        this.broadcast({ t: 'chat', id: p.id, name: p.name, text });
        return;
      }
      case 'dev': {
        if (!this.dev) return this.sendTo(p, { t: 'devAck', q: m.q ?? null, ok: false, msg: 'dev commands are off' });
        let msg: string;
        let ok = true;
        try {
          msg = this.devCommand(p.id, m.cmd);
        } catch (e) {
          ok = false;
          msg = e instanceof Error ? e.message : String(e);
        }
        this.log.info('dev', { id: p.id, cmd: m.cmd.c, ok, msg });
        this.arena.note(`dev ${m.cmd.c}`, p.id, { ok, msg });
        return this.sendTo(p, { t: 'devAck', q: m.q ?? null, ok, msg });
      }
    }
  }

  // ------------------------------------------------------------------ dev commands

  /** Runs a dev command for player `by`; returns a short result, throws with the reason it cannot. */
  devCommand(by: number, cmd: DevCmd): string {
    const target = (id: number | undefined) => {
      const tid = id ?? by;
      if (!this.arena.pawns.has(tid)) throw new Error(`no bean #${tid} in this arena`);
      return tid;
    };
    const near = (i: number) => {
      const me = this.arena.pawns.get(by)?.body.pos;
      return me ? me.clone().add(new THREE.Vector3(Math.cos(i * 1.3) * 2, 0.3, Math.sin(i * 1.3) * 2)) : undefined;
    };
    switch (cmd.c) {
      case 'skipIntro': {
        const left = this.arena.startAt - this.now();
        if (this.arena.kind !== 'round' || left <= 0) return 'already started';
        this.warp(left + TICK_MS);
        return `skipped ${Math.round(left)} ms`;
      }
      case 'warp':
        this.warp(cmd.ms);
        return `warped ${cmd.ms} ms`;
      case 'endRound':
        if (this.phase !== 'round' || !this.round || this.round.over) throw new Error('no round running');
        this.endRound();
        return 'round ended';
      case 'start': {
        if (cmd.games?.length) {
          const bad = cmd.games.filter((g) => !getGame(g));
          if (bad.length) throw new Error(`unknown games: ${bad.join(', ')}`);
        }
        if (this.session) this.backToLobby();
        // `bots`: exactly that many (the ones left from before are replaced).
        if (cmd.bots !== undefined) {
          this.fill = false;
          for (const p of [...this.players.values()]) if (p.bot) this.removePlayer(p);
        }
        const want = Math.min(this.max, this.humans().length + (cmd.bots ?? 0));
        while (this.players.size < want) this.addBot();
        this.playlist = cmd.games?.length
          ? { mode: 'custom', games: cmd.games, rounds: cmd.rounds ?? cmd.games.length }
          : { ...this.playlist, rounds: cmd.rounds ?? this.playlist.rounds };
        this.startGame();
        return `started: ${this.session?.plan.join(', ')}`;
      }
      case 'lobby':
        this.backToLobby();
        return 'back in the lobby';
      case 'rate':
        this.setRate(cmd.k);
        return cmd.k === 0 ? 'paused' : `game time ×${cmd.k}`;
      case 'step':
        if (this.clock.rate !== 0) throw new Error('pause first (rate 0)');
        this.warp(cmd.ticks * TICK_MS);
        return `stepped ${cmd.ticks} ticks (tick ${this.arena.tick})`;
      case 'teleport': {
        const id = target(cmd.id);
        this.arena.devTeleport(id, new THREE.Vector3(...cmd.p), cmd.yaw);
        return `#${id} → ${cmd.p.map((v) => v.toFixed(1)).join(' ')}`;
      }
      case 'goto': {
        const id = target(cmd.id);
        const at = this.arena.devPlace(id, cmd.to);
        if (!at) throw new Error(`this map has no ${cmd.to === 'finish' ? 'finish' : `checkpoint ${cmd.to}`}`);
        this.arena.devTeleport(id, at);
        return `#${id} → ${cmd.to}`;
      }
      case 'bot': {
        const added: number[] = [];
        for (let i = 0; i < (cmd.n ?? 1) && this.players.size < this.max; i++) {
          const id = this.addBot();
          added.push(id);
          const at = cmd.near ? near(i) : undefined;
          if (this.arena.kind !== 'lobby') this.arena.addLatePawn(id, true, at);
          else if (at) this.arena.devTeleport(id, at);
        }
        if (!added.length) throw new Error('room is full');
        this.sendLobby();
        return `bots ${added.join(', ')}`;
      }
      case 'bots':
        this.arena.botsOn = cmd.on;
        return cmd.on ? 'bots think' : 'bots frozen';
      case 'kill': {
        const id = target(cmd.id);
        this.arena.devKill(id);
        return `#${id} dropped`;
      }
      case 'knock': {
        const id = target(cmd.id);
        this.arena.devKnock(id, cmd.v);
        return `#${id} knocked`;
      }
      case 'grab': {
        const actor = target(cmd.actor);
        const err = this.arena.devGrab(actor, target(cmd.target), cmd.s ?? 3);
        if (err) throw new Error(err);
        return `#${actor} holds #${cmd.target}`;
      }
      case 'seed':
        this.nextSeed = cmd.seed;
        return `next round seed ${cmd.seed}`;
    }
  }

  /** Dev: game time runs `k` times as fast (0 pauses); clients re-sync their clocks. */
  private setRate(k: number) {
    const now = this.clock.setRate(k);
    this.broadcast({ t: 'clock', rate: k, s: now });
  }

  /** Dev: moves game time forward by `ms`, simulating every tick on the way (and running due timers). */
  private warp(ms: number) {
    this.clock.rebase();
    this.warping = true;
    try {
      for (let left = ms; left > 0; left -= 50) {
        this.clock.skip(Math.min(50, left));
        this.update(this.now(), true);
      }
    } finally {
      this.warping = false;
    }
    this.broadcast({ t: 'clock', rate: this.clock.rate, s: this.now() });
    this.snapshot(this.arena.tick - (this.arena.tick % SNAPSHOT_EVERY));
  }

  datagram(id: number, conn: Conn, data: Uint8Array) {
    const p = this.players.get(id);
    if (!p || p.conn !== conn) return;
    const pkt = decodeInput(data);
    if (!pkt) return;
    this.arena.input(id, pkt, this.now());
  }

  private allowRate(p: Player): boolean {
    // (The real clock: game time may be paused.)
    const now = this.clock.real();
    if (now - p.msgWindow > 1000) {
      p.msgWindow = now;
      p.msgCount = 0;
    }
    if (++p.msgCount === MSG_RATE + 1) this.log.warn('rate limited', { id: p.id });
    return p.msgCount <= MSG_RATE;
  }

  private welcome(p: Player, resumed: boolean) {
    this.sendTo(p, {
      t: 'welcome',
      id: p.id,
      room: this.id,
      solo: this.opts.minPlayers <= 1,
      practice: this.practice,
      resumed,
    });
    if (this.clock.shifted) this.sendTo(p, { t: 'clock', rate: this.clock.rate, s: this.now() });
    this.sendLobby();
    this.sendTo(p, { t: 'arena', ...this.arenaInfo(true) });
  }

  // ------------------------------------------------------------------ players, host, bots

  private isHost(p: Player) {
    return p.id === this.host;
  }

  private freeColor(): string {
    const used = new Set([...this.players.values()].map((p) => p.color));
    return COLORS.find((c) => !used.has(c)) ?? COLORS[0];
  }

  private addBot(auto = false): number {
    const id = this.nextId++;
    const name = botName(
      [...this.players.values()].map((p) => p.name),
      id,
    );
    this.players.set(id, makePlayer(id, name, this.freeColor(), { auto }));
    if (this.arena.kind === 'lobby') this.arena.addPawn(id, true);
    return id;
  }

  /** With "fill with bots" on, the lobby is kept full while someone is there to play with them. */
  private syncBots() {
    if (!this.fill || this.phase !== 'lobby' || this.practice) return;
    if (!this.humans().some((p) => p.conn)) return;
    while (this.players.size < this.max) this.addBot(true);
  }

  /** The bot that gives up its place to a person: in a round, one that no longer plays if there is one. */
  private spareBot(): Player | undefined {
    const bots = [...this.players.values()].filter((p) => p.bot).reverse();
    return bots.find((b) => this.arena.pawns.get(b.id)?.status !== 'play') ?? bots[0];
  }

  private humans() {
    return [...this.players.values()].filter((p) => !p.bot);
  }

  /** `p` just connected: the room's owner takes the host role back, anyone else may fill a vacancy. */
  private seatHost(p: Player) {
    if (this.owner !== null && p.uid === this.owner) this.host = p.id;
    else this.updateHost();
  }

  /** The host must be connected: the owner if they are here, else whoever has been here longest. */
  private updateHost() {
    const cur = this.host !== null ? this.players.get(this.host) : undefined;
    if (cur?.conn) return;
    const humans = this.humans();
    const next = humans.find((p) => p.conn && p.uid === this.owner) ?? humans.find((p) => p.conn);
    this.host = next?.id ?? cur?.id ?? humans[0]?.id ?? null;
  }

  /** Takes a player out of the room and its arena (what that means for the room: removePlayer). */
  private drop(p: Player): boolean {
    if (!this.players.delete(p.id)) return false;
    this.arena.removePawn(p.id);
    this.broadcast({ t: 'left', id: p.id });
    return true;
  }

  private removePlayer(p: Player) {
    if (!this.drop(p)) return;
    if (!p.bot && !this.humans().length) {
      // The last person left: the bots go too, and the room waits in its lobby.
      for (const b of [...this.players.values()]) this.players.delete(b.id);
      this.backToLobby();
      return;
    }
    this.updateHost();
    this.syncBots();
    this.sendLobby();
    this.checkRound();
  }

  private later(ms: number, fn: () => void) {
    this.timer = { at: this.now() + ms, fn };
  }

  // ------------------------------------------------------------------ game flow

  private newArenaId() {
    this.arenaSeq = (this.arenaSeq % 65535) + 1;
    return this.arenaSeq;
  }

  private makeLobbyArena(): ServerArena {
    const now = this.now();
    const arena = new ServerArena({
      id: this.newArenaId(),
      kind: 'lobby',
      module: LOBBY,
      seed: 1,
      startAt: now,
      participants: [...this.players.keys()],
      now,
      hooks: this.hooks(),
      prof: this.prof,
    });
    for (const p of this.players.values()) arena.addPawn(p.id, p.bot);
    return arena;
  }

  private setArena(a: ServerArena) {
    const rec = this.arena.takeRecording();
    if (rec) {
      this.replays.push(rec);
      if (this.replays.length > 5) this.replays.shift();
    }
    this.arena.dispose();
    this.arena = a;
    this.broadcast({ t: 'arena', ...this.arenaInfo(false) });
  }

  private hooks() {
    return {
      onFinish: (id: number, time: number) => this.onFinish(id, time),
      onKo: (ko: KoInfo) => this.onKo(ko),
      onEvent: (n: string, d: unknown) => this.broadcast({ t: 'ev', n, d }),
      onScore: (id: number, v: number) => this.broadcast({ t: 'scores', s: [[id, v]] }),
      onSnapshot: (tick: number) => this.snapshot(tick),
      onEmote: (id: number, e: number) => this.broadcast({ t: 'emote', id, e }),
      warn: (msg: string, data?: Record<string, unknown>) => this.log.warn(msg, data),
    };
  }

  /** Players who take part in the next round: bots and connected humans. */
  private roster(): number[] {
    return [...this.players.values()].filter((p) => p.bot || p.conn).map((p) => p.id);
  }

  private startGame() {
    const ids = this.roster().slice(0, this.max);
    for (const p of this.players.values()) {
      p.score = 0;
      p.stats = emptyGameStats();
      p.spectator = false;
    }
    const pr = this.opts.practice;
    const plan = pr ? [pr.game] : planGame(ids.length, this.playlist, this.rng);
    this.session = { plan, index: 0, started: ids.length };
    this.log.info('game started', { players: ids.length, plan });
    this.nextRound();
  }

  private nextRound() {
    this.timer = null;
    const session = this.session;
    if (!session) return;
    const ids = this.roster();
    if (!ids.length) return this.backToLobby();
    if (!this.practice && session.index >= session.plan.length) return this.endGame();
    const gameId = this.practice ? session.plan[0] : session.plan[session.index];
    const mod = gameId ? getMap(gameId) : undefined;
    if (!mod) return this.endGame();
    const game = mod.meta;
    if (!this.practice) session.index++;
    this.phase = 'round';
    const seed = this.nextSeed ?? Math.floor(this.rng() * 1e9);
    // A dev seed fixes the spawn order too (screenshots, repeatable tests).
    const order = this.nextSeed !== null ? mulberry32(this.nextSeed ^ 0x5eed) : this.rng;
    this.nextSeed = null;
    const now = this.now();
    const participants = shuffle(ids, order);
    for (const p of this.players.values()) p.spectator = !participants.includes(p.id);
    this.round = {
      game,
      over: false,
      index: this.practice ? 1 : session.index,
      total: this.practice ? 1 : session.plan.length,
    };
    let arena: ServerArena;
    try {
      arena = new ServerArena({
        id: this.newArenaId(),
        kind: 'round',
        module: mod,
        seed,
        startAt: now + (this.opts.introMs ?? INTRO_MS),
        participants,
        now,
        hooks: this.hooks(),
        prof: this.prof,
        record: this.dev,
      });
    } catch (e) {
      this.log.warn('map build failed', { game: game.id, err: String(e) });
      return this.backToLobby();
    }
    participants.forEach((id, i) => {
      arena.addPawn(id, this.players.get(id)?.bot ?? false, i);
    });
    this.log.info('round', { game: game.id, players: participants.length });
    this.setArena(arena);
    this.sendLobby();
  }

  private onFinish(id: number, time: number) {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round') return;
    this.broadcast({ t: 'fin', id, place: this.arena.finished.length, time });
    this.checkRound();
  }

  private onKo(ko: KoInfo) {
    if (this.arena.kind === 'round') {
      const r = this.round;
      if (!r || r.over || this.phase !== 'round') return;
    }
    const { shortcut, ...rest } = ko;
    this.broadcast({ t: 'ko', ...rest, ...(shortcut ? { shortcut } : {}) });
    if (ko.out) this.checkRound();
  }

  private view(): RoundView {
    const a = this.arena;
    const r = this.round!;
    return {
      genre: r.game.genre,
      participants: a.participants,
      connected: (id) => this.players.has(id),
      finished: a.finished,
      out: a.out,
      scores: a.scores,
      progress: (id) => a.pawns.get(id)?.progress ?? Number.NEGATIVE_INFINITY,
      timeUp: this.now() > a.endAt,
      solo: (this.session?.started ?? 1) <= 1,
      bots: new Set([...this.players.values()].filter((p) => p.bot).map((p) => p.id)),
      // Places the bots left when a round ends early.
      rng: this.rng,
    };
  }

  private checkRound() {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round' || this.arena.kind !== 'round') return;
    if (isRoundOver(this.view())) this.endRound();
  }

  private endRound() {
    const r = this.round!;
    r.over = true;
    const a = this.arena;
    a.frozen = true;
    const view = this.view();
    const stats = new Map<number, RoundStats>();
    for (const [id, pawn] of a.pawns) stats.set(id, pawn.stats);
    const totals = new Map([...this.players.values()].map((p) => [p.id, p.score]));
    const bots = new Set([...this.players.values()].filter((p) => p.bot).map((p) => p.id));
    const rows = scoreRound(view, stats, totals, bots);
    const n = rows.length;
    const roundSecs = Math.max(0, a.time);
    for (const row of rows) {
      const p = this.players.get(row.id);
      if (!p) continue;
      p.score = row.total;
      const s = stats.get(row.id);
      if (!s) continue;
      const g = p.stats;
      g.falls += s.falls;
      g.kos += s.kos;
      g.grabs += s.grabs;
      g.tackles += s.tackles;
      g.shortcuts += s.shortcuts;
      if (row.place === 1 && n > 1) g.wins++;
      if (r.game.genre === 'race') g.raceRanks.push(n > 1 ? (row.place - 1) / (n - 1) : 0);
      if (r.game.genre === 'survival') g.survived += s.outAt ?? roundSecs;
    }
    this.log.info('round over', { game: r.game.id, rows: rows.map((e) => [e.id, e.delta]) });
    this.phase = 'results';
    this.broadcast({ t: 'roundEnd', game: r.game.id, index: r.index, total: r.total, rows, practice: this.practice });
    this.sendLobby();
    this.later(this.practice ? PRACTICE_RESULTS_MS : RESULTS_MS, () => this.nextRound());
  }

  /** Final standings: points, then round wins, then fewer falls. */
  private standings(): Standing[] {
    const list = [...this.players.values()].sort(
      (a, b) => b.score - a.score || b.stats.wins - a.stats.wins || a.stats.falls - b.stats.falls || a.id - b.id,
    );
    return list.map((p, i) => ({
      id: p.id,
      name: p.name,
      color: p.color,
      place: i + 1,
      total: p.score,
      wins: p.stats.wins,
      falls: p.stats.falls,
    }));
  }

  private endGame() {
    this.timer = null;
    if (this.round) this.round.over = true;
    const standings = this.standings();
    const winner = standings[0];
    if (!winner) return this.backToLobby();
    this.phase = 'podium';
    const p = this.players.get(winner.id);
    if (p) p.crowns++;
    const awards = computeAwards([...this.players.values()].map((x) => ({ id: x.id, s: x.stats })));
    this.log.info('game over', { winner: winner.id, name: winner.name, standings: standings.map((s) => [s.id, s.total]) });
    const now = this.now();
    const order = standings.map((s) => s.id);
    const arena = new ServerArena({
      id: this.newArenaId(),
      kind: 'podium',
      module: PODIUM,
      seed: 1,
      startAt: now,
      participants: order,
      now,
      hooks: this.hooks(),
      prof: this.prof,
    });
    order.forEach((id, i) => {
      arena.addPawn(id, this.players.get(id)?.bot ?? false, i);
    });
    this.setArena(arena);
    this.broadcast({ t: 'gameEnd', standings, awards });
    this.sendLobby();
    this.later(PODIUM_MS, () => this.backToLobby());
  }

  private backToLobby() {
    this.timer = null;
    this.phase = 'lobby';
    this.session = null;
    this.round = null;
    for (const p of [...this.players.values()]) {
      if (!p.bot && !p.conn && !this.practice) {
        this.players.delete(p.id);
        this.broadcast({ t: 'left', id: p.id });
        continue;
      }
      p.spectator = false;
    }
    this.updateHost();
    if (this.arena.kind !== 'lobby' || this.arena.pawns.size !== this.players.size) this.setArena(this.makeLobbyArena());
    this.syncBots();
    this.sendLobby();
  }

  // ------------------------------------------------------------------ simulation

  /** Advances timers and the simulation to `now`; called by the room timer or by tests. */
  update(now = this.now(), all = false) {
    if (this.disposed) return;
    for (const p of [...this.players.values()]) {
      if (!p.bot && !p.conn && p.disconnectedAt && now - p.disconnectedAt > RECONNECT_GRACE_MS) {
        this.log.info('player timed out', { id: p.id });
        this.removePlayer(p);
      }
    }
    if (this.timer && now >= this.timer.at) {
      const { fn } = this.timer;
      this.timer = null;
      fn();
    }
    const t0 = performance.now();
    const steps = this.arena.advance(now, all);
    if (steps) this.meter.add(steps, performance.now() - t0);
    this.checkRound();
  }

  private snapshot(tick: number) {
    if (tick % SNAPSHOT_EVERY !== 0 || this.warping) return;
    const a = this.arena;
    let spectatorPacket: Uint8Array | null = null;
    for (const p of this.players.values()) {
      if (!p.conn) continue;
      let data: Uint8Array;
      if (a.pawns.get(p.id)?.status === 'play') data = encodeSnapshot(a.snapshotFor(p.id));
      else data = spectatorPacket ??= encodeSnapshot(a.snapshotFor(null));
      try {
        p.conn.datagram(data);
      } catch (e) {
        this.log.warn('datagram failed', { id: p.id, err: String(e) });
      }
    }
    a.clearTeleports();
  }

  /** A recorded round: the last finished ones, or (`current`) the one running now. */
  debugReplay(i: number | 'current'): Recording | null {
    if (i === 'current') return this.arena.takeRecording();
    return this.replays.at(i < 0 ? i : -1 - i) ?? null;
  }

  // ------------------------------------------------------------------ messages

  arenaInfo(late: boolean): ArenaInfo {
    const a = this.arena;
    const r = this.round;
    const isRound = a.kind === 'round' && !!r;
    return {
      id: a.id,
      kind: a.kind,
      game: a.module.meta.id,
      seed: a.seed,
      startAt: a.startAt,
      endAt: a.endAt,
      participants: a.participants,
      index: isRound ? r.index : 0,
      total: isRound ? r.total : 0,
      practice: this.practice,
      late,
      finished: a.finished,
      out: a.out,
      events: a.events,
      scores: [...a.scores],
      hash: a.staticHash,
    };
  }

  /** The room's line in the room list. */
  info(): RoomInfo {
    const humans = this.humans().length;
    return {
      id: this.id,
      title: this.title,
      private: !!this.pin,
      host: (this.host !== null ? this.players.get(this.host)?.name : undefined) ?? '',
      players: humans,
      bots: this.players.size - humans,
      max: this.max,
      phase: this.phase,
    };
  }

  lobbyMsg(): Extract<ServerMsg, { t: 'lobby' }> {
    return {
      t: 'lobby',
      room: { id: this.id, title: this.title, private: !!this.pin },
      phase: this.phase,
      host: this.host,
      min: this.opts.minPlayers,
      max: this.max,
      playlist: this.playlist,
      fill: this.fill,
      pin: null,
      players: [...this.players.values()].map(
        (p): LobbyPlayer => ({
          id: p.id,
          name: p.name,
          color: p.color,
          score: p.score,
          crowns: p.crowns,
          spectator: p.spectator,
          bot: p.bot,
          connected: p.bot || !!p.conn,
          ping: Math.round(p.rtt),
        }),
      ),
    };
  }

  private sendLobby() {
    const msg = this.lobbyMsg();
    // Only the host is told the PIN: the others ask them for it.
    for (const p of this.players.values()) this.sendTo(p, this.pin && p.id === this.host ? { ...msg, pin: this.pin } : msg);
    this.opts.onChange?.();
  }

  private sendTo(p: Player, msg: ServerMsg) {
    if (!p.conn) return;
    try {
      p.conn.send(msg);
    } catch (e) {
      this.log.warn('send failed', { id: p.id, err: String(e) });
    }
  }

  private broadcast(msg: ServerMsg) {
    for (const p of this.players.values()) this.sendTo(p, msg);
  }
}
