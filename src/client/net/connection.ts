import { BASE_PATH, PROTOCOL_VERSION } from '../../shared/consts';
import type { ClientMsg, ServerMsg } from '../../shared/protocol';
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
    this.rtt = this.samples.reduce((a, s) => a + s.rtt, 0) / this.samples.length;
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
  token: () => string | null;
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
  private wtFailed = false;

  constructor(private readonly o: ConnectionOptions) {}

  get kind() {
    return this.transport?.kind ?? null;
  }

  async start() {
    this.stopped = false;
    conn.value = { ...conn.value, status: this.retries ? 'reconnecting' : 'connecting' };
    const s = await fetchSession();
    if (s === 'auth') {
      location.replace(`${BASE_PATH}pin/index.html`);
      return;
    }
    if (s === 'error') return this.retry();
    if (s.version !== PROTOCOL_VERSION) {
      conn.value = { ...conn.value, status: 'rejected', message: 'Вышла новая версия игры — обновите страницу' };
      return;
    }
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
    this.send({
      t: 'hello',
      v: PROTOCOL_VERSION,
      name: this.o.name(),
      ticket: s.ticket,
      ...(token ? { token } : {}),
      ...(this.o.practice ? { practice: this.o.practice } : {}),
    });
    this.ping();
    let n = 0;
    this.pingTimer = setInterval(() => {
      if (++n < 6 || n % 4 === 0) this.ping();
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
    if (m.t === 'welcome') {
      this.retries = 0;
      conn.value = { status: 'online', transport: this.transport?.kind ?? null, message: '', ping: Math.round(this.clock.rtt) };
    }
    if (m.t === 'reject') {
      this.stopped = true;
      conn.value = { ...conn.value, status: 'rejected', message: m.msg };
      if (m.reason === 'auth') setTimeout(() => location.replace(`${BASE_PATH}pin/index.html`), 1500);
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
    if (!this.stopped) this.retry();
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
