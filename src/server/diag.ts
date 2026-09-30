import type { ClientReport } from '../shared/debug';
import type { Logger } from './room';

/**
 * Server diagnostics for the debug page and API: recent log lines, client error reports, and
 * process health (memory, CPU, event-loop lag), sampled once a second.
 */

export interface LogLine {
  ts: string;
  level: 'info' | 'warn';
  msg: string;
  data?: Record<string, unknown>;
}

export interface StoredReport extends ClientReport {
  ts: string;
  ip: string;
  count: number;
}

export interface HealthSample {
  /** Seconds since the process started. */
  up: number;
  rssMB: number;
  heapMB: number;
  /** CPU used by the process over the last second, % of one core. */
  cpu: number;
  /** How late a 1 s timer fired (ms): event-loop stalls. */
  lagMs: number;
}

const MAX_LINES = 400;
const MAX_REPORTS = 200;
const MAX_SAMPLES = 300;
/** Client reports: per address, per minute. */
const REPORTS_PER_MIN = 12;

export class Diagnostics {
  readonly lines: LogLine[] = [];
  readonly reports: StoredReport[] = [];
  readonly health: HealthSample[] = [];
  readonly started = Date.now();
  private readonly perIp = new Map<string, number[]>();
  private timer: ReturnType<typeof setInterval> | null = null;

  /** Wraps a logger: every line is also kept here. */
  wrap(log: Logger): Logger {
    const keep = (level: LogLine['level'], msg: string, data?: Record<string, unknown>) => {
      this.lines.push({ ts: new Date().toISOString(), level, msg, ...(data ? { data } : {}) });
      if (this.lines.length > MAX_LINES) this.lines.shift();
    };
    return {
      info: (m, d) => {
        keep('info', m, d);
        log.info(m, d);
      },
      warn: (m, d) => {
        keep('warn', m, d);
        log.warn(m, d);
      },
    };
  }

  /** A client report, if this address has not sent too many. Returns false when rate-limited. */
  addReport(r: ClientReport, ip: string): boolean {
    const now = Date.now();
    const mine = (this.perIp.get(ip) ?? []).filter((t) => t > now - 60_000);
    if (mine.length >= REPORTS_PER_MIN) return false;
    mine.push(now);
    this.perIp.set(ip, mine);
    if (this.perIp.size > 5000) this.perIp.clear();
    const same = this.reports.find((x) => x.msg === r.msg && x.kind === r.kind && x.stack === r.stack);
    if (same) {
      same.count++;
      same.ts = new Date(now).toISOString();
      return true;
    }
    this.reports.push({ ...r, ts: new Date(now).toISOString(), ip, count: 1 });
    if (this.reports.length > MAX_REPORTS) this.reports.shift();
    return true;
  }

  start() {
    if (this.timer) return;
    let cpu = process.cpuUsage();
    let at = performance.now();
    this.timer = setInterval(() => {
      const now = performance.now();
      const used = process.cpuUsage(cpu);
      cpu = process.cpuUsage();
      const m = process.memoryUsage();
      this.health.push({
        up: Math.round((Date.now() - this.started) / 1000),
        rssMB: Math.round(m.rss / 1048576),
        heapMB: Math.round(m.heapUsed / 1048576),
        cpu: Math.round(((used.user + used.system) / 1000 / (now - at)) * 1000) / 10,
        lagMs: Math.max(0, Math.round(now - at - 1000)),
      });
      at = now;
      if (this.health.length > MAX_SAMPLES) this.health.shift();
    }, 1000);
    this.timer.unref?.();
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }
}

/** Simulation cost per second: ticks run, total and worst update time. */
export interface TickBucket {
  /** Unix seconds */
  at: number;
  ticks: number;
  ms: number;
  maxMs: number;
}

export class TickMeter {
  readonly buckets: TickBucket[] = [];
  private cur: TickBucket = { at: 0, ticks: 0, ms: 0, maxMs: 0 };

  add(ticks: number, ms: number) {
    const at = Math.floor(Date.now() / 1000);
    if (at !== this.cur.at) {
      if (this.cur.ticks) {
        this.buckets.push(this.cur);
        if (this.buckets.length > 120) this.buckets.shift();
      }
      this.cur = { at, ticks: 0, ms: 0, maxMs: 0 };
    }
    this.cur.ticks += ticks;
    this.cur.ms += ms;
    this.cur.maxMs = Math.max(this.cur.maxMs, ms);
  }

  /** Average cost of one tick (ms) and the worst update, over the last `seconds`. */
  summary(seconds = 10) {
    const recent = this.buckets.slice(-seconds);
    const ticks = recent.reduce((a, b) => a + b.ticks, 0);
    const ms = recent.reduce((a, b) => a + b.ms, 0);
    return {
      seconds: recent.length,
      ticksPerSec: recent.length ? Math.round(ticks / recent.length) : 0,
      msPerTick: ticks ? Math.round((ms / ticks) * 1000) / 1000 : 0,
      /** Share of one core spent simulating. */
      load: recent.length ? Math.round((ms / (recent.length * 1000)) * 1000) / 10 : 0,
      worstUpdateMs: Math.round(Math.max(0, ...recent.map((b) => b.maxMs)) * 100) / 100,
    };
  }
}
