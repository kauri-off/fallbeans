import { expect, type Page, test } from '@playwright/test';

interface Probe {
  id: number | null;
  pos: [number, number, number] | null;
  arena: string | null;
  kind: string | null;
  transport: 'wt' | 'ws' | null;
  corrections: number;
  drawCalls: number;
}

test.beforeEach(({ page }) => {
  // Surface browser-side problems in the test output (CI has no screen to look at).
  page.on('pageerror', (e) => console.log(`[pageerror] ${e.stack ?? e}`));
  page.on('console', (m) => {
    if (m.type() === 'error') console.log(`[console] ${m.text()} ${m.location().url}`);
  });
});

const state = (page: Page) => page.evaluate(() => (window as unknown as { __fallbeans: { state(): Probe } }).__fallbeans.state());

async function login(page: Page) {
  await page.goto('/fallbeans/');
  await expect(page).toHaveURL(/\/fallbeans\/pin\//);
  await page.getByLabel('Цифра 1').pressSequentially('5050');
  await expect(page).toHaveURL(/\/fallbeans\/$/);
}

async function hold(page: Page, key: string, ms: number) {
  await page.keyboard.down(key);
  await page.waitForTimeout(ms);
  await page.keyboard.up(key);
}

test('PIN gate, lobby, movement and transport', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  // A wrong PIN is refused.
  await page.goto('/fallbeans/pin/');
  await page.getByLabel('Цифра 1').pressSequentially('1234');
  await expect(page.getByText('Неверный PIN-код')).toBeVisible();

  await login(page);
  await expect(page.getByText('Игроки 1/8')).toBeVisible({ timeout: 20_000 });
  await expect.poll(async () => (await state(page)).kind).toBe('lobby');
  const probe = await state(page);
  console.log('transport:', probe.transport);

  await page.getByRole('button', { name: 'Побегать' }).click();
  const before = (await state(page)).pos!;
  await hold(page, 'KeyW', 1200);
  await page.waitForTimeout(300);
  const after = (await state(page)).pos!;
  expect(Math.hypot(after[0] - before[0], after[2] - before[2])).toBeGreaterThan(2);
  await page.screenshot({ path: 'test-results/lobby.png' });

  // The PIN is remembered: a new page goes straight in.
  const again = await page.context().newPage();
  await again.goto('/fallbeans/');
  await expect(again).toHaveURL(/\/fallbeans\/$/);
  await again.close();
  expect(errors).toEqual([]);
});

const MAPS = [
  'door-dash',
  'hammer-swing',
  'ball-hill',
  'hidden-bridge',
  'drum-roll',
  'jump-club',
  'roll-out',
  'wall-rush',
  'fruit-memory',
  'tail-tag',
  'hex-a-gone',
  'crown-peak',
  'plate-drop',
];

test('every map loads and plays in practice', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  page.on('console', (m) => {
    if (m.type() === 'error' && !m.text().includes('WebTransport') && !m.text().includes('QUIC')) errors.push(m.text());
  });
  await login(page);
  for (const id of MAPS) {
    await page.goto(`/fallbeans/?practice=${id}`);
    await expect.poll(async () => (await state(page)).arena, { timeout: 20_000 }).toBe(id);
    // Intro (7 s), then run forward for a bit.
    await page.waitForTimeout(7600);
    const before = (await state(page)).pos;
    await hold(page, 'KeyW', 1000);
    const after = (await state(page)).pos;
    await page.screenshot({ path: `test-results/map-${id}.png` });
    if (before && after) expect(Math.hypot(after[0] - before[0], after[2] - before[2]), id).toBeGreaterThan(0.5);
  }
  expect(errors).toEqual([]);
});
