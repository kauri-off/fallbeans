/**
 * Traces bots through a round (headless, deterministic): what each step looked like and what happened.
 *   bun run trace <map> [--seed 11] [--bot 1] [--from 0] [--to 60] [--every 12] [--csv out.csv] [--stuck]
 *
 * Every `--every` ticks (default 12 = 0.1 s) one line: time, position, velocity, body state, ground,
 * hazard touched, the input used and the bot's waypoint; journal events (falls, grabs, finish) in
 * between. --stuck plays the round first, finds where bots stood still for 8+ s and traces the two
 * seconds before each such moment (for all bots involved). --csv writes every line to a file.
 */
import { writeFileSync } from 'node:fs';
import { harness, mapOrThrow } from '../src/audit/harness';
import type { Pawn } from '../src/server/rooms/arena';
import { BTN } from '../src/shared/codec';
import { DT } from '../src/shared/consts';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const mapId = args[0];
if (!mapId || mapId.startsWith('--')) {
  console.error('usage: bun run trace <map> [--seed 11] [--bot 1] [--from 0] [--to 60] [--every 12] [--csv file] [--stuck]');
  process.exit(2);
}
const mod = mapOrThrow(mapId);
const seed = Number(opt('--seed') ?? 11);
const every = Math.max(1, Number(opt('--every') ?? 12));
const f2 = (v: number) => v.toFixed(2);

function line(p: Pawn, t: number) {
  const b = p.body;
  const f = p.last;
  const buttons = `${f.buttons & BTN.jump ? 'J' : ''}${f.buttons & BTN.dive ? 'D' : ''}${f.buttons & BTN.grab ? 'G' : ''}`;
  return {
    t: f2(t),
    id: p.id,
    x: f2(b.pos.x),
    y: f2(b.pos.y),
    z: f2(b.pos.z),
    vx: f2(b.vel.x),
    vy: f2(b.vel.y),
    vz: f2(b.vel.z),
    state: b.state,
    ground: b.grounded
      ? b.groundCol
        ? `#${b.groundCol.index}${b.groundCol.tag ? `:${b.groundCol.tag}` : ''}${b.groundCol.isStatic ? '' : '~'}`
        : 'yes'
      : '-',
    hazard: b.hazard ?? '',
    input: `${f.mx},${f.mz}${buttons ? ` ${buttons}` : ''}`,
    wp: p.bot?.mem.wp ?? '',
    status: p.status,
  };
}

type Line = ReturnType<typeof line>;

/** Plays from the start to `to`, collecting lines for `ids` between `from` and `to`. */
function trace(ids: number[] | null, from: number, to: number): { lines: (Line | string)[] } {
  const h = harness(mod, { seed });
  const out: (Line | string)[] = [];
  let seen = 0;
  const who = ids ?? h.ids;
  for (let t = 0; t <= to; t += every * DT) {
    h.runTo(t);
    for (const e of h.arena.journal.slice(seen))
      if (e.t >= from && (e.id === undefined || who.includes(e.id)))
        out.push(`  ${e.t.toFixed(2)} #${e.id ?? '-'} ${e.what} ${e.data ? JSON.stringify(e.data) : ''}`);
    seen = h.arena.journal.length;
    if (t < from) continue;
    for (const id of who) {
      const p = h.arena.pawns.get(id);
      if (p && p.status === 'play') out.push(line(p, h.arena.time));
    }
    if (!h.alive()) break;
  }
  h.dispose();
  return { lines: out };
}

function print(lines: (Line | string)[]) {
  console.log('t      id  x       y      z       vx     vy     vz     state   ground      hazard   input        wp');
  for (const l of lines) {
    if (typeof l === 'string') console.log(l);
    else
      console.log(
        `${l.t.padEnd(6)} ${String(l.id).padStart(2)} ${l.x.padStart(7)} ${l.y.padStart(6)} ${l.z.padStart(7)} ${l.vx.padStart(6)} ${l.vy.padStart(6)} ${l.vz.padStart(6)} ${l.state.padEnd(7)} ${l.ground.padEnd(11)} ${l.hazard.padEnd(8)} ${l.input.padEnd(12)} ${l.wp}`,
      );
  }
}

if (args.includes('--stuck')) {
  // Find bots that stood still for 8 s, then trace the 2 s before they got stuck.
  const h = harness(mod, { seed });
  const last = new Map<number, { x: number; z: number; t: number }>();
  const found: { id: number; t: number; x: number; y: number; z: number }[] = [];
  for (let t = 0; t <= mod.meta.duration; t += 0.5) {
    h.runTo(t);
    for (const id of h.ids) {
      const p = h.arena.pawns.get(id);
      if (p?.status !== 'play') continue;
      const l = last.get(id);
      const b = p.body.pos;
      if (!l || Math.hypot(b.x - l.x, b.z - l.z) > 0.5) last.set(id, { x: b.x, z: b.z, t });
      else if (t - l.t >= 8 && !found.some((f) => f.id === id && Math.abs(f.t - l.t) < 1))
        found.push({ id, t: l.t, x: b.x, y: b.y, z: b.z });
    }
    if (!h.alive()) break;
  }
  h.dispose();
  if (!found.length) console.log(`${mapId} seed ${seed}: no bot stood still for 8 s`);
  for (const f of found.slice(0, 5)) {
    console.log(`\n=== bot ${f.id} still from t=${f.t} at ${f2(f.x)} ${f2(f.y)} ${f2(f.z)} (seed ${seed}) ===`);
    print(trace([f.id], Math.max(0, f.t - 2), f.t + 1.5).lines);
  }
} else {
  const bot = opt('--bot');
  const from = Number(opt('--from') ?? 0);
  const to = Number(opt('--to') ?? Math.min(mod.meta.duration, from + 12));
  const { lines } = trace(bot ? [Number(bot)] : [1], from, to);
  const csv = opt('--csv');
  if (csv) {
    const rows = lines.filter((l): l is Line => typeof l !== 'string');
    const keys = Object.keys(rows[0] ?? {}) as (keyof Line)[];
    writeFileSync(csv, [keys.join(','), ...rows.map((r) => keys.map((k) => JSON.stringify(r[k])).join(','))].join('\n'));
    console.log(`wrote ${rows.length} lines to ${csv}`);
  } else print(lines);
}
