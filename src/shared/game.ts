export type RuleKind = 'race' | 'survival' | 'points' | 'lastStanding' | 'raceFinal';
export type Genre = 'race' | 'survival' | 'points' | 'final';

export const GENRE_LABEL: Record<Genre, string> = {
  race: 'Гонка',
  survival: 'Выживание',
  points: 'Очки',
  final: 'Финал',
};

export interface GameMeta {
  id: string;
  title: string;
  genre: Genre;
  rules: RuleKind;
  desc: string;
  goal: string;
  /** Round length in seconds. */
  duration: number;
  minPlayers?: number;
  /** Grab (Q / right mouse) does something in this game. */
  grab?: boolean;
}

export function defineGame(meta: GameMeta): GameMeta {
  if (!/^[a-z][a-z0-9-]*$/.test(meta.id)) throw new Error(`bad game id: ${meta.id}`);
  if ((meta.genre === 'final') !== (meta.rules === 'lastStanding' || meta.rules === 'raceFinal'))
    throw new Error(`game ${meta.id}: finals must use final rules and vice versa`);
  return meta;
}

export type FallBehaviour = 'checkpoint' | 'spawn' | 'out';

export function fallBehaviour(rules: RuleKind): FallBehaviour {
  if (rules === 'race' || rules === 'raceFinal') return 'checkpoint';
  if (rules === 'points') return 'spawn';
  return 'out';
}
