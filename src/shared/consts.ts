export const PROTOCOL_VERSION = 9;
export const BASE_PATH = '/fallbeans/';

export const MAX_PLAYERS = 8;
export const COLORS = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a', '#a66bff', '#ff8a3d', '#39e0d0', '#ffffff'] as const;
export const NAME_MAX = 16;
/** Emotes 1…EMOTES: wave, dance, laugh, cry, fright. */
export const EMOTES = 5;

/** Simulation: fixed 120 Hz steps on the server and in client prediction. */
export const TICK_RATE = 120;
export const DT = 1 / TICK_RATE;
export const TICK_MS = 1000 / TICK_RATE;
/** Server snapshots every 4 ticks (30 Hz). */
export const SNAPSHOT_EVERY = 4;
/** Clients send inputs every 2 ticks (60 Hz), each packet repeating recent inputs. */
export const INPUT_EVERY = 2;
export const INPUT_REDUNDANCY = 12;
/** Bot brains decide at 20 Hz. */
export const BOT_EVERY = 6;

export const INTRO_MS = 6000;
export const RESULTS_MS = 8000;
export const PRACTICE_RESULTS_MS = 3500;
export const PODIUM_MS = 20000;
export const RECONNECT_GRACE_MS = 30000;
export const MAX_PRACTICE_ROOMS = 3;
/** Rooms open at once (each simulates its own lobby or round). */
export const MAX_ROOMS = 16;
/** A room nobody is in is closed after this long (its owner may be reloading the page). */
export const ROOM_EMPTY_MS = 30000;
/** Dev servers keep one room open under this id for the tools (`?room=dev`). */
export const DEV_ROOM_ID = 'dev';
export const ROOM_TITLE_MAX = 24;
export const ROOM_PIN_DIGITS = 4;
export const CHAT_MAX = 160;

export const ANIM = {
  idle: 0,
  air: 1,
  dive: 2,
  stun: 3,
  grab: 4,
  slide: 5,
  tumble: 6,
  getup: 7,
  reach: 8,
  climb: 9,
  climbOver: 10,
} as const;
export type AnimCode = (typeof ANIM)[keyof typeof ANIM];

export const tickToTime = (tick: number) => tick * DT;
