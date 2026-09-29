import type { RuleKind } from './game';
import type { RankEntry } from './protocol';
import { type Rng, shuffle } from './rng';

export interface RoundView {
  participants: readonly number[];
  connected: (id: number) => boolean;
  finished: readonly number[];
  out: readonly number[];
  scores: ReadonlyMap<number, number>;
  progress: (id: number) => number;
  eliminate: number;
  qualify: number;
  timeUp: boolean;
  solo: boolean;
}

export interface Outcome {
  ranking: RankEntry[];
  winner?: number;
}

export interface Rule {
  final: boolean;
  isOver(r: RoundView): boolean;
  outcome(r: RoundView, rng: Rng): Outcome;
}

const connectedParts = (r: RoundView) => r.participants.filter(r.connected);
const remaining = (r: RoundView) => connectedParts(r).filter((id) => !r.finished.includes(id) && !r.out.includes(id));

function split(
  order: number[],
  keep: number,
  points: (id: number, i: number) => number,
  note: (id: number, ok: boolean) => string,
) {
  return order.map((id, i) => {
    const ok = i < keep;
    return { id, ok, points: points(id, i), note: note(id, ok) };
  });
}

function withWinner(ranking: RankEntry[], winner: number | undefined): Outcome {
  return winner === undefined ? { ranking } : { ranking, winner };
}

const race: Rule = {
  final: false,
  isOver(r) {
    const parts = connectedParts(r);
    const done = r.finished.filter(r.connected).length;
    if (r.eliminate > 0 && done >= Math.min(r.qualify, parts.length)) return true;
    return remaining(r).length === 0 || r.timeUp;
  },
  outcome(r) {
    const parts = connectedParts(r);
    const fin = r.finished.filter((id) => parts.includes(id));
    const rest = parts.filter((id) => !fin.includes(id)).sort((a, b) => r.progress(b) - r.progress(a));
    const keep = r.eliminate > 0 ? r.qualify : parts.length;
    return {
      ranking: split(
        [...fin, ...rest],
        keep,
        (id, i) => (fin.includes(id) ? Math.max(1, 6 - i) : 0),
        (id) => (fin.includes(id) ? `финиш #${fin.indexOf(id) + 1}` : 'не добежал'),
      ),
    };
  },
};

const survival: Rule = {
  final: false,
  isOver(r) {
    if (r.eliminate > 0 && r.out.filter(r.connected).length >= r.eliminate) return true;
    return remaining(r).length === 0 || r.timeUp;
  },
  outcome(r) {
    const parts = connectedParts(r);
    const outs = r.out.filter((id) => parts.includes(id));
    const eliminated = r.eliminate > 0 ? outs.slice(0, r.eliminate) : [];
    const stayed = parts.filter((id) => !outs.includes(id));
    const order = [...stayed, ...outs.filter((id) => !eliminated.includes(id)).reverse(), ...[...eliminated].reverse()];
    return {
      ranking: split(
        order,
        parts.length - eliminated.length,
        (id) => (stayed.includes(id) ? 3 : 0),
        (id) => (stayed.includes(id) ? 'выстоял' : 'упал'),
      ),
    };
  },
};

const points: Rule = {
  final: false,
  isOver(r) {
    return remaining(r).length === 0 || r.timeUp;
  },
  outcome(r, rng) {
    const parts = shuffle(connectedParts(r), rng);
    const sc = (id: number) => r.scores.get(id) ?? 0;
    parts.sort((a, b) => sc(b) - sc(a));
    const keep = parts.length - Math.min(r.eliminate, parts.length);
    return {
      ranking: split(
        parts,
        keep,
        (id) => Math.max(0, sc(id)),
        (id) => `очки: ${sc(id)}`,
      ),
    };
  },
};

const lastStanding: Rule = {
  final: true,
  isOver(r) {
    const rem = remaining(r).length;
    return (!r.solo && rem <= 1) || rem === 0 || r.timeUp;
  },
  outcome(r, rng) {
    const parts = connectedParts(r);
    const rem = shuffle(remaining(r), rng);
    const outs = r.out.filter((id) => parts.includes(id)).reverse();
    const order = [...rem, ...outs];
    return withWinner(
      split(
        order,
        1,
        (_id, i) => (i === 0 ? 10 : 0),
        (_id, ok) => (ok ? 'победитель' : 'упал'),
      ),
      order[0],
    );
  },
};

const raceFinal: Rule = {
  final: true,
  isOver(r) {
    return r.finished.some(r.connected) || remaining(r).length === 0 || r.timeUp;
  },
  outcome(r) {
    const parts = connectedParts(r);
    const fin = r.finished.filter((id) => parts.includes(id));
    const rest = parts.filter((id) => !fin.includes(id)).sort((a, b) => r.progress(b) - r.progress(a));
    const order = [...fin, ...rest];
    return withWinner(
      split(
        order,
        1,
        (_id, i) => (i === 0 ? 10 : 0),
        (_id, ok) => (ok ? 'схватил корону' : ''),
      ),
      order[0],
    );
  },
};

export const RULES: Record<RuleKind, Rule> = { race, survival, points, lastStanding, raceFinal };
