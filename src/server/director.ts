import { GAMES, getGame } from '../games';
import type { GameMeta } from '../shared/game';
import { type Playlist, ROUND_COUNTS } from '../shared/protocol';
import { type Rng, shuffle } from '../shared/rng';

function poolFor(mode: Playlist['mode'], players: number): GameMeta[] {
  const fits = (g: GameMeta) => (g.minPlayers ?? 1) <= players;
  const base = GAMES.filter(fits);
  if (mode === 'races') return base.filter((g) => g.genre === 'race');
  if (mode === 'survival') return base.filter((g) => g.genre !== 'race');
  return base;
}

/**
 * The rounds of one game. Every player plays every round; genres alternate where possible and a
 * big "finale" map closes the game when the pool has one.
 */
export function planGame(players: number, pl: Playlist, rng: Rng): string[] {
  const custom = pl.games.filter((id) => (getGame(id)?.minPlayers ?? 1) <= players && getGame(id));
  if (pl.mode === 'custom' && custom.length) return custom;
  const n = Math.max(1, Math.min(12, pl.rounds));
  const pool = poolFor(pl.mode === 'custom' ? 'mix' : pl.mode, players);
  const finales = pool.filter((g) => g.finale);
  const last = n > 1 && finales.length ? shuffle(finales, rng)[0]!.id : null;
  const bag: string[] = [];
  const rounds: string[] = [];
  let lastGenre = '';
  const want = last ? n - 1 : n;
  while (rounds.length < want) {
    if (!bag.length)
      bag.push(
        ...shuffle(
          pool.map((g) => g.id).filter((id) => id !== last || pool.length === 1),
          rng,
        ),
      );
    const i = Math.max(
      0,
      bag.findIndex((id) => getGame(id)?.genre !== lastGenre && !rounds.includes(id)),
    );
    const id = bag.splice(i, 1)[0] as string;
    rounds.push(id);
    lastGenre = getGame(id)?.genre ?? '';
  }
  if (last) rounds.push(last);
  return rounds;
}

export function validPlaylist(pl: Playlist): Playlist {
  const ids = new Set(GAMES.map((g) => g.id));
  return {
    mode: pl.mode,
    games: pl.games.filter((id) => ids.has(id)).slice(0, 12),
    rounds: (ROUND_COUNTS as readonly number[]).includes(pl.rounds) ? pl.rounds : 5,
  };
}
