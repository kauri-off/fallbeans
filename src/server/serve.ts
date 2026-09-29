import { MAX_PLAYERS } from '../shared/consts';
import type { ServerMsg } from '../shared/protocol';
import { createLogger } from './log';
import { Room, type Session } from './room';

export interface ServeOptions {
  port: number;
  solo: boolean;
  dev: boolean;
  resolveFile(path: string): Blob | null;
}

const MIME: Record<string, string> = {
  html: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  glb: 'model/gltf-binary',
  png: 'image/png',
  svg: 'image/svg+xml',
  json: 'application/json',
  ico: 'image/x-icon',
  webmanifest: 'application/manifest+json',
};

export function parseArgs(argv: string[]) {
  const val = (k: string) => {
    const i = argv.indexOf(k);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  return {
    port: Number(val('--port') ?? process.env.PORT ?? 7777),
    solo: argv.includes('--solo') || process.env.SOLO === '1',
    dev: argv.includes('--dev'),
  };
}

export function serve(opts: ServeOptions) {
  const log = createLogger(opts.dev);
  const room = new Room({ minPlayers: opts.solo ? 1 : 2, maxPlayers: MAX_PLAYERS, log });
  type Data = { session: Session | null };

  const server = Bun.serve<Data>({
    port: opts.port,
    hostname: '0.0.0.0',
    fetch(req, srv) {
      const url = new URL(req.url);
      if (url.pathname === '/ws') {
        if (srv.upgrade(req, { data: { session: null } })) return;
        return new Response('upgrade failed', { status: 400 });
      }
      if (url.pathname === '/health') return Response.json({ ok: true, players: room.players.size, phase: room.phase });
      if (url.pathname === '/log' && req.method === 'POST' && opts.dev) {
        return req.text().then((t) => {
          log.warn('client error', { report: t.slice(0, 2000) });
          return new Response('ok');
        });
      }
      let path = decodeURIComponent(url.pathname);
      if (path.endsWith('/')) path += 'index.html';
      const f = opts.resolveFile(path) ?? (path.includes('.') ? null : opts.resolveFile('/index.html'));
      if (!f) return new Response('not found', { status: 404 });
      const ext = path.split('.').pop() ?? '';
      const immutable = path.startsWith('/assets/') && /-[\w-]{8,}\./.test(path);
      return new Response(f, {
        headers: {
          'Content-Type': MIME[ext] ?? 'application/octet-stream',
          'Cache-Control': immutable ? 'public, max-age=31536000, immutable' : 'no-cache',
        },
      });
    },
    websocket: {
      idleTimeout: 60,
      open(ws) {
        ws.data.session = room.open({
          send: (msg: ServerMsg) => ws.send(JSON.stringify(msg)),
          close: () => ws.close(),
        });
      },
      message(ws, data) {
        ws.data.session?.message(typeof data === 'string' ? data : new TextDecoder().decode(data));
      },
      close(ws) {
        ws.data.session?.close();
      },
    },
  });
  log.info(`Fall Beans: http://localhost:${server.port}`, { min: opts.solo ? 1 : 2, max: MAX_PLAYERS });
  return { server, room };
}
