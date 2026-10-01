import { PROTOCOL_VERSION } from '../../shared/consts';

/** How often a page waiting out an update asks whether the new version is up (ms). */
const UPDATE_POLL_MS = 1500;
/** sessionStorage: the build this page last reloaded for (no reload loop if it did not take). */
const RELOADED_FOR = 'fb_reloaded_for';

import type { ClientMsg, Hello, ServerMsg } from '../../shared/protocol';
import { conn } from '../state';
import { connect, fetchSession, type Transport } from './transport';

/**
 * Server clock estimate: the offset from the lowest-latency ping of the last few. Game time on the
 * server normally runs at the real rate; dev commands may slow, pause or warp it (see `setRate`).
 */
export class Clock {
  offset = 0;
  rtt = 80;
  /** Game time speed (dev: slow motion, 0 = paused). */
  rate = 1;
  /** Real time at which the current rate started, and the offset then. */
  private rateAt = 0;
  private samples: { rtt: number; offset: number }[] = [];
  private synced = false;

  sample(clientSent: number, serverTime: number) {
    const now = performance.now();
    const rtt = Math.max(0, now - clientSent);
    // Offset as it would be at rateAt: serverNow() = now + offset + (now - rateAt) · (rate − 1).
    const offset = serverTime + (rtt / 2) * this.rate - now - (now - this.rateAt) * (this.rate - 1);
    this.samples.push({ rtt, offset });
    if (this.samples.length > 12) this.samples.shift();
    const best = this.samples.reduce((a, s) => (s.rtt < a.rtt ? s : a));
    // The median: a pong handled late by a busy page (a map being built) is one sample, not a
    // spike in everybody's ping for the next half minute as with the mean.
    const sorted = this.samples.map((s) => s.rtt).sort((a, b) => a - b);
    const mid = sorted.length >> 1;
    this.rtt = sorted.length % 2 ? sorted[mid]! : (sorted[mid - 1]! + sorted[mid]!) / 2;
    if (!this.synced) {
      this.offset = best.offset;
      this.synced = true;
    } else this.offset += (best.offset - this.offset) * 0.15;
  }

  /** The server changed the speed of game time or jumped it (`serverTime` = its clock when it did). */
  setRate(rate: number, serverTime: number) {
    const now = performance.now();
    this.rate = rate;
    this.rateAt = now;
    this.offset = serverTime + (this.rtt / 2) * rate - now;
    // Older samples were taken at another rate or before a jump.
    this.samples = [];
  }

  serverNow() {
    const now = performance.now();
    return now + this.offset + (now - this.rateAt) * (this.rate - 1);
  }
}

export interface ConnectionOptions {
  name: () => string;
  /** The suit colour and outfit the player picked (kept in the browser). */
  look: () => Pick<Hello, 'color' | 'outfit'>;
  /** The identity token from an earlier `ready`, if the browser kept one. */
  token: () => string | null;
  /** The room to enter right after the hello: the one from the link, or the one the player is in (reconnects). */
  room: () => string | null;
  practice: string | null;
  onMessage(msg: ServerMsg): void;
  onDatagram(data: Uint8Array): void;
  onDisconnect(): void;
}

/** Keeps a session to the server alive: ticket → transport → hello, with reconnects. */
export class Connection {
  readonly clock = new Clock();
  private transport: Transport | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private retries = 0;
  private stopped = false;
  /** The server said the game is being updated: waiting for the new version (see waitUpdate). */
  private updating = false;
  private wtFailed = false;

  constructor(private readonly o: ConnectionOptions) {}

  get kind() {
    return this.transport?.kind ?? null;
  }

  async start() {
    this.stopped = false;
    if (!this.updating) conn.value = { ...conn.value, status: this.retries ? 'reconnecting' : 'connecting' };
    const s = await fetchSession();
    if (s === 'updating') return this.waitUpdate();
    if (s === 'error') return this.updating ? this.waitUpdate() : this.retry();
    // Another version is up: load it (once; if that did not take, ask for a manual reload).
    const otherBuild = !!s.build && s.build !== __BUILD__;
    if (s.version !== PROTOCOL_VERSION || otherBuild) {
      const target = s.build ?? `protocol ${s.version}`;
      let last: string | null = null;
      try {
        last = sessionStorage.getItem(RELOADED_FOR);
        sessionStorage.setItem(RELOADED_FOR, target);
      } catch {}
      if (last !== target) {
        conn.value = { ...conn.value, status: 'updating', message: '' };
        location.reload();
        return;
      }
      conn.value = { ...conn.value, status: 'rejected', message: 'Вышла новая версия игры — обновите страницу' };
      return;
    }
    this.updating = false;
    try {
      this.transport = await connect(
        s,
        {
          control: (m) => this.onControl(m),
          datagram: (d) => this.o.onDatagram(d),
          close: () => this.onClose(),
        },
        this.wtFailed,
      );
    } catch {
      return this.retry();
    }
    const token = this.o.token();
    const room = this.o.practice ? null : this.o.room();
    this.send({
      t: 'hello',
      v: PROTOCOL_VERSION,
      name: this.o.name(),
      ...this.o.look(),
      ticket: s.ticket,
      ...(token ? { token } : {}),
      ...(room ? { room } : {}),
      ...(this.o.practice ? { practice: this.o.practice } : {}),
    });
    this.ping();
    let n = 0;
    // Every 0.5 s at first, then every second (a fresher median, and the clock follows drift sooner).
    this.pingTimer = setInterval(() => {
      if (++n < 6 || n % 2 === 0) this.ping();
    }, 500);
  }

  private ping() {
    this.send({ t: 'ping', c: performance.now(), rtt: Math.round(this.clock.rtt) });
  }

  private onControl(m: ServerMsg) {
    if (m.t === 'pong') {
      this.clock.sample(m.c, m.s);
      conn.value = { ...conn.value, ping: Math.round(this.clock.rtt) };
      return;
    }
    if (m.t === 'ready') {
      this.retries = 0;
      conn.value = { status: 'online', transport: this.transport?.kind ?? null, message: '', ping: Math.round(this.clock.rtt) };
    }
    if (m.t === 'updating') {
      // The connection is about to close: no "connection lost", just wait for the new version.
      this.updating = true;
      conn.value = { ...conn.value, status: 'updating', message: '' };
      return;
    }
    if (m.t === 'reject') {
      this.stopped = true;
      conn.value = { ...conn.value, status: 'rejected', message: m.msg };
    }
    this.o.onMessage(m);
  }

  private onClose() {
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = null;
    // A WebTransport session that dies right away (UDP blocked): fall back to WebSocket.
    if (this.transport?.kind === 'wt' && conn.value.status !== 'online') this.wtFailed = true;
    this.transport = null;
    this.o.onDisconnect();
    if (this.stopped) return;
    if (this.updating) this.waitUpdate();
    else this.retry();
  }

  /** The game is being updated: show it, and ask again shortly (the new version reloads the page). */
  private waitUpdate() {
    if (this.stopped) return;
    this.updating = true;
    conn.value = { ...conn.value, status: 'updating', message: '' };
    setTimeout(() => this.start(), UPDATE_POLL_MS);
  }

  private retry() {
    if (this.stopped) return;
    this.retries++;
    conn.value = { ...conn.value, status: 'reconnecting' };
    setTimeout(() => this.start(), Math.min(10_000, 500 * 2 ** Math.min(5, this.retries)));
  }

  send(msg: ClientMsg) {
    this.transport?.sendControl(msg);
  }

  datagram(data: Uint8Array<ArrayBuffer>) {
    this.transport?.sendDatagram(data);
  }

  stop() {
    this.stopped = true;
    this.transport?.close();
  }
}
