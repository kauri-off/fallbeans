import { FINALS, GAMES, getGame, NON_FINALS } from '../games';
import type { GameMeta } from '../shared/game';
import type { Playlist } from '../shared/protocol';
import { pick, type Rng, shuffle } from '../shared/rng';

export function finalistTarget(started: number): number {
  if (started <= 1) return 1;
  return started >= 6 ? 3 : 2;
}

export function nonFinalCount(started: number): number {
  return Math.min(4, Math.max(2, started - finalistTarget(started)));
}

function poolFor(mode: Playlist['mode'], players: number): GameMeta[] {
  const fits = (g: GameMeta) => (g.minPlayers ?? 1) <= players;
  const base = NON_FINALS.filter(fits);
  if (mode === 'races') return base.filter((g) => g.genre === 'race');
  if (mode === 'survival') return base.filter((g) => g.genre !== 'race');
  return base;
}

export function planShow(players: number, pl: Playlist, rng: Rng): string[] {
  let rounds: string[];
  const custom = pl.games.filter((id) => {
    const g = getGame(id);
    return g && g.genre !== 'final' && (g.minPlayers ?? 1) <= players;
  });
  if (pl.mode === 'custom' && custom.length) {
    rounds = custom;
  } else {
    const n = nonFinalCount(players);
    const pool = poolFor(pl.mode === 'custom' ? 'mix' : pl.mode, players);
    const bag: string[] = [];
    rounds = [];
    let lastGenre = '';
    while (rounds.length < n) {
      if (!bag.length)
        bag.push(
          ...shuffle(
            pool.map((g) => g.id),
            rng,
          ),
        );
      const i = Math.max(
        0,
        bag.findIndex((id) => getGame(id)?.genre !== lastGenre),
      );
      const id = bag.splice(i, 1)[0] as string;
      rounds.push(id);
      lastGenre = getGame(id)?.genre ?? '';
    }
  }
  const fin = getGame(pl.final);
  const finals = FINALS.filter((g) => (g.minPlayers ?? 1) <= players);
  rounds.push(fin && fin.genre === 'final' ? fin.id : pick(finals.length ? finals : FINALS, rng).id);
  return rounds;
}

export function eliminationFor(alive: number, remainingNonFinal: number, started: number): number {
  if (remainingNonFinal <= 0) return 0;
  const need = alive - finalistTarget(started);
  if (need <= 0) return 0;
  return Math.min(Math.ceil(need / remainingNonFinal), Math.floor(alive / 2));
}

export function validPlaylist(pl: Playlist): Playlist {
  const ids = new Set(GAMES.map((g) => g.id));
  return {
    mode: pl.mode,
    games: pl.games.filter((id) => ids.has(id) && getGame(id)?.genre !== 'final').slice(0, 8),
    final: pl.final === 'random' || FINALS.some((g) => g.id === pl.final) ? pl.final : 'random',
  };
}
