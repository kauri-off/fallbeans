import type * as THREE from 'three';
import type { GameMeta } from '../shared/game';
import type { Rng } from '../shared/rng';
import type { Builder } from './builder';
import type { NavGrid, NavPoint } from './nav';
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
  /** Progress threshold (z unless the map defines progress()). */
  z: number;
  p: THREE.Vector3;
}

export interface BotInput extends BodyInput {
  grab: boolean;
  /** Play an emote (1–3), 0 for none. */
  emote: number;
}

export interface BotMem {
  [k: string]: number | undefined;
}

/** A bot's current route (see sim/nav.ts). */
export interface BotPlan {
  path: NavPoint[] | null;
  i: number;
  tx: number;
  tz: number;
  at: number;
}

export interface BotView {
  id: number;
  body: PlayerBody;
  /** Sim time. */
  t: number;
  rng: Rng;
  mem: BotMem;
  plan: BotPlan;
  others: readonly { id: number; pos: THREE.Vector3; vel: THREE.Vector3; down: boolean }[];
  /** Walkable ground of the static course (null before the start). */
  nav: NavGrid | null;
  /** Bonuses lying on the course right now. */
  bonuses?: readonly { x: number; y: number; z: number }[];
}

export type BotBrain = (bot: BotView, out: BotInput) => void;

export interface MapSpec {
  spawns: THREE.Vector3[];
  killY: number;
  isOut?(p: THREE.Vector3): boolean;
  checkpoints?: Checkpoint[];
  /** Progress along the course (default: z); checkpoint thresholds and race ranking use it. */
  progress?(p: THREE.Vector3): number;
  /** Crossing z (above y − 2, within |x| ≤ halfWidth when given) finishes the race. */
  finish?: { z: number; y: number; halfWidth?: number };
  /** Out-of-course places (on top of frames, behind walls): standing there counts as a shortcut. */
  forbidden?(p: THREE.Vector3): boolean;
  /** What a fall is blamed on when no hazard or player was involved (default 'fall'). */
  fallCause?: string;
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

/** Problems with a built map that would break a round (the audits check much more, offline). */
export function specProblems(spec: MapSpec): string[] {
  const out: string[] = [];
  const finite = (v: { x: number; y: number; z: number }) => Number.isFinite(v.x + v.y + v.z);
  if (!spec.spawns.length) out.push('no spawns');
  if (spec.spawns.some((p) => !finite(p))) out.push('a spawn is not a finite position');
  if (!Number.isFinite(spec.killY)) out.push('killY is not a number');
  else if (spec.spawns.some((p) => p.y <= spec.killY)) out.push('a spawn is below killY');
  if (spec.checkpoints?.some((c) => !finite(c.p) || !Number.isFinite(c.z))) out.push('a checkpoint is not finite');
  if (spec.finish && !Number.isFinite(spec.finish.z + spec.finish.y)) out.push('the finish is not finite');
  return out;
}

export function defineMap(meta: GameMeta, build: BuildFn): MapModule {
  return { meta, build };
}
