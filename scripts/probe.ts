/**
 * Drives the game in a headless browser and runs code against the debug probe (window.__fallbeans).
 *   bun run probe [--url http://localhost:5173/fallbeans/] [--headed] [--practice map] [--room id | --home]
 *                 [--shot out.png] [--wait round] [--timeout 60] [--size 1280x720] "<async JS; `p` is the probe>" …
 *
 * Each snippet runs in the page in turn and its result is printed as JSON, e.g.
 *   bun run probe "await p.dev({c:'start', games:['door-dash'], bots:3})" "await p.dev({c:'skipIntro'})" "p.snapshot()"
 * Snippets may also call `await shot('/tmp/a.png')` to save a screenshot at that point.
 * Enters the dev server's permanent room (`dev`) unless --room names another or --home stays at the
 * room list; page errors and console errors are printed as they happen. Needs a running server
 * (bun run dev) or --url of one. Browser: installed Edge on Windows (PW_CHANNEL to change).
 */
import { chromium } from '@playwright/test';
import { DEV_ROOM_ID } from '../src/shared/consts';
import { channel, gpuArgs } from './browser';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const valued = new Set(['--url', '--practice', '--room', '--shot', '--wait', '--timeout', '--size']);
const snippets = args.filter((a, i) => !a.startsWith('--') && !valued.has(args[i - 1] ?? ''));
const base = (opt('--url') ?? 'http://localhost:5173/fallbeans/').replace(/\/?$/, '/');
const timeout = Number(opt('--timeout') ?? 60) * 1000;
// Window size (e.g. 1366x768), to check the UI fits.
const [width = 1280, height = 720] = (opt('--size') ?? '')
  .split('x')
  .map(Number)
  .filter((n) => n > 0);

const AsyncFunction = (async () => {}).constructor as new (...args: string[]) => unknown;
const parses = (body: string) => {
  try {
    new AsyncFunction('p', 'shot', body);
    return true;
  } catch {
    return false;
  }
};

/**
 * The snippet as a function body whose result is the snippet's value: an expression as is; statements
 * separated by ';' give the value of the last one; anything else (loops, blocks) runs as written.
 */
function snippetBody(code: string): string {
  const expr = `return (${code.trim().replace(/;\s*$/, '')});`;
  if (parses(expr)) return expr;
  const cut = code.trimEnd().replace(/;$/, '').lastIndexOf(';');
  const split = `${code.slice(0, cut + 1)} return (${code.slice(cut + 1).replace(/;\s*$/, '')});`;
  if (cut >= 0 && parses(split)) return split;
  return code;
}

const browser = await chromium.launch({
  headless: !args.includes('--headed'),
  ...(channel ? { channel } : {}),
  args: gpuArgs,
});
const page = await browser.newPage({ viewport: { width, height } });
page.on('pageerror', (e) => console.error(`[page error] ${e.message}`));
// Snippets can take screenshots along the way: await shot('/tmp/a.png').
await page.exposeFunction('shot', async (path: string) => {
  await page.screenshot({ path });
  return path;
});
page.on('console', (m) => {
  // D3D shader compiler notes (X4122) are noise.
  if ((m.type() === 'error' || m.type() === 'warning') && !m.text().includes('X4122'))
    console.error(`[console.${m.type()}] ${m.text()}`);
});
try {
  const practice = opt('--practice');
  const home = args.includes('--home');
  await page.goto(`${base}${practice ? `?practice=${practice}` : home ? '' : `?room=${opt('--room') ?? DEV_ROOM_ID}`}`);
  if (home) await page.waitForFunction(() => window.__fallbeans?.rooms().list, null, { timeout: 30_000 });
  else await page.waitForFunction(() => window.__fallbeans?.time().kind, null, { timeout: 30_000 });
  const wait = opt('--wait');
  if (wait) await page.waitForFunction((k) => window.__fallbeans?.time().kind === k, wait, { timeout: 30_000 });
  for (const code of snippets) {
    const body = snippetBody(code);
    const result = await Promise.race([
      page.evaluate(`(async () => { const p = window.__fallbeans; ${body} })()`),
      new Promise((_, reject) => setTimeout(() => reject(new Error(`timed out after ${timeout / 1000} s: ${code}`)), timeout)),
    ]);
    console.log(JSON.stringify(result, null, 1));
  }
  const shot = opt('--shot');
  if (shot) {
    await page.screenshot({ path: shot });
    console.error(`[probe] screenshot: ${shot}`);
  }
} catch (e) {
  console.error(`[probe] ${e instanceof Error ? e.message : String(e)}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
