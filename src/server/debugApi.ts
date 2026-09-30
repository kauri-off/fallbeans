import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { GAMES } from '../games';
import { BASE_PATH, PROTOCOL_VERSION } from '../shared/consts';
import { ClientReportSchema, MAX_REPORT_BYTES } from '../shared/debug';
import { type Auth, DEBUG_COOKIE, debugCookieHeader, readCookie, sameKey } from './auth';
import type { Config } from './config';
import type { Diagnostics } from './diag';
import type { Logger } from './log';
import type { Gateway } from './net/gateway';
import { roomState, roomTrace } from './rooms/roomDebug';

/**
 * Client error reports (POST api/report) and the read-only debug API (GET api/debug/…), used by
 * the /fallbeans/debug/ page and by tools (curl, Claude). Access to the debug API:
 *   dev server: from this machine;
 *   production: the debug cookie, earned once with api/debug/login?key=FB_DEBUG_KEY.
 * Any endpoint takes ?format=text for a compact plain-text answer.
 */

export interface DebugCtx {
  cfg: Config;
  auth: Auth;
  gateway: Gateway;
  diag: Diagnostics;
  log: Logger;
  sameOrigin(req: Request): boolean;
  clientIp: string;
}

const LOOPBACK = new Set(['127.0.0.1', '::1', '::ffff:127.0.0.1']);

function buildStamp(): string {
  // Production: /opt/fallbeans/current/VERSION next to server/main.js.
  for (const p of [join(import.meta.dir, '..', 'VERSION')]) if (existsSync(p)) return readFileSync(p, 'utf8').trim();
  return 'dev';
}
const BUILD = buildStamp();

let auditRunning = false;

/** Audits take seconds of CPU: run them in a worker so the rooms keep simulating. */
function auditInWorker(opts: { maps?: string[]; only?: string[]; quick: boolean }): Promise<unknown> {
  // Source in development; next to the bundled main.js in production (see scripts/build.ts).
  const url = import.meta.url.endsWith('.ts')
    ? new URL('../audit/worker.ts', import.meta.url)
    : new URL('./worker.js', import.meta.url);
  const w = new Worker(url.href);
  return new Promise((resolve, reject) => {
    const done = setTimeout(() => {
      w.terminate();
      reject(new Error('audit timed out'));
    }, 120_000);
    w.onmessage = (e: MessageEvent<{ ok: boolean; report?: unknown; error?: string }>) => {
      clearTimeout(done);
      w.terminate();
      if (e.data.ok) resolve(e.data.report);
      else reject(new Error(e.data.error));
    };
    w.onerror = (e) => {
      clearTimeout(done);
      w.terminate();
      reject(new Error(e.message));
    };
    w.postMessage(opts);
  });
}

/** Compact, readable text for ?format=text (YAML-like, arrays of scalars on one line). */
export function toText(v: unknown, indent = ''): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v !== 'object') return String(v);
  if (Array.isArray(v)) {
    if (!v.length) return '[]';
    if (v.every((x) => x === null || typeof x !== 'object')) return `[${v.join(', ')}]`;
    return v.map((x) => `\n${indent}- ${toText(x, `${indent}  `).trimStart()}`).join('');
  }
  const entries = Object.entries(v as Record<string, unknown>).filter(([, x]) => x !== undefined);
  if (!entries.length) return '{}';
  return entries
    .map(([k, x]) => {
      const t = toText(x, `${indent}  `);
      return `\n${indent}${k}: ${t.startsWith('\n') ? t : t}`;
    })
    .join('');
}

function respond(data: unknown, url: URL, status = 200): Response {
  if (url.searchParams.get('format') === 'text')
    return new Response(`${toText(data).trimStart()}\n`, {
      status,
      headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store' },
    });
  return Response.json(data, { status, headers: { 'Cache-Control': 'no-store' } });
}

export function debugAllowed(req: Request, ctx: DebugCtx): boolean {
  if (ctx.cfg.dev && LOOPBACK.has(ctx.clientIp)) return true;
  if (!ctx.cfg.debugKey) return false;
  return ctx.auth.validDebugCookie(readCookie(req.headers.get('cookie'), DEBUG_COOKIE));
}

/** POST api/report: a browser's error report. */
export async function handleReport(req: Request, ctx: DebugCtx): Promise<Response> {
  const json = (d: unknown, status = 200) => Response.json(d, { status, headers: { 'Cache-Control': 'no-store' } });
  if (!ctx.sameOrigin(req)) return json({ ok: false }, 403);
  const body = await req.text();
  if (body.length > MAX_REPORT_BYTES) return json({ ok: false, error: 'too_large' }, 413);
  let data: unknown;
  try {
    data = JSON.parse(body);
  } catch {
    return json({ ok: false, error: 'json' }, 400);
  }
  const r = ClientReportSchema.safeParse(data);
  if (!r.success) return json({ ok: false, error: r.error.issues[0]?.message ?? 'invalid' }, 400);
  if (!ctx.diag.addReport(r.data, ctx.clientIp)) return json({ ok: false, error: 'rate_limit' }, 429);
  ctx.log.warn('client error', {
    ip: ctx.clientIp,
    kind: r.data.kind,
    msg: r.data.msg,
    at: r.data.stack.split('\n').find((l) => l.includes(':')) ?? '',
    build: r.data.build,
    ctx: r.data.ctx,
  });
  return json({ ok: true });
}

/** GET api/debug/…; null when the path is not a debug endpoint. */
export async function handleDebug(path: string, req: Request, ctx: DebugCtx): Promise<Response | null> {
  const prefix = `${BASE_PATH}api/debug/`;
  if (!path.startsWith(prefix)) return null;
  const url = new URL(req.url);
  const what = path.slice(prefix.length);

  if (what === 'login') {
    const key = url.searchParams.get('key') ?? '';
    const secure = ctx.cfg.trustProxy;
    if (!ctx.auth.allowAttempt(ctx.clientIp)) return new Response('too many attempts', { status: 429 });
    if (!ctx.cfg.debugKey || !sameKey(key, ctx.cfg.debugKey)) {
      ctx.log.warn('bad debug key', { ip: ctx.clientIp });
      return new Response('wrong key', { status: 403 });
    }
    return new Response(null, {
      status: 302,
      headers: { Location: `${BASE_PATH}debug/`, 'Set-Cookie': debugCookieHeader(ctx.auth.issueDebugCookie(), secure) },
    });
  }
  if (!debugAllowed(req, ctx))
    return respond({ ok: false, error: ctx.cfg.dev || ctx.cfg.debugKey ? 'forbidden' : 'debug API is off' }, url, 403);

  const rooms = ctx.gateway.rooms;
  /** ?room= a room id, or its number in the state listing (default: the first room). */
  const roomAt = () => {
    const q = url.searchParams.get('room') ?? '0';
    return ctx.gateway.hub.rooms.get(q) ?? rooms[Number(q)] ?? rooms[0];
  };
  const noRoom = () => respond({ ok: false, error: 'no rooms are open' }, url, 404);
  switch (what) {
    case 'state': {
      const h = ctx.diag.health.at(-1) ?? null;
      return respond(
        {
          server: {
            build: BUILD,
            protocol: PROTOCOL_VERSION,
            dev: ctx.cfg.dev,
            bun: Bun.version,
            pid: process.pid,
            uptime: Math.round((Date.now() - ctx.diag.started) / 1000),
            health: h,
            webtransport: ctx.cfg.wtPort,
            reports: ctx.diag.reports.length,
            warnings: ctx.diag.lines.filter((l) => l.level === 'warn').length,
          },
          rooms: rooms.map((r, i) => ({ room: i, ...roomState(r) })),
        },
        url,
      );
    }
    case 'health':
      return respond({ samples: ctx.diag.health.slice(-Number(url.searchParams.get('n') ?? 120)) }, url);
    case 'logs': {
      const level = url.searchParams.get('level');
      const n = Number(url.searchParams.get('n') ?? 100);
      return respond({ lines: ctx.diag.lines.filter((l) => !level || l.level === level).slice(-n) }, url);
    }
    case 'errors':
      return respond({ reports: ctx.diag.reports.slice().reverse() }, url);
    case 'trace': {
      const id = url.searchParams.get('id');
      const s = Number(url.searchParams.get('s') ?? 10);
      const room = roomAt();
      if (!room) return noRoom();
      return respond(roomTrace(room, id === null ? undefined : Number(id), Math.min(30, Math.max(1, s))), url);
    }
    case 'replay': {
      const which = url.searchParams.get('i') ?? 'current';
      const rec = roomAt()?.debugReplay(which === 'current' ? 'current' : Number(which));
      if (!rec) return respond({ ok: false, error: 'no recording (dev rooms record rounds)' }, url, 404);
      return Response.json(rec, {
        headers: { 'Cache-Control': 'no-store', 'Content-Disposition': `inline; filename="${rec.game}-${rec.seed}.json"` },
      });
    }
    case 'maps':
      return respond({ maps: GAMES }, url);
    case 'audit': {
      const maps = url.searchParams.get('map')?.split(',').filter(Boolean);
      const only = url.searchParams.get('only')?.split(',').filter(Boolean);
      if (auditRunning) return respond({ ok: false, error: 'an audit is already running' }, url, 429);
      auditRunning = true;
      try {
        const report = await auditInWorker({
          ...(maps ? { maps } : {}),
          ...(only ? { only } : {}),
          quick: url.searchParams.get('full') !== '1',
        });
        return respond(report, url);
      } catch (e) {
        return respond({ ok: false, error: String(e) }, url, 500);
      } finally {
        auditRunning = false;
      }
    }
    default:
      return respond(
        {
          ok: false,
          error: 'unknown endpoint',
          endpoints: ['state', 'health', 'logs', 'errors', 'trace', 'replay', 'maps', 'audit'],
        },
        url,
        404,
      );
  }
}
