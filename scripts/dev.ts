/**
 * Development: the game server (with --dev: PIN 5050, WebTransport on udp/4433 with a self-signed
 * certificate) and Vite with hot reload at http://localhost:5173/fallbeans/.
 *   bun run dev [-- --solo]
 */
import { ensureDevCert } from './dev-cert';

const extra = process.argv.slice(2);
ensureDevCert();

const bun = process.execPath;
const server = Bun.spawn([bun, '--watch', 'src/server/main.ts', '--dev', ...extra], { stdout: 'inherit', stderr: 'inherit' });
const vite = Bun.spawn([bun, 'x', 'vite'], { stdout: 'inherit', stderr: 'inherit' });

const stop = () => {
  server.kill();
  vite.kill();
  process.exit(0);
};
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
await Promise.race([server.exited, vite.exited]);
stop();
