/**
 * Production build into dist/:
 *   dist/client/                 static site (served by nginx at /fallbeans/)
 *   dist/server/main.js          bundled server (Bun)
 *   dist/server/node_modules/@webtransport-bun/webtransport   native addon (Linux x64 only)
 *   dist/VERSION
 */
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.cwd();
const dist = join(root, 'dist');
const run = (cmd: string[]) => {
  const r = Bun.spawnSync(cmd, { stdout: 'inherit', stderr: 'inherit' });
  if (r.exitCode !== 0) {
    console.error(`[build] failed: ${cmd.join(' ')}`);
    process.exit(1);
  }
};

rmSync(join(dist, 'server'), { recursive: true, force: true });
console.log('[build] client');
run([process.execPath, 'x', 'vite', 'build']);

console.log('[build] server');
const out = await Bun.build({
  // The audit worker is started by the debug API (src/server/debugApi.ts) as ./worker.js.
  entrypoints: ['src/server/main.ts', 'src/audit/worker.ts'],
  outdir: join(dist, 'server'),
  naming: '[name].[ext]',
  target: 'bun',
  minify: false,
  sourcemap: 'linked',
  external: ['@webtransport-bun/webtransport'],
});
if (!out.success) {
  for (const l of out.logs) console.error(l);
  process.exit(1);
}

// The WebTransport native addon: the package with only the Linux x64 prebuild.
const pkg = join(root, 'node_modules', '@webtransport-bun', 'webtransport');
const target = join(dist, 'server', 'node_modules', '@webtransport-bun', 'webtransport');
mkdirSync(join(target, 'prebuilds'), { recursive: true });
for (const f of ['package.json', 'LICENSE', 'dist']) cpSync(join(pkg, f), join(target, f), { recursive: true });
for (const f of readdirSync(join(pkg, 'prebuilds')))
  if (f.includes('linux-x64') || f === 'SHA256SUMS') cpSync(join(pkg, 'prebuilds', f), join(target, 'prebuilds', f));
if (!existsSync(join(target, 'prebuilds', 'webtransport-native.linux-x64-gnu.node'))) {
  console.error('[build] linux-x64 WebTransport prebuild missing');
  process.exit(1);
}

// "v3.0.0+bec7403" (with "-dirty" for uncommitted changes); just the package version without git.
const git = (a: string[]) => {
  const r = Bun.spawnSync(['git', ...a], { stdout: 'pipe', stderr: 'ignore' });
  return r.exitCode === 0 ? r.stdout.toString().trim() : undefined;
};
const commit = git(['rev-parse', '--short', 'HEAD']);
const dirty = commit && git(['status', '--porcelain', '--untracked-files=no']) ? '-dirty' : '';
const sha = `v${(await Bun.file(join(root, 'package.json')).json()).version}${commit ? `+${commit}${dirty}` : ''}`;
writeFileSync(join(dist, 'VERSION'), `${sha} ${new Date().toISOString()}\n`);
console.log(`[build] done: dist/ (${sha})`);
