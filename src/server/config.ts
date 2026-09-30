import { randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export interface Config {
  /** HTTP + WebSocket (behind nginx in production). */
  host: string;
  port: number;
  /** WebTransport over UDP; null disables it. */
  wtPort: number | null;
  certPem: string | null;
  keyPem: string | null;
  /** Serve the client build from this directory (dev, e2e); in production nginx does it. */
  staticDir: string | null;
  solo: boolean;
  dev: boolean;
  /** nginx in front: trust X-Real-IP, set Secure cookies. */
  trustProxy: boolean;
  /** Signs tickets, player identities and the debug cookie (FB_SECRET). */
  secret: Buffer;
  /** Production debug page/API key (FB_DEBUG_KEY); without it the debug API is off outside dev. */
  debugKey: string | null;
}

type Env = Record<string, string | undefined>;

export function loadConfig(argv: string[], env: Env = process.env, root = process.cwd()): Config {
  const val = (k: string) => {
    const i = argv.indexOf(k);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const has = (k: string) => argv.includes(k);
  const dev = has('--dev') || env.FB_DEV === '1';

  const credDir = env.CREDENTIALS_DIRECTORY;
  const certFile = env.FB_CERT ?? (credDir ? join(credDir, 'cert.pem') : dev ? join(root, '.dev', 'cert.pem') : undefined);
  const keyFile = env.FB_KEY ?? (credDir ? join(credDir, 'key.pem') : dev ? join(root, '.dev', 'key.pem') : undefined);
  const certPem = certFile && existsSync(certFile) ? readFileSync(certFile, 'utf8') : null;
  const keyPem = keyFile && existsSync(keyFile) ? readFileSync(keyFile, 'utf8') : null;
  const noWt = has('--no-wt') || env.FB_WT === '0';
  const wtPort = noWt || !certPem || !keyPem ? null : Number(val('--wt-port') ?? env.FB_WT_PORT ?? (dev ? 4433 : 443));

  let secretHex = env.FB_SECRET;
  if (!secretHex && !dev) throw new Error('FB_SECRET is not set (see deploy/remote-install.sh)');
  if (!secretHex) {
    // Development: a stable secret in .dev/ so player identities survive restarts.
    const file = join(root, '.dev', 'secret');
    if (existsSync(file)) secretHex = readFileSync(file, 'utf8').trim();
    else {
      secretHex = randomBytes(32).toString('hex');
      mkdirSync(join(root, '.dev'), { recursive: true });
      writeFileSync(file, secretHex);
    }
  }
  if (!/^[0-9a-f]{64}$/i.test(secretHex)) throw new Error('FB_SECRET must be 64 hex characters');

  return {
    host: val('--host') ?? env.FB_HTTP_HOST ?? (dev ? '0.0.0.0' : '127.0.0.1'),
    port: Number(val('--port') ?? env.FB_HTTP_PORT ?? 7777),
    wtPort,
    certPem,
    keyPem,
    staticDir: val('--static') ?? env.FB_STATIC ?? null,
    solo: has('--solo') || env.FB_SOLO === '1',
    dev,
    trustProxy: env.FB_TRUST_PROXY === '1',
    secret: Buffer.from(secretHex, 'hex'),
    debugKey: env.FB_DEBUG_KEY && env.FB_DEBUG_KEY.length >= 16 ? env.FB_DEBUG_KEY : null,
  };
}
