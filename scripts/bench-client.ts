/**
 * Client benchmark: every map at each quality preset in a real browser on this machine's GPU.
 *   bun run bench:client [map…] [--quality medium,high,ultra] [--url http://localhost:5173/fallbeans/]
 *                        [--seconds 4] [--headed] [--ablate]
 *
 * Per map and preset: frame time (p50/p95), CPU time per frame and its biggest sections, GPU time
 * per frame (timer queries, saturated so the numbers are real work) and its biggest passes, draw
 * calls, triangles, JS heap. --ablate adds the on/off experiments (slow). Needs a dev server with
 * dev commands (bun run dev). Results: .bench/client-latest.json.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { chromium } from '@playwright/test';
import { GAMES } from '../src/games';
import { channel, gpuArgs } from './browser';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const valued = new Set(['--quality', '--url', '--seconds']);
const maps = args.filter((a, i) => !a.startsWith('--') && !valued.has(args[i - 1] ?? ''));
const ids = maps.length ? maps : GAMES.map((g) => g.id);
const qualities = (opt('--quality') ?? 'medium,high,ultra').split(',');
const base = (opt('--url') ?? 'http://localhost:5173/fallbeans/').replace(/\/?$/, '/');
const seconds = Number(opt('--seconds') ?? 4);

const browser = await chromium.launch({
  headless: !args.includes('--headed'),
  ...(channel ? { channel } : {}),
  args: gpuArgs,
});
const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
page.on('pageerror', (e) => console.error(`[page error] ${e.message}`));
await page.goto(`${base}pin/index.html`);
await page.evaluate(async (u) => fetch(`${u}api/auth`, { method: 'POST', body: JSON.stringify({ pin: '5050' }) }), base);
await page.goto(base);
await page.waitForFunction(() => window.__fallbeans?.time().kind === 'lobby', null, { timeout: 30_000 });

interface Row {
  map: string;
  quality: string;
  frameP50: number;
  frameP95: number;
  cpu: number;
  cpuTop: string;
  gpu: number | null;
  gpuTop: string;
  calls: number;
  triangles: number;
  heapMB: number | null;
  ablation?: unknown;
}
const rows: Row[] = [];
for (const map of ids) {
  for (const q of qualities) {
    const row = await page.evaluate(
      async ({ map, q, seconds, ablate }) => {
        const p = window.__fallbeans!;
        if (p.time().arena !== map) {
          await p.dev({ c: 'seed', seed: 99 });
          await p.dev({ c: 'start', games: [map], bots: 7 });
          await p.waitFor(() => p.time().kind === 'round' && p.time().arena === map, 15_000);
          await p.dev({ c: 'skipIntro' });
        }
        p.quality(q as 'medium' | 'high' | 'ultra');
        // The camera follows the bean; bots play around it.
        await p.frames(60);
        const passes = await p.profile.passes(Math.round(seconds * 60));
        const r = p.render();
        const cpu = Object.entries(passes.cpu.sections);
        return {
          map,
          quality: q,
          frameP50: r.frame.p50,
          frameP95: r.frame.p95,
          cpu: Math.round(cpu.reduce((s, [, v]) => s + v.perFrame, 0) * 100) / 100,
          cpuTop: cpu
            .slice(0, 2)
            .map(([k, v]) => `${k} ${v.perFrame}`)
            .join(', '),
          gpu: passes.frameGpu,
          gpuTop: (passes.gpu?.labels ?? [])
            .slice(0, 3)
            .map((l) => `${l.label} ${l.share}%`)
            .join(', '),
          calls: r.calls,
          triangles: r.triangles,
          heapMB: p.memory()?.heapMB ?? null,
          ...(ablate ? { ablation: await p.profile.ablate(60) } : {}),
        };
      },
      { map, q, seconds, ablate: args.includes('--ablate') },
    );
    rows.push(row);
    console.error(`  ${map} ${q}: ${row.frameP50} ms frame, gpu ${row.gpu ?? '—'} ms`);
  }
  await page.evaluate(() => window.__fallbeans!.dev({ c: 'lobby' }));
}
await browser.close();

mkdirSync('.bench', { recursive: true });
writeFileSync('.bench/client-latest.json', JSON.stringify({ at: new Date().toISOString(), rows }, null, 1));
console.log(`map            quality  frame p50/p95  cpu ms  gpu ms  calls  ktris  heap  top cpu · top gpu`);
for (const r of rows)
  console.log(
    `${r.map.padEnd(15)} ${r.quality.padEnd(7)} ${`${r.frameP50}/${r.frameP95}`.padStart(13)} ${String(r.cpu).padStart(7)} ${String(r.gpu ?? '—').padStart(7)} ${String(r.calls).padStart(6)} ${String(Math.round(r.triangles / 1000)).padStart(6)} ${String(r.heapMB ?? '—').padStart(5)}  ${r.cpuTop} · ${r.gpuTop}`,
  );
