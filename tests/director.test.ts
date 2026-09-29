import { describe, expect, it } from 'vitest';
import { getGame } from '../src/games';
import { planGame, validPlaylist } from '../src/server/director';
import { DEFAULT_PLAYLIST } from '../src/shared/protocol';
import { mulberry32 } from '../src/shared/rng';

describe('director', () => {
  it.each([3, 5, 7])('plans %i rounds with a finale map last', (rounds) => {
    const rng = mulberry32(42);
    for (let n = 2; n <= 8; n++) {
      const plan = planGame(n, { ...DEFAULT_PLAYLIST, rounds }, rng);
      expect(plan).toHaveLength(rounds);
      expect(getGame(plan.at(-1)!)?.finale).toBe(true);
      expect(new Set(plan).size).toBe(plan.length);
    }
  });

  it('alternates genres where it can', () => {
    const plan = planGame(6, { ...DEFAULT_PLAYLIST, rounds: 5 }, mulberry32(9));
    const genres = plan.slice(0, -1).map((id) => getGame(id)?.genre);
    for (let i = 1; i < genres.length; i++) expect(genres[i]).not.toBe(genres[i - 1]);
  });

  it('respects minimum player counts and playlist modes', () => {
    const rng = mulberry32(1);
    for (let i = 0; i < 50; i++) {
      expect(planGame(1, DEFAULT_PLAYLIST, rng)).not.toContain('tail-tag');
      const races = planGame(5, { ...DEFAULT_PLAYLIST, mode: 'races' }, rng);
      for (const id of races) expect(getGame(id)?.genre).toBe('race');
    }
  });

  it('uses custom playlists as given', () => {
    const plan = planGame(4, { mode: 'custom', games: ['jump-club', 'door-dash', 'crown-peak'], rounds: 5 }, mulberry32(3));
    expect(plan).toEqual(['jump-club', 'door-dash', 'crown-peak']);
  });

  it('sanitizes playlists', () => {
    expect(validPlaylist({ mode: 'custom', games: ['nope', 'hex-a-gone', 'wall-rush'], rounds: 4 })).toEqual({
      mode: 'custom',
      games: ['hex-a-gone', 'wall-rush'],
      rounds: 5,
    });
  });
});
