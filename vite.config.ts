import preact from '@preact/preset-vite';
import { defineConfig } from 'vite';

const server = `http://127.0.0.1:${process.env.FB_HTTP_PORT ?? 7777}`;

export default defineConfig({
  root: 'src/client',
  base: '/fallbeans/',
  publicDir: '../../public',
  plugins: [preact()],
  resolve: { dedupe: ['three'] },
  build: {
    outDir: '../../dist/client',
    emptyOutDir: true,
    sourcemap: true,
    target: 'es2023',
    chunkSizeWarningLimit: 1500,
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
