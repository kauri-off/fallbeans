/**
 * Plays a recorded round again (dev servers record every round; see Recording in src/server/arena.ts).
 *   bun run replay [file.json | --server http://127.0.0.1:7777] [--i 0|current] [--room 0] [--save file]
 *                  [--bot id] [--from t] [--to t] [--every 12]
 *
 * Without --bot: checks that the replay ends in exactly the recorded state and prints what happened
 * (falls, grabs, finishes). With --bot: prints that bean's state every `--every` ticks between
 * --from and --to (seconds of round time), like scripts/trace.ts, so a bug a player hit can be
 * stepped through.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import type { Recording } from '../src/server/rooms/arena';
import { replay } from '../src/server/rooms/replay';
import { DT } from '../src/shared/consts';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};

let rec: Recording;
const file = args.find((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--'));
if (file) rec = JSON.parse(readFileSync(file, 'utf8'));
else {
  const server = opt('--server') ?? 'http://127.0.0.1:7777';
  const url = `${server}/fallbeans/api/debug/replay?i=${opt('--i') ?? '0'}&room=${opt('--room') ?? '0'}`;
  const r = await fetch(url);
  if (!r.ok) {
    console.error(`${url}: ${r.status} ${await r.text()}`);
    process.exit(1);
  }
  rec = (await r.json()) as Recording;
}
const save = opt('--save');
if (save) writeFileSync(save, JSON.stringify(rec));

const bot = opt('--bot');
const from = Number(opt('--from') ?? -99);
const to = Number(opt('--to') ?? 1e9);
const every = Number(opt('--every') ?? 12);
const humans = Object.keys(rec.frames);
console.log(
  `${rec.game} · seed ${rec.seed} · ${rec.pawns.length} beans (humans ${humans.join(', ') || '—'}) · ${rec.ops.length} ops · ticks to ${rec.endTick}`,
);
if (bot) console.log('t      x       y      z       vx     vy     vz     state   ground  input');
const t0 = performance.now();
const r = replay(rec, (a) => {
  if (!bot || a.tick % every) return;
  const t = a.tick * DT;
  if (t < from) return;
  if (t > to) return true;
  const p = a.pawns.get(Number(bot));
  if (!p) return;
  const b = p.body;
  const f = (v: number) => v.toFixed(2);
  console.log(
    `${f(t).padEnd(6)} ${f(b.pos.x).padStart(7)} ${f(b.pos.y).padStart(6)} ${f(b.pos.z).padStart(7)} ${f(b.vel.x).padStart(6)} ${f(b.vel.y).padStart(6)} ${f(b.vel.z).padStart(6)} ${b.state.padEnd(7)} ${(b.grounded ? (b.groundCol?.tag ?? 'yes') : '-').padEnd(7)} ${p.last.mx},${p.last.mz},${p.last.buttons}`,
  );
  return undefined;
});
if (!bot) {
  for (const e of r.arena.journal)
    console.log(`  ${e.t.toFixed(2).padStart(7)} #${e.id ?? '-'} ${e.what} ${e.data ? JSON.stringify(e.data) : ''}`);
  console.log(
    `\nreplayed ${r.ticks} ticks in ${Math.round(performance.now() - t0)} ms · state ${r.hash} vs recorded ${rec.hash}: ${r.match ? 'identical ✔' : 'DIFFERENT ✖ (the simulation is not deterministic, or the code changed)'}`,
  );
}
for (const w of r.warnings) console.warn(`warning: ${w}`);
r.arena.dispose();
process.exit(bot || r.match ? 0 : 1);
