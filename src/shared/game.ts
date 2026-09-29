import { z } from 'zod';
import type { Rng } from './rng';

export type RuleKind = 'race' | 'survival' | 'points' | 'lastStanding' | 'raceFinal';
export type Genre = 'race' | 'survival' | 'points' | 'final';

export const GENRE_LABEL: Record<Genre, string> = {
  race: 'Гонка',
  survival: 'Выживание',
  points: 'Очки',
  final: 'Финал',
};

export interface GameServerCtx {
  readonly rng: Rng;
  readonly participants: readonly number[];
  now(): number;
  roundTime(): number;
  active(): number[];
  emit(name: string, data: unknown, by?: number | null): void;
  score(id: number): number;
  setScore(id: number, v: number): void;
  position(id: number): readonly [number, number, number] | null;
}

type EventSchemas = Record<string, z.ZodType>;

export interface GameServer<S, E extends EventSchemas> {
  init?(ctx: GameServerCtx): S;
  tick?(ctx: GameServerCtx, state: S): void;
  on?: { [K in keyof E]?: (ctx: GameServerCtx, state: S, from: number, data: z.infer<E[K]>) => void };
}

export interface GameMeta<S = unknown, E extends EventSchemas = EventSchemas> {
  id: string;
  title: string;
  genre: Genre;
  rules: RuleKind;
  desc: string;
  goal: string;
  duration: number;
  minPlayers?: number;
  finishZ?: number;
  events?: E;
  server?: GameServer<S, E>;
}

export type AnyGameMeta = GameMeta<any, EventSchemas>;

export function defineGame<S = undefined, E extends EventSchemas = Record<never, z.ZodType>>(meta: GameMeta<S, E>): GameMeta<S, E> {
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

export function relayOnce(delayMs: number) {
  return (ctx: GameServerCtx, state: { seen: Set<number> }, from: number, data: { i: number }) => {
    if (state.seen.has(data.i)) return;
    state.seen.add(data.i);
    ctx.emit('at', { i: data.i, at: ctx.now() + delayMs }, from);
  };
}

export const AtEvent = z.object({ i: z.number().int(), at: z.number() });
