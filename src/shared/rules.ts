import type { Genre } from './game';
import type { RoundRow } from './protocol';

/** Placement points for first place; last place gets 0, the rest are spread evenly between. */
export const TOP_POINTS = 10;
export const FALL_PENALTY = 1;
export const SHORTCUT_PENALTY = 2;
/** Penalties never take more than this from one round. */
export const MAX_PENALTY = 4;
/** Continuous idle time (s) after which a player gets no placement points for the round. */
export const AFK_SECONDS = 20;

/** Per-player numbers the server counts during a round. */
export interface RoundStats {
  falls: number;
  shortcuts: number;
  /** Knockouts caused (others fell or were eliminated after our hit). */
  kos: number;
  grabs: number;
  tackles: number;
  /** Longest stretch without any input (s). */
  idle: number;
  /** Finish time in a race (s since the start). */
  finishAt: number | null;
  /** When the player was eliminated in a survival round (s since the start). */
  outAt: number | null;
}

export const emptyStats = (): RoundStats => ({
  falls: 0,
  shortcuts: 0,
  kos: 0,
  grabs: 0,
  tackles: 0,
  idle: 0,
  finishAt: null,
  outAt: null,
});

export interface RoundView {
  genre: Genre;
  participants: readonly number[];
  connected: (id: number) => boolean;
  /** In finishing order. */
  finished: readonly number[];
  /** In elimination order. */
  out: readonly number[];
  scores: ReadonlyMap<number, number>;
  progress: (id: number) => number;
  timeUp: boolean;
  solo: boolean;
}

const inRound = (r: RoundView) => r.participants.filter(r.connected);
const remaining = (r: RoundView) => inRound(r).filter((id) => !r.finished.includes(id) && !r.out.includes(id));

export function isRoundOver(r: RoundView): boolean {
  if (r.timeUp) return true;
  const rem = remaining(r).length;
  if (rem === 0) return true;
  // Survival: the last bean standing has nothing left to prove.
  return r.genre === 'survival' && !r.solo && rem <= 1;
}

/** Ranked groups, best first; players in one group tie. */
export function rankGroups(r: RoundView): number[][] {
  const ids = inRound(r);
  if (r.genre === 'race') {
    const fin = r.finished.filter((id) => ids.includes(id));
    const rest = ids.filter((id) => !fin.includes(id));
    return [...fin.map((id) => [id]), ...groupBy(rest, (id) => Math.round(r.progress(id)))];
  }
  if (r.genre === 'survival') {
    const outs = r.out.filter((id) => ids.includes(id));
    const stayed = ids.filter((id) => !outs.includes(id));
    return [...(stayed.length ? [stayed] : []), ...[...outs].reverse().map((id) => [id])];
  }
  return groupBy(ids, (id) => r.scores.get(id) ?? 0);
}

function groupBy(ids: readonly number[], key: (id: number) => number): number[][] {
  const sorted = [...ids].sort((a, b) => key(b) - key(a));
  const groups: number[][] = [];
  for (const id of sorted) {
    const last = groups.at(-1);
    if (last && key(last[0]!) === key(id)) last.push(id);
    else groups.push([id]);
  }
  return groups;
}

/** Placement points: ties share the average of the places they occupy. */
export function placementPoints(groups: readonly number[][]): Map<number, { place: number; points: number }> {
  const n = groups.reduce((s, g) => s + g.length, 0);
  const out = new Map<number, { place: number; points: number }>();
  let pos = 0;
  for (const g of groups) {
    const avg = pos + (g.length - 1) / 2;
    const points = n <= 1 ? TOP_POINTS : Math.round((TOP_POINTS * (n - 1 - avg)) / (n - 1));
    for (const id of g) out.set(id, { place: pos + 1, points });
    pos += g.length;
  }
  return out;
}

/** Did the player do what the round asked (finish, survive, score)? */
function succeeded(r: RoundView, id: number): boolean {
  if (r.genre === 'race') return r.finished.includes(id);
  if (r.genre === 'survival') return !r.out.includes(id);
  return (r.scores.get(id) ?? 0) > 0;
}

function note(r: RoundView, id: number, s: RoundStats): string {
  if (r.genre === 'race') return s.finishAt !== null ? `финиш за ${s.finishAt.toFixed(1).replace('.', ',')} с` : 'без финиша';
  if (r.genre === 'survival') return s.outAt !== null ? `в игре ${Math.floor(s.outAt)} с` : 'до конца раунда';
  return `очки: ${r.scores.get(id) ?? 0}`;
}

/**
 * Scores a finished round: placement points by rank (scaled to the number of players), minus
 * penalties for falls and shortcuts, capped. An idle (AFK) player gets no placement points.
 * `totals` are the game totals before this round; a total never drops below 0.
 */
export function scoreRound(
  r: RoundView,
  stats: ReadonlyMap<number, RoundStats>,
  totals: ReadonlyMap<number, number>,
  bots: ReadonlySet<number> = new Set(),
): RoundRow[] {
  const groups = rankGroups(r);
  const placed = placementPoints(groups);
  // Alone in a round there is nobody to rank against: points for doing the job.
  const soloPoints = (id: number) => (succeeded(r, id) ? TOP_POINTS : Math.round(TOP_POINTS / 2));
  const rows: RoundRow[] = [];
  for (const g of groups)
    for (const id of g) {
      const s = stats.get(id) ?? emptyStats();
      const p = placed.get(id)!;
      const afk = !bots.has(id) && s.idle >= AFK_SECONDS;
      const points = afk ? 0 : r.solo || groups.flat().length === 1 ? soloPoints(id) : p.points;
      // Falling out of a survival round already costs its place: only races and points games add a fine.
      const fallFine = r.genre === 'survival' ? 0 : s.falls * FALL_PENALTY;
      const penalty = Math.min(MAX_PENALTY, fallFine + s.shortcuts * SHORTCUT_PENALTY);
      const before = totals.get(id) ?? 0;
      const total = Math.max(0, before + points - penalty);
      rows.push({
        id,
        place: p.place,
        points,
        penalty,
        delta: total - before,
        total,
        ok: succeeded(r, id),
        afk,
        note: note(r, id, s),
        falls: s.falls,
      });
    }
  return rows;
}
