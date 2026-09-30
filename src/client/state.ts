import { signal } from '@preact/signals';
import type { ArenaInfo, RoomInfo, RoomRef, ServerMsgOf } from '../shared/protocol';

export type ConnStatus = 'loading' | 'connecting' | 'online' | 'reconnecting' | 'rejected';

export const conn = signal<{ status: ConnStatus; transport: 'wt' | 'ws' | null; message: string; ping: number }>({
  status: 'loading',
  transport: null,
  message: '',
  ping: 0,
});
export const loadProgress = signal(0);
export const myId = signal(-1);
export const lobby = signal<ServerMsgOf<'lobby'> | null>(null);
export const arenaInfo = signal<ArenaInfo | null>(null);
export const results = signal<ServerMsgOf<'roundEnd'> | null>(null);
export const gameEnd = signal<ServerMsgOf<'gameEnd'> | null>(null);
export const practiceGame = signal<string | null>(null);

/** The room the player is in; null at the room list (the home screen). */
export const room = signal<RoomRef | null>(null);
/** The room list, shown while in no room; null until the server sent it. */
export const roomList = signal<RoomInfo[] | null>(null);
/** The room this player created, if it still exists. */
export const ownRoom = signal<string | null>(null);
/** Why entering or creating a room failed (`reason: 'pin'`: that room asks for its PIN). */
export const denied = signal<ServerMsgOf<'denied'> | null>(null);

export type PlayStatus = 'play' | 'finished' | 'out' | 'spectating';

export interface Hud {
  /** Round time in seconds (negative during the intro). */
  t: number;
  timeLeft: number;
  /** Seconds until the next scene starts on its own (-1: none). */
  nextIn: number;
  status: 'lobby' | 'podium' | PlayStatus;
  place: number;
  /** Who the camera follows while not playing ('' = overview). */
  spectating: string;
  mapText: string | null;
  /** The local player's bonus in effect ("⚡ Ускорение · 5 с"), if any. */
  bonus: string | null;
  /** Round status of every participant. */
  roster: Record<number, { status: PlayStatus; place: number }>;
  roundScores: Record<number, number>;
  fps: number;
  drawCalls: number;
}

export const hud = signal<Hud>({
  t: 0,
  timeLeft: 0,
  nextIn: -1,
  status: 'lobby',
  place: 0,
  spectating: '',
  mapText: null,
  bonus: null,
  roster: {},
  roundScores: {},
  fps: 0,
  drawCalls: 0,
});

/** The Esc menu (lobby, game settings, options) is open; the mouse is free. */
export const menuOpen = signal(true);
export const menuTab = signal<'game' | 'settings' | 'dev'>('game');
/** In play but the mouse is free (focus lost, lock refused): the field asks for a click. */
export const needClick = signal(false);

export interface FeedEntry {
  id: number;
  victim: number;
  by: number | null;
  cause: string;
  out: boolean;
  shortcut: boolean;
  /** Plain line (finishes, system notes) instead of a knockout. */
  text?: string;
}

export const feed = signal<FeedEntry[]>([]);
let feedSeq = 0;
export function pushFeed(e: Omit<FeedEntry, 'id'>) {
  const id = ++feedSeq;
  feed.value = [...feed.value.slice(-5), { ...e, id }];
  setTimeout(() => {
    feed.value = feed.value.filter((f) => f.id !== id);
  }, 6000);
}
export const note = (text: string) => pushFeed({ victim: -1, by: null, cause: '', out: false, shortcut: false, text });

export interface ChatLine {
  n: number;
  name: string;
  color: string;
  text: string;
  mine: boolean;
}

/** How long the chat stays on screen after a new line. */
const CHAT_SHOW_MS = 8000;
/** The room's chat: the last lines (it starts empty in every room). */
export const chatLog = signal<ChatLine[]>([]);
/** The player is typing a line (Enter): the chat is fully visible and takes the keyboard. */
export const chatOpen = signal(false);
/** Someone wrote just now: the chat shows through, half transparent, then fades away again. */
export const chatFresh = signal(false);
let chatSeq = 0;
let chatTimer: ReturnType<typeof setTimeout> | null = null;
export function pushChat(line: Omit<ChatLine, 'n'>) {
  chatLog.value = [...chatLog.value.slice(-49), { ...line, n: ++chatSeq }];
  chatFresh.value = true;
  if (chatTimer) clearTimeout(chatTimer);
  chatTimer = setTimeout(() => {
    chatFresh.value = false;
  }, CHAT_SHOW_MS);
}
export function clearChat() {
  if (chatTimer) clearTimeout(chatTimer);
  chatLog.value = [];
  chatOpen.value = false;
  chatFresh.value = false;
}

/** The server accepts dev commands (started with --dev): the menu shows the Dev tab. */
export const devMode = signal(false);
/** F3: performance and network overlay. */
export const debugOverlay = signal(false);
/** Screenshots (?shot or the probe): no interface over the scene. */
export const uiHidden = signal(new URLSearchParams(location.search).has('shot'));
/** Screenshots: a still picture — fixed sky and effects clock, no motes, beans do not blink or fidget. */
export const shotMode = signal(new URLSearchParams(location.search).has('shot'));
