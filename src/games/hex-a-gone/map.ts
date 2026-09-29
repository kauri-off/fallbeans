import * as THREE from 'three';
import { z } from 'zod';
import { humanize, initBot, unstick } from '../../sim/bots';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import meta from './meta';

const TileEvent = z.object({ i: z.number().int().min(0).max(4095), at: z.number() });

const SIZE = 1.5;
const RINGS = 8;
const THICK = 0.5;
const FLOORS = [0, -10, -20];
const FLOOR_COLORS = ['#ff8cc8', '#7ccfff', '#ffd84a'];
const FALL_DELAY = 0.5;
const SQ3 = Math.sqrt(3);

interface Tile {
  i: number;
  floor: number;
  x: number;
  y: number;
  z: number;
  fallAt: number | null;
  col: Collider;
}

export default defineMap(meta, (b, ctx) => {
  const tiles: Tile[] = [];
  const index = new Map<string, Tile>();
  FLOORS.forEach((y, floor) => {
    for (let q = -RINGS; q <= RINGS; q++)
      for (let r = -RINGS; r <= RINGS; r++) {
        if (Math.abs(q + r) > RINGS) continue;
        const x = SIZE * SQ3 * (q + r / 2);
        const zz = SIZE * 1.5 * r;
        const tile: Tile = {
          i: tiles.length,
          floor,
          x,
          y,
          z: zz,
          fallAt: null,
          col: b.collider(b.anchor(x, y - THICK / 2, zz), { type: 'cyl', r: SIZE * 0.92, hh: THICK / 2 }, { isStatic: true }),
        };
        tile.col.onGround = () => {
          // Nothing falls before the start.
          if (tile.fallAt !== null || ctx.now() < 0) return;
          const at = ctx.now() + FALL_DELAY;
          if (ctx.server) ctx.emit('tile', { i: tile.i, at });
          else tile.fallAt = at;
        };
        tiles.push(tile);
        index.set(`${floor}:${q}:${r}`, tile);
      }
  });

  b.move((t) => {
    for (const tile of tiles) tile.col.enabled = tile.fallAt === null || t < tile.fallAt;
  });

  if (b.view) {
    const inst = b.view.instanced(
      'cyl',
      [SIZE * 0.97, THICK, 6],
      b.view.plain('#ffffff', { roughness: 0.35 }, 'tile'),
      tiles.length,
    );
    b.group.add(inst);
    const cols = FLOOR_COLORS.map((c) => new THREE.Color(c));
    const warn = new THREE.Color('#ffffff');
    const c = new THREE.Color();
    const m = new THREE.Matrix4();
    const hidden = new THREE.Matrix4().makeScale(0, 0, 0);
    b.anim((t) => {
      for (const tile of tiles) {
        const base = cols[tile.floor]!;
        let y = tile.y - THICK / 2;
        let gone = false;
        if (tile.fallAt === null) c.copy(base);
        else {
          const left = tile.fallAt - t;
          if (left > 0) {
            c.copy(base).lerp(warn, 1 - left / FALL_DELAY);
            y -= (1 - left / FALL_DELAY) * 0.08;
          } else {
            y -= 14 * left * left;
            gone = -left > 1.2;
            c.copy(base).multiplyScalar(0.7);
          }
        }
        inst.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(tile.x, y, tile.z));
        inst.setColorAt(tile.i, c);
      }
      inst.instanceMatrix.needsUpdate = true;
      if (inst.instanceColor) inst.instanceColor.needsUpdate = true;
    });
  }
  b.clouds(0, 0, 55, 30, -45, -26);

  /** The tile under (x, z) on a floor, if any (axial rounding). */
  function tileAt(x: number, zz: number, floor: number): Tile | undefined {
    const qf = ((SQ3 / 3) * x - zz / 3) / SIZE;
    const rf = ((2 / 3) * zz) / SIZE;
    const sf = -qf - rf;
    let q = Math.round(qf);
    let r = Math.round(rf);
    const s = Math.round(sf);
    const dq = Math.abs(q - qf);
    const dr = Math.abs(r - rf);
    const ds = Math.abs(s - sf);
    if (dq > dr && dq > ds) q = -r - s;
    else if (dr > ds) r = -q - s;
    return index.get(`${floor}:${q}:${r}`);
  }
  const intact = (tile: Tile | undefined, t: number) => !!tile && (tile.fallAt === null || tile.fallAt > t + 0.2);
  const floorOf = (y: number) => {
    let best = 0;
    FLOORS.forEach((fy, i) => {
      if (Math.abs(y - fy) < Math.abs(y - FLOORS[best]!)) best = i;
    });
    return best;
  };

  const spawns = b.ringSpawns(8, 6, 0.1, Math.PI / 8);
  return {
    spawns,
    killY: -30,
    faceCenter: true,
    view: new THREE.Vector3(0, -6, 0),
    onEvent(name, data) {
      if (name !== 'tile') return;
      const d = TileEvent.safeParse(data);
      const tile = d.success ? tiles[d.data.i] : undefined;
      if (!d.success || !tile) return;
      tile.fallAt = d.data.at;
    },
    bot(bot, out) {
      initBot(bot);
      const p = bot.body.pos;
      const floor = floorOf(p.y);
      const speed = bot.mem.spd ?? 1;
      // Keep moving (tiles drop half a second after being touched) towards intact ground,
      // preferring directions with more intact tiles ahead and staying away from the rim.
      const heading = bot.mem.heading ?? bot.rng() * Math.PI * 2;
      const t = bot.t;
      let best = heading;
      let bestScore = -1e9;
      for (let k = 0; k < 12; k++) {
        const a = heading + (k / 12) * Math.PI * 2;
        let score = 0;
        for (let d = 1.4; d <= 5.6; d += 1.4) {
          const tile = tileAt(p.x + Math.sin(a) * d, p.z + Math.cos(a) * d, floor);
          if (!intact(tile, t + d / 8)) {
            score -= d < 2 ? 12 : 6 / d;
            break;
          }
          score += 1;
        }
        score -= Math.abs(Math.atan2(Math.sin(a - heading), Math.cos(a - heading))) * 0.6;
        const ex = p.x + Math.sin(a) * 4;
        const ez = p.z + Math.cos(a) * 4;
        score -= Math.max(0, Math.hypot(ex, ez) - SIZE * 1.5 * (RINGS - 1)) * 2;
        score += (bot.rng() - 0.5) * 0.4;
        if (score > bestScore) {
          bestScore = score;
          best = a;
        }
      }
      bot.mem.heading = best;
      out.mx = Math.sin(best) * speed * 0.7;
      out.mz = Math.cos(best) * speed * 0.7;
      // Hop along: tiles only drop where we land.
      if (bot.body.grounded && t > 0 && bot.rng() < 0.8) out.jump = true;
      // A gap right ahead: jump it (if there is ground beyond).
      const ahead = tileAt(p.x + Math.sin(best) * 1.5, p.z + Math.cos(best) * 1.5, floor);
      const beyond = tileAt(p.x + Math.sin(best) * 3.2, p.z + Math.cos(best) * 3.2, floor);
      if (bot.body.grounded && !intact(ahead, t + 0.1) && intact(beyond, t + 0.4)) out.jump = true;
      humanize(bot, out, { precise: true });
      unstick(bot, out);
    },
  };
});
