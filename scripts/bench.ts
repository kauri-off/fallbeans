/**
 * Server benchmark: what a round costs on this machine, map by map, with deterministic seeds.
 *   bun run bench [map…] [--seconds 20] [--profile] [--save-baseline] [--json]
 *
 * Per map (8 bots, after a 3 s warm-up): ms per tick (average, p99 of 0.1 s chunks), where the tick
 * goes (bots, physics, movers…), snapshot size and encode time (bandwidth per player), memory held
 * by a room, and how many such rooms one core can run. Compared with bench/baseline.json (saved
 * with --save-baseline) so regressions show. --profile adds a sampling CPU profile of the hottest
 * functions (bun:jsc).
 */

import { profile } from 'bun:jsc';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { harness, mapOrThrow, quantile } from '../src/audit/harness';
import { MAPS } from '../src/games';
import { encodeSnapshot } from '../src/shared/codec';
import { SNAPSHOT_EVERY, TICK_RATE } from '../src/shared/consts';
import { Sections } from '../src/shared/prof';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const seconds = Number(opt('--seconds') ?? 20);
const only = args.filter((a, i) => !a.startsWith('--') && args[i - 1] !== '--seconds');
const maps = only.length ? only.map((id) => mapOrThrow(id)) : MAPS;
const json = args.includes('--json');
const r3 = (v: number) => Math.round(v * 1000) / 1000;

interface MapBench {
  map: string;
  buildMs: number;
  msPerTick: number;
  p99ChunkMsPerTick: number;
  sections: Record<string, number>;
  snapshotBytes: number;
  snapshotEncodeUs: number;
  kbpsPerPlayer: number;
  heapMB: number;
  roomsPerCore: number;
}

function benchMap(id: string): MapBench {
  const mod = mapOrThrow(id);
  Bun.gc(true);
  const heap0 = process.memoryUsage().heapUsed;
  const tb = performance.now();
  const prof = new Sections(1e9);
  const h = harness(mod, { seed: 11, prof });
  const buildMs = performance.now() - tb;
  h.runTo(3);
  prof.reset();
  const tick0 = h.arena.tick;
  const chunks: number[] = [];
  const t0 = performance.now();
  for (let t = 3.1; t <= 3 + seconds; t += 0.1) {
    const k = h.arena.tick;
    const c0 = performance.now();
    h.runTo(t);
    const ticks = h.arena.tick - k;
    if (ticks) chunks.push((performance.now() - c0) / ticks);
  }
  const ticks = h.arena.tick - tick0;
  const msPerTick = (performance.now() - t0) / Math.max(1, ticks);
  const sections = Object.fromEntries(Object.entries(prof.report().sections).map(([k, v]) => [k, r3(v.perFrame)]));
  // Snapshots: one per player every SNAPSHOT_EVERY ticks.
  let bytes = 0;
  const e0 = performance.now();
  const reps = 200;
  for (let i = 0; i < reps; i++) bytes = encodeSnapshot(h.arena.snapshotFor(h.ids[i % h.ids.length]!)).byteLength;
  const encodeUs = ((performance.now() - e0) / reps) * 1000;
  Bun.gc(true);
  const heapMB = (process.memoryUsage().heapUsed - heap0) / 1048576;
  h.dispose();
  return {
    map: id,
    buildMs: r3(buildMs),
    msPerTick: r3(msPerTick),
    p99ChunkMsPerTick: r3(quantile(chunks, 0.99)),
    sections,
    snapshotBytes: bytes,
    snapshotEncodeUs: r3(encodeUs),
    kbpsPerPlayer: r3((bytes * 8 * (TICK_RATE / SNAPSHOT_EVERY)) / 1000),
    heapMB: r3(heapMB),
    // One core, 120 ticks per second, half of it kept free for the rest of the server.
    roomsPerCore: Math.floor(500 / (msPerTick * TICK_RATE)),
  };
}

const run = () =>
  maps.map((m) => {
    const r = benchMap(m.meta.id);
    if (!json) process.stderr.write(`  ${r.map} ${r.msPerTick} ms/tick\n`);
    return r;
  });
let results: MapBench[];
let hot: string | null = null;
if (args.includes('--profile')) {
  const p = profile(() => {
    results = run();
  }, 200);
  hot = String(p.functions).split('\n').slice(0, 40).join('\n');
} else results = run();
results = results!;

const report = {
  at: new Date().toISOString(),
  bun: Bun.version,
  platform: `${process.platform} ${process.arch}`,
  seconds,
  results,
};
mkdirSync('.bench', { recursive: true });
writeFileSync('.bench/latest.json', JSON.stringify(report, null, 1));

const baseFile = 'bench/baseline.json';
const base: typeof report | null = existsSync(baseFile) ? JSON.parse(readFileSync(baseFile, 'utf8')) : null;
if (args.includes('--save-baseline')) {
  mkdirSync('bench', { recursive: true });
  writeFileSync(baseFile, JSON.stringify(report, null, 1));
}

if (json) {
  console.log(JSON.stringify({ ...report, hot }, null, 1));
} else {
  const pct = (a: number, b: number | undefined) => (b ? `${a > b ? '+' : ''}${Math.round(((a - b) / b) * 100)}%` : '');
  console.log(`Server benchmark · ${report.platform} · Bun ${report.bun} · ${seconds} s per map, 8 bots\n`);
  console.log('map             ms/tick  p99     vs base  rooms/core  snap B  kbit/s  heap MB  top sections (ms/tick)');
  for (const r of results) {
    const b = base?.results.find((x) => x.map === r.map);
    const top = Object.entries(r.sections)
      .slice(0, 3)
      .map(([k, v]) => `${k} ${v}`)
      .join(', ');
    console.log(
      `${r.map.padEnd(15)} ${String(r.msPerTick).padStart(7)} ${String(r.p99ChunkMsPerTick).padStart(6)} ${pct(r.msPerTick, b?.msPerTick).padStart(8)} ${String(r.roomsPerCore).padStart(11)} ${String(r.snapshotBytes).padStart(7)} ${String(r.kbpsPerPlayer).padStart(7)} ${String(r.heapMB).padStart(8)}  ${top}`,
    );
  }
  const slower = results.filter((r) => {
    const b = base?.results.find((x) => x.map === r.map);
    return b && r.msPerTick > b.msPerTick * 1.25 + 0.01;
  });
  if (base)
    console.log(
      `\nbaseline ${base.at}: ${slower.length ? `slower by >25%: ${slower.map((r) => r.map).join(', ')}` : 'no regressions'}`,
    );
  if (hot) console.log(`\nHottest functions (samples):\n${hot}`);
}
