export type Rng = () => number;

export function mulberry32(seed: number): Rng {
  let a = seed | 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function shuffle<T>(a: T[], rng: Rng = Math.random): T[] {
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [a[i], a[j]] = [a[j] as T, a[i] as T];
  }
  return a;
}

export function pick<T>(a: readonly T[], rng: Rng = Math.random): T {
  if (!a.length) throw new Error('pick from empty array');
  return a[Math.floor(rng() * a.length)] as T;
}

export const randInt = (lo: number, hi: number, rng: Rng = Math.random) => lo + Math.floor(rng() * (hi - lo + 1));
