import { getGame, getMap, LOBBY } from '../games';
import { decodeInput, encodeSnapshot } from '../shared/codec';
import {
  COLORS,
  INTRO_MS,
  MAX_PLAYERS,
  PRACTICE_RESULTS_MS,
  RECONNECT_GRACE_MS,
  RESULTS_MS,
  SNAPSHOT_EVERY,
  WINNER_MS,
} from '../shared/consts';
import type { GameMeta } from '../shared/game';
import {
  type ArenaInfo,
  type ClientMsg,
  DEFAULT_PLAYLIST,
  type Hello,
  type LobbyPlayer,
  type Phase,
  type Playlist,
  type ServerMsg,
  sanitizeName,
} from '../shared/protocol';
import { mulberry32, type Rng, shuffle } from '../shared/rng';
import { type RoundView, RULES, type Rule } from '../shared/rules';
import { ServerArena } from './arena';
import { eliminationFor, planShow, validPlaylist } from './director';

/** A client connection: reliable control messages plus unreliable datagrams. */
export interface Conn {
  readonly kind: 'ws' | 'wt';
  readonly ip: string;
  send(msg: ServerMsg): void;
  datagram(data: Uint8Array): void;
  close(reason?: string): void;
}

export interface Logger {
  info(msg: string, data?: Record<string, unknown>): void;
  warn(msg: string, data?: Record<string, unknown>): void;
}

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
}

interface Player {
  id: number;
  name: string;
  color: string;
  score: number;
  crowns: number;
  token: string;
  bot: boolean;
  conn: Conn | null;
  disconnectedAt: number;
  spectator: boolean;
  msgWindow: number;
  msgCount: number;
  rtt: number;
}

interface Show {
  plan: string[];
  index: number;
  alive: Set<number>;
  started: number;
}

interface Round {
  game: GameMeta;
  rule: Rule;
  eliminate: number;
  qualify: number;
  over: boolean;
  rng: Rng;
  index: number;
  total: number;
}

const BOT_NAMES = ['Кекс', 'Пончик', 'Жужа', 'Бублик', 'Мармелад', 'Хрустик', 'Пельмень', 'Зефир', 'Кнопка', 'Шмель'];
const MSG_RATE = 60;
const NOOP_LOG: Logger = { info() {}, warn() {} };

function makeToken(): string {
  return globalThis.crypto.randomUUID();
}

export class Room {
  readonly players = new Map<number, Player>();
  host: number | null = null;
  phase: Phase = 'lobby';
  show: Show | null = null;
  round: Round | null = null;
  arena: ServerArena;
  playlist: Playlist = DEFAULT_PLAYLIST;
  private nextId = 1;
  private arenaSeq = 1;
  private timer: { at: number; fn: () => void } | null = null;
  private readonly interval: ReturnType<typeof setInterval> | null;
  private readonly rng: Rng;
  private readonly log: Logger;
  private readonly max: number;
  private readonly clock: () => number;
  private disposed = false;

  constructor(private readonly opts: RoomOptions) {
    this.rng = opts.seed !== undefined ? mulberry32(opts.seed) : Math.random;
    this.log = opts.log ?? NOOP_LOG;
    this.max = Math.min(opts.maxPlayers ?? MAX_PLAYERS, MAX_PLAYERS);
    this.clock = opts.clock ?? (() => performance.now());
    if (opts.practice && !getGame(opts.practice.game)) throw new Error(`unknown game ${opts.practice.game}`);
    this.arena = this.makeLobbyArena();
    this.interval = opts.autoTick === false ? null : setInterval(() => this.update(), 4);
  }

  get practice() {
    return !!this.opts.practice;
  }

  get empty() {
    return this.humans().length === 0;
  }

  now() {
    return this.clock();
  }

  dispose() {
    this.disposed = true;
    if (this.interval) clearInterval(this.interval);
    this.arena.dispose();
  }

  // ------------------------------------------------------------------ connections

  /** A validated, authenticated hello. Returns the player id, or null if rejected. */
  join(conn: Conn, m: Hello): number | null {
    const resumed = m.token ? [...this.players.values()].find((p) => !p.bot && p.token === m.token) : undefined;
    if (resumed) {
      const old = resumed.conn;
      resumed.conn = conn;
      resumed.disconnectedAt = 0;
      old?.close('replaced');
      this.log.info('player resumed', { id: resumed.id, name: resumed.name, via: conn.kind });
      this.welcome(resumed, true);
      return resumed.id;
    }
    if (this.players.size >= this.max) {
      const bot = this.phase === 'lobby' ? [...this.players.values()].reverse().find((p) => p.bot) : undefined;
      if (bot) this.removePlayer(bot);
      else {
        conn.send({ t: 'reject', reason: 'full', msg: `Сервер заполнен (максимум ${this.max} игроков)` });
        conn.close('full');
        return null;
      }
    }
    const id = this.nextId++;
    const p: Player = {
      id,
      name: sanitizeName(m.name) || `Боб ${id}`,
      color: this.freeColor(),
      score: 0,
      crowns: 0,
      token: makeToken(),
      bot: false,
      conn,
      disconnectedAt: 0,
      spectator: this.phase !== 'lobby',
      msgWindow: 0,
      msgCount: 0,
      rtt: 0,
    };
    this.players.set(id, p);
    this.log.info('player joined', { id, name: p.name, via: conn.kind, practice: this.practice });
    this.updateHost();
    if (this.arena.kind === 'lobby') this.arena.addPawn(id, false);
    this.welcome(p, false);
    const pr = this.opts.practice;
    if (pr && !this.show) {
      const need = Math.max(pr.bots, (getGame(pr.game)?.minPlayers ?? 1) - 1);
      for (let i = 0; i < need && this.players.size < this.max; i++) this.addBot();
      this.startShow();
    }
    return id;
  }

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

  control(id: number, conn: Conn, m: ClientMsg) {
    const p = this.players.get(id);
    if (!p || p.conn !== conn || !this.allowRate(p)) return;
    switch (m.t) {
      case 'hello':
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
        if (this.isHost(p) && this.phase === 'lobby' && this.players.size >= this.opts.minPlayers) this.startShow();
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
      case 'emote':
        if (this.arena.pawns.get(p.id)?.status === 'play') this.broadcast({ t: 'emote', id: p.id, e: m.e });
        return;
    }
  }

  datagram(id: number, conn: Conn, data: Uint8Array) {
    const p = this.players.get(id);
    if (!p || p.conn !== conn) return;
    const pkt = decodeInput(data);
    if (!pkt) return;
    this.arena.input(id, pkt, this.now());
  }

  private allowRate(p: Player): boolean {
    const now = this.now();
    if (now - p.msgWindow > 1000) {
      p.msgWindow = now;
      p.msgCount = 0;
    }
    if (++p.msgCount === MSG_RATE + 1) this.log.warn('rate limited', { id: p.id });
    return p.msgCount <= MSG_RATE;
  }

  private welcome(p: Player, resumed: boolean) {
    this.sendTo(p, { t: 'welcome', id: p.id, token: p.token, solo: this.opts.minPlayers <= 1, practice: this.practice, resumed });
    this.sendLobby();
    this.sendTo(p, { t: 'arena', ...this.arenaInfo(true) });
  }

  private isHost(p: Player) {
    return p.id === this.host;
  }

  private freeColor(): string {
    const used = new Set([...this.players.values()].map((p) => p.color));
    return COLORS.find((c) => !used.has(c)) ?? COLORS[0];
  }

  private addBot() {
    const id = this.nextId++;
    const used = new Set([...this.players.values()].map((p) => p.name));
    const base = BOT_NAMES.find((n) => !used.has(`Бот ${n}`)) ?? String(id);
    this.players.set(id, {
      id,
      name: `Бот ${base}`,
      color: this.freeColor(),
      score: 0,
      crowns: 0,
      token: '',
      bot: true,
      conn: null,
      disconnectedAt: 0,
      spectator: false,
      msgWindow: 0,
      msgCount: 0,
      rtt: 0,
    });
    if (this.arena.kind === 'lobby') this.arena.addPawn(id, true);
  }

  private humans() {
    return [...this.players.values()].filter((p) => !p.bot);
  }

  private updateHost() {
    const cur = this.host !== null ? this.players.get(this.host) : undefined;
    if (!cur?.conn) {
      const next = this.humans().find((p) => p.conn);
      this.host = next?.id ?? (cur ? cur.id : (this.humans()[0]?.id ?? null));
    }
  }

  private removePlayer(p: Player) {
    if (!this.players.delete(p.id)) return;
    this.show?.alive.delete(p.id);
    this.arena.removePawn(p.id);
    this.broadcast({ t: 'left', id: p.id });
    if (!this.humans().length) {
      for (const b of [...this.players.values()]) this.players.delete(b.id);
      this.backToLobby();
      return;
    }
    this.updateHost();
    this.sendLobby();
    if (this.phase === 'results' && this.show && this.show.alive.size <= 1 && !this.practice) this.nextRound();
    else this.checkRound();
  }

  private later(ms: number, fn: () => void) {
    this.timer = { at: this.now() + ms, fn };
  }

  // ------------------------------------------------------------------ show flow

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
    });
    for (const p of this.players.values()) arena.addPawn(p.id, p.bot);
    return arena;
  }

  private setArena(a: ServerArena) {
    this.arena.dispose();
    this.arena = a;
    this.broadcast({ t: 'arena', ...this.arenaInfo(false) });
  }

  private hooks() {
    return {
      onFinish: (id: number) => this.onFinish(id),
      onOut: (id: number) => this.onOut(id),
      onEvent: (n: string, d: unknown) => this.broadcast({ t: 'ev', n, d }),
      onScore: (id: number, v: number) => this.broadcast({ t: 'scores', s: [[id, v]] }),
      onSnapshot: (tick: number) => this.snapshot(tick),
      warn: (msg: string, data?: Record<string, unknown>) => this.log.warn(msg, data),
    };
  }

  private startShow() {
    const ids = [...this.players.keys()].slice(0, this.max);
    for (const p of this.players.values()) {
      p.score = 0;
      p.spectator = !ids.includes(p.id);
    }
    const pr = this.opts.practice;
    const plan = pr ? [pr.game] : planShow(ids.length, this.playlist, this.rng);
    this.show = { plan, index: 0, alive: new Set(ids), started: ids.length };
    this.log.info('show started', { players: ids.length, plan });
    this.nextRound();
  }

  private nextRound() {
    this.timer = null;
    const show = this.show;
    if (!show) return;
    const alive = [...show.alive].filter((id) => this.players.has(id));
    if (!alive.length) return this.backToLobby();
    if (!this.practice && alive.length === 1 && show.started > 1) return this.declareWinner(alive[0]!);
    const gameId = this.practice ? show.plan[0] : show.plan[show.index];
    const mod = gameId ? getMap(gameId) : undefined;
    if (!mod) return this.declareWinner(alive[0]!);
    const game = mod.meta;
    const rule = RULES[game.rules];
    const remainingNonFinal = show.plan.length - 1 - show.index;
    const eliminate = this.practice || rule.final ? 0 : eliminationFor(alive.length, remainingNonFinal, show.started);
    if (!this.practice) show.index++;
    this.phase = 'round';
    const seed = Math.floor(this.rng() * 1e9);
    const now = this.now();
    const participants = shuffle([...alive], this.rng);
    this.round = {
      game,
      rule,
      eliminate,
      qualify: alive.length - eliminate,
      over: false,
      rng: mulberry32(seed ^ 0x5bd1e995),
      index: this.practice ? 1 : show.index,
      total: this.practice ? 1 : show.plan.length,
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
      });
    } catch (e) {
      this.log.warn('map build failed', { game: game.id, err: String(e) });
      return this.backToLobby();
    }
    participants.forEach((id, i) => {
      arena.addPawn(id, this.players.get(id)?.bot ?? false, i);
    });
    this.log.info('round', { game: game.id, eliminate, players: participants.length });
    this.setArena(arena);
    this.sendLobby();
  }

  private onFinish(id: number) {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round') return;
    this.broadcast({ t: 'fin', id, place: this.arena.finished.length });
    this.checkRound();
  }

  private onOut(id: number) {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round') return;
    this.broadcast({ t: 'out', id });
    this.checkRound();
  }

  private view(): RoundView {
    const a = this.arena;
    const r = this.round!;
    return {
      participants: a.participants,
      connected: (id) => this.players.has(id),
      finished: a.finished,
      out: a.out,
      scores: a.scores,
      progress: (id) => a.pawns.get(id)?.progress ?? Number.NEGATIVE_INFINITY,
      eliminate: r.eliminate,
      qualify: r.qualify,
      timeUp: this.now() > a.endAt,
      solo: (this.show?.started ?? 1) <= 1,
    };
  }

  private checkRound() {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round' || this.arena.kind !== 'round') return;
    if (r.rule.isOver(this.view())) this.endRound();
  }

  private endRound() {
    const r = this.round!;
    r.over = true;
    this.arena.frozen = true;
    const { ranking, winner } = r.rule.outcome(this.view(), r.rng);
    for (const e of ranking) {
      const p = this.players.get(e.id);
      if (p) p.score += e.points;
    }
    this.log.info('round over', { game: r.game.id, ranking: ranking.map((e) => [e.id, e.ok]) });
    if (this.practice) {
      this.phase = 'results';
      this.broadcast({ t: 'roundEnd', game: r.game.id, ranking, practice: true });
      this.sendLobby();
      this.later(PRACTICE_RESULTS_MS, () => this.nextRound());
      return;
    }
    if (r.rule.final) {
      if (winner === undefined) return this.backToLobby();
      return this.declareWinner(winner);
    }
    for (const e of ranking) if (!e.ok) this.show?.alive.delete(e.id);
    this.phase = 'results';
    this.broadcast({ t: 'roundEnd', game: r.game.id, ranking, practice: false });
    this.sendLobby();
    this.later(RESULTS_MS, () => this.nextRound());
  }

  private declareWinner(id: number) {
    this.timer = null;
    if (this.round) this.round.over = true;
    this.phase = 'winner';
    const p = this.players.get(id);
    if (p) p.crowns++;
    this.log.info('winner', { id, name: p?.name });
    this.setArena(this.makeLobbyArena());
    this.broadcast({ t: 'winner', id, name: p?.name ?? '???' });
    this.sendLobby();
    this.later(WINNER_MS, () => this.backToLobby());
  }

  private backToLobby() {
    this.timer = null;
    this.phase = 'lobby';
    this.show = null;
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
    this.sendLobby();
  }

  // ------------------------------------------------------------------ simulation

  /** Advances timers and the simulation to `now`; called by the room timer or by tests. */
  update(now = this.now()) {
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
    this.arena.advance(now);
    this.checkRound();
  }

  private snapshot(tick: number) {
    if (tick % SNAPSHOT_EVERY !== 0) return;
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
      eliminate: isRound ? r.eliminate : 0,
      qualify: isRound ? r.qualify : 0,
      index: isRound ? r.index : 0,
      total: isRound ? r.total : 0,
      practice: this.practice,
      late,
      finished: a.finished,
      out: a.out,
      events: a.events,
      scores: [...a.scores],
    };
  }

  lobbyMsg(): Extract<ServerMsg, { t: 'lobby' }> {
    return {
      t: 'lobby',
      phase: this.phase,
      host: this.host,
      min: this.opts.minPlayers,
      max: this.max,
      playlist: this.playlist,
      players: [...this.players.values()].map(
        (p): LobbyPlayer => ({
          id: p.id,
          name: p.name,
          color: p.color,
          score: p.score,
          crowns: p.crowns,
          alive: this.show ? this.show.alive.has(p.id) : true,
          spectator: p.spectator,
          bot: p.bot,
          connected: p.bot || !!p.conn,
          ping: Math.round(p.rtt),
        }),
      ),
    };
  }

  private sendLobby() {
    this.broadcast(this.lobbyMsg());
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
