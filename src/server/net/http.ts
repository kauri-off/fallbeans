import { createHash } from 'node:crypto';
import { existsSync, statSync } from 'node:fs';
import { join, normalize, sep } from 'node:path';
import type { Server, ServerWebSocket } from 'bun';
import { BASE_PATH, PROTOCOL_VERSION } from '../../shared/consts';
import type { ServerMsg } from '../../shared/protocol';
import type { Auth } from '../auth';
import type { Config } from '../config';
import { handleDebug, handleReport } from '../debugApi';
import type { Diagnostics } from '../diag';
import type { Logger } from '../log';
import type { Conn } from './conn';
import type { ConnSession, Gateway } from './gateway';

const MIME: Record<string, string> = {
  html: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  glb: 'model/gltf-binary',
  png: 'image/png',
  svg: 'image/svg+xml',
  json: 'application/json',
  ico: 'image/x-icon',
  map: 'application/json',
  webmanifest: 'application/manifest+json',
};

/** Same headers nginx adds in production (deploy/nginx/fallbeans.conf). */
export const SECURITY_HEADERS: Record<string, string> = {
  'Content-Security-Policy':
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' https: wss:; worker-src 'self' blob:; font-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'same-origin',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=(), gamepad=(self), fullscreen=(self)',
  'Cross-Origin-Opener-Policy': 'same-origin',
};

type WsData = { session: ConnSession | null; ip: string };

export function devCertHash(certPem: string): string {
  const b64 = certPem.replace(/-----[^-]+-----/g, '').replace(/\s+/g, '');
  return createHash('sha256').update(Buffer.from(b64, 'base64')).digest('base64');
}

export function startHttp(cfg: Config, auth: Auth, gateway: Gateway, log: Logger, diag: Diagnostics): Server<WsData> {
  const staticRoot = cfg.staticDir ? normalize(join(process.cwd(), cfg.staticDir)) : null;
  const certHashes = cfg.dev && cfg.certPem ? [devCertHash(cfg.certPem)] : [];

  const clientIp = (req: Request, srv: Server<WsData>) =>
    (cfg.trustProxy ? req.headers.get('x-real-ip') : null) ?? srv.requestIP(req)?.address ?? 'unknown';
  const sameOrigin = (req: Request) => {
    const origin = req.headers.get('origin');
    // Development goes through the Vite proxy, which rewrites Host.
    if (!origin || cfg.dev) return true;
    try {
      return new URL(origin).host === (req.headers.get('x-forwarded-host') ?? req.headers.get('host'));
    } catch {
      return false;
    }
  };
  const json = (data: unknown, status = 200, headers: Record<string, string> = {}) =>
    Response.json(data, { status, headers: { 'Cache-Control': 'no-store', ...headers } });

  function serveStatic(path: string): Response {
    const root = staticRoot!;
    const rel = path.slice(BASE_PATH.length);
    let file = normalize(join(root, rel.endsWith('/') || rel === '' ? `${rel}index.html` : rel));
    if (!file.startsWith(root + sep) && file !== root) return new Response('not found', { status: 404 });
    if (!existsSync(file) || !statSync(file).isFile()) {
      if (rel.includes('.')) return new Response('not found', { status: 404 });
      file = join(root, 'index.html');
    }
    const ext = file.split('.').pop() ?? '';
    const immutable = rel.startsWith('assets/') && /-[\w-]{8,}\./.test(rel);
    return new Response(Bun.file(file), {
      headers: {
        ...SECURITY_HEADERS,
        'Content-Type': MIME[ext] ?? 'application/octet-stream',
        'Cache-Control': immutable ? 'public, max-age=31536000, immutable' : 'no-cache',
      },
    });
  }

  const server = Bun.serve<WsData>({
    hostname: cfg.host,
    port: cfg.port,
    async fetch(req, srv) {
      const url = new URL(req.url);
      let path: string;
      try {
        path = decodeURIComponent(url.pathname);
      } catch {
        return new Response('bad request', { status: 400 });
      }
      if (path === `${BASE_PATH}health`) {
        const { hub } = gateway;
        return json({
          ok: true,
          version: PROTOCOL_VERSION,
          rooms: hub.rooms.size,
          players: [...hub.rooms.values()].reduce((n, r) => n + r.players.size, 0),
          practice: hub.practice.size,
        });
      }
      if (path.startsWith(`${BASE_PATH}api/report`) || path.startsWith(`${BASE_PATH}api/debug/`)) {
        const ctx = { cfg, auth, gateway, diag, log, sameOrigin, clientIp: clientIp(req, srv) };
        if (path === `${BASE_PATH}api/report`)
          return req.method === 'POST' ? handleReport(req, ctx) : new Response('method not allowed', { status: 405 });
        const r = await handleDebug(path, req, ctx);
        if (r) return r;
      }
      // What a page needs to open a game connection: a ticket for the hello, and how to reach WebTransport.
      if (path === `${BASE_PATH}api/session`) {
        return json({
          ok: true,
          version: PROTOCOL_VERSION,
          ticket: auth.issueTicket(),
          wt: cfg.wtPort ? { port: cfg.wtPort, hashes: certHashes } : null,
        });
      }
      if (path === `${BASE_PATH}ws`) {
        if (!sameOrigin(req)) return new Response('forbidden', { status: 403 });
        if (srv.upgrade(req, { data: { session: null, ip: clientIp(req, srv) } })) return undefined;
        return new Response('upgrade failed', { status: 400 });
      }
      if (staticRoot) {
        if (path === '/' || path === BASE_PATH.slice(0, -1)) return Response.redirect(BASE_PATH, 302);
        if (path.startsWith(BASE_PATH)) return serveStatic(path);
      }
      return new Response('not found', { status: 404 });
    },
    websocket: {
      idleTimeout: 60,
      maxPayloadLength: 64 * 1024,
      open(ws: ServerWebSocket<WsData>) {
        const conn: Conn = {
          kind: 'ws',
          ip: ws.data.ip,
          send: (msg: ServerMsg) => {
            ws.send(JSON.stringify(msg));
          },
          datagram: (data) => {
            // Unreliable semantics on TCP: drop instead of queueing behind a slow link.
            if (ws.getBufferedAmount() < 256 * 1024) ws.send(data);
          },
          close: () => ws.close(),
        };
        ws.data.session = gateway.open(conn);
      },
      message(ws, data) {
        const s = ws.data.session;
        if (!s) return;
        if (typeof data === 'string') s.control(data);
        else s.datagram(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
      },
      close(ws) {
        ws.data.session?.close();
      },
    },
  });
  return server;
}
