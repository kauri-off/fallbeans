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
export const winner = signal<ServerMsgOf<'winner'> | null>(null);
export const practiceGame = signal<string | null>(null);

export interface Hud {
  /** Round time in seconds (negative during the intro). */
  t: number;
  timeLeft: number;
  status: 'lobby' | 'play' | 'finished' | 'out' | 'spectating';
  place: number;
  spectating: string;
  mapText: string | null;
  finished: number;
  out: number;
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
  finished: 0,
  out: 0,
  fps: 0,
  drawCalls: 0,
});

/** Mouse is free (menu) while the game would like it captured. */
export const paused = signal(false);
export const panelOpen = signal(true);
export const settingsOpen = signal(false);
export const scoreboard = signal(false);

export const feed = signal<{ id: number; text: string }[]>([]);
let feedSeq = 0;
export function pushFeed(text: string) {
  const id = ++feedSeq;
  feed.value = [...feed.value.slice(-4), { id, text }];
  setTimeout(() => {
    feed.value = feed.value.filter((f) => f.id !== id);
  }, 5000);
}
