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

test.beforeEach(async ({ page }) => {
  // A fixed, light preset: headless browsers render in software, and an automatic quality switch
  // (shader recompiles) mid-test would stall them for seconds.
  await page.addInitScript(() => localStorage.setItem('fb_settings', JSON.stringify({ quality: 'medium' })));
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
  await expect(page.getByText('Игроки: 1 из 8')).toBeVisible({ timeout: 20_000 });
  await expect.poll(async () => (await state(page)).kind).toBe('lobby');
  const probe = await state(page);
  console.log('transport:', probe.transport);

  // The Esc menu starts open in the lobby; «Продолжить» closes it and the bean runs.
  await expect(page.getByRole('button', { name: /Начать игру/ })).toBeVisible();
  await page.getByRole('button', { name: /Продолжить/ }).click();
  await expect(page.locator('.menu')).toHaveCount(0);
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
  'tail-tag',
  'hex-a-gone',
  'crown-peak',
  'plate-drop',
];

test('every map loads and plays in practice', async ({ page }) => {
  test.setTimeout(240_000);
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  page.on('console', (m) => {
    if (m.type() === 'error' && !m.text().includes('WebTransport') && !m.text().includes('QUIC')) errors.push(m.text());
  });
  await login(page);
  for (const id of MAPS) {
    await page.goto(`/fallbeans/?practice=${id}`);
    await expect.poll(async () => (await state(page)).arena, { timeout: 20_000 }).toBe(id);
    // Intro (6 s, nobody moves), then run forward for a bit.
    const frozen = (await state(page)).pos;
    await hold(page, 'KeyW', 800);
    const still = (await state(page)).pos;
    if (frozen && still) expect(Math.hypot(still[0] - frozen[0], still[2] - frozen[2]), `${id} intro`).toBeLessThan(0.05);
    await page.waitForTimeout(5600);
    const before = (await state(page)).pos;
    await hold(page, 'KeyW', 1000);
    const after = (await state(page)).pos;
    await page.screenshot({ path: `test-results/map-${id}.png` });
    if (before && after) expect(Math.hypot(after[0] - before[0], after[2] - before[2]), id).toBeGreaterThan(0.5);
  }
  expect(errors).toEqual([]);
});
