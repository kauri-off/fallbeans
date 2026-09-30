import { z } from 'zod';
import { COLORS, EMOTES, NAME_MAX } from './consts';
import type { ArenaKind } from './game';

/**
 * Reliable control messages are JSON (WebSocket text frames, or length-prefixed frames on the
 * WebTransport stream). Inputs and snapshots are binary (see codec.ts) and travel as WebTransport
 * datagrams, or WebSocket binary frames on the fallback transport.
 */

const PlayerId = z.number().int().min(0).max(65535);

export const ROUND_COUNTS = [3, 5, 7] as const;

export const PlaylistSchema = z.object({
  mode: z.enum(['mix', 'races', 'survival', 'custom']),
  games: z.array(z.string().max(32)).max(12),
  rounds: z.number().int().min(1).max(12),
});
export type Playlist = z.infer<typeof PlaylistSchema>;
export const DEFAULT_PLAYLIST: Playlist = { mode: 'mix', games: [], rounds: 5 };

export const HelloSchema = z.object({
  t: z.literal('hello'),
  v: z.number().int(),
  name: z.string().max(64),
  ticket: z.string().max(256),
  token: z.string().max(64).optional(),
  practice: z.string().max(32).optional(),
});
export type Hello = z.infer<typeof HelloSchema>;

const Vec3 = z.tuple([z.number().finite(), z.number().finite(), z.number().finite()]);

/**
 * Development commands (accepted only by a server started with --dev). `id` defaults to the sender.
 * See DevCmd handling in server/room.ts, and window.__fallbeans.dev() in the client.
 */
export const DevCmdSchema = z.discriminatedUnion('c', [
  /** Jump to the start of the round (fast-forwards through the intro). */
  z.object({ c: z.literal('skipIntro') }),
  /** Simulate this much time right away (ms of game time, every tick is simulated). */
  z.object({ c: z.literal('warp'), ms: z.number().min(0).max(180_000) }),
  z.object({ c: z.literal('endRound') }),
  /** Start a game now: optional list of games, rounds, and exactly how many bots (replacing any). */
  z.object({
    c: z.literal('start'),
    games: z.array(z.string().max(32)).max(12).optional(),
    rounds: z.number().int().min(1).max(12).optional(),
    bots: z.number().int().min(0).max(7).optional(),
  }),
  z.object({ c: z.literal('lobby') }),
  /** Speed of game time: 1 normal, 0.25 slow motion, 0 paused. */
  z.object({ c: z.literal('rate'), k: z.number().min(0).max(8) }),
  /** While paused: advance this many ticks. */
  z.object({ c: z.literal('step'), ticks: z.number().int().min(1).max(1200) }),
  z.object({ c: z.literal('teleport'), id: PlayerId.optional(), p: Vec3, yaw: z.number().finite().optional() }),
  /** Teleport to the spawn, a checkpoint (index) or just before the finish. */
  z.object({
    c: z.literal('goto'),
    id: PlayerId.optional(),
    to: z.union([z.literal('spawn'), z.literal('finish'), z.number().int().min(0).max(64)]),
  }),
  /** Add bots (in any phase; in a round they join it), optionally right next to the sender. */
  z.object({ c: z.literal('bot'), n: z.number().int().min(1).max(7).optional(), near: z.boolean().optional() }),
  /** Freeze (false) or resume (true) bot brains. */
  z.object({ c: z.literal('bots'), on: z.boolean() }),
  z.object({ c: z.literal('kill'), id: PlayerId.optional() }),
  /** Knock a bean over with velocity [vx, vy, vz]. */
  z.object({ c: z.literal('knock'), id: PlayerId.optional(), v: Vec3 }),
  /** Make `actor` hold `target` for `s` seconds (as if the grab button were held). */
  z.object({ c: z.literal('grab'), actor: PlayerId.optional(), target: PlayerId, s: z.number().min(0).max(10).optional() }),
  /** Seed of the next round's map. */
  z.object({
    c: z.literal('seed'),
    seed: z
      .number()
      .int()
      .min(0)
      .max(2 ** 31),
  }),
]);
export type DevCmd = z.infer<typeof DevCmdSchema>;

export const ClientMsgSchema = z.discriminatedUnion('t', [
  HelloSchema,
  z.object({ t: z.literal('ping'), c: z.number().finite(), rtt: z.number().min(0).max(60000).optional() }),
  z.object({ t: z.literal('name'), name: z.string().max(64) }),
  z.object({ t: z.literal('color'), c: z.enum(COLORS) }),
  z.object({ t: z.literal('start') }),
  z.object({ t: z.literal('abort') }),
  z.object({ t: z.literal('playlist'), pl: PlaylistSchema }),
  z.object({ t: z.literal('addBot') }),
  z.object({ t: z.literal('removeBot'), id: PlayerId }),
  /** The host makes another player the host. */
  z.object({ t: z.literal('host'), id: PlayerId }),
  z.object({ t: z.literal('emote'), e: z.number().int().min(1).max(EMOTES) }),
  z.object({ t: z.literal('dev'), q: z.number().int().min(0).optional(), cmd: DevCmdSchema }),
]);
export type ClientMsg = z.infer<typeof ClientMsgSchema>;

/** Largest accepted control message, in bytes. */
export const MAX_CONTROL_BYTES = 4096;

export function sanitizeName(raw: string): string {
  return raw
    .replace(/[\p{C}<>]/gu, '')
    .trim()
    .slice(0, NAME_MAX);
}

export type Phase = 'lobby' | 'round' | 'results' | 'podium';

export interface LobbyPlayer {
  id: number;
  name: string;
  color: string;
  /** Points in the current game. */
  score: number;
  /** Games won. */
  crowns: number;
  spectator: boolean;
  bot: boolean;
  connected: boolean;
  ping: number;
}

export type GameEventRecord = [name: string, data: unknown];

/** A simulated world the client should build: the lobby playground or a round's map. */
export interface ArenaInfo {
  /** Changes with every new arena; inputs and snapshots carry it to ignore stale packets. */
  id: number;
  kind: ArenaKind;
  /** Game id, 'lobby' or 'podium'. */
  game: string;
  seed: number;
  /** Server clock (ms) at tick 0; ticks before it are the intro. */
  startAt: number;
  endAt: number;
  participants: number[];
  /** Round number in the game (1-based) and the number of rounds; 0 outside rounds. */
  index: number;
  total: number;
  practice: boolean;
  late: boolean;
  finished: number[];
  out: number[];
  events: GameEventRecord[];
  scores: [number, number][];
  /** World.hash(true) of the server's build: a client building the map differently reports it. */
  hash: string;
}

/** One player's line in the results of a round. */
export interface RoundRow {
  id: number;
  place: number;
  /** Placement points (0…10). */
  points: number;
  /** Points taken for falls and shortcuts. */
  penalty: number;
  /** Change of the game total (points − penalty, the total never going below 0). */
  delta: number;
  total: number;
  /** Did the job: finished, survived, scored. */
  ok: boolean;
  afk: boolean;
  note: string;
  falls: number;
}

export interface Standing {
  id: number;
  name: string;
  color: string;
  place: number;
  total: number;
  wins: number;
  falls: number;
}

export interface Award {
  key: string;
  title: string;
  icon: string;
  id: number;
  text: string;
}

/** What knocked a bean off (map hazards are tagged colliders; see builder). */
export type KoCause = string;

export type ServerMsg =
  | { t: 'welcome'; id: number; token: string; solo: boolean; practice: boolean; resumed: boolean; dev: boolean }
  | { t: 'reject'; reason: 'full' | 'version' | 'auth' | 'bad' | 'busy'; msg: string }
  | { t: 'pong'; c: number; s: number }
  | { t: 'lobby'; phase: Phase; host: number | null; min: number; max: number; players: LobbyPlayer[]; playlist: Playlist }
  | ({ t: 'arena' } & ArenaInfo)
  | { t: 'roundEnd'; game: string; index: number; total: number; rows: RoundRow[]; practice: boolean }
  | { t: 'gameEnd'; standings: Standing[]; awards: Award[] }
  | { t: 'fin'; id: number; place: number; time: number }
  /** A bean fell (respawned) or was eliminated, with who or what caused it. */
  | { t: 'ko'; id: number; out: boolean; by: number | null; cause: KoCause; shortcut?: boolean }
  | { t: 'ev'; n: string; d: unknown }
  | { t: 'scores'; s: [number, number][] }
  | { t: 'emote'; id: number; e: number }
  | { t: 'left'; id: number }
  /** Reply to a dev command (`q` echoes the request's). */
  | { t: 'devAck'; q: number | null; ok: boolean; msg: string }
  /** The server clock changed speed or jumped (dev): `s` is the server time when it did. */
  | { t: 'clock'; rate: number; s: number };

export type ServerMsgOf<T extends ServerMsg['t']> = Extract<ServerMsg, { t: T }>;
