import { expect, type Page, test } from '@playwright/test';
import { MAPS } from '../src/games';

/**
 * Visual regression: every map from a fixed camera, with a fixed seed, bots frozen in place, game
 * time paused and the scene still (see shotMode). Compared with screenshots saved on this machine
 * (bun run visual:update after an intended change). Local only: GPU, driver and fonts change pixels.
 * Also checks that each map renders without errors, draws something, and builds the same collision
 * geometry as the server (desync reports).
 */

type Probe = NonNullable<Window['__fallbeans']>;
const probe = <T>(page: Page, fn: (p: Probe) => T | Promise<T>) =>
  page.evaluate(`(async () => (${fn.toString()})(window.__fallbeans))()`) as Promise<T>;

test.describe.configure({ mode: 'serial' });

/** One login for all tests (the PIN endpoint allows 5 attempts a minute). */
let cookies: Awaited<ReturnType<import('@playwright/test').BrowserContext['cookies']>> = [];

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('fb_settings', JSON.stringify({ quality: 'high', name: 'Шот' })));
  if (cookies.length) await page.context().addCookies(cookies);
  else {
    await page.goto('/fallbeans/pin/index.html');
    const status = await page.evaluate(
      async () => (await fetch('/fallbeans/api/auth', { method: 'POST', body: JSON.stringify({ pin: '5050' }) })).status,
    );
    expect(status).toBe(200);
    cookies = await page.context().cookies();
  }
  await page.goto('/fallbeans/?shot');
  await page.waitForFunction(() => window.__fallbeans?.time().kind === 'lobby', null, { timeout: 30_000 });
});

for (const mod of MAPS) {
  const id = mod.meta.id;
  test(`map ${id}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    const started = await page.evaluate(async (map) => {
      const p = window.__fallbeans!;
      await p.dev({ c: 'seed', seed: 1234 });
      const r = await p.dev({ c: 'start', games: [map], bots: 3 });
      await p.waitFor(() => p.time().kind === 'round' && p.time().arena === map, 10_000);
      // Paused first: the warps then land on exactly the same tick every run.
      await p.dev({ c: 'rate', k: 0 });
      await p.dev({ c: 'skipIntro' });
      await p.dev({ c: 'warp', ms: 1500 });
      return r;
    }, id);
    expect(started.ok, started.msg).toBe(true);
    // A fixed camera: over the start looking down a race, or above an arena's centre.
    await page.evaluate(() => {
      const p = window.__fallbeans!;
      const w = p.world()!;
      const me = p.body()?.pos ?? [0, 0, 0];
      if (w.finish) p.camera.set([me[0] + 6, me[1] + 9, me[2] - 12], [0, me[1], me[2] + 25]);
      else {
        const v = w.view ?? [0, 0, 0];
        p.camera.set([v[0] + 14, v[1] + 16, v[2] + 18], v);
      }
      p.shot(true);
    });
    await probe(page, (p) => p.frames(45));
    const info = await probe(page, (p) => ({ errors: p.errors().map((e) => e.msg), render: p.render(), scene: p.audit.scene() }));
    expect([...errors, ...info.errors]).toEqual([]);
    expect(info.render.calls).toBeGreaterThan(20);
    expect(info.scene.problems).toEqual([]);
    await expect(page).toHaveScreenshot(`${id}.png`, { maxDiffPixelRatio: 0.015, animations: 'disabled' });
    await probe(page, (p) => p.dev({ c: 'rate', k: 1 }));
    await probe(page, (p) => p.dev({ c: 'lobby' }));
  });
}
