export type Genre = 'race' | 'survival' | 'points';

export const GENRE_LABEL: Record<Genre, string> = {
  race: 'Гонка',
  survival: 'Выживание',
  points: 'Очки',
};

export interface GameMeta {
  id: string;
  title: string;
  genre: Genre;
  desc: string;
  goal: string;
  /** Round length in seconds (60–120). */
  duration: number;
  minPlayers?: number;
  /** Grab (Q / right mouse) does something special in this game. */
  grab?: boolean;
  /** A big, busy map: planned as the last round of a game when possible. */
  finale?: boolean;
}

export function defineGame(meta: GameMeta): GameMeta {
  if (!/^[a-z][a-z0-9-]*$/.test(meta.id)) throw new Error(`bad game id: ${meta.id}`);
  return meta;
}

/** What happens when a bean falls off: back to the last checkpoint, back to its spawn, or out of the round. */
export type FallBehaviour = 'checkpoint' | 'spawn' | 'out';

export function fallBehaviour(genre: Genre): FallBehaviour {
  if (genre === 'race') return 'checkpoint';
  if (genre === 'points') return 'spawn';
  return 'out';
}

export type ArenaKind = 'lobby' | 'round' | 'podium';

/** Beans stand still before the start of a round (the intro) and on the podium. */
export function canMove(kind: ArenaKind, t: number): boolean {
  return kind === 'lobby' || (kind === 'round' && t >= 0);
}
