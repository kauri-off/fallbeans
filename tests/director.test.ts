import { describe, expect, it } from 'vitest';
import { getGame } from '../src/games';
import { eliminationFor, finalistTarget, nonFinalCount, planShow, validPlaylist } from '../src/server/director';
import { DEFAULT_PLAYLIST } from '../src/shared/protocol';
import { mulberry32 } from '../src/shared/rng';

function simulate(started: number) {
  let alive = started;
  const n = nonFinalCount(started);
  for (let i = 0; i < n; i++) alive -= eliminationFor(alive, n - i, started);
  return alive;
}

describe('director', () => {
  it.each([1, 2, 3, 4, 5, 6, 7, 8])('show for %i players reaches the finalist target', (n) => {
    expect(simulate(n)).toBe(Math.max(1, Math.min(n, finalistTarget(n))));
  });

  it('never eliminates more than half the players in a round', () => {
    for (let alive = 2; alive <= 8; alive++)
      for (let k = 1; k <= 4; k++) expect(eliminationFor(alive, k, 8)).toBeLessThanOrEqual(alive / 2);
  });

  it('plans non-final rounds followed by exactly one final', () => {
    const rng = mulberry32(42);
    for (let n = 1; n <= 8; n++) {
      const plan = planShow(n, DEFAULT_PLAYLIST, rng);
      expect(plan).toHaveLength(nonFinalCount(n) + 1);
      plan.forEach((id, i) => {
        expect(getGame(id)?.genre === 'final').toBe(i === plan.length - 1);
      });
    }
  });

  it('respects minimum player counts and playlist modes', () => {
    const rng = mulberry32(1);
    for (let i = 0; i < 50; i++) {
      expect(planShow(1, DEFAULT_PLAYLIST, rng)).not.toContain('tail-tag');
      const races = planShow(5, { ...DEFAULT_PLAYLIST, mode: 'races' }, rng);
      for (const id of races.slice(0, -1)) expect(getGame(id)?.genre).toBe('race');
    }
  });

  it('uses custom playlists and chosen final', () => {
    const plan = planShow(4, { mode: 'custom', games: ['jump-club', 'door-dash'], final: 'crown-peak' }, mulberry32(3));
    expect(plan).toEqual(['jump-club', 'door-dash', 'crown-peak']);
  });

  it('sanitizes playlists', () => {
    expect(validPlaylist({ mode: 'custom', games: ['nope', 'hex-a-gone', 'wall-rush'], final: 'door-dash' })).toEqual({
      mode: 'custom',
      games: ['wall-rush'],
      final: 'random',
    });
  });
});
