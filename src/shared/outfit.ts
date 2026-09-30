import { z } from 'zod';

/** What a bean wears besides its suit colour (visual only; chosen once and kept in the browser). */
export const HATS = [
  'none',
  'cap',
  'beanie',
  'party',
  'tophat',
  'cowboy',
  'viking',
  'propeller',
  'bunny',
  'cat',
  'horns',
  'halo',
  'flower',
  'antenna',
] as const;
export const GLASSES = ['none', 'round', 'shades', 'hearts', 'monocle', 'visor'] as const;
/** Colours of the hat, the belly and the shoes ('' leaves each its own default). */
export const TINTS = [
  '#ffffff',
  '#ffd23f',
  '#ff8a3d',
  '#ff3b3b',
  '#ff5fa2',
  '#a66bff',
  '#3fa9ff',
  '#39e0d0',
  '#4fdc6a',
  '#8b5a2b',
  '#9ea3b0',
  '#2b2b33',
] as const;

export type Hat = (typeof HATS)[number];
export type Glasses = (typeof GLASSES)[number];

const Tint = z.union([z.literal(''), z.enum(TINTS)]);
export const OutfitSchema = z.object({
  hat: z.enum(HATS),
  hatColor: Tint,
  glasses: z.enum(GLASSES),
  belly: Tint,
  shoes: Tint,
});
export type Outfit = z.infer<typeof OutfitSchema>;

export const DEFAULT_OUTFIT: Outfit = { hat: 'none', hatColor: '', glasses: 'none', belly: '', shoes: '' };

/** A stored outfit made valid (unknown parts fall back to the defaults). */
export function validOutfit(raw: unknown): Outfit {
  const o = { ...DEFAULT_OUTFIT, ...(raw && typeof raw === 'object' ? raw : {}) };
  const r = OutfitSchema.safeParse(o);
  return r.success ? r.data : { ...DEFAULT_OUTFIT };
}

/** A bot's outfit: picked from its id, so the same bot always looks the same. */
export function botOutfit(id: number): Outfit {
  const h = (n: number) => {
    const x = Math.imul((id * 31 + n + 1) ^ 0x5bd1e995, 0x9e3779b1);
    const y = Math.imul(x ^ (x >>> 15), 0x85ebca77);
    return (y ^ (y >>> 13)) >>> 0;
  };
  const pick = <T>(a: readonly T[], n: number) => a[h(n) % a.length]!;
  return {
    hat: h(0) % 3 === 0 ? 'none' : pick(HATS, 1),
    hatColor: h(2) % 2 ? '' : pick(TINTS, 3),
    glasses: h(4) % 3 ? 'none' : pick(GLASSES, 5),
    belly: '',
    shoes: h(6) % 2 ? '' : pick(TINTS, 7),
  };
}

export const sameOutfit = (a: Outfit, b: Outfit) =>
  a.hat === b.hat && a.hatColor === b.hatColor && a.glasses === b.glasses && a.belly === b.belly && a.shoes === b.shoes;
