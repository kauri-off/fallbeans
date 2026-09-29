import { z } from 'zod';
import { COLORS, NAME_MAX } from './consts';

/**
 * Reliable control messages are JSON (WebSocket text frames, or length-prefixed frames on the
 * WebTransport stream). Inputs and snapshots are binary (see codec.ts) and travel as WebTransport
 * datagrams, or WebSocket binary frames on the fallback transport.
 */

const PlayerId = z.number().int().min(0).max(65535);

export const PlaylistSchema = z.object({
  mode: z.enum(['mix', 'races', 'survival', 'custom']),
  games: z.array(z.string().max(32)).max(8),
  final: z.string().max(32),
});
export type Playlist = z.infer<typeof PlaylistSchema>;
export const DEFAULT_PLAYLIST: Playlist = { mode: 'mix', games: [], final: 'random' };

export const HelloSchema = z.object({
  t: z.literal('hello'),
  v: z.number().int(),
  name: z.string().max(64),
  ticket: z.string().max(256),
  token: z.string().max(64).optional(),
  practice: z.string().max(32).optional(),
});
export type Hello = z.infer<typeof HelloSchema>;

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
  z.object({ t: z.literal('emote'), e: z.number().int().min(1).max(3) }),
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

export type Phase = 'lobby' | 'round' | 'results' | 'winner';

export interface LobbyPlayer {
  id: number;
  name: string;
  color: string;
  score: number;
  crowns: number;
  alive: boolean;
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
  kind: 'lobby' | 'round';
  /** Game id, or 'lobby'. */
  game: string;
  seed: number;
  /** Server clock (ms) at tick 0; ticks before it are the intro. */
  startAt: number;
  endAt: number;
  participants: number[];
  eliminate: number;
  qualify: number;
  index: number;
  total: number;
  practice: boolean;
  late: boolean;
  finished: number[];
  out: number[];
  events: GameEventRecord[];
  scores: [number, number][];
}

export interface RankEntry {
  id: number;
  ok: boolean;
  points: number;
  note: string;
}

export type ServerMsg =
  | { t: 'welcome'; id: number; token: string; solo: boolean; practice: boolean; resumed: boolean }
  | { t: 'reject'; reason: 'full' | 'version' | 'auth' | 'bad' | 'busy'; msg: string }
  | { t: 'pong'; c: number; s: number }
  | { t: 'lobby'; phase: Phase; host: number | null; min: number; max: number; players: LobbyPlayer[]; playlist: Playlist }
  | ({ t: 'arena' } & ArenaInfo)
  | { t: 'roundEnd'; game: string; ranking: RankEntry[]; practice: boolean }
  | { t: 'winner'; id: number; name: string }
  | { t: 'fin'; id: number; place: number }
  | { t: 'out'; id: number }
  | { t: 'ev'; n: string; d: unknown }
  | { t: 'scores'; s: [number, number][] }
  | { t: 'emote'; id: number; e: number }
  | { t: 'left'; id: number };

export type ServerMsgOf<T extends ServerMsg['t']> = Extract<ServerMsg, { t: T }>;
