import { defineConfig, devices } from '@playwright/test';

const PORT = 7790;

export default defineConfig({
  testDir: 'e2e',
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
    launchOptions: { args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] },
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: `bun src/server/main.ts --static dist/client --port ${PORT} --no-wt --pin 5050`,
    url: `http://127.0.0.1:${PORT}/fallbeans/health`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
