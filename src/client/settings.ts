import { effect, signal } from '@preact/signals';
import type { Quality } from './game/renderer';

export interface Settings {
  name: string;
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
    if (raw) return { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Settings>) };
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

/** Per-tab session storage (resume token), never required to work. */
export const session = {
  get(key: string): string | null {
    try {
      return sessionStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string) {
    try {
      sessionStorage.setItem(key, value);
    } catch {}
  },
};
