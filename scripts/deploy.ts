/**
 * Deploys Fall Beans to https://киберщит-социум.рф/fallbeans/:  bun run deploy -- [options]
 *
 *   --pin <digits>    set (or change) the PIN; changing it signs everyone out. Needed on the first deploy.
 *   --pack-only       build the bundle (.build/fallbeans-update.tar.gz) without uploading
 *   --skip-checks     skip typecheck, lint and tests
 *   --yes             do not ask for confirmation
 *   --host user@host  SSH target (default DEPLOY_HOST or deploy@168.113.157.12)
 *   --key <file>      SSH key (default DEPLOY_KEY or ~/.ssh/cybershield_deploy)
 *
 * Builds locally (the host is small), uploads one bundle and runs deploy/remote-install.sh with sudo:
 * checks first, release switch with health check, rollback on failure, checks from outside.
 * The nginx site itself belongs to the shared server configuration (SharedServer repository),
 * which must be deployed once before this.
 */
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import os from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline/promises';
import { hashPin } from '../src/server/auth';

const args = process.argv.slice(2);
const flag = (n: string) => args.includes(n);
const option = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const fail = (msg: string): never => {
  console.error(`[deploy] ${msg}`);
  process.exit(1);
};
const log = (msg: string) => console.log(`[deploy] ${msg}`);
const run = (cmd: string[], opts: { cwd?: string } = {}) => {
  const r = Bun.spawnSync(cmd, { stdout: 'inherit', stderr: 'inherit', ...(opts.cwd ? { cwd: opts.cwd } : {}) });
  if (r.exitCode !== 0) fail(`failed: ${cmd.join(' ')}`);
};

const root = process.cwd();
const host = option('--host') ?? process.env.DEPLOY_HOST ?? 'deploy@168.113.157.12';
const key = (option('--key') ?? process.env.DEPLOY_KEY ?? '~/.ssh/cybershield_deploy').replace(/^~(?=$|[\\/])/, os.homedir());
const pin = option('--pin');
if (pin !== undefined && !/^\d{4,12}$/.test(pin)) fail('PIN must be 4–12 digits');

const bun = process.execPath;
if (!flag('--skip-checks')) {
  log('checks');
  run([bun, 'x', 'tsc', '--noEmit']);
  run([bun, 'x', 'biome', 'check', '.']);
  run([bun, 'x', 'vitest', 'run']);
}
log('build');
run([bun, 'scripts/build.ts']);

const stage = join(root, '.build');
rmSync(stage, { recursive: true, force: true });
const rel = join(stage, 'release');
const bundle = join(stage, 'bundle');
mkdirSync(join(rel, 'www'), { recursive: true });
mkdirSync(join(bundle, 'nginx'), { recursive: true });
cpSync(join(root, 'dist', 'client'), join(rel, 'www', 'fallbeans'), { recursive: true });
cpSync(join(root, 'dist', 'server'), join(rel, 'server'), { recursive: true });
cpSync(join(root, 'dist', 'VERSION'), join(rel, 'VERSION'));
// Relative paths: GNU tar would read "C:\…" as a remote host.
run(['tar', '-czf', '../bundle/release.tar.gz', '.'], { cwd: rel });
for (const f of ['remote-install.sh', 'fallbeans.service']) cpSync(join(root, 'deploy', f), join(bundle, f));
for (const f of ['fallbeans.http', 'fallbeans.conf', 'fallbeans.headers'])
  cpSync(join(root, 'deploy', 'nginx', f), join(bundle, 'nginx', f));
const archive = join(stage, 'fallbeans-update.tar.gz');
run(['tar', '-czf', '../fallbeans-update.tar.gz', '.'], { cwd: bundle });
log(`bundle: ${archive} (${Math.round(Bun.file(archive).size / 1024)} KB)`);
if (flag('--pack-only')) process.exit(0);

if (!existsSync(key)) fail(`SSH key not found: ${key}`);
if (!flag('--yes')) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const answer = await rl.question(
    `[deploy] Update Fall Beans on ${host} (production)${pin ? ' and set a new PIN' : ''}? [y/N] `,
  );
  rl.close();
  if (!/^y(es)?$/i.test(answer.trim())) fail('cancelled');
}
const ssh = ['-i', key, '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15'];
const dir = `fallbeans-update-${Date.now()}`;
run(['scp', ...ssh, archive, `${host}:${dir}.tar.gz`]);
const env = pin ? `PIN_HASH_B64=${Buffer.from(await hashPin(pin)).toString('base64')} ` : '';
run([
  'ssh',
  ...ssh,
  host,
  `mkdir -p ~/${dir} && tar -xzf ~/${dir}.tar.gz -C ~/${dir} && sudo ${env}bash ~/${dir}/remote-install.sh; code=$?; rm -rf ~/${dir} ~/${dir}.tar.gz; exit $code`,
]);
log('done: https://киберщит-социум.рф/fallbeans/');
