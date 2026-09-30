import type { Award } from '../../shared/protocol';

/** A player's numbers over a whole game. */
export interface GameStats {
  falls: number;
  kos: number;
  grabs: number;
  tackles: number;
  shortcuts: number;
  /** Rounds won (placed first). */
  wins: number;
  /** Places in race rounds, normalized to 0 (first) … 1 (last). */
  raceRanks: number[];
  /** Seconds survived in survival rounds (the full round when not eliminated). */
  survived: number;
}

export const emptyGameStats = (): GameStats => ({
  falls: 0,
  kos: 0,
  grabs: 0,
  tackles: 0,
  shortcuts: 0,
  wins: 0,
  raceRanks: [],
  survived: 0,
});

interface Candidate {
  id: number;
  s: GameStats;
}

const plural = (n: number, one: string, few: string, many: string) => {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return `${n} ${one}`;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return `${n} ${few}`;
  return `${n} ${many}`;
};

/** Fun titles at the end of a game; each goes to the single best player for it (ties: nobody). */
export function computeAwards(players: readonly Candidate[]): Award[] {
  const awards: Award[] = [];
  const best = (score: (c: Candidate) => number | null, min: number) => {
    let top: Candidate | null = null;
    let topScore = Number.NEGATIVE_INFINITY;
    let tie = false;
    for (const c of players) {
      const v = score(c);
      if (v === null || v < min) continue;
      if (v > topScore) {
        top = c;
        topScore = v;
        tie = false;
      } else if (v === topScore) tie = true;
    }
    return top && !tie ? { c: top, v: topScore } : null;
  };
  const add = (key: string, icon: string, title: string, r: { c: Candidate; v: number } | null, text: (v: number) => string) => {
    if (r) awards.push({ key, icon, title, id: r.c.id, text: text(r.v) });
  };
  add(
    'fastest',
    '⚡',
    'Молния',
    best((c) => (c.s.raceRanks.length ? 1 - c.s.raceRanks.reduce((a, b) => a + b, 0) / c.s.raceRanks.length : null), 0.5),
    () => 'лучшие места в гонках',
  );
  add(
    'survivor',
    '🛡️',
    'Несокрушимость',
    best((c) => Math.round(c.s.survived), 1),
    (v) => `${v} с в игре`,
  );
  add(
    'bully',
    '💥',
    'Задира',
    best((c) => c.s.kos, 1),
    (v) => plural(v, 'сбитый соперник', 'сбитых соперника', 'сбитых соперников'),
  );
  add(
    'grabber',
    '🤲',
    'Цепкие руки',
    best((c) => c.s.grabs, 3),
    (v) => plural(v, 'захват', 'захвата', 'захватов'),
  );
  add(
    'clumsy',
    '🍌',
    'Неваляшка',
    best((c) => c.s.falls, 2),
    (v) => plural(v, 'падение', 'падения', 'падений'),
  );
  add(
    'sly',
    '🦊',
    'Хитрая лиса',
    best((c) => c.s.shortcuts, 1),
    (v) => `${plural(v, 'срезка', 'срезки', 'срезок')} пути (и штрафы за них)`,
  );
  return awards;
}
