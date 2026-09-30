import { getMap } from '../games';
import { type KoInfo, ServerArena } from '../server/rooms/arena';
import { TICK_MS } from '../shared/consts';
import type { Sections } from '../shared/prof';
import type { MapModule } from '../sim/map';

/**
 * Headless rounds for audits, benchmarks and scripts: a server arena with bots only (or pawns
 * driven by the caller), started at a fixed seed, with everything it reports collected.
 */

export interface Fall {
  id: number;
  t: number;
  out: boolean;
  cause: string;
  by: number | null;
  /** Progress (z, or the map's progress()) reached before the fall. */
  progress: number;
  pos: [number, number, number];
}

export interface Harness {
  arena: ServerArena;
  mod: MapModule;
  ids: number[];
  finishes: { id: number; t: number }[];
  falls: Fall[];
  events: [string, unknown][];
  warnings: string[];
  /** Advances to sim time `t` seconds (every tick simulated). */
  runTo(t: number): void;
  alive(): number;
  dispose(): void;
}

export interface HarnessOpts {
  seed?: number;
  /** Number of pawns (ids 1…n). */
  players?: number;
  /** Which of them are bots (default: all). */
  bots?: (id: number) => boolean;
  kind?: 'round' | 'lobby';
  /** CPU profile of the ticks (see shared/prof.ts). */
  prof?: Sections;
  /** Intro before t = 0 (ms). */
  introMs?: number;
}

export function mapOrThrow(id: string): MapModule {
  const m = getMap(id);
  if (!m) throw new Error(`unknown map ${id}`);
  return m;
}

export function harness(mod: MapModule, o: HarnessOpts = {}): Harness {
  const players = o.players ?? 8;
  const ids = Array.from({ length: players }, (_, i) => i + 1);
  const intro = o.introMs ?? 500;
  const finishes: Harness['finishes'] = [];
  const falls: Fall[] = [];
  const events: [string, unknown][] = [];
  const warnings: string[] = [];
  const arena: ServerArena = new ServerArena({
    id: 1,
    kind: o.kind ?? 'round',
    module: mod,
    seed: o.seed ?? 11,
    startAt: intro,
    participants: ids,
    now: 0,
    ...(o.prof ? { prof: o.prof } : {}),
    hooks: {
      onFinish: (id, t) => finishes.push({ id, t }),
      onKo: (k: KoInfo) => {
        const p = arena.pawns.get(k.id);
        const pos = p?.body.pos;
        falls.push({
          id: k.id,
          t: arena.time,
          out: k.out,
          cause: k.cause,
          by: k.by,
          progress: p?.progress ?? 0,
          pos: pos ? [pos.x, pos.y, pos.z] : [0, 0, 0],
        });
      },
      onEvent: (n, d) => events.push([n, d]),
      onScore: () => {},
      onSnapshot: () => {},
      warn: (m, d) => warnings.push(d ? `${m} ${JSON.stringify(d)}` : m),
    },
  });
  ids.forEach((id, i) => {
    arena.addPawn(id, o.bots ? o.bots(id) : true, i);
  });
  return {
    arena,
    mod,
    ids,
    finishes,
    falls,
    events,
    warnings,
    runTo(t: number) {
      arena.advance(intro + t * 1000 + TICK_MS / 2, true);
    },
    alive: () => ids.filter((id) => arena.pawns.get(id)?.status === 'play').length,
    dispose: () => arena.dispose(),
  };
}

/** Runs bots through a round until everyone is done or `seconds` (default: the round length). */
export function playRound(mod: MapModule, o: HarnessOpts & { seconds?: number; step?: number } = {}) {
  const h = harness(mod, o);
  const end = o.seconds ?? mod.meta.duration;
  const step = o.step ?? 0.5;
  const t0 = performance.now();
  for (let t = 0; t <= end; t += step) {
    h.runTo(t);
    if (!h.alive()) break;
  }
  const ms = performance.now() - t0;
  return { ...h, simMs: ms, simSeconds: Math.min(end, Math.max(0, h.arena.time)) };
}

export function median(xs: readonly number[]): number {
  if (!xs.length) return Number.NaN;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m]! : (s[m - 1]! + s[m]!) / 2;
}

export function quantile(xs: readonly number[], q: number): number {
  if (!xs.length) return Number.NaN;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(q * s.length))]!;
}
