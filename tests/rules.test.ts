import { describe, expect, it } from 'vitest';
import { computeAwards, emptyGameStats } from '../src/server/awards';
import { emptyStats, isRoundOver, placementPoints, type RoundStats, type RoundView, scoreRound } from '../src/shared/rules';

function view(p: Partial<RoundView>): RoundView {
  return {
    genre: 'race',
    participants: [1, 2, 3, 4],
    connected: () => true,
    finished: [],
    out: [],
    scores: new Map(),
    progress: () => 0,
    timeUp: false,
    solo: false,
    ...p,
  };
}

const stats = (m: Record<number, Partial<RoundStats>>) =>
  new Map(Object.entries(m).map(([id, s]) => [Number(id), { ...emptyStats(), ...s }]));

describe('scoring', () => {
  it('spreads placement points evenly from 10 to 0, ties sharing', () => {
    const p = placementPoints([[1], [2, 3], [4]]);
    expect(p.get(1)).toEqual({ place: 1, points: 10 });
    expect(p.get(2)).toEqual({ place: 2, points: 5 });
    expect(p.get(3)).toEqual({ place: 2, points: 5 });
    expect(p.get(4)).toEqual({ place: 4, points: 0 });
  });

  it('ranks races by finish order, then progress', () => {
    const progress = new Map([
      [3, 50],
      [4, 80],
    ]);
    const rows = scoreRound(
      view({ finished: [2, 1], progress: (id) => progress.get(id) ?? 0 }),
      stats({ 2: { finishAt: 40 }, 1: { finishAt: 45 } }),
      new Map(),
    );
    expect(rows.map((r) => r.id)).toEqual([2, 1, 4, 3]);
    expect(rows.map((r) => r.points)).toEqual([10, 7, 3, 0]);
  });

  it('ranks survival by elimination order; survivors share the top', () => {
    const rows = scoreRound(view({ genre: 'survival', out: [4, 3], timeUp: true }), stats({}), new Map());
    expect(rows.find((r) => r.id === 1)?.points).toBe(8);
    expect(rows.find((r) => r.id === 2)?.points).toBe(8);
    expect(rows.find((r) => r.id === 3)?.points).toBe(3);
    expect(rows.find((r) => r.id === 4)?.points).toBe(0);
  });

  it('fines falls and shortcuts (capped), never below a total of 0, and zeroes AFK', () => {
    const rows = scoreRound(
      view({ finished: [1, 2, 3, 4] }),
      stats({ 1: { falls: 2 }, 2: { falls: 9 }, 3: { shortcuts: 1 }, 4: { idle: 60 } }),
      new Map([[4, 3]]),
    );
    const by = new Map(rows.map((r) => [r.id, r]));
    expect(by.get(1)).toMatchObject({ points: 10, penalty: 2, delta: 8 });
    expect(by.get(2)).toMatchObject({ points: 7, penalty: 4, delta: 3 });
    expect(by.get(3)).toMatchObject({ points: 3, penalty: 2, delta: 1 });
    expect(by.get(4)).toMatchObject({ points: 0, afk: true, total: 3 });
    const broke = scoreRound(view({ finished: [2, 1] }), stats({ 1: { falls: 3 } }), new Map([[1, 1]]));
    expect(broke.find((r) => r.id === 1)?.total).toBe(1 + 7 - 3);
  });

  it('does not fine falls in survival (falling already costs the place)', () => {
    const rows = scoreRound(view({ genre: 'survival', out: [1] }), stats({ 1: { falls: 1 } }), new Map());
    expect(rows.find((r) => r.id === 1)?.penalty).toBe(0);
  });

  it('ranks points games by score', () => {
    const rows = scoreRound(
      view({
        genre: 'points',
        scores: new Map([
          [1, 5],
          [2, 20],
          [3, 5],
          [4, 0],
        ]),
      }),
      stats({}),
      new Map(),
    );
    expect(rows.map((r) => [r.id, r.points])).toEqual([
      [2, 10],
      [1, 5],
      [3, 5],
      [4, 0],
    ]);
  });

  it('ends survival when one is left, races when everyone is done', () => {
    expect(isRoundOver(view({ genre: 'survival', out: [1, 2, 3] }))).toBe(true);
    expect(isRoundOver(view({ genre: 'survival', out: [1, 2] }))).toBe(false);
    expect(isRoundOver(view({ finished: [1, 2], out: [] }))).toBe(false);
    expect(isRoundOver(view({ finished: [1, 2, 3, 4] }))).toBe(true);
    expect(isRoundOver(view({ timeUp: true }))).toBe(true);
  });
});

describe('awards', () => {
  it('gives each title to a single clear leader', () => {
    const a = { ...emptyGameStats(), falls: 5, kos: 2, raceRanks: [0, 0] };
    const b = { ...emptyGameStats(), falls: 1, kos: 2, raceRanks: [1, 1], survived: 80 };
    const awards = computeAwards([
      { id: 1, s: a },
      { id: 2, s: b },
    ]);
    const by = new Map(awards.map((x) => [x.key, x.id]));
    expect(by.get('fastest')).toBe(1);
    expect(by.get('clumsy')).toBe(1);
    expect(by.get('survivor')).toBe(2);
    expect(by.has('bully')).toBe(false);
  });
});
