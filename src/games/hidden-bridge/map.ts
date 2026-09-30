import * as THREE from 'three';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { z } from 'zod';
import { humanize, initBot, steer } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import {
  gloveAlley,
  movingPlatforms,
  pickSections,
  pistons,
  raceCourse,
  rotorDecks,
  type Segment,
  tippingBridge,
  withRests,
} from '../../sim/course';
import { type BotInput, type BotView, defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import { glovePuncher } from '../../sim/props';
import meta from './meta';

/**
 * Glass bridges with one safe pane per row. A fake pane is not solid: whoever steps on it falls
 * straight through (it shatters for everybody to see). Panes that held light up green for everybody. Gloves punch across some rows of the later bridges.
 */

const TileEvent = z.object({ i: z.number().int().min(0).max(255), at: z.number().optional() });
const PANE = 2.6;
const THICK = 0.3;
const SHARDS = 9;

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

function glassBridge(id: number, rows: number, cols: number, gloveRows: readonly number[] = []): Segment {
  return (s) => {
    const { b, rng, ctx } = s;
    const dx = 3;
    const dz = 3.2;
    const z0 = s.z + 1.3 + PANE / 2;
    const tiles: Tile[] = [];
    const rowsOf: Tile[][] = [];
    let c = Math.floor(rng() * cols);
    for (let r = 0; r < rows; r++) {
      if (r > 0) c = Math.max(0, Math.min(cols - 1, c + Math.floor(rng() * 3) - 1));
      const row: Tile[] = [];
      for (let k = 0; k < cols; k++) {
        const x = (k - (cols - 1) / 2) * dx;
        const zz = z0 + r * dz;
        const real = k === c;
        const tile: Tile = {
          i: tiles.length,
          row: r,
          col: k,
          x,
          z: zz,
          real,
          trusted: false,
          fallAt: null,
          collider: b.collider(
            b.anchor(x, s.y - THICK / 2, zz),
            { type: 'box', hx: PANE / 2, hy: THICK / 2, hz: PANE / 2 },
            { isStatic: true, trigger: !real },
          ),
        };
        const touched = () => {
          if (tile.real) {
            if (!tile.trusted && ctx.server) s.emit('safe', { i: tile.i });
            return;
          }
          if (tile.fallAt !== null || ctx.now() < 0) return;
          if (ctx.server) s.emit('tile', { i: tile.i, at: ctx.now() });
          else breakTile(tile, ctx.now());
        };
        tile.collider.onGround = touched;
        // Brushing a fake pane's side from the next pane does not break it.
        tile.collider.onTouch = (_c, n) => {
          if (n.y > 0.3) touched();
        };
        tiles.push(tile);
        row.push(tile);
      }
      rowsOf.push(row);
    }
    function breakTile(tile: Tile, at: number) {
      if (tile.real || tile.fallAt !== null) return;
      tile.fallAt = at;
      ctx.sfx('break');
    }
    const onTile = (name: string) => (data: unknown) => {
      const d = TileEvent.safeParse(data);
      const tile = d.success ? tiles[d.data.i] : undefined;
      if (!d.success || !tile) return;
      if (name === 'safe' && tile.real) tile.trusted = true;
      else if (name === 'tile' && !tile.real && d.data.at !== undefined) {
        // The server's time wins over the local guess (the collider follows it).
        if (tile.fallAt === null) breakTile(tile, d.data.at);
        else tile.fallAt = d.data.at;
      }
    };
    s.on('safe', onTile('safe'));
    s.on('tile', onTile('tile'));
    b.move((t) => {
      for (const tile of tiles) tile.collider.enabled = tile.fallAt === null || t < tile.fallAt;
    });
    const end = z0 + (rows - 1) * dz + PANE / 2;
    b.box(0, s.y - 1, s.z + 0.65, 16, 2, 1.3, PAL.purple);
    const half = ((cols - 1) / 2) * dx + PANE / 2 + 1.1;
    const gloves = gloveRows.map((row, k) => {
      const side = k % 2 ? 1 : -1;
      const rest = side * (half + 1.6);
      const gx = glovePuncher(b, {
        x: rest,
        y: s.y + 0.95,
        z: z0 + row * dz,
        side,
        w: 1.1 + rng() * 0.3,
        ph: rng() * 6,
        reach: half + 0.6,
        scale: 1.35,
        postTo: s.y - 4.8,
      });
      return { row, gx, rest };
    });

    if (b.view) {
      const view = b.view;
      const inst = view.instanced(
        'box',
        [PANE, THICK, PANE],
        view.plain('#ffffff', { roughness: 0.08, metalness: 0.1 }, 'glass'),
        tiles.length,
      );
      b.group.add(inst);
      const bar = (w: number, d: number, x: number, zz: number) =>
        new THREE.BoxGeometry(w, 0.14, d).translate(x, THICK / 2 + 0.02, zz);
      const e = PANE / 2 - 0.06;
      const frameGeo = view.own(
        mergeGeometries([bar(PANE, 0.12, 0, e), bar(PANE, 0.12, 0, -e), bar(0.12, PANE, e, 0), bar(0.12, PANE, -e, 0)])!,
      );
      const frames = view.own(new THREE.InstancedMesh(frameGeo, view.plain('#9aa3c7', {}, 'metal'), tiles.length));
      frames.frustumCulled = false;
      frames.castShadow = true;
      b.group.add(frames);
      const shards = view.own(
        new THREE.InstancedMesh(
          view.own(new THREE.BoxGeometry(0.7, 0.12, 0.7)),
          view.plain('#d9f3ff', { roughness: 0.05 }, 'glass'),
          tiles.length * SHARDS,
        ),
      );
      shards.frustumCulled = false;
      b.group.add(shards);
      // Girders along both sides (scenery, well outside the panes).
      for (const sx of [-1, 1])
        b.box(sx * half, s.y - 1.1, (z0 - PANE / 2 + end) / 2, 0.5, 0.7, end - z0 + PANE + 1.2, '#7d86ad', {
          noCollide: true,
          surface: 'metal',
        });
      const glass = new THREE.Color('#bfe9ff');
      const safe = new THREE.Color('#8ceaa2');
      const m = new THREE.Matrix4();
      const hidden = new THREE.Matrix4().makeScale(0, 0, 0);
      const q = new THREE.Quaternion();
      const eu = new THREE.Euler();
      const p = new THREE.Vector3();
      const one = new THREE.Vector3(1, 1, 1);
      // Shards fly apart in a fixed pattern per pane (visual only).
      const spread = Array.from({ length: SHARDS }, (_, k) => ({
        ox: ((k % 3) - 1) * 0.8,
        oz: (Math.floor(k / 3) - 1) * 0.8,
        spin: 3 + ((k * 7) % 5),
        up: 1 + ((k * 3) % 4) * 0.6,
      }));
      b.anim((t) => {
        for (const tile of tiles) {
          const gone = tile.fallAt !== null && t >= tile.fallAt;
          const y = s.y - THICK / 2;
          inst.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(tile.x, y, tile.z));
          frames.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(tile.x, y, tile.z));
          inst.setColorAt(tile.i, tile.trusted ? safe : glass);
          for (let k = 0; k < SHARDS; k++) {
            const f = gone ? t - tile.fallAt! : -1;
            if (f < 0 || f > 2.5) {
              shards.setMatrixAt(tile.i * SHARDS + k, hidden);
              continue;
            }
            const sp = spread[k]!;
            p.set(tile.x + sp.ox * (1 + f * 1.5), y + sp.up * f - 14 * f * f, tile.z + sp.oz * (1 + f * 1.5));
            q.setFromEuler(eu.set(f * sp.spin, f * sp.spin * 0.7, f * sp.spin * 0.3));
            shards.setMatrixAt(tile.i * SHARDS + k, m.compose(p, q, one));
          }
        }
        inst.instanceMatrix.needsUpdate = true;
        frames.instanceMatrix.needsUpdate = true;
        shards.instanceMatrix.needsUpdate = true;
        if (inst.instanceColor) inst.instanceColor.needsUpdate = true;
      });
    }
    b.box(0, s.y - 1, end + 0.1 + 3, 16, 2, 6, PAL.purple);

    const firstRow = (zz: number) => rowsOf.findIndex((r) => (r[0]?.z ?? 0) > zz + 0.5);
    const K = (n: string) => `${n}${id}`;
    // Like a player: pick a pane (a green one if any), hesitate before a guess, step straight across.
    const drive = (bot: BotView, out: BotInput) => {
      initBot(bot);
      const p = bot.body.pos;
      if (p.z > end + 0.4) return false;
      const ri = firstRow(p.z);
      if (ri < 0) {
        // Off the last pane straight ahead (a diagonal step could land on a fake one beside it).
        steer(bot, p.z < end + 0.5 ? p.x : (bot.mem.off ?? 0) * 3, end + 3, out, bot.mem.spd);
        return true;
      }
      const row = rowsOf[ri]!;
      const chosen = row.find((t) => t.col === bot.mem[K('col')]);
      if (bot.mem[K('row')] !== ri || !chosen || chosen.fallAt !== null) {
        bot.mem[K('row')] = ri;
        const cur = bot.mem[K('col')] ?? Math.floor(row.length / 2);
        const open = row.filter((t) => t.fallAt === null && Math.abs(t.col - cur) <= 1);
        const choices = open.length ? open : row.filter((t) => t.fallAt === null);
        const known = choices.find((t) => t.trusted);
        const real = choices.find((t) => t.real);
        // Nobody has stood on this row yet: a guess (a good eye sometimes spots the right pane).
        const eye = 0.1 + (bot.mem.skill ?? 0.7) * 0.12;
        const pick = known ?? (real && bot.rng() < eye ? real : choices[Math.floor(bot.rng() * choices.length)]);
        bot.mem[K('col')] = pick?.col ?? cur;
        bot.mem[K('wait')] = bot.t + (known ? 0.05 : 0.3 + bot.rng() * 0.6);
      }
      const target = row.find((t) => t.col === bot.mem[K('col')]) ?? row[0]!;
      const standing = bot.body.grounded && bot.body.groundCol !== null;
      if (bot.t < (bot.mem[K('wait')] ?? 0) && standing) {
        out.mx = 0;
        out.mz = 0;
        return true;
      }
      // A glove row ahead: step onto it only when the glove has pulled back for a while.
      const glove = gloves.find((g) => g.row === ri);
      if (glove && standing && [0, 0.3, 0.6, 0.9].some((dt) => Math.abs(glove.gx(bot.t + dt) - glove.rest) > 1.5)) {
        out.mx = 0;
        out.mz = 0;
        return true;
      }
      // Line up first, then step straight across: cutting the corner would cross a broken pane's hole.
      const on = tiles.find((t) => t.collider === bot.body.groundCol);
      const exitX = on ? Math.max(on.x - 0.95, Math.min(on.x + 0.95, target.x)) : target.x;
      const exitZ = (on ? on.z + PANE / 2 : target.z - PANE / 2 - 0.4) - 0.45;
      const aligned = Math.abs(p.x - exitX) < 0.35 || p.z > exitZ + 0.2;
      if (standing && !aligned && p.z < target.z - PANE / 2)
        steer(bot, exitX, Math.min(exitZ, Math.max(p.z, exitZ - 1)), out, 0.7);
      else steer(bot, target.x, target.z + 0.3, out, bot.mem.spd);
      humanize(bot, out, { precise: true });
      return true;
    };
    return {
      z: end + 6.1,
      y: s.y,
      routes: [
        [
          { x: 0, z: end + 1.5, w: 0.5, drive },
          { x: 0, z: end + 3.4, w: 1 },
        ],
      ],
      checkpoint: { from: end + 0.9, p: new THREE.Vector3(0, s.y + 0.1, end + 3.4) },
    };
  };
}

export default defineMap(meta, (b, ctx) => {
  b.style.pattern = 'checker';
  const mids = pickSections(b.rng, [rotorDecks(1), movingPlatforms(4), gloveAlley(3), pistons(3), tippingBridge(5)], 2);
  const rows = (lo: number) => lo + Math.floor(b.rng() * 3);
  return raceCourse(b, ctx, {
    sections: withRests(
      [glassBridge(0, rows(8), 5), mids[0]!, glassBridge(1, rows(6), 4, [2, 5]), mids[1]!, glassBridge(2, rows(5), 3, [1, 3])],
      6,
    ),
  });
});
