import type { AnyGameMeta } from '../shared/game';
import ballHill from './ball-hill/meta';
import crownPeak from './crown-peak/meta';
import doorDash from './door-dash/meta';
import drumRoll from './drum-roll/meta';
import fruitMemory from './fruit-memory/meta';
import hammerSwing from './hammer-swing/meta';
import hexAGone from './hex-a-gone/meta';
import hiddenBridge from './hidden-bridge/meta';
import jumpClub from './jump-club/meta';
import plateDrop from './plate-drop/meta';
import rollOut from './roll-out/meta';
import tailTag from './tail-tag/meta';
import wallRush from './wall-rush/meta';

export const GAMES: readonly AnyGameMeta[] = [
  doorDash,
  hammerSwing,
  ballHill,
  hiddenBridge,
  drumRoll,
  jumpClub,
  rollOut,
  wallRush,
  fruitMemory,
  tailTag,
  hexAGone,
  crownPeak,
  plateDrop,
];

const byId = new Map(GAMES.map((g) => [g.id, g]));
if (byId.size !== GAMES.length) throw new Error('duplicate game id');

export function getGame(id: string): AnyGameMeta | undefined {
  return byId.get(id);
}

export const FINALS = GAMES.filter((g) => g.genre === 'final');
export const NON_FINALS = GAMES.filter((g) => g.genre !== 'final');
