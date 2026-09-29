import { describe, expect, it } from 'vitest';
import { MAPS } from '../src/games';
import { ServerArena } from '../src/server/arena';
import { TICK_MS } from '../src/shared/consts';
import type { MapModule } from '../src/sim/map';

/** Runs a round of `mod` with only server bots, for up to `seconds` of game time. */
function runBots(mod: MapModule, bots: number, seconds: number, seed = 11) {
  const finished: number[] = [];
  const out: number[] = [];
  const warnings: string[] = [];
  const participants = Array.from({ length: bots }, (_, i) => i + 1);
  const a = new ServerArena({
    id: 1,
    kind: 'round',
    module: mod,
    seed,
    startAt: 0,
    participants,
    now: -500,
    hooks: {
      onFinish: (id) => finished.push(id),
      onKo: (k) => {
        if (k.out) out.push(k.id);
      },
      onEvent: () => {},
      onScore: () => {},
      onSnapshot: () => {},
      warn: (m) => warnings.push(m),
    },
  });
  participants.forEach((id, i) => {
    a.addPawn(id, true, i);
  });
  const firstOutAt: number[] = [];
  for (let t = -500; t < seconds * 1000; t += 50) {
    a.advance(t);
    if (out.length && !firstOutAt.length) firstOutAt.push(t);
    const alive = participants.filter((id) => a.pawns.get(id)?.status === 'play').length;
    if (alive === 0) break;
  }
  a.dispose();
  return { finished, out, warnings, firstOutAt: firstOutAt[0] ?? null, ticks: a.tick, tickMs: TICK_MS };
}

describe('maps with server bots', () => {
  for (const mod of MAPS) {
    const g = mod.meta;
    it(`${g.id}: builds and plays`, () => {
      const r = runBots(mod, 6, g.genre === 'race' ? g.duration : 30);
      expect(r.warnings).toEqual([]);
      if (g.genre === 'race') expect(r.finished.length).toBeGreaterThan(0);
      // Nobody should drop in the first few seconds of an arena game.
      if (r.firstOutAt !== null && g.genre !== 'race') expect(r.firstOutAt).toBeGreaterThan(3000);
    });
  }
});
