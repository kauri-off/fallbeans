import { defineConfig, devices } from '@playwright/test';
import { channel, gpuArgs } from './scripts/browser';

const PORT = 7790;

export default defineConfig({
  testDir: 'e2e',
  timeout: 120_000,
  retries: 0,
  workers: 1,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
    viewport: { width: 1280, height: 720 },
    ...(channel ? { channel } : {}),
    launchOptions: { args: gpuArgs },
  },
  projects: [
    { name: 'e2e', testIgnore: /visual.spec/, use: { ...devices['Desktop Chrome'], ...(channel ? { channel } : {}) } },
    // Screenshots per map (bun run visual); baselines are this machine's, see e2e/visual.spec.ts.
    {
      name: 'visual',
      testMatch: /visual.spec/,
      use: {
        ...devices['Desktop Chrome'],
        ...(channel ? { channel } : {}),
        viewport: { width: 960, height: 540 },
        deviceScaleFactor: 1,
      },
    },
  ],
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.015 } },
  webServer: {
    // Built client served by the game server itself (no nginx), dev commands on, WebTransport on udp/4434.
    command: `bun src/server/main.ts --dev --static dist/client --host 127.0.0.1 --port ${PORT} --wt-port 4434 --solo`,
    url: `http://127.0.0.1:${PORT}/fallbeans/health`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
