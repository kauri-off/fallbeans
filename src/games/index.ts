import { type GameMeta, GameMetaSchema } from '../shared/game';
import type { MapModule } from '../sim/map';
import ballHill from './ball-hill/map';
import bouncePark from './bounce-park/map';
import cliffClimb from './cliff-climb/map';
import crownPeak from './crown-peak/map';
import doorDash from './door-dash/map';
import drumRoll from './drum-roll/map';
import frostSky from './frost-sky/map';
import hammerSwing from './hammer-swing/map';
import hexAGone from './hex-a-gone/map';
import hiddenBridge from './hidden-bridge/map';
import jumpClub from './jump-club/map';
import lobby from './lobby/map';
import plateDrop from './plate-drop/map';
import podium from './podium/map';
import portalPanic from './portal-panic/map';
import rollOut from './roll-out/map';
import starFall from './star-fall/map';
import tailTag from './tail-tag/map';
import wallRush from './wall-rush/map';

export const MAPS: readonly MapModule[] = [
  doorDash,
  hammerSwing,
  ballHill,
  hiddenBridge,
  drumRoll,
  jumpClub,
  rollOut,
  wallRush,
  tailTag,
  hexAGone,
  crownPeak,
  plateDrop,
  portalPanic,
  bouncePark,
  cliffClimb,
  frostSky,
  starFall,
];

export const GAMES: readonly GameMeta[] = MAPS.map((m) => m.meta);
export const LOBBY: MapModule = lobby;
export const PODIUM: MapModule = podium;

const byId = new Map(MAPS.map((m) => [m.meta.id, m]));
if (byId.size !== MAPS.length) throw new Error('duplicate game id');
for (const m of MAPS) {
  const r = GameMetaSchema.safeParse(m.meta);
  if (!r.success)
    throw new Error(`game ${m.meta.id}: ${r.error.issues.map((i) => `${i.path.join('.')} ${i.message}`).join('; ')}`);
}

export function getGame(id: string): GameMeta | undefined {
  return byId.get(id)?.meta;
}

export function getMap(id: string): MapModule | undefined {
  if (id === LOBBY.meta.id) return LOBBY;
  if (id === PODIUM.meta.id) return PODIUM;
  return byId.get(id);
}

export const FINALES = GAMES.filter((g) => g.finale);
