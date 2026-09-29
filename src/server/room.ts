import { getGame } from '../games';
import { COLORS, MAX_PLAYERS, PROTOCOL_VERSION, RECONNECT_GRACE_MS, RESULTS_MS, SNAPSHOT_MS, WINNER_MS, INTRO_MS } from '../shared/consts';
import { type AnyGameMeta, fallBehaviour, type GameServerCtx } from '../shared/game';
import {
  type ClientMsg,
  ClientMsgSchema,
  DEFAULT_PLAYLIST,
  type GameEventRecord,
  type LobbyPlayer,
  type Phase,
  type Playlist,
  type RoundInfo,
  type ServerMsg,
  type Snapshot,
  sanitizeName,
} from '../shared/protocol';
import { mulberry32, type Rng, shuffle } from '../shared/rng';
import { RULES, type Rule, type RoundView } from '../shared/rules';
import { eliminationFor, planShow, validPlaylist } from './director';

export interface Conn {
  send(msg: ServerMsg): void;
  close(): void;
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
}

export interface Session {
  message(data: unknown): void;
  close(): void;
}

interface PlayerState {
  p: [number, number, number];
  r: number;
  a: number;
}

interface Player {
  id: number;
  name: string;
  color: string;
  score: number;
  crowns: number;
  token: string;
  bot: boolean;
  owner: number | null;
  conn: Conn | null;
  disconnectedAt: number;
  state: PlayerState | null;
  progress: number;
  spectator: boolean;
  msgWindow: number;
  msgCount: number;
  lastBump: number;
}

interface Show {
  plan: string[];
  index: number;
  alive: Set<number>;
  started: number;
}

interface Round {
  game: AnyGameMeta;
  rule: Rule;
  eliminate: number;
  qualify: number;
  participants: number[];
  finished: number[];
  out: number[];
  scores: Map<number, number>;
  events: GameEventRecord[];
  seed: number;
  startAt: number;
  endAt: number;
  over: boolean;
  announced: boolean;
  state: unknown;
  rng: Rng;
  ctx: GameServerCtx;
  index: number;
  total: number;
}

const BOT_NAMES = ['Кекс', 'Пончик', 'Жужа', 'Бублик', 'Мармелад', 'Хрустик', 'Пельмень', 'Зефир', 'Кнопка', 'Шмель'];
const MSG_RATE = 400;
const NOOP_LOG: Logger = { info() {}, warn() {} };

function makeToken(): string {
  const c = globalThis.crypto;
  if (c?.randomUUID) return c.randomUUID();
  return Array.from({ length: 4 }, () => Math.random().toString(36).slice(2, 10)).join('');
}

const r2 = (v: number) => Math.round(v * 100) / 100;

export class Room {
  readonly players = new Map<number, Player>();
  host: number | null = null;
  phase: Phase = 'lobby';
  show: Show | null = null;
  round: Round | null = null;
  playlist: Playlist = DEFAULT_PLAYLIST;
  private nextId = 1;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly interval: ReturnType<typeof setInterval>;
  private readonly rng: Rng;
  private readonly log: Logger;
  private readonly max: number;

  constructor(private readonly opts: RoomOptions) {
    this.rng = opts.seed !== undefined ? mulberry32(opts.seed) : Math.random;
    this.log = opts.log ?? NOOP_LOG;
    this.max = Math.min(opts.maxPlayers ?? MAX_PLAYERS, MAX_PLAYERS);
    this.interval = setInterval(() => this.tick(), SNAPSHOT_MS);
    if (opts.practice && !getGame(opts.practice.game)) throw new Error(`unknown game ${opts.practice.game}`);
  }

  dispose() {
    clearInterval(this.interval);
    this.clearTimer();
  }

  get practice() {
    return !!this.opts.practice;
  }

  open(conn: Conn): Session {
    let me: Player | null = null;
    return {
      message: (raw) => {
        let data: unknown = raw;
        if (typeof raw === 'string') {
          try {
            data = JSON.parse(raw);
          } catch {
            return;
          }
        }
        const parsed = ClientMsgSchema.safeParse(data);
        if (!parsed.success) {
          this.log.warn('bad message', { from: me?.id, issue: parsed.error.issues[0]?.message });
          return;
        }
        const m = parsed.data;
        if (!me) {
          if (m.t === 'hello') me = this.hello(conn, m);
          else if (m.t === 'ping') conn.send({ t: 'pong', c: m.c, s: Date.now() });
          return;
        }
        if (me.conn !== conn) return;
        if (!this.allowRate(me)) return;
        this.onMessage(me, m);
      },
      close: () => {
        if (me && me.conn === conn) this.disconnect(me);
      },
    };
  }

  private allowRate(p: Player): boolean {
    const now = Date.now();
    if (now - p.msgWindow > 1000) {
      p.msgWindow = now;
      p.msgCount = 0;
    }
    if (++p.msgCount === MSG_RATE + 1) this.log.warn('rate limited', { id: p.id });
    return p.msgCount <= MSG_RATE;
  }

  private hello(conn: Conn, m: Extract<ClientMsg, { t: 'hello' }>): Player | null {
    if (m.v !== PROTOCOL_VERSION) {
      conn.send({ t: 'reject', reason: 'version', msg: 'Версия игры устарела — обновите страницу' });
      conn.close();
      return null;
    }
    const resumed = m.token ? [...this.players.values()].find((p) => !p.bot && p.token === m.token) : undefined;
    if (resumed) {
      const old = resumed.conn;
      resumed.conn = conn;
      resumed.disconnectedAt = 0;
      old?.close();
      this.log.info('player resumed', { id: resumed.id, name: resumed.name });
      this.welcome(resumed, true);
      return resumed;
    }
    if (this.players.size >= this.max) {
      const bot = this.phase === 'lobby' ? [...this.players.values()].reverse().find((p) => p.bot) : undefined;
      if (bot) this.removePlayer(bot);
      else {
        conn.send({ t: 'reject', reason: 'full', msg: `Сервер заполнен (максимум ${this.max} игроков)` });
        conn.close();
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
      owner: null,
      conn,
      disconnectedAt: 0,
      state: null,
      progress: Number.NEGATIVE_INFINITY,
      spectator: this.phase !== 'lobby',
      msgWindow: 0,
      msgCount: 0,
      lastBump: 0,
    };
    this.players.set(id, p);
    this.log.info('player joined', { id, name: p.name });
    this.updateHost();
    this.welcome(p, false);
    const pr = this.opts.practice;
    if (pr && !this.show) {
      const need = Math.max(pr.bots, (getGame(pr.game)?.minPlayers ?? 1) - 1);
      for (let i = 0; i < need && this.players.size < this.max; i++) this.addBot();
      this.startShow();
    }
    return p;
  }

  private welcome(p: Player, resumed: boolean) {
    this.sendTo(p, { t: 'welcome', id: p.id, token: p.token, solo: this.opts.minPlayers <= 1, practice: this.practice, resumed });
    this.sendLobby();
    if (this.round && this.phase !== 'lobby') this.sendTo(p, { t: 'round', ...this.roundInfo(true) });
  }

  private onMessage(p: Player, m: ClientMsg) {
    switch (m.t) {
      case 'hello':
        return;
      case 'ping':
        return this.sendTo(p, { t: 'pong', c: m.c, s: Date.now() });
      case 's': {
        const a = this.actor(p, m.as);
        if (!a) return;
        a.state = { p: [r2(m.p[0]), r2(m.p[1]), r2(m.p[2])], r: r2(m.r), a: m.a };
        if (this.round && this.phase === 'round') a.progress = Math.max(a.progress, m.p[2]);
        return;
      }
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
      case 'finish':
        return this.onFinish(this.actor(p, m.as));
      case 'out':
        return this.onOut(this.actor(p, m.as));
      case 'ev':
        return this.onGameEvent(this.actor(p, m.as), m.n, m.d);
      case 'bump':
      case 'grab': {
        const a = this.actor(p, m.as);
        const target = this.players.get(m.to);
        if (!a || !target || target === a) return;
        const now = Date.now();
        if (m.t === 'bump') {
          if (now - a.lastBump < 250) return;
          a.lastBump = now;
          const v = m.v.map((x) => Math.max(-20, Math.min(20, x))) as [number, number, number];
          this.sendTo(target, { t: 'bump', to: target.id, from: a.id, v });
        } else this.sendTo(target, { t: 'grab', to: target.id, from: a.id });
        return;
      }
      case 'emote': {
        const a = this.actor(p, m.as);
        if (a) this.broadcast({ t: 'emote', id: a.id, e: m.e });
        return;
      }
    }
  }

  private actor(p: Player, as: number | undefined): Player | null {
    if (as === undefined || as === p.id) return p;
    const b = this.players.get(as);
    return b?.bot && b.owner === p.id ? b : null;
  }

  private isHost(p: Player) {
    return p.id === this.host;
  }

  private inPlay(r: Round, id: number) {
    return r.participants.includes(id) && !r.finished.includes(id) && !r.out.includes(id);
  }

  private onFinish(a: Player | null) {
    const r = this.round;
    if (!a || !r || r.over || this.phase !== 'round' || !this.inPlay(r, a.id)) return;
    if (fallBehaviour(r.game.rules) !== 'checkpoint' || Date.now() < r.startAt) return;
    if (r.game.finishZ !== undefined && (a.state?.p[2] ?? Number.NEGATIVE_INFINITY) < r.game.finishZ - 6) {
      this.log.warn('finish rejected', { id: a.id, z: a.state?.p[2] });
      return;
    }
    r.finished.push(a.id);
    this.broadcast({ t: 'fin', id: a.id, place: r.finished.length });
    this.checkRound();
  }

  private onOut(a: Player | null) {
    const r = this.round;
    if (!a || !r || r.over || this.phase !== 'round' || !this.inPlay(r, a.id)) return;
    if (fallBehaviour(r.game.rules) !== 'out') return;
    r.out.push(a.id);
    this.broadcast({ t: 'out', id: a.id });
    this.checkRound();
  }

  private onGameEvent(a: Player | null, name: string, data: unknown) {
    const r = this.round;
    if (!a || !r || r.over || this.phase !== 'round' || !r.participants.includes(a.id)) return;
    const schema = r.game.events?.[name];
    const handler = r.game.server?.on?.[name];
    if (!schema || !handler) return;
    const parsed = schema.safeParse(data);
    if (!parsed.success) {
      this.log.warn('bad game event', { id: a.id, name });
      return;
    }
    try {
      handler(r.ctx, r.state, a.id, parsed.data);
    } catch (e) {
      this.log.warn('game event handler failed', { game: r.game.id, name, err: String(e) });
    }
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
      owner: this.host,
      conn: null,
      disconnectedAt: 0,
      state: null,
      progress: Number.NEGATIVE_INFINITY,
      spectator: false,
      msgWindow: 0,
      msgCount: 0,
      lastBump: 0,
    });
  }

  private humans() {
    return [...this.players.values()].filter((p) => !p.bot);
  }

  private updateHost() {
    const cur = this.host !== null ? this.players.get(this.host) : undefined;
    if (!cur || !cur.conn) {
      const next = this.humans().find((p) => p.conn);
      this.host = next?.id ?? (cur ? cur.id : (this.humans()[0]?.id ?? null));
    }
    for (const p of this.players.values()) if (p.bot) p.owner = this.host;
  }

  private disconnect(p: Player) {
    p.conn = null;
    p.disconnectedAt = Date.now();
    this.log.info('player disconnected', { id: p.id });
    if (this.phase === 'lobby' && !this.practice) {
      this.removePlayer(p);
      return;
    }
    this.updateHost();
    this.sendLobby();
  }

  private removePlayer(p: Player) {
    if (!this.players.delete(p.id)) return;
    this.show?.alive.delete(p.id);
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

  private clearTimer() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private later(ms: number, fn: () => void) {
    this.clearTimer();
    this.timer = setTimeout(() => {
      this.timer = null;
      fn();
    }, ms);
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
    this.clearTimer();
    const show = this.show;
    if (!show) return;
    const alive = [...show.alive].filter((id) => this.players.has(id));
    if (!alive.length) return this.backToLobby();
    if (!this.practice && alive.length === 1 && show.started > 1) return this.declareWinner(alive[0] as number);
    const gameId = this.practice ? show.plan[0] : show.plan[show.index];
    const game = gameId ? getGame(gameId) : undefined;
    if (!game) return this.declareWinner(alive[0] as number);
    const rule = RULES[game.rules];
    const remainingNonFinal = show.plan.length - 1 - show.index;
    const eliminate = this.practice || rule.final ? 0 : eliminationFor(alive.length, remainingNonFinal, show.started);
    if (!this.practice) show.index++;
    this.phase = 'round';
    for (const p of this.players.values()) {
      p.state = null;
      p.progress = Number.NEGATIVE_INFINITY;
    }
    const seed = Math.floor(this.rng() * 1e9);
    const startAt = Date.now() + (this.opts.introMs ?? INTRO_MS);
    const participants = shuffle([...alive], this.rng);
    const round: Round = {
      game,
      rule,
      eliminate,
      qualify: alive.length - eliminate,
      participants,
      finished: [],
      out: [],
      scores: new Map(),
      events: [],
      seed,
      startAt,
      endAt: startAt + game.duration * 1000,
      over: false,
      announced: false,
      state: undefined,
      rng: mulberry32(seed ^ 0x5bd1e995),
      ctx: undefined as unknown as GameServerCtx,
      index: this.practice ? 1 : show.index,
      total: this.practice ? 1 : show.plan.length,
    };
    round.ctx = this.makeCtx(round);
    this.round = round;
    try {
      round.state = game.server?.init?.(round.ctx);
    } catch (e) {
      this.log.warn('game init failed', { game: game.id, err: String(e) });
    }
    round.announced = true;
    this.log.info('round', { game: game.id, eliminate, players: participants.length });
    this.broadcast({ t: 'round', ...this.roundInfo(false) });
    this.sendLobby();
  }

  private makeCtx(r: Round): GameServerCtx {
    return {
      rng: r.rng,
      participants: r.participants,
      now: () => Date.now(),
      roundTime: () => (Date.now() - r.startAt) / 1000,
      active: () => r.participants.filter((id) => this.players.has(id) && this.inPlay(r, id)),
      emit: (n, d, by = null) => {
        r.events.push([n, d, by]);
        if (r.announced && this.round === r) this.broadcast({ t: 'ev', n, d, by });
      },
      score: (id) => r.scores.get(id) ?? 0,
      setScore: (id, v) => {
        r.scores.set(id, v);
        if (r.announced && this.round === r) this.broadcast({ t: 'scores', s: [[id, v]] });
      },
      position: (id) => this.players.get(id)?.state?.p ?? null,
    };
  }

  private roundInfo(late: boolean): RoundInfo {
    const r = this.round!;
    return {
      game: r.game.id,
      eliminate: r.eliminate,
      qualify: r.qualify,
      participants: r.participants,
      startAt: r.startAt,
      endAt: r.endAt,
      seed: r.seed,
      index: r.index,
      total: r.total,
      practice: this.practice,
      late,
      finished: r.finished,
      out: r.out,
      events: r.events,
      scores: [...r.scores],
    };
  }

  private view(r: Round): RoundView {
    return {
      participants: r.participants,
      connected: (id) => this.players.has(id),
      finished: r.finished,
      out: r.out,
      scores: r.scores,
      progress: (id) => this.players.get(id)?.progress ?? Number.NEGATIVE_INFINITY,
      eliminate: r.eliminate,
      qualify: r.qualify,
      timeUp: Date.now() > r.endAt,
      solo: (this.show?.started ?? 1) <= 1,
    };
  }

  private checkRound() {
    const r = this.round;
    if (!r || r.over || this.phase !== 'round') return;
    if (r.rule.isOver(this.view(r))) this.endRound();
  }

  private endRound() {
    const r = this.round!;
    r.over = true;
    const { ranking, winner } = r.rule.outcome(this.view(r), r.rng);
    for (const e of ranking) {
      const p = this.players.get(e.id);
      if (p) p.score += e.points;
    }
    this.log.info('round over', { game: r.game.id, ranking: ranking.map((e) => [e.id, e.ok]) });
    if (this.practice) {
      this.phase = 'results';
      this.broadcast({ t: 'roundEnd', game: r.game.id, ranking, practice: true });
      this.sendLobby();
      this.later(3500, () => this.nextRound());
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
    this.clearTimer();
    if (this.round) this.round.over = true;
    this.phase = 'winner';
    const p = this.players.get(id);
    if (p) p.crowns++;
    this.log.info('winner', { id, name: p?.name });
    this.broadcast({ t: 'winner', id, name: p?.name ?? '???' });
    this.sendLobby();
    this.later(WINNER_MS, () => this.backToLobby());
  }

  private backToLobby() {
    this.clearTimer();
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
      p.progress = Number.NEGATIVE_INFINITY;
      p.state = null;
    }
    this.updateHost();
    this.sendLobby();
  }

  private tick() {
    const now = Date.now();
    for (const p of [...this.players.values()]) {
      if (!p.bot && !p.conn && p.disconnectedAt && now - p.disconnectedAt > RECONNECT_GRACE_MS) {
        this.log.info('player timed out', { id: p.id });
        this.removePlayer(p);
      }
    }
    const r = this.round;
    if (r && !r.over && this.phase === 'round' && r.game.server?.tick) {
      try {
        r.game.server.tick(r.ctx, r.state);
      } catch (e) {
        this.log.warn('game tick failed', { game: r.game.id, err: String(e) });
      }
    }
    this.checkRound();
    const l: Snapshot[] = [];
    for (const id of this.snapshotIds()) {
      const s = this.players.get(id)?.state;
      if (s) l.push([id, s.p[0], s.p[1], s.p[2], s.r, s.a]);
    }
    if (l.length) this.broadcast({ t: 'S', s: now, l });
  }

  private snapshotIds(): number[] {
    if (this.phase === 'lobby') return [...this.players.keys()];
    const r = this.round;
    if (!r || this.phase === 'winner') return [];
    return r.participants.filter((id) => this.players.has(id) && this.inPlay(r, id));
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
          owner: p.owner,
          connected: p.bot ? p.owner !== null : !!p.conn,
        }),
      ),
    };
  }

  private sendLobby() {
    this.broadcast(this.lobbyMsg());
  }

  private sendTo(p: Player, msg: ServerMsg) {
    const target = p.bot ? (p.owner !== null ? this.players.get(p.owner) : undefined) : p;
    if (!target?.conn) return;
    try {
      target.conn.send(msg);
    } catch (e) {
      this.log.warn('send failed', { id: target.id, err: String(e) });
    }
  }

  private broadcast(msg: ServerMsg) {
    for (const p of this.players.values()) {
      if (!p.conn) continue;
      try {
        p.conn.send(msg);
      } catch (e) {
        this.log.warn('send failed', { id: p.id, err: String(e) });
      }
    }
  }
}
