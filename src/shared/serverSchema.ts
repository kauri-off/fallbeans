import { z } from 'zod';
import { CHAT_MAX, EMOTES, NAME_MAX, ROOM_TITLE_MAX } from './consts';
import { PlaylistSchema, type ServerMsg } from './protocol';

/**
 * Runtime check of what the server sends (protocol.ts declares the types). Used by the room tests
 * (every message the server produces must pass) and by dev clients (a failing message is reported).
 */

const Id = z.number().int().min(0).max(65535);
const Num = z.number().finite();
const Phase = z.enum(['lobby', 'round', 'results', 'podium']);

const LobbyPlayer = z.object({
  id: Id,
  name: z.string().max(16),
  color: z.string(),
  score: z.number().int().min(0),
  crowns: z.number().int().min(0),
  spectator: z.boolean(),
  bot: z.boolean(),
  connected: z.boolean(),
  ping: z.number().min(0),
});

const RoundRow = z.object({
  id: Id,
  place: z.number().int().min(1),
  points: z.number().int().min(0).max(10),
  penalty: z.number().int().min(0),
  delta: z.number().int(),
  total: z.number().int().min(0),
  ok: z.boolean(),
  afk: z.boolean(),
  note: z.string(),
  falls: z.number().int().min(0),
});

const RoomRef = z.object({ id: z.string(), title: z.string().max(ROOM_TITLE_MAX), private: z.boolean() });

export const ServerMsgSchema = z.discriminatedUnion('t', [
  z.object({ t: z.literal('ready'), token: z.string(), dev: z.boolean() }),
  z.object({ t: z.literal('reject'), reason: z.enum(['version', 'auth', 'bad', 'busy', 'moved']), msg: z.string() }),
  z.object({ t: z.literal('pong'), c: Num, s: Num }),
  z.object({
    t: z.literal('rooms'),
    rooms: z.array(
      RoomRef.extend({
        host: z.string(),
        players: z.number().int().min(0),
        bots: z.number().int().min(0),
        max: z.number().int().min(1).max(8),
        phase: Phase,
      }),
    ),
    mine: z.string().nullable(),
  }),
  z.object({
    t: z.literal('denied'),
    room: z.string().nullable(),
    reason: z.enum(['pin', 'full', 'gone', 'limit']),
    msg: z.string(),
  }),
  z.object({ t: z.literal('home'), msg: z.string() }),
  z.object({
    t: z.literal('welcome'),
    id: Id,
    room: z.string(),
    solo: z.boolean(),
    practice: z.boolean(),
    resumed: z.boolean(),
  }),
  z.object({
    t: z.literal('lobby'),
    room: RoomRef,
    phase: Phase,
    host: Id.nullable(),
    min: z.number().int().min(1),
    max: z.number().int().min(1).max(8),
    players: z.array(LobbyPlayer).max(8),
    playlist: PlaylistSchema,
    fill: z.boolean(),
    pin: z.string().nullable(),
    next: Num.nullable(),
  }),
  z.object({
    t: z.literal('arena'),
    id: Id,
    kind: z.enum(['lobby', 'round', 'podium']),
    game: z.string(),
    seed: z.number().int(),
    startAt: Num,
    endAt: Num,
    participants: z.array(Id),
    index: z.number().int().min(0),
    total: z.number().int().min(0),
    practice: z.boolean(),
    late: z.boolean(),
    finished: z.array(Id),
    out: z.array(Id),
    events: z.array(z.tuple([z.string(), z.unknown()])),
    scores: z.array(z.tuple([Id, Num])),
    hash: z.string(),
  }),
  z.object({
    t: z.literal('roundEnd'),
    game: z.string(),
    index: z.number().int(),
    total: z.number().int(),
    rows: z.array(RoundRow),
    practice: z.boolean(),
  }),
  z.object({
    t: z.literal('gameEnd'),
    standings: z.array(
      z.object({
        id: Id,
        name: z.string(),
        color: z.string(),
        place: z.number().int().min(1),
        total: z.number().int().min(0),
        wins: z.number().int().min(0),
        falls: z.number().int().min(0),
      }),
    ),
    awards: z.array(z.object({ key: z.string(), title: z.string(), icon: z.string(), id: Id, text: z.string() })),
  }),
  z.object({ t: z.literal('fin'), id: Id, place: z.number().int().min(1), time: Num }),
  z.object({
    t: z.literal('ko'),
    id: Id,
    out: z.boolean(),
    by: Id.nullable(),
    cause: z.string(),
    shortcut: z.boolean().optional(),
  }),
  z.object({ t: z.literal('ev'), n: z.string(), d: z.unknown() }),
  z.object({ t: z.literal('scores'), s: z.array(z.tuple([Id, Num])) }),
  z.object({ t: z.literal('emote'), id: Id, e: z.number().int().min(1).max(EMOTES) }),
  z.object({ t: z.literal('chat'), id: Id, name: z.string().max(NAME_MAX), text: z.string().max(CHAT_MAX * 2) }),
  z.object({ t: z.literal('left'), id: Id }),
  z.object({ t: z.literal('devAck'), q: z.number().int().nullable(), ok: z.boolean(), msg: z.string() }),
  z.object({ t: z.literal('clock'), rate: z.number().min(0).max(8), s: Num }),
]);

// Every message the types allow must be accepted by the schema (compile-time check).
type Accepts<T extends z.input<typeof ServerMsgSchema>> = T;
export type _ServerMsgCovered = Accepts<ServerMsg>;

/** The first problem with a server message, or null when it is valid. */
export function checkServerMsg(m: unknown): string | null {
  const r = ServerMsgSchema.safeParse(m);
  if (r.success) return null;
  const i = r.error.issues[0];
  return `${(m as { t?: string })?.t ?? '?'}: ${i?.path.join('.')} ${i?.message}`;
}
