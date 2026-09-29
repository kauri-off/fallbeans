import * as THREE from 'three';
import { z } from 'zod';
import { humanize, initBot, steer, unstick } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import meta from './meta';

const TileEvent = z.object({ i: z.number().int().min(0).max(255), at: z.number().optional() });
const FALL_DELAY = 0.35;
const SIZE = 2.6;

interface Tile {
  i: number;
  row: number;
  col: number;
  x: number;
  z: number;
  real: boolean;
  trusted: boolean;
  fallAt: number | null;
  collider: Collider;
}

export default defineMap(meta, (b, ctx) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 12.2, 18, 2, 10.4, PAL.purple);

  const tiles: Tile[] = [];
  const rowsOf: Tile[][] = [];

  function bridge(z0: number, rows: number, cols: number) {
    let c = Math.floor(b.rng() * cols);
    for (let r = 0; r < rows; r++) {
      if (r > 0) c = Math.max(0, Math.min(cols - 1, c + Math.floor(b.rng() * 3) - 1));
      const row: Tile[] = [];
      for (let k = 0; k < cols; k++) {
        const x = (k - (cols - 1) / 2) * 3;
        const z = z0 + r * 3;
        const real = k === c || b.rng() < 0.12;
        const tile: Tile = {
          i: tiles.length,
          row: rowsOf.length,
          col: k,
          x,
          z,
          real,
          trusted: false,
          fallAt: null,
          collider: b.collider(b.anchor(x, -0.3, z), { type: 'box', hx: SIZE / 2, hy: 0.3, hz: SIZE / 2 }, { isStatic: true }),
        };
        tile.collider.onGround = () => {
          if (tile.real) {
            if (!tile.trusted && ctx.server) ctx.emit('safe', { i: tile.i });
            return;
          }
          if (tile.fallAt !== null || ctx.now() < 0) return;
          if (ctx.server) ctx.emit('tile', { i: tile.i, at: ctx.now() + FALL_DELAY });
          else tile.fallAt = ctx.now() + FALL_DELAY;
        };
        tiles.push(tile);
        row.push(tile);
      }
      rowsOf.push(row);
    }
  }
  bridge(18.7, 12, 5);
  b.box(0, -1, 58, 14, 2, 10, PAL.purple);
  bridge(64.3, 6, 3);
  b.box(0, -1, 92.8, 18, 2, 24.4, PAL.yellow);
  b.finish(0, 0, 96);
  b.clouds(0, 50, 60, 36);

  b.move((t) => {
    for (const tile of tiles) tile.collider.enabled = tile.fallAt === null || t < tile.fallAt;
  });

  if (b.view) {
    const inst = b.view.instanced('box', [SIZE, 0.6, SIZE], b.view.plain('#ffffff', { roughness: 0.4 }, 'tile'), tiles.length);
    b.group.add(inst);
    const warn = new THREE.Color('#ff6b6b');
    const safe = new THREE.Color('#8ceaa2');
    const base = new THREE.Color('#bca4ff');
    const c = new THREE.Color();
    const m = new THREE.Matrix4();
    const hidden = new THREE.Matrix4().makeScale(0, 0, 0);
    b.anim((t) => {
      for (const tile of tiles) {
        let y = -0.3;
        let x = tile.x;
        if (tile.fallAt === null) c.copy(tile.trusted ? safe : base);
        else {
          const left = tile.fallAt - t;
          if (left > 0) {
            c.copy(base).lerp(warn, 1 - left / FALL_DELAY);
            x += Math.sin(t * 90 + tile.i) * 0.06;
          } else {
            const f = -left;
            y -= 12 * f * f;
            c.copy(warn);
          }
        }
        const gone = tile.fallAt !== null && t - tile.fallAt > 2;
        inst.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(x, y, tile.z));
        inst.setColorAt(tile.i, c);
      }
      inst.instanceMatrix.needsUpdate = true;
      if (inst.instanceColor) inst.instanceColor.needsUpdate = true;
    });
  }

  const firstRow = (z: number) => rowsOf.findIndex((r) => (r[0]?.z ?? 0) > z + 0.5);

  return {
    spawns,
    killY: -12,
    finish: { z: 96, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: 54, p: new THREE.Vector3(0, 0.1, 57) },
    ],
    onEvent(name, data) {
      const d = TileEvent.safeParse(data);
      if (!d.success) return;
      const tile = tiles[d.data.i];
      if (!tile) return;
      if (name === 'safe' && tile.real) tile.trusted = true;
      else if (name === 'tile' && !tile.real && d.data.at !== undefined) {
        const first = tile.fallAt === null;
        tile.fallAt = d.data.at;
        if (first || !ctx.server) ctx.sfx('break');
      }
    },
    bot(bot, out) {
      initBot(bot);
      const p = bot.body.pos;
      const ri = firstRow(p.z);
      if (ri < 0 || p.z > 84) {
        steer(bot, (bot.mem.off ?? 0) * 3, p.z < 60 ? 60 : 100, out, bot.mem.spd);
        humanize(bot, out, {});
        unstick(bot, out);
        return;
      }
      const row = rowsOf[ri]!;
      const chosen = row.find((t) => t.col === bot.mem.col);
      if (bot.mem.row !== ri || !chosen || chosen.fallAt !== null) {
        bot.mem.row = ri;
        const cur = bot.mem.col ?? Math.floor(row.length / 2);
        const open = row.filter((t) => t.fallAt === null && Math.abs(t.col - cur) <= 1);
        const choices = open.length ? open : row.filter((t) => t.fallAt === null);
        const known = choices.find((t) => t.trusted);
        const real = choices.find((t) => t.real);
        const pickT = known ?? (real && bot.rng() < 0.55 ? real : choices[Math.floor(bot.rng() * choices.length)]);
        bot.mem.col = pickT?.col ?? cur;
        bot.mem.wait = bot.t + 0.2 + bot.rng() * 0.4;
      }
      const target = row.find((t) => t.col === bot.mem.col) ?? row[0]!;
      if (bot.t < (bot.mem.wait ?? 0)) {
        out.mx = 0;
        out.mz = 0;
        return;
      }
      steer(bot, target.x, target.z + 0.4, out, bot.mem.spd);
      humanize(bot, out, { precise: true });
    },
  };
});
