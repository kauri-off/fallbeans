import { BASE_PATH, PROTOCOL_VERSION } from '../../shared/consts';
import type { ClientReport } from '../../shared/debug';

/**
 * Collects what went wrong in this page: uncaught errors, rejected promises, console errors and
 * warnings, plus problems the game reports itself (WebGL context loss, failed assets, map mismatch).
 * Everything lands in a ring buffer (read by the debug probe and the F3 overlay); errors are also
 * sent to the server (POST api/report), deduplicated and rate-limited.
 */

export interface LogEntry {
  /** performance.now() */
  at: number;
  level: 'error' | 'warn' | 'info';
  kind: ClientReport['kind'] | 'console';
  msg: string;
  stack?: string;
  count: number;
}

const MAX_LOG = 200;
const MAX_REPORTS = 20;
const REPORT_GAP_MS = 1000;

export const log: LogEntry[] = [];
const sent = new Set<string>();
let reports = 0;
let lastReport = -1e9;
let context: () => Record<string, unknown> = () => ({});

/** Game state attached to reports (arena, phase, transport…). */
export function setReportContext(fn: () => Record<string, unknown>) {
  context = fn;
}

function remember(e: Omit<LogEntry, 'at' | 'count'>): LogEntry {
  const last = log.at(-1);
  if (last && last.msg === e.msg && last.level === e.level) {
    last.count++;
    last.at = performance.now();
    return last;
  }
  const entry = { ...e, at: performance.now(), count: 1 };
  log.push(entry);
  if (log.length > MAX_LOG) log.shift();
  return entry;
}

/** Records a problem; errors are reported to the server too. */
export function report(kind: ClientReport['kind'], msg: string, stack?: string, level: LogEntry['level'] = 'error') {
  const text = msg.slice(0, 500);
  remember({ level, kind, msg: text, ...(stack ? { stack: stack.slice(0, 2000) } : {}) });
  if (level !== 'error') return;
  const key = `${kind}|${text}|${(stack ?? '').slice(0, 200)}`;
  const now = performance.now();
  if (sent.has(key) || reports >= MAX_REPORTS || now - lastReport < REPORT_GAP_MS) return;
  sent.add(key);
  reports++;
  lastReport = now;
  let ctx: Record<string, unknown> = {};
  try {
    ctx = context();
  } catch {}
  const body: ClientReport = {
    kind,
    msg: text,
    stack: (stack ?? '').slice(0, 2000),
    v: PROTOCOL_VERSION,
    build: __BUILD__,
    path: location.pathname + location.search,
    ua: navigator.userAgent.slice(0, 200),
    t: Math.round(now),
    ctx: JSON.parse(JSON.stringify(ctx, (_k, v) => (typeof v === 'number' ? Math.round(v * 1000) / 1000 : v))),
  };
  try {
    void fetch(`${BASE_PATH}api/report`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      keepalive: true,
    }).catch(() => {});
  } catch {}
}

const text = (args: unknown[]) =>
  args
    .map((a) => {
      if (a instanceof Error) return `${a.name}: ${a.message}`;
      if (typeof a === 'string') return a;
      try {
        return JSON.stringify(a);
      } catch {
        return String(a);
      }
    })
    .join(' ');

let installed = false;

export function installCapture() {
  if (installed) return;
  installed = true;
  window.addEventListener('error', (e) => {
    // Resource errors (img, script) have no message; the asset loader reports its own failures.
    if (!e.message) return;
    report('error', e.message, e.error instanceof Error ? e.error.stack : `${e.filename}:${e.lineno}:${e.colno}`);
  });
  window.addEventListener('unhandledrejection', (e) => {
    const r = e.reason;
    report('rejection', r instanceof Error ? `${r.name}: ${r.message}` : String(r), r instanceof Error ? r.stack : undefined);
  });
  const origError = console.error.bind(console);
  const origWarn = console.error === console.warn ? origError : console.warn.bind(console);
  console.error = (...args: unknown[]) => {
    origError(...args);
    const err = args.find((a): a is Error => a instanceof Error);
    report('console', text(args), err?.stack);
  };
  console.warn = (...args: unknown[]) => {
    origWarn(...args);
    remember({ level: 'warn', kind: 'console', msg: text(args).slice(0, 500) });
  };
}

/** Notes something worth seeing in the debug log that is not an error. */
export function info(msg: string) {
  remember({ level: 'info', kind: 'console', msg: msg.slice(0, 500) });
}

// Catch errors from the moment this module loads (main.tsx imports it early).
installCapture();
