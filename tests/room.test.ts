import * as THREE from 'three';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { Auth } from '../src/server/auth';
import { Gateway } from '../src/server/gateway';
import { replay } from '../src/server/replay';
import { type Conn, Room } from '../src/server/room';
import { BTN, decodeSnapshot, encodeInput, type Snapshot } from '../src/shared/codec';
import { PROTOCOL_VERSION, TICK_MS } from '../src/shared/consts';
import type { ServerMsg, ServerMsgOf } from '../src/shared/protocol';
import { mulberry32 } from '../src/shared/rng';
import { checkServerMsg } from '../src/shared/serverSchema';

let now = 0;
const clock = () => now;

class Client implements Conn {
  readonly kind = 'ws' as const;
  readonly ip = '127.0.0.1';
  msgs: ServerMsg[] = [];
  snaps: Snapshot[] = [];
  closed = false;
  id = -1;
  constructor(readonly room: Room) {}
  send(m: ServerMsg) {
    // Everything the server sends must match the protocol schema.
    const bad = checkServerMsg(JSON.parse(JSON.stringify(m)));
    if (bad) throw new Error(`invalid server message ${bad}`);
    this.msgs.push(structuredClone(m));
  }
  datagram(d: Uint8Array) {
    const s = decodeSnapshot(d);
    if (s) this.snaps.push(s);
  }
  close() {
    this.closed = true;
  }
  hello(name: string, token?: string) {
    const id = this.room.join(this, { t: 'hello', v: PROTOCOL_VERSION, name, ticket: 'x', ...(token ? { token } : {}) });
    if (id !== null) this.id = id;
    return this;
  }
  ctl(m: Parameters<Room['control']>[2]) {
    this.room.control(this.id, this, m);
  }
  last<T extends ServerMsg['t']>(t: T): ServerMsgOf<T> | undefined {
    return this.msgs.filter((m): m is ServerMsgOf<T> => m.t === t).at(-1);
  }
  /** Sends inputs for the next `ticks` ticks (as a client running just ahead of the server). */
  input(buttons: number, mz: number, ticks = 8) {
    const a = this.room.arena;
    const frames = Array.from({ length: ticks }, (_, i) => ({ mx: 0, mz, buttons: i === 0 ? buttons : buttons & BTN.grab }));
    this.room.datagram(this.id, this, encodeInput({ arena: a.id, firstTick: a.tick + 1, frames }));
  }
}

function advance(room: Room, ms: number) {
  const end = now + ms;
  while (now < end) {
    now = Math.min(end, now + 16);
    room.update(now);
  }
}

let room: Room;
beforeEach(() => {
  now = 1000;
  room = new Room({ minPlayers: 2, seed: 7, introMs: 1000, clock, autoTick: false });
});
afterEach(() => room.dispose());

describe('room', () => {
  it('assigns host and colors, and puts players into the lobby world', () => {
    const a = new Client(room).hello('Аня');
    const b = new Client(room).hello('Боря');
    const lobby = b.last('lobby')!;
    expect(lobby.host).toBe(a.id);
    expect(new Set(lobby.players.map((p) => p.color)).size).toBe(2);
    expect(b.last('arena')?.kind).toBe('lobby');
    advance(room, 200);
    expect(a.snaps.at(-1)?.own).not.toBeNull();
    expect(a.snaps.at(-1)?.bodies.map((x) => x.id)).toEqual([b.id]);
  });

  it('moves players only through their inputs (server authority)', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    advance(room, 500);
    const pa = room.arena.pawns.get(a.id)!;
    const pb = room.arena.pawns.get(b.id)!;
    // Out of reach of the lobby rotor.
    pa.body.reset(new THREE.Vector3(-3, 0.05, -12));
    pb.body.reset(new THREE.Vector3(3, 0.05, -12.5));
    advance(room, 300);
    const startA = pa.body.pos.clone();
    const startB = pb.body.pos.clone();
    for (let i = 0; i < 6; i++) {
      a.input(0, 127, 12);
      advance(room, 50);
    }
    expect(pa.ack).toBeGreaterThan(room.arena.tick - 8);
    expect(pa.body.pos.distanceTo(startA)).toBeGreaterThan(1);
    expect(pb.body.pos.distanceTo(startB)).toBeLessThan(0.2);
  });

  it('rejects inputs for the wrong arena or too far ahead', () => {
    const a = new Client(room).hello('A');
    const arena = room.arena;
    expect(
      arena.input(a.id, { arena: arena.id + 1, firstTick: arena.tick + 1, frames: [{ mx: 0, mz: 0, buttons: 0 }] }, now),
    ).toBe(false);
    expect(arena.input(a.id, { arena: arena.id, firstTick: arena.tick + 500, frames: [{ mx: 0, mz: 0, buttons: 0 }] }, now)).toBe(
      false,
    );
    expect(arena.input(a.id, { arena: arena.id, firstTick: arena.tick + 1, frames: [{ mx: 0, mz: 0, buttons: 0 }] }, now)).toBe(
      true,
    );
  });

  it('only the host starts, and only with enough players', () => {
    const a = new Client(room).hello('A');
    a.ctl({ t: 'start' });
    expect(room.phase).toBe('lobby');
    const b = new Client(room).hello('B');
    b.ctl({ t: 'start' });
    expect(room.phase).toBe('lobby');
    a.ctl({ t: 'start' });
    expect(room.phase).toBe('round');
    expect(b.last('arena')?.participants.sort()).toEqual([a.id, b.id].sort());
  });

  it('runs a game of points to a podium, freezing beans before each start', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    room.playlist = { mode: 'custom', games: ['jump-club', 'hex-a-gone'], rounds: 5 };
    a.ctl({ t: 'start' });
    expect(a.last('arena')?.game).toBe('jump-club');
    const until = (cond: () => boolean, ms: number, each?: () => void) => {
      for (let t = 0; t < ms && !cond(); t += 50) {
        each?.();
        advance(room, 50);
      }
      expect(cond()).toBe(true);
    };
    // During the intro inputs move nobody.
    advance(room, 200);
    const pb = room.arena.pawns.get(b.id)!;
    const start = pb.body.pos.clone();
    for (let i = 0; i < 10; i++) {
      b.input(BTN.jump, 127, 12);
      advance(room, 50);
    }
    expect(pb.body.pos.distanceTo(start)).toBeLessThan(0.05);
    // B walks off the edge; the server notices, not the client.
    advance(room, 400);
    until(
      () => room.arena.out.includes(b.id),
      5000,
      () => b.input(0, 127, 12),
    );
    expect(a.last('ko')).toMatchObject({ id: b.id, out: true });
    until(() => room.phase === 'results', 5000);
    const rows = a.last('roundEnd')!.rows;
    expect(rows.find((r) => r.id === a.id)?.points).toBe(10);
    expect(rows.find((r) => r.id === b.id)?.points).toBe(0);
    until(() => room.arena.module.meta.id === 'hex-a-gone', 10_000);
    advance(room, 1100);
    until(
      () => room.phase === 'podium',
      20_000,
      () => {
        b.input(0, 127, 12);
        a.input(BTN.jump, 0, 12);
      },
    );
    const end = a.last('gameEnd')!;
    expect(end.standings[0]).toMatchObject({ id: a.id, total: 20 });
    expect(room.arena.kind).toBe('podium');
    advance(room, 21_000);
    expect(room.phase).toBe('lobby');
    expect(a.last('lobby')?.players.find((p) => p.id === a.id)?.crowns).toBe(1);
  });

  it('bots count as players and are simulated on the server', () => {
    const a = new Client(room).hello('A');
    a.ctl({ t: 'addBot' });
    const bot = a.last('lobby')!.players.find((p) => p.bot)!;
    room.playlist = { mode: 'custom', games: ['door-dash'], rounds: 5 };
    a.ctl({ t: 'start' });
    expect(room.phase).toBe('round');
    advance(room, 6000);
    expect(room.arena.pawns.get(bot.id)!.progress).toBeGreaterThan(5);
  });

  it('keeps disconnected players during a game and resumes by token', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.ctl({ t: 'start' });
    const token = b.last('welcome')!.token;
    room.leave(b.id, b);
    expect(room.players.has(b.id)).toBe(true);
    expect(room.host).toBe(a.id);
    const b2 = new Client(room).hello('B', token);
    expect(b2.last('welcome')).toMatchObject({ id: b.id, resumed: true });
    expect(b2.last('arena')?.late).toBe(true);
  });

  it('drops disconnected players after the grace period', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.ctl({ t: 'start' });
    room.leave(b.id, b);
    advance(room, 31_000);
    expect(room.players.has(b.id)).toBe(false);
  });

  it('transfers host when the host leaves the lobby', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.ctl({ t: 'addBot' });
    room.leave(a.id, a);
    expect(b.last('lobby')!.host).toBe(b.id);
  });

  it('lets the host hand the role to another connected player', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.ctl({ t: 'addBot' });
    const bot = a.last('lobby')!.players.find((p) => p.bot)!;
    // Not to a bot, and nobody but the host can do it.
    a.ctl({ t: 'host', id: bot.id });
    b.ctl({ t: 'host', id: b.id });
    expect(b.last('lobby')!.host).toBe(a.id);
    a.ctl({ t: 'host', id: b.id });
    expect(a.last('lobby')!.host).toBe(b.id);
    a.ctl({ t: 'start' });
    expect(room.phase).toBe('lobby');
  });

  it('tail tag steals only by grabbing within reach', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    room.playlist = { mode: 'custom', games: ['tail-tag'], rounds: 5 };
    a.ctl({ t: 'start' });
    advance(room, 1100);
    const pa = room.arena.pawns.get(a.id)!;
    const pb = room.arena.pawns.get(b.id)!;
    // Put them face to face; whoever has no tail grabs.
    pa.body.pos.set(0, 1.6, 0);
    pb.body.pos.set(0, 1.6, 1);
    pa.body.yaw = 0;
    pb.body.yaw = Math.PI;
    expect(a.last('ev')).toBeUndefined();
    a.input(BTN.grab, 0, 4);
    b.input(BTN.grab, 0, 4);
    advance(room, 60);
    const ev = a.last('ev');
    expect(ev?.n).toBe('tails');
  });

  it('practice rooms start immediately with bots and loop', () => {
    const pr = new Room({
      minPlayers: 1,
      practice: { game: 'jump-club', bots: 2 },
      introMs: 100,
      clock,
      autoTick: false,
      seed: 7,
    });
    const a = new Client(pr).hello('A');
    expect(pr.phase).toBe('round');
    expect(a.last('arena')?.participants).toHaveLength(3);
    // Stop right at the first round's end: rounds can be short (everyone falls), so the phase at a fixed time is not stable.
    for (let t = 0; t < 80_000 && !a.last('roundEnd'); t += 100) advance(pr, 100);
    expect(a.last('roundEnd')?.practice).toBe(true);
    const arenas = a.msgs.filter((m) => m.t === 'arena').length;
    advance(pr, 4000);
    expect(a.msgs.filter((m) => m.t === 'arena').length).toBeGreaterThan(arenas);
    pr.dispose();
  });
});

describe('gateway', () => {
  it('requires a valid ticket and protocol version', () => {
    const auth = new Auth(Buffer.alloc(32, 1), async () => true);
    const g = new Gateway(auth, { info() {}, warn() {} }, { minPlayers: 2, clock, autoTick: false });
    const c = new Client(g.main);
    const s = g.open(c);
    s.control(JSON.stringify({ t: 'hello', v: PROTOCOL_VERSION, name: 'x', ticket: 'forged' }));
    expect(c.last('reject')?.reason).toBe('auth');
    const c2 = new Client(g.main);
    g.open(c2).control(JSON.stringify({ t: 'hello', v: 1, name: 'x', ticket: auth.issueTicket() }));
    expect(c2.last('reject')?.reason).toBe('version');
    const c3 = new Client(g.main);
    const s3 = g.open(c3);
    s3.control('not json');
    s3.control(JSON.stringify({ t: 'hello', v: PROTOCOL_VERSION, name: 'ok', ticket: auth.issueTicket() }));
    expect(c3.last('welcome')).toBeDefined();
    const c4 = new Client(g.main);
    g.open(c4).control(
      JSON.stringify({ t: 'hello', v: PROTOCOL_VERSION, name: 'p', ticket: auth.issueTicket(), practice: 'hex-a-gone' }),
    );
    expect(g.practice.size).toBe(1);
    expect(c4.last('arena')?.game).toBe('hex-a-gone');
    g.dispose();
  });
});

describe('timing', () => {
  it('uses a 120 Hz tick', () => {
    expect(TICK_MS).toBeCloseTo(8.333, 2);
  });
});

describe('dev tools', () => {
  it('ignores dev commands unless the server runs with --dev', () => {
    const a = new Client(room).hello('A');
    a.ctl({ t: 'dev', q: 1, cmd: { c: 'skipIntro' } });
    expect(a.last('devAck')).toMatchObject({ q: 1, ok: false });
  });

  it('replays a recorded round to exactly the same state', () => {
    const dev = new Room({ minPlayers: 1, seed: 3, introMs: 1000, clock, autoTick: false, dev: true });
    const a = new Client(dev).hello('A');
    const b = new Client(dev).hello('B');
    const cmd = (c: Client, x: Parameters<Room['devCommand']>[1]) => c.ctl({ t: 'dev', cmd: x });
    cmd(a, { c: 'start', games: ['hammer-swing'], bots: 2 });
    cmd(a, { c: 'skipIntro' });
    const rng = mulberry32(5);
    for (let i = 0; i < 160; i++) {
      for (const c of [a, b]) {
        const ar = dev.arena;
        // Some packets arrive late or not at all, like on a real connection.
        if (rng() < 0.15) continue;
        const first = ar.tick + 1 - (rng() < 0.2 ? 4 : 0);
        const frames = Array.from({ length: 8 }, () => ({
          mx: Math.round(rng() * 254 - 127),
          mz: 100,
          buttons: rng() < 0.05 ? BTN.jump : rng() < 0.03 ? BTN.dive : rng() < 0.1 ? BTN.grab : 0,
        }));
        dev.datagram(c.id, c, encodeInput({ arena: ar.id, firstTick: first, frames }));
      }
      if (i === 40) cmd(a, { c: 'goto', to: 1 });
      if (i === 60) cmd(a, { c: 'knock', id: b.id, v: [2, 5, -3] });
      if (i === 80) cmd(b, { c: 'bot', near: true });
      if (i === 100) cmd(a, { c: 'grab', target: b.id, s: 1 });
      if (i === 120) cmd(b, { c: 'bots', on: false });
      advance(dev, 50);
    }
    cmd(a, { c: 'lobby' });
    const rec = dev.debugReplay(0)!;
    expect(rec.game).toBe('hammer-swing');
    expect(rec.ops.map((o) => o[1])).toEqual(expect.arrayContaining(['teleport', 'knock', 'late', 'grab', 'bots']));
    const r = replay(JSON.parse(JSON.stringify(rec)));
    expect(r.ticks).toBeGreaterThan(900);
    expect(r.hash).toBe(rec.hash);
    r.arena.dispose();
    dev.dispose();
  });
});
