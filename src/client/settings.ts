import { effect, signal } from '@preact/signals';
import type { Quality } from './game/renderer';

/** Esc menu layout: a column beside the player panel (the game stays in view), or a wide panel in the middle. */
export type MenuLayout = 'side' | 'center';

export interface Settings {
  name: string;
  menuLayout: MenuLayout;
  sensitivity: number;
  invertY: boolean;
  fov: number;
  volume: number;
  quality: Quality;
  showFps: boolean;
}

const KEY = 'fb_settings';
const DEFAULTS: Settings = {
  name: '',
  menuLayout: 'side',
  sensitivity: 1,
  invertY: false,
  fov: 70,
  volume: 0.8,
  quality: 'high',
  showFps: false,
};

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const s = { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Settings>) };
      // Ultra was folded into high.
      if (!['medium', 'high'].includes(s.quality)) s.quality = 'high';
      return s;
    }
  } catch {}
  return { ...DEFAULTS };
}

export const settings = signal<Settings>(load());

effect(() => {
  const s = settings.value;
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {}
});

export function updateSettings(patch: Partial<Settings>) {
  settings.value = { ...settings.value, ...patch };
}

/**
 * The player's identity token from the server, one per browser: every tab is the same player (a
 * second tab takes over from the first), so a player is in one room at a time and their own room
 * stays theirs across reloads. Never required to work: without storage every connection is a new player.
 * Development: `?profile=b` keeps a separate identity, to play against oneself in two tabs.
 */
const ID_KEY = `fb_id${import.meta.env.DEV ? (new URLSearchParams(location.search).get('profile') ?? '') : ''}`;
export const identity = {
  get(): string | null {
    try {
      return localStorage.getItem(ID_KEY);
    } catch {
      return null;
    }
  },
  set(token: string) {
    try {
      localStorage.setItem(ID_KEY, token);
    } catch {}
  },
};
