import { signal } from '@preact/signals';
import type { ArenaInfo, ServerMsgOf } from '../shared/protocol';

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

export type PlayStatus = 'play' | 'finished' | 'out' | 'spectating';

export interface Hud {
  /** Round time in seconds (negative during the intro). */
  t: number;
  timeLeft: number;
  status: 'lobby' | 'podium' | PlayStatus;
  place: number;
  /** Who the camera follows while not playing ('' = overview). */
  spectating: string;
  mapText: string | null;
  /** Round status of every participant. */
  roster: Record<number, { status: PlayStatus; place: number }>;
  roundScores: Record<number, number>;
  fps: number;
  drawCalls: number;
}

export const hud = signal<Hud>({
  t: 0,
  timeLeft: 0,
  status: 'lobby',
  place: 0,
  spectating: '',
  mapText: null,
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

/** The server accepts dev commands (started with --dev): the menu shows the Dev tab. */
export const devMode = signal(false);
/** F3: performance and network overlay. */
export const debugOverlay = signal(false);
/** Screenshots (?shot or the probe): no interface over the scene. */
export const uiHidden = signal(new URLSearchParams(location.search).has('shot'));
/** Screenshots: a still picture — fixed sky and effects clock, no motes, beans do not blink or fidget. */
export const shotMode = signal(new URLSearchParams(location.search).has('shot'));
