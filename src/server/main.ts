import { timingSafeEqual } from 'node:crypto';
import { Auth } from './auth';
import { loadConfig } from './config';
import { Gateway } from './gateway';
import { startHttp } from './http';
import { createLogger } from './log';
import { startWebTransport } from './webtransport';

const cfg = loadConfig(process.argv.slice(2));
const log = createLogger(cfg.dev);

const checkPin = async (pin: string) => {
  if (cfg.pinHash) return Bun.password.verify(pin, cfg.pinHash);
  const a = Buffer.from(pin);
  const b = Buffer.from(cfg.pinPlain ?? '');
  return a.length === b.length && timingSafeEqual(a, b);
};
const auth = new Auth(cfg.secret, checkPin);
const gateway = new Gateway(auth, log, { minPlayers: cfg.solo ? 1 : 2 });
const http = startHttp(cfg, auth, gateway, log);

let wt: ReturnType<typeof startWebTransport> | null = null;
if (cfg.wtPort && cfg.certPem && cfg.keyPem) {
  try {
    wt = startWebTransport({ port: cfg.wtPort, certPem: cfg.certPem, keyPem: cfg.keyPem, gateway, log });
  } catch (e) {
    log.warn('WebTransport disabled', { err: String(e) });
  }
}

log.info(`Fall Beans: http://${cfg.host}:${http.port}/fallbeans/`, {
  webtransport: wt ? `udp/${cfg.wtPort}` : 'off',
  static: cfg.staticDir ?? 'nginx',
  solo: cfg.solo,
});

const shutdown = async () => {
  log.info('shutting down');
  http.stop(true);
  await wt?.close().catch(() => {});
  gateway.dispose();
  process.exit(0);
};
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
