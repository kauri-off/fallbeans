import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Room, type Session } from '../src/server/room';
import { PROTOCOL_VERSION } from '../src/shared/consts';
import type { ClientMsg, ServerMsg, ServerMsgOf } from '../src/shared/protocol';

class Client {
  msgs: ServerMsg[] = [];
  closed = false;
  session: Session;
  constructor(room: Room) {
    this.session = room.open({ send: (m) => this.msgs.push(structuredClone(m)), close: () => (this.closed = true) });
  }
  send(m: ClientMsg | Record<string, unknown>) {
    this.session.message(JSON.stringify(m));
  }
  hello(name: string, token?: string) {
    this.send({ t: 'hello', v: PROTOCOL_VERSION, name, token });
    return this;
  }
  last<T extends ServerMsg['t']>(t: T): ServerMsgOf<T> | undefined {
    return this.msgs.filter((m): m is ServerMsgOf<T> => m.t === t).at(-1);
  }
  get id() {
    return this.last('welcome')!.id;
  }
}

let room: Room;
beforeEach(() => {
  vi.useFakeTimers();
  room = new Room({ minPlayers: 2, seed: 7, introMs: 1000 });
});
afterEach(() => {
  room.dispose();
  vi.useRealTimers();
});

describe('room', () => {
  it('assigns host, colors and rejects bad versions', () => {
    const a = new Client(room).hello('Аня');
    const b = new Client(room).hello('Боря');
    const lobby = b.last('lobby')!;
    expect(lobby.host).toBe(a.id);
    expect(new Set(lobby.players.map((p) => p.color)).size).toBe(2);
    const old = new Client(room);
    old.send({ t: 'hello', v: 1, name: 'x' });
    expect(old.last('reject')?.reason).toBe('version');
    expect(old.closed).toBe(true);
  });

  it('ignores malformed messages', () => {
    const a = new Client(room).hello('A');
    a.session.message('not json');
    a.send({ t: 's', p: [Number.NaN, 0, 0], r: 0, a: 0 });
    a.send({ t: 'color', c: '#000000' });
    a.send({ t: 'nonsense' });
    expect(room.players.get(a.id)?.state).toBeNull();
  });

  it('only host starts and only with enough players', () => {
    const a = new Client(room).hello('A');
    a.send({ t: 'start' });
    expect(room.phase).toBe('lobby');
    const b = new Client(room).hello('B');
    b.send({ t: 'start' });
    expect(room.phase).toBe('lobby');
    a.send({ t: 'start' });
    expect(room.phase).toBe('round');
    expect(b.last('round')?.participants.sort()).toEqual([a.id, b.id].sort());
  });

  it('bots count as players and are driven by the host', () => {
    const a = new Client(room).hello('A');
    a.send({ t: 'addBot' });
    const bot = a.last('lobby')!.players.find((p) => p.bot)!;
    expect(bot.owner).toBe(a.id);
    a.send({ t: 'start' });
    expect(room.phase).toBe('round');
    a.send({ t: 's', p: [1, 2, 3], r: 0, a: 0, as: bot.id });
    expect(room.players.get(bot.id)?.state?.p).toEqual([1, 2, 3]);
    const b = new Client(room).hello('B');
    b.send({ t: 's', p: [9, 9, 9], r: 0, a: 0, as: bot.id });
    expect(room.players.get(bot.id)?.state?.p).toEqual([1, 2, 3]);
  });

  it('runs a full show to a winner', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    room.playlist = { mode: 'custom', games: ['jump-club'], final: 'hex-a-gone' };
    a.send({ t: 'start' });
    expect(a.last('round')?.game).toBe('jump-club');
    vi.advanceTimersByTime(1500);
    a.send({ t: 'out' });
    expect(room.phase).toBe('round');
    vi.advanceTimersByTime(80_000);
    expect(room.phase).toBe('results');
    vi.advanceTimersByTime(7000);
    expect(a.last('round')?.game).toBe('hex-a-gone');
    vi.advanceTimersByTime(1500);
    b.send({ t: 'out' });
    expect(room.phase).toBe('winner');
    expect(a.last('winner')?.id).toBe(a.id);
    vi.advanceTimersByTime(12_000);
    expect(room.phase).toBe('lobby');
    expect(a.last('lobby')?.players.find((p) => p.id === a.id)?.crowns).toBe(1);
  });

  it('eliminates in races and validates finish position', () => {
    const cs = ['A', 'B', 'C', 'D'].map((n) => new Client(room).hello(n));
    room.playlist = { mode: 'custom', games: ['door-dash', 'jump-club'], final: 'hex-a-gone' };
    cs[0]!.send({ t: 'start' });
    const r = cs[0]!.last('round')!;
    expect(r.eliminate).toBe(1);
    vi.advanceTimersByTime(1500);
    cs[0]!.send({ t: 'finish' });
    expect(cs[0]!.last('fin')).toBeUndefined();
    for (const c of cs.slice(0, 3)) {
      c.send({ t: 's', p: [0, 4, 171], r: 0, a: 0 });
      c.send({ t: 'finish' });
    }
    expect(room.phase).toBe('results');
    const end = cs[0]!.last('roundEnd')!;
    expect(end.ranking.filter((e) => !e.ok).map((e) => e.id)).toEqual([cs[3]!.id]);
  });

  it('keeps disconnected players during a show and resumes by token', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.send({ t: 'start' });
    const token = b.last('welcome')!.token;
    b.session.close();
    expect(room.players.has(b.id)).toBe(true);
    expect(room.host).toBe(a.id);
    const b2 = new Client(room).hello('B', token);
    expect(b2.last('welcome')).toMatchObject({ id: b.id, resumed: true });
    expect(b2.last('round')?.late).toBe(true);
  });

  it('drops disconnected players after the grace period', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.send({ t: 'start' });
    b.session.close();
    vi.advanceTimersByTime(31_000);
    expect(room.players.has(b.id)).toBe(false);
  });

  it('transfers host and bots when the host leaves the lobby', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    a.send({ t: 'addBot' });
    a.session.close();
    const lobby = b.last('lobby')!;
    expect(lobby.host).toBe(b.id);
    expect(lobby.players.find((p) => p.bot)?.owner).toBe(b.id);
  });

  it('tail tag steals only within range', () => {
    const a = new Client(room).hello('A');
    const b = new Client(room).hello('B');
    room.playlist = { mode: 'custom', games: ['tail-tag'], final: 'random' };
    a.send({ t: 'start' });
    const tails = a.last('round')!.events.find((e) => e[0] === 'tails')![1] as { ids: number[] };
    expect(tails.ids).toHaveLength(1);
    const holder = tails.ids[0] === a.id ? a : b;
    const thief = holder === a ? b : a;
    vi.advanceTimersByTime(1500);
    holder.send({ t: 's', p: [0, 0, 0], r: 0, a: 0 });
    thief.send({ t: 's', p: [10, 0, 0], r: 0, a: 0 });
    thief.send({ t: 'ev', n: 'steal', d: { from: holder.id } });
    expect(thief.last('ev')).toBeUndefined();
    thief.send({ t: 's', p: [1, 0, 0], r: 0, a: 0 });
    thief.send({ t: 'ev', n: 'steal', d: { from: holder.id } });
    expect(thief.last('ev')).toMatchObject({ n: 'tails', d: { ids: [thief.id] } });
  });

  it('practice rooms start immediately with bots and loop', () => {
    const pr = new Room({ minPlayers: 1, practice: { game: 'tail-tag', bots: 0 }, introMs: 100 });
    const a = new Client(pr).hello('A');
    expect(pr.phase).toBe('round');
    expect(a.last('round')?.participants).toHaveLength(2);
    vi.advanceTimersByTime(80_000);
    expect(a.last('roundEnd')?.practice).toBe(true);
    vi.advanceTimersByTime(4000);
    expect(pr.phase).toBe('round');
    pr.dispose();
  });
});
