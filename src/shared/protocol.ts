import { z } from 'zod';
import { COLORS, NAME_MAX } from './consts';

const num = z.number().finite().min(-1e5).max(1e5);
const Vec3 = z.tuple([num, num, num]);
const PlayerId = z.number().int().min(0);
const Actor = PlayerId.optional();

export const PlaylistSchema = z.object({
  mode: z.enum(['mix', 'races', 'survival', 'custom']),
  games: z.array(z.string().max(32)).max(8),
  final: z.string().max(32),
});
export type Playlist = z.infer<typeof PlaylistSchema>;
export const DEFAULT_PLAYLIST: Playlist = { mode: 'mix', games: [], final: 'random' };

export const ClientMsgSchema = z.discriminatedUnion('t', [
  z.object({ t: z.literal('hello'), v: z.number().int(), name: z.string().max(64), token: z.string().max(64).optional() }),
  z.object({ t: z.literal('ping'), c: z.number().finite() }),
  z.object({ t: z.literal('s'), p: Vec3, r: num, a: z.number().int().min(0).max(15), as: Actor }),
  z.object({ t: z.literal('name'), name: z.string().max(64) }),
  z.object({ t: z.literal('color'), c: z.enum(COLORS) }),
  z.object({ t: z.literal('start') }),
  z.object({ t: z.literal('abort') }),
  z.object({ t: z.literal('playlist'), pl: PlaylistSchema }),
  z.object({ t: z.literal('addBot') }),
  z.object({ t: z.literal('removeBot'), id: PlayerId }),
  z.object({ t: z.literal('finish'), as: Actor }),
  z.object({ t: z.literal('out'), as: Actor }),
  z.object({ t: z.literal('ev'), n: z.string().min(1).max(32), d: z.unknown(), as: Actor }),
  z.object({ t: z.literal('bump'), to: PlayerId, v: Vec3, as: Actor }),
  z.object({ t: z.literal('grab'), to: PlayerId, as: Actor }),
  z.object({ t: z.literal('emote'), e: z.number().int().min(1).max(3), as: Actor }),
]);
export type ClientMsg = z.infer<typeof ClientMsgSchema>;

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
  owner: number | null;
  connected: boolean;
}

export type GameEventRecord = [name: string, data: unknown, by: number | null];

export interface RoundInfo {
  game: string;
  eliminate: number;
  qualify: number;
  participants: number[];
  startAt: number;
  endAt: number;
  seed: number;
  index: number;
  total: number;
  practice: boolean;
  late?: boolean;
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

export type Snapshot = [id: number, x: number, y: number, z: number, r: number, a: number];

export type ServerMsg =
  | { t: 'welcome'; id: number; token: string; solo: boolean; practice: boolean; resumed: boolean }
  | { t: 'reject'; reason: 'full' | 'version' | 'bad'; msg: string }
  | { t: 'pong'; c: number; s: number }
  | { t: 'lobby'; phase: Phase; host: number | null; min: number; max: number; players: LobbyPlayer[]; playlist: Playlist }
  | ({ t: 'round' } & RoundInfo)
  | { t: 'roundEnd'; game: string; ranking: RankEntry[]; practice: boolean }
  | { t: 'winner'; id: number; name: string }
  | { t: 'S'; s: number; l: Snapshot[] }
  | { t: 'fin'; id: number; place: number }
  | { t: 'out'; id: number }
  | { t: 'ev'; n: string; d: unknown; by: number | null }
  | { t: 'scores'; s: [number, number][] }
  | { t: 'bump'; to: number; from: number; v: [number, number, number] }
  | { t: 'grab'; to: number; from: number }
  | { t: 'emote'; id: number; e: number }
  | { t: 'left'; id: number };

export type ServerMsgOf<T extends ServerMsg['t']> = Extract<ServerMsg, { t: T }>;
