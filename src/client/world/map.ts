import type * as THREE from 'three';
import type { AnyGameMeta } from '../../shared/game';
import type { Rng } from '../../shared/rng';
import type { Sfx } from '../engine/audio';
import type { Bean } from '../engine/bean';
import type { BodyInput, Collider, PlayerBody } from '../engine/physics';
import type { Builder } from './builder';

export interface MapCtx {
  readonly seed: number;
  emit(name: string, data: unknown, actor?: number): void;
  serverNow(): number;
  sfx(s: Sfx): void;
  bean(id: number): Bean | undefined;
  myId(): number;
}

export interface Checkpoint {
  z: number;
  p: THREE.Vector3;
}

export interface BotInput extends BodyInput {
  grab: boolean;
}

export interface BotView {
  id: number;
  body: PlayerBody;
  t: number;
  rng: Rng;
  mem: Record<string, number>;
  others: readonly { id: number; pos: THREE.Vector3 }[];
}

export type BotBrain = (bot: BotView, out: BotInput) => void;

export interface MapSpec {
  spawns: THREE.Vector3[];
  killY: number;
  isOut?(p: THREE.Vector3): boolean;
  checkpoints?: Checkpoint[];
  finish?: { z: number; y: number };
  view?: THREE.Vector3;
  faceCenter?: boolean;
  onEvent?(name: string, data: unknown, by: number | null): void;
  onGrab?(actor: number, target: number): void;
  hud?(): string | null;
  bot?: BotBrain;
}

export interface GameMap extends MapSpec {
  readonly id: string;
  readonly colliders: readonly Collider[];
  readonly group: THREE.Group;
  update(t: number, dt: number): void;
  dispose(): void;
}

export type BuildFn = (b: Builder, ctx: MapCtx) => MapSpec;

export interface MapModule {
  meta: AnyGameMeta;
  build: BuildFn;
}

export function defineMap(meta: AnyGameMeta, build: BuildFn): MapModule {
  return { meta, build };
}
