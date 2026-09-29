import type * as THREE from 'three';
import type { GameMeta } from '../shared/game';
import type { Rng } from '../shared/rng';
import type { Builder } from './builder';
import type { BodyInput, PlayerBody } from './physics';

/** Sounds a map may ask the client to play. */
export type MapSfx = 'break' | 'warn' | 'steal' | 'boing' | 'count';

/**
 * What map code can do besides building. On the server, `emit` records an authoritative event,
 * applies it locally (spec.onEvent) and broadcasts it; on clients it does nothing, and the events
 * arrive from the server instead.
 */
export interface MapCtx {
  readonly server: boolean;
  readonly seed: number;
  readonly participants: readonly number[];
  /** Current sim time in seconds (negative during the intro). */
  now(): number;
  emit(name: string, data: unknown): void;
  score(id: number): number;
  setScore(id: number, v: number): void;
  /** Positions of bodies in play (server: all; client: the local one only). */
  bodies(): ReadonlyMap<number, PlayerBody>;
  /** Client: play a sound. */
  sfx(s: MapSfx): void;
  /** Client: id of the local player (-1 on the server). */
  me(): number;
  /** Client: set the bean decoration of a player (tail etc.). */
  decorate(id: number, deco: { tail?: boolean }): void;
}

export interface Checkpoint {
  z: number;
  p: THREE.Vector3;
}

export interface BotInput extends BodyInput {
  grab: boolean;
}

export interface BotMem {
  [k: string]: number | undefined;
}

export interface BotView {
  id: number;
  body: PlayerBody;
  /** Sim time. */
  t: number;
  rng: Rng;
  mem: BotMem;
  others: readonly { id: number; pos: THREE.Vector3 }[];
}

export type BotBrain = (bot: BotView, out: BotInput) => void;

export interface MapSpec {
  spawns: THREE.Vector3[];
  killY: number;
  isOut?(p: THREE.Vector3): boolean;
  checkpoints?: Checkpoint[];
  /** Crossing z (above y − 2, within |x| ≤ halfWidth when given) finishes the race. */
  finish?: { z: number; y: number; halfWidth?: number };
  /** Point the camera looks at in arenas. */
  view?: THREE.Vector3;
  faceCenter?: boolean;
  /** Authoritative event (both sides apply it; the server first). */
  onEvent?(name: string, data: unknown): void;
  /** Server: per-tick game logic. */
  tick?(t: number): void;
  /** Server: `actor` grabbed `target` (grab button, target in reach). */
  onGrab?(actor: number, target: number): void;
  /** Client: one line of HUD text (e.g. "Ваши очки: 12"). */
  hud?(): string | null;
  bot?: BotBrain;
}

export type BuildFn = (b: Builder, ctx: MapCtx) => MapSpec;

export interface MapModule {
  meta: GameMeta;
  build: BuildFn;
}

export function defineMap(meta: GameMeta, build: BuildFn): MapModule {
  return { meta, build };
}
