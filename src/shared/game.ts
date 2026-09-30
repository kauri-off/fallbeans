import { z } from 'zod';

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
  /** Round length in seconds (60–180). */
  duration: number;
  minPlayers?: number;
  /** Grab (Q / right mouse) does something special in this game. */
  grab?: boolean;
  /** A big, busy map: planned as the last round of a game when possible. */
  finale?: boolean;
}

/** What a game description must look like (checked when a game is defined, and by the audits). */
export const GameMetaSchema = z.object({
  id: z.string().regex(/^[a-z][a-z0-9-]*$/, 'lowercase letters, digits and dashes'),
  title: z.string().min(2).max(24),
  genre: z.enum(['race', 'survival', 'points']),
  desc: z.string().min(10).max(220),
  goal: z.string().min(3).max(40),
  duration: z.number().int().min(60).max(180),
  minPlayers: z.number().int().min(1).max(8).optional(),
  grab: z.boolean().optional(),
  finale: z.boolean().optional(),
});

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
