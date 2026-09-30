import { getMap } from '../games';
import { TICK_MS } from '../shared/consts';
import { type ArenaHooks, type Recording, ServerArena } from './arena';

/**
 * Plays a recorded round again (see Recording in arena.ts). The simulation is deterministic, so
 * the replay must end in exactly the recorded state; `each` sees the arena after every tick.
 */
export function replay(rec: Recording, each?: (arena: ServerArena) => boolean | undefined) {
  const mod = getMap(rec.game);
  if (!mod) throw new Error(`unknown map ${rec.game}`);
  const journal: string[] = [];
  const hooks: ArenaHooks = {
    onFinish: () => {},
    onKo: () => {},
    onEvent: () => {},
    onScore: () => {},
    onSnapshot: () => {},
    warn: (m) => journal.push(m),
  };
  const a = new ServerArena({
    id: 1,
    kind: rec.kind,
    module: mod,
    seed: rec.seed,
    startAt: rec.startAt,
    participants: [...rec.participants],
    now: rec.createdAt,
    hooks,
  });
  const first = a.tick;
  for (const [id, bot, spawn, at] of rec.pawns) if (at <= first) a.addPawn(id, bot, spawn ?? undefined);
  const later = rec.pawns.filter(([, , , at]) => at > first);
  // Current frame of each human (run-length decoded).
  const cursor = new Map<number, number>();
  const ops = [...rec.ops];
  let stopped = false;
  for (let k = first + 1; k <= rec.endTick && !stopped; k++) {
    while (ops.length && ops[0]![0] < k) {
      const [, name, args] = ops.shift()!;
      a.applyOp(name, args);
    }
    for (const p of later) if (p[3] === k - 1) a.addPawn(p[0], p[1], p[2] ?? undefined);
    for (const [idS, frames] of Object.entries(rec.frames)) {
      const id = Number(idS);
      let i = cursor.get(id) ?? -1;
      while (i + 1 < frames.length && frames[i + 1]![0] <= k) i++;
      cursor.set(id, i);
      const f = frames[i];
      if (f) a.forceInput(id, k, { mx: f[1], mz: f[2], buttons: f[3] });
    }
    a.advance(rec.startAt + k * TICK_MS + TICK_MS / 4, true);
    if (each?.(a)) stopped = true;
  }
  const hash = a.stateHash();
  return { arena: a, hash, match: !stopped && hash === rec.hash, ticks: a.tick - first, warnings: journal };
}
