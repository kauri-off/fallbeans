export const PROTOCOL_VERSION = 2;
export const MAX_PLAYERS = 8;
export const COLORS = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a', '#a66bff', '#ff8a3d', '#39e0d0', '#ffffff'] as const;
export const SNAPSHOT_MS = 50;
export const INTRO_MS = 7000;
export const RESULTS_MS = 6500;
export const WINNER_MS = 11000;
export const RECONNECT_GRACE_MS = 30000;
export const NAME_MAX = 16;

export const ANIM = { idle: 0, air: 1, dive: 2, stun: 3, grab: 4, slide: 5 } as const;
export type AnimCode = (typeof ANIM)[keyof typeof ANIM];
