import { z } from 'zod';
import { CHAT_MAX, COLORS, EMOTES, NAME_MAX, ROOM_PIN_DIGITS, ROOM_TITLE_MAX } from './consts';
import type { ArenaKind } from './game';
import { type Outfit, OutfitSchema } from './outfit';

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

/** Room ids: short codes, as in a link to the room (`?room=k7qxm`). */
const RoomId = z.string().regex(/^[a-z0-9]{2,8}$/);
/** PIN of a private room (its host sees it in the menu). */
const RoomPin = z.string().regex(new RegExp(`^\\d{${ROOM_PIN_DIGITS}}$`));

export const HelloSchema = z.object({
  t: z.literal('hello'),
  v: z.number().int(),
  name: z.string().max(64),
  ticket: z.string().max(256),
  /** The player's identity from an earlier visit (see `ready`); without it the server makes a new one. */
  token: z.string().max(128).optional(),
  /** Go straight into this room (a link to it, a page reload, a reconnect). */
  room: RoomId.optional(),
  pin: RoomPin.optional(),
  practice: z.string().max(32).optional(),
  /** The suit colour the player picked last time (taken if nobody in the room has it) and their outfit. */
  color: z.enum(COLORS).optional(),
  outfit: OutfitSchema.optional(),
});
export type Hello = z.infer<typeof HelloSchema>;

const Vec3 = z.tuple([z.number().finite(), z.number().finite(), z.number().finite()]);

/**
 * Development commands (accepted only by a server started with --dev). `id` defaults to the sender.
 * See DevCmd handling in server/rooms/room.ts, and window.__fallbeans.dev() in the client.
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
  /** From the room list: open a room of one's own (each player has at most one) and enter it. */
  z.object({ t: z.literal('create'), title: z.string().max(64), private: z.boolean() }),
  /** From the room list: enter a room; a private one wants its PIN from those it has not let in before. */
  z.object({ t: z.literal('join'), room: RoomId, pin: RoomPin.optional() }),
  /** Back to the room list. */
  z.object({ t: z.literal('leave') }),
  z.object({ t: z.literal('color'), c: z.enum(COLORS) }),
  z.object({ t: z.literal('outfit'), o: OutfitSchema }),
  z.object({ t: z.literal('start') }),
  z.object({ t: z.literal('abort') }),
  z.object({ t: z.literal('playlist'), pl: PlaylistSchema }),
  z.object({ t: z.literal('addBot') }),
  z.object({ t: z.literal('removeBot'), id: PlayerId }),
  /** The host makes another player the host. */
  z.object({ t: z.literal('host'), id: PlayerId }),
  /** The host makes the room private (the server picks a PIN) or public. */
  z.object({ t: z.literal('access'), private: z.boolean() }),
  /** The host has the empty places of the lobby filled with bots. */
  z.object({ t: z.literal('fill'), on: z.boolean() }),
  z.object({ t: z.literal('emote'), e: z.number().int().min(1).max(EMOTES) }),
  /** A line for the room's chat. */
  z.object({ t: z.literal('chat'), text: z.string().max(CHAT_MAX * 2) }),
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

export function sanitizeTitle(raw: string): string {
  return raw
    .replace(/[\p{C}<>]/gu, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, ROOM_TITLE_MAX);
}

/** A chat line as the room passes it on: one line, no control characters, at most CHAT_MAX characters. */
export function sanitizeChat(raw: string): string {
  const text = raw
    .replace(/\p{Cc}+/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  return [...text].slice(0, CHAT_MAX).join('');
}

export type Phase = 'lobby' | 'round' | 'results' | 'podium';

/** A room as its members know it. */
export interface RoomRef {
  /** '' for a practice room (not in the list). */
  id: string;
  title: string;
  private: boolean;
}

/** A room in the list on the home screen. */
export interface RoomInfo extends RoomRef {
  /** Name of the current host. */
  host: string;
  /** People in the room (bots are counted apart). */
  players: number;
  bots: number;
  max: number;
  phase: Phase;
}

export interface LobbyPlayer {
  id: number;
  name: string;
  color: string;
  outfit: Outfit;
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
  /** The hello was accepted: `token` is the player's identity, to be sent with every later hello. */
  | { t: 'ready'; token: string; dev: boolean }
  /** The connection is refused and closed (`moved`: the player opened the game in another tab). */
  | { t: 'reject'; reason: 'version' | 'auth' | 'bad' | 'busy' | 'moved'; msg: string }
  /** The game is being updated: the connection closes; the page waits for the new version and reloads. */
  | { t: 'updating' }
  | { t: 'pong'; c: number; s: number }
  /** The room list, sent while the player is in no room; `mine` is the room they created, if it still exists. */
  | { t: 'rooms'; rooms: RoomInfo[]; mine: string | null }
  /** Creating or entering a room failed (`pin` with an empty msg: the room asks for its PIN). */
  | { t: 'denied'; room: string | null; reason: 'pin' | 'full' | 'gone' | 'limit'; msg: string }
  /** The player is out of the room, back at the room list. */
  | { t: 'home'; msg: string }
  /** Entered a room (`id` is the player's id there); its lobby and arena follow. */
  | { t: 'welcome'; id: number; room: string; solo: boolean; practice: boolean; resumed: boolean }
  | {
      t: 'lobby';
      room: RoomRef;
      phase: Phase;
      host: number | null;
      min: number;
      max: number;
      players: LobbyPlayer[];
      playlist: Playlist;
      /** Empty places are filled with bots. */
      fill: boolean;
      /** PIN of a private room: only the host is told. */
      pin: string | null;
      /** Server clock (ms) when the next scene starts on its own (results → round, podium → lobby). */
      next: number | null;
    }
  | ({ t: 'arena' } & ArenaInfo)
  | { t: 'roundEnd'; game: string; index: number; total: number; rows: RoundRow[]; practice: boolean }
  | { t: 'gameEnd'; standings: Standing[]; awards: Award[] }
  | { t: 'fin'; id: number; place: number; time: number }
  /** A bean fell (respawned) or was eliminated, with who or what caused it. */
  | { t: 'ko'; id: number; out: boolean; by: number | null; cause: KoCause; shortcut?: boolean }
  | { t: 'ev'; n: string; d: unknown }
  | { t: 'scores'; s: [number, number][] }
  | { t: 'emote'; id: number; e: number }
  | { t: 'chat'; id: number; name: string; text: string }
  | { t: 'left'; id: number }
  /** Reply to a dev command (`q` echoes the request's). */
  | { t: 'devAck'; q: number | null; ok: boolean; msg: string }
  /** The server clock changed speed or jumped (dev): `s` is the server time when it did. */
  | { t: 'clock'; rate: number; s: number };

export type ServerMsgOf<T extends ServerMsg['t']> = Extract<ServerMsg, { t: T }>;
