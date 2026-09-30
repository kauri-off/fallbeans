import * as THREE from 'three';
import { mulberry32 } from '../shared/rng';

/**
 * How a map looks: its colours (the palette its parts are painted with), the patterns on them, the
 * sky, sun, fog and ambient light, the ground far below, and the kind of scenery around it. Each
 * map has a few looks of its own (the first one is its signature); a round picks one by its seed
 * and shifts the colours a little, so no two rounds look quite the same. Visual only: nothing here
 * touches colliders or the map's random layout.
 */

/** Palette names the maps paint with (sim/builder.ts PAL). */
export type PalKey = 'blue' | 'purple' | 'pink' | 'yellow' | 'green' | 'white' | 'orange' | 'red' | 'teal';
export type Palette = readonly [string, string];
export type PatternKind = 'stripes' | 'checker' | 'dots' | 'chevron' | 'waves';

export type LookId =
  | 'classic'
  | 'meadow'
  | 'castle'
  | 'factory'
  | 'snow'
  | 'starlight'
  | 'circus'
  | 'neon'
  | 'ocean'
  | 'desert'
  | 'jungle'
  | 'lava'
  | 'royal'
  | 'candy';

export interface Look {
  id: LookId;
  /** Base colour of each palette (the lighter second tone is derived). */
  colors: Record<PalKey, string>;
  patterns: readonly PatternKind[];
  sky: {
    top: string;
    horizon: string;
    /** Tint of the clouds (in the sky and the cloud models). */
    cloud: string;
    /** Stars in the sky, 0…1 (night looks). */
    stars: number;
  };
  /** Sun (or moon): colour, strength, compass angle and height (degrees; high keeps shadows under the beans). */
  sun: { color: string; intensity: number; azimuth: number; elevation: number };
  hemi: { sky: string; ground: string; intensity: number };
  fog: { color: string; near: number; far: number };
  exposure: number;
  saturation: number;
  /** Image-based light strength. */
  env: number;
  /** Specks in the air: colour and vertical drift (m/s; negative falls, like snow). */
  motes: { color: string; rise: number };
  /** The land far below the course (null: open sky). `glow` makes it shine (lava). */
  ground: { c1: string; c2: string; kind: PatternKind; freq: number; speed: number; glow: boolean } | null;
  /** The floating islands: grass (top), rock (underside) and bush colours. */
  island: { grass: string; rock: string; leaves: string };
  /** Birds and hot-air balloons on the horizon. */
  birds: boolean;
  balloons: boolean;
}

/** A look as a round uses it: the palettes and the pattern resolved for its seed. */
export interface ResolvedLook extends Look {
  palette: Record<PalKey, Palette>;
  pattern: PatternKind;
}

const DAY_SUN = { color: '#fff1dc', intensity: 2.2, azimuth: 38, elevation: 74 };
const DAY_HEMI = { sky: '#cfe8ff', ground: '#b99be0', intensity: 0.9 };
const GREEN_ISLAND = { grass: '#6fd46a', rock: '#9b7a5e', leaves: '#4fbf5a' };

export const LOOKS: Record<LookId, Look> = {
  // The original look (lobby, podium).
  classic: {
    id: 'classic',
    colors: {
      blue: '#7ccfff',
      purple: '#a98bff',
      pink: '#ff8cc8',
      yellow: '#ffd84a',
      green: '#6fe08a',
      white: '#f4f1ff',
      orange: '#ff9f4a',
      red: '#ff6070',
      teal: '#39e0d0',
    },
    patterns: ['stripes'],
    sky: { top: '#6fb8ff', horizon: '#ffd9f2', cloud: '#ffffff', stars: 0 },
    sun: DAY_SUN,
    hemi: DAY_HEMI,
    fog: { color: '#f1d4f7', near: 120, far: 520 },
    exposure: 0.95,
    saturation: 1.06,
    env: 0.3,
    motes: { color: '#fff7e0', rise: 0.12 },
    ground: null,
    island: GREEN_ISLAND,
    birds: true,
    balloons: true,
  },
  meadow: {
    id: 'meadow',
    colors: {
      blue: '#6ec3ff',
      purple: '#b39cff',
      pink: '#ff9ccf',
      yellow: '#ffe066',
      green: '#7fe07a',
      white: '#f7f5ea',
      orange: '#ffae5c',
      red: '#ff6f6f',
      teal: '#4fe0c4',
    },
    patterns: ['dots', 'waves', 'stripes'],
    sky: { top: '#58aefc', horizon: '#e6f7ff', cloud: '#ffffff', stars: 0 },
    sun: { color: '#fff4d6', intensity: 2.3, azimuth: 60, elevation: 72 },
    hemi: { sky: '#d8efff', ground: '#9fce8a', intensity: 0.95 },
    fog: { color: '#e2f2fb', near: 130, far: 560 },
    exposure: 0.95,
    saturation: 1.08,
    env: 0.3,
    motes: { color: '#fffbe0', rise: 0.15 },
    ground: { c1: '#7ccf6a', c2: '#93dc7c', kind: 'waves', freq: 0.02, speed: 0, glow: false },
    island: GREEN_ISLAND,
    birds: true,
    balloons: true,
  },
  castle: {
    id: 'castle',
    colors: {
      blue: '#6f8fd9',
      purple: '#8e79c9',
      pink: '#d98ab0',
      yellow: '#f2c94c',
      green: '#79b87a',
      white: '#e9e4da',
      orange: '#e39a5b',
      red: '#d9534f',
      teal: '#5bbfb0',
    },
    patterns: ['checker', 'stripes', 'chevron'],
    sky: { top: '#6fa6e0', horizon: '#f6e3c8', cloud: '#fff8ee', stars: 0 },
    sun: { color: '#ffe8c8', intensity: 2.3, azimuth: 20, elevation: 70 },
    hemi: { sky: '#dbe6ff', ground: '#a89a86', intensity: 0.9 },
    fog: { color: '#eee4d6', near: 120, far: 520 },
    exposure: 0.95,
    saturation: 1.02,
    env: 0.3,
    motes: { color: '#fff2d8', rise: 0.1 },
    ground: { c1: '#6fae5a', c2: '#86c26c', kind: 'checker', freq: 0.012, speed: 0, glow: false },
    island: { grass: '#78bf62', rock: '#8f8a86', leaves: '#5aa860' },
    birds: true,
    balloons: false,
  },
  factory: {
    id: 'factory',
    colors: {
      blue: '#5f8fb8',
      purple: '#7f7fa8',
      pink: '#d07a8a',
      yellow: '#f5c542',
      green: '#7fa86a',
      white: '#d8dde3',
      orange: '#f08a3a',
      red: '#e0523e',
      teal: '#4fb5ac',
    },
    patterns: ['chevron', 'stripes', 'checker'],
    sky: { top: '#7f9cbc', horizon: '#f2d6ae', cloud: '#e4ddd2', stars: 0 },
    sun: { color: '#ffdcb0', intensity: 2.2, azimuth: 120, elevation: 66 },
    hemi: { sky: '#dfe6ee', ground: '#8a7a6a', intensity: 0.9 },
    fog: { color: '#e6d8c4', near: 90, far: 430 },
    exposure: 0.95,
    saturation: 0.98,
    env: 0.35,
    motes: { color: '#ffe0b0', rise: 0.25 },
    ground: { c1: '#5b6270', c2: '#6a7280', kind: 'checker', freq: 0.03, speed: 0, glow: false },
    island: { grass: '#8a8f78', rock: '#6b6660', leaves: '#7a8a5a' },
    birds: false,
    balloons: false,
  },
  snow: {
    id: 'snow',
    colors: {
      blue: '#8fd0ff',
      purple: '#b7b0ff',
      pink: '#ffb7d9',
      yellow: '#fff0a0',
      green: '#9fe0c0',
      white: '#ffffff',
      orange: '#ffc38a',
      red: '#ff7f8f',
      teal: '#8ff0ea',
    },
    patterns: ['waves', 'dots', 'chevron'],
    sky: { top: '#8ec6f5', horizon: '#f4fbff', cloud: '#ffffff', stars: 0 },
    sun: { color: '#f4f8ff', intensity: 2.0, azimuth: 200, elevation: 68 },
    hemi: { sky: '#e6f4ff', ground: '#c8d8f0', intensity: 1.0 },
    fog: { color: '#eef6ff', near: 90, far: 420 },
    exposure: 0.92,
    saturation: 1.0,
    env: 0.35,
    motes: { color: '#ffffff', rise: -1.1 },
    ground: { c1: '#f4f9ff', c2: '#dfeefa', kind: 'waves', freq: 0.015, speed: 0, glow: false },
    island: { grass: '#f6fbff', rock: '#8a9bb0', leaves: '#e8f4ff' },
    birds: false,
    balloons: false,
  },
  starlight: {
    id: 'starlight',
    colors: {
      blue: '#6f86ff',
      purple: '#9a6bff',
      pink: '#ff7ad9',
      yellow: '#ffd86b',
      green: '#5fe0a8',
      white: '#dfe6ff',
      orange: '#ff9f6b',
      red: '#ff5f87',
      teal: '#46e0e6',
    },
    patterns: ['dots', 'checker', 'waves'],
    sky: { top: '#0b1238', horizon: '#46307a', cloud: '#5a4a8a', stars: 1 },
    sun: { color: '#c8d4ff', intensity: 1.7, azimuth: 300, elevation: 70 },
    hemi: { sky: '#9fb0ff', ground: '#4a3a7e', intensity: 0.95 },
    fog: { color: '#2e2860', near: 110, far: 480 },
    exposure: 1.0,
    saturation: 1.1,
    env: 0.4,
    motes: { color: '#bfe8ff', rise: 0.08 },
    ground: { c1: '#1a1f4a', c2: '#283070', kind: 'dots', freq: 0.03, speed: 0, glow: false },
    island: { grass: '#4a5aa0', rock: '#2e2a58', leaves: '#6a7ae0' },
    birds: false,
    balloons: false,
  },
  circus: {
    id: 'circus',
    colors: {
      blue: '#4fa3ff',
      purple: '#a06bff',
      pink: '#ff6fb0',
      yellow: '#ffd23f',
      green: '#4fdc6a',
      white: '#fff8ef',
      orange: '#ff8a3d',
      red: '#ff4d5a',
      teal: '#2fd3c4',
    },
    patterns: ['stripes', 'dots', 'chevron'],
    sky: { top: '#58b9ff', horizon: '#ffe9c9', cloud: '#fffaf0', stars: 0 },
    sun: { color: '#fff0d0', intensity: 2.3, azimuth: 80, elevation: 73 },
    hemi: DAY_HEMI,
    fog: { color: '#fbe8d6', near: 120, far: 520 },
    exposure: 0.95,
    saturation: 1.12,
    env: 0.3,
    motes: { color: '#fff0c8', rise: 0.15 },
    ground: { c1: '#ffe3b0', c2: '#ffd08a', kind: 'stripes', freq: 0.02, speed: 0, glow: false },
    island: GREEN_ISLAND,
    birds: true,
    balloons: true,
  },
  neon: {
    id: 'neon',
    colors: {
      blue: '#3fd0ff',
      purple: '#b45cff',
      pink: '#ff4fcf',
      yellow: '#fff04f',
      green: '#4fff9f',
      white: '#e8e0ff',
      orange: '#ff9a3f',
      red: '#ff4f6f',
      teal: '#2ff5e0',
    },
    patterns: ['checker', 'chevron', 'stripes'],
    sky: { top: '#1b0f3d', horizon: '#ff5fa2', cloud: '#7a3a8a', stars: 0.6 },
    sun: { color: '#ffc0ec', intensity: 1.9, azimuth: 180, elevation: 66 },
    hemi: { sky: '#b0a0ff', ground: '#5a2a70', intensity: 0.95 },
    fog: { color: '#5a2a6a', near: 100, far: 460 },
    exposure: 1.0,
    saturation: 1.15,
    env: 0.4,
    motes: { color: '#ff9ff0', rise: 0.2 },
    ground: { c1: '#1a0f33', c2: '#44207a', kind: 'checker', freq: 0.04, speed: 0, glow: false },
    island: { grass: '#3a2a7a', rock: '#20143e', leaves: '#ff4fcf' },
    birds: false,
    balloons: false,
  },
  ocean: {
    id: 'ocean',
    colors: {
      blue: '#3fb6ff',
      purple: '#8f8fff',
      pink: '#ff9fbf',
      yellow: '#ffe27a',
      green: '#5fe0a0',
      white: '#fffaf0',
      orange: '#ffb06a',
      red: '#ff7070',
      teal: '#2fe0d0',
    },
    patterns: ['waves', 'stripes', 'dots'],
    sky: { top: '#45b0ff', horizon: '#e0fbff', cloud: '#ffffff', stars: 0 },
    sun: { color: '#fff6e0', intensity: 2.4, azimuth: 250, elevation: 72 },
    hemi: { sky: '#d0f0ff', ground: '#8fd0d8', intensity: 0.95 },
    fog: { color: '#d8f4fc', near: 130, far: 560 },
    exposure: 0.95,
    saturation: 1.08,
    env: 0.35,
    motes: { color: '#ffffff', rise: 0.1 },
    ground: { c1: '#1f9fd6', c2: '#37c1ec', kind: 'waves', freq: 0.035, speed: 0.4, glow: false },
    island: { grass: '#f2dca8', rock: '#b09070', leaves: '#4fcf6a' },
    birds: true,
    balloons: true,
  },
  desert: {
    id: 'desert',
    colors: {
      blue: '#6fb6d9',
      purple: '#b08fc9',
      pink: '#e8a0a0',
      yellow: '#f2cf6b',
      green: '#a8c46a',
      white: '#f6ead2',
      orange: '#e8944a',
      red: '#d8654a',
      teal: '#5fc0a8',
    },
    patterns: ['waves', 'chevron', 'stripes'],
    sky: { top: '#6cb2ea', horizon: '#ffe0b0', cloud: '#fff4e0', stars: 0 },
    sun: { color: '#ffe2b8', intensity: 2.6, azimuth: 140, elevation: 76 },
    hemi: { sky: '#e0ecff', ground: '#d8b080', intensity: 0.85 },
    fog: { color: '#f6e2c4', near: 110, far: 500 },
    exposure: 0.93,
    saturation: 1.04,
    env: 0.3,
    motes: { color: '#ffe8c0', rise: 0.3 },
    ground: { c1: '#e8c690', c2: '#f0d6a8', kind: 'waves', freq: 0.012, speed: 0, glow: false },
    island: { grass: '#e8c690', rock: '#b8764a', leaves: '#8fae5a' },
    birds: true,
    balloons: true,
  },
  jungle: {
    id: 'jungle',
    colors: {
      blue: '#5fb8e0',
      purple: '#9f7fd9',
      pink: '#ff8fb0',
      yellow: '#ffe05f',
      green: '#4fcf5f',
      white: '#f0f8e8',
      orange: '#ffa04f',
      red: '#ff6050',
      teal: '#3fd0a0',
    },
    patterns: ['dots', 'waves', 'chevron'],
    sky: { top: '#62c0f8', horizon: '#e4ffdc', cloud: '#f8fff4', stars: 0 },
    sun: { color: '#fff4c8', intensity: 2.2, azimuth: 330, elevation: 72 },
    hemi: { sky: '#dcffe8', ground: '#6a9a5a', intensity: 0.95 },
    fog: { color: '#dff4dc', near: 100, far: 470 },
    exposure: 0.95,
    saturation: 1.1,
    env: 0.3,
    motes: { color: '#f4ffc0', rise: 0.15 },
    ground: { c1: '#2f8f4f', c2: '#3fa85f', kind: 'dots', freq: 0.03, speed: 0, glow: false },
    island: { grass: '#4fbf4a', rock: '#7a5a3e', leaves: '#2f9f3f' },
    birds: true,
    balloons: false,
  },
  lava: {
    id: 'lava',
    colors: {
      blue: '#6a7fb0',
      purple: '#7a5a9a',
      pink: '#e0708a',
      yellow: '#ffc84a',
      green: '#8aa06a',
      white: '#e8dcd0',
      orange: '#ff7a2a',
      red: '#e8402a',
      teal: '#4aa8a0',
    },
    patterns: ['chevron', 'checker', 'waves'],
    sky: { top: '#3a2a3e', horizon: '#ff8a4a', cloud: '#6a4a4a', stars: 0.2 },
    sun: { color: '#ffb888', intensity: 2.2, azimuth: 90, elevation: 68 },
    hemi: { sky: '#ffd0b0', ground: '#6a2a2a', intensity: 0.9 },
    fog: { color: '#8a4a3a', near: 90, far: 420 },
    exposure: 1.0,
    saturation: 1.06,
    env: 0.35,
    motes: { color: '#ffa040', rise: 1.0 },
    ground: { c1: '#ff5a1a', c2: '#ffb030', kind: 'waves', freq: 0.03, speed: 0.25, glow: true },
    island: { grass: '#4a3a3a', rock: '#2a2020', leaves: '#8a4a2a' },
    birds: false,
    balloons: false,
  },
  royal: {
    id: 'royal',
    colors: {
      blue: '#6fa8ff',
      purple: '#9a6bff',
      pink: '#ff8fc0',
      yellow: '#ffd23f',
      green: '#6fd08a',
      white: '#fff6e6',
      orange: '#ffa04a',
      red: '#ff5a6a',
      teal: '#4fd6c8',
    },
    patterns: ['chevron', 'checker', 'stripes'],
    sky: { top: '#6a98e6', horizon: '#ffc89a', cloud: '#fff0e0', stars: 0 },
    sun: { color: '#ffd8a8', intensity: 2.3, azimuth: 260, elevation: 67 },
    hemi: { sky: '#ffe8d8', ground: '#b88ac0', intensity: 0.9 },
    fog: { color: '#f6d8c8', near: 120, far: 520 },
    exposure: 0.95,
    saturation: 1.08,
    env: 0.35,
    motes: { color: '#ffe8a0', rise: 0.2 },
    ground: { c1: '#9a6bd0', c2: '#b48ae0', kind: 'chevron', freq: 0.015, speed: 0, glow: false },
    island: GREEN_ISLAND,
    birds: true,
    balloons: true,
  },
  candy: {
    id: 'candy',
    colors: {
      blue: '#8fd8ff',
      purple: '#c8a8ff',
      pink: '#ff9fd0',
      yellow: '#ffec8f',
      green: '#a8f0b0',
      white: '#fff6fb',
      orange: '#ffc08f',
      red: '#ff7fa0',
      teal: '#8ff0e0',
    },
    patterns: ['dots', 'checker', 'stripes'],
    sky: { top: '#ff9fd6', horizon: '#fff0f8', cloud: '#fff0fb', stars: 0 },
    sun: { color: '#fff0f4', intensity: 2.2, azimuth: 10, elevation: 74 },
    hemi: { sky: '#ffe8f6', ground: '#d8b0f0', intensity: 1.0 },
    fog: { color: '#ffe4f2', near: 120, far: 520 },
    exposure: 0.93,
    saturation: 1.05,
    env: 0.3,
    motes: { color: '#ffffff', rise: 0.15 },
    ground: { c1: '#ffc2e0', c2: '#ffffff', kind: 'checker', freq: 0.02, speed: 0, glow: false },
    island: { grass: '#ffb0d8', rock: '#c88a6a', leaves: '#a8f0b0' },
    birds: false,
    balloons: true,
  },
};

/** Hue steps a round may shift a look by (few: materials are cached per colour). */
const HUE_STEPS = [-0.03, -0.015, 0, 0.015, 0.03];
const _c = new THREE.Color();

function shift(hex: string, dh: number, ds = 0, dl = 0): string {
  return `#${_c.set(hex).offsetHSL(dh, ds, dl).getHexString()}`;
}

/** A look with its palettes worked out and a hue shift (0 keeps its colours as designed). */
export function resolveLook(look: Look, hue = 0, pattern: PatternKind = look.patterns[0] ?? 'stripes'): ResolvedLook {
  const palette = {} as Record<PalKey, Palette>;
  for (const [k, c] of Object.entries(look.colors) as [PalKey, string][]) {
    const base = shift(c, hue);
    palette[k] = [base, shift(base, 0, -0.04, 0.07)];
  }
  // The original look keeps its exact second tones (PAL in builder.ts).
  return { ...look, palette, pattern };
}

/** The look of a round: one of the map's looks (its signature one most often), shifted a little. */
export function lookFor(ids: readonly LookId[] | undefined, seed: number): ResolvedLook {
  if (!ids?.length) return CLASSIC;
  const rng = mulberry32((seed ^ 0x100c5eed) >>> 0);
  const id = rng() < 0.55 || ids.length === 1 ? ids[0]! : ids[1 + Math.floor(rng() * (ids.length - 1))]!;
  const look = LOOKS[id];
  const hue = HUE_STEPS[Math.floor(rng() * HUE_STEPS.length)]!;
  const pattern = look.patterns[Math.floor(rng() * look.patterns.length)] ?? 'stripes';
  return resolveLook(look, hue, pattern);
}

export const CLASSIC: ResolvedLook = resolveLook(LOOKS.classic);
