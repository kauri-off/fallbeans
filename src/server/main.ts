import { existsSync, statSync } from 'node:fs';
import { join, normalize } from 'node:path';
import { parseArgs, serve } from './serve';

const args = parseArgs(process.argv.slice(2));
const root = join(import.meta.dir, '../../dist/client');
if (!existsSync(root)) console.warn('dist/client не найден — выполните `bun run build:client` или используйте `bun run dev`');

serve({
  ...args,
  resolveFile(path) {
    const full = normalize(join(root, path));
    if (!full.startsWith(root) || !existsSync(full) || !statSync(full).isFile()) return null;
    return Bun.file(full);
  },
});
