import { DEFAULT_OUTFIT, type Outfit } from '../../shared/outfit';
import type { Conn } from '../net/conn';
import { emptyGameStats, type GameStats } from './awards';

/** Someone in a room: a person (connected or within the reconnect grace) or a bot. */
export interface Player {
  id: number;
  name: string;
  color: string;
  outfit: Outfit;
  /** Points in the current game. */
  score: number;
  crowns: number;
  stats: GameStats;
  /** The person's identity (see Auth.issueIdentity); '' for bots. */
  uid: string;
  bot: boolean;
  /** A bot added to fill the empty places (it goes when the host turns that off). */
  auto: boolean;
  conn: Conn | null;
  disconnectedAt: number;
  spectator: boolean;
  msgWindow: number;
  msgCount: number;
  chatAt: number;
  rtt: number;
}

const BOT_NAMES = ['Кекс', 'Пончик', 'Жужа', 'Бублик', 'Мармелад', 'Хрустик', 'Пельмень', 'Зефир', 'Кнопка', 'Шмель'];

/** A bot name nobody in the room has yet. */
export function botName(taken: Iterable<string>, id: number): string {
  const used = new Set(taken);
  return `Бот ${BOT_NAMES.find((n) => !used.has(`Бот ${n}`)) ?? id}`;
}

export function makePlayer(
  id: number,
  name: string,
  color: string,
  o: { uid?: string; conn?: Conn; spectator?: boolean; auto?: boolean; outfit?: Outfit | undefined } = {},
): Player {
  return {
    id,
    name,
    color,
    outfit: o.outfit ?? DEFAULT_OUTFIT,
    score: 0,
    crowns: 0,
    stats: emptyGameStats(),
    uid: o.uid ?? '',
    bot: !o.conn,
    auto: !!o.auto,
    conn: o.conn ?? null,
    disconnectedAt: 0,
    spectator: !!o.spectator,
    msgWindow: 0,
    msgCount: 0,
    chatAt: -1e9,
    rtt: 0,
  };
}
