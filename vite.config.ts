import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import preact from '@preact/preset-vite';
import { defineConfig } from 'vite';

const server = `http://127.0.0.1:${process.env.FB_HTTP_PORT ?? 7777}`;

/** "v3.0.0+abc1234" from package.json and git (same stamp as dist/VERSION). */
function buildStamp(): string {
  const version = JSON.parse(readFileSync('package.json', 'utf8')).version as string;
  try {
    const sha = execSync('git rev-parse --short HEAD', { stdio: ['ignore', 'pipe', 'ignore'] })
      .toString()
      .trim();
    return `v${version}+${sha}`;
  } catch {
    return `v${version}`;
  }
}

export default defineConfig({
  root: 'src/client',
  base: '/fallbeans/',
  publicDir: '../../public',
  plugins: [preact()],
  // scripts/build.ts passes the build id it gives the server too (see src/server/build.ts).
  define: { __BUILD__: JSON.stringify(process.env.FB_BUILD_ID ?? buildStamp()) },
  resolve: { dedupe: ['three'] },
  build: {
    outDir: '../../dist/client',
    emptyOutDir: true,
    sourcemap: true,
    target: 'es2023',
    chunkSizeWarningLimit: 1500,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('src/client/index.html', import.meta.url)),
        debug: fileURLToPath(new URL('src/client/debug/index.html', import.meta.url)),
      },
    },
  },
  server: {
    port: 5173,
    host: true,
    proxy: {
      '/fallbeans/ws': { target: server.replace('http', 'ws'), ws: true },
      '/fallbeans/api': server,
      '/fallbeans/health': server,
    },
  },
});
