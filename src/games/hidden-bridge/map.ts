import * as THREE from 'three';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { z } from 'zod';
import { humanize, initBot, steer, unstick } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import { armContactEta, glovePuncher } from '../../sim/props';
import meta from './meta';

/**
 * Two glass bridges with one safe pane per row. A fake pane gives way the instant anyone touches it
 * (no time to step back), so whoever goes first falls; panes that held light up for everybody.
 * On the second bridge boxing gloves punch across two rows: a knock sideways lands you on a pane
 * nobody has tried.
 */

const TileEvent = z.object({ i: z.number().int().min(0).max(255), at: z.number().optional() });
const PANE = 2.6;
const THICK = 0.3;
const SHARDS = 9;

/** Section A: wide, rows touching (a step apart). */
const A = { z0: 19.1, rows: 10, cols: 5, dx: 3, dz: 3.2 };
/** Mid platform with a slow sweeper. */
const MID = { z0: 49.6, z1: 58, hubZ: 54.6 };
/** Section B: narrower, with punching gloves. */
const B = { z0: 60, rows: 7, cols: 4, dx: 3, dz: 3.2 };
/** Gloves punching across section B's rows (row index, side they come from, rhythm). */
const GLOVES = [
  { row: 2, side: -1, w: 1.3, ph: 0 },
  { row: 5, side: 1, w: 1.1, ph: 1.9 },
] as const;
const GLOVE_REST = 7.6;
/** Glove x: resting beside the bridge, then a quick punch across half of it. */
const gloveX = (g: (typeof GLOVES)[number], t: number) =>
  g.side * (GLOVE_REST - 6.4 * Math.max(0, Math.sin(Math.max(0, t) * g.w + g.ph)) ** 3);
const FINISH_Z = 86;
const sweep = (t: number) => (t <= 0 ? 0 : t * 0.9);

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
  b.style.pattern = 'checker';
  const spawns = b.startArea(0);
  b.box(0, -1, 12.2, 18, 2, 10.4, PAL.purple);

  const tiles: Tile[] = [];
  const rowsOf: Tile[][] = [];

  function bridge(s: typeof A | typeof B) {
    let c = Math.floor(b.rng() * s.cols);
    for (let r = 0; r < s.rows; r++) {
      if (r > 0) c = Math.max(0, Math.min(s.cols - 1, c + Math.floor(b.rng() * 3) - 1));
      const row: Tile[] = [];
      for (let k = 0; k < s.cols; k++) {
        const x = (k - (s.cols - 1) / 2) * s.dx;
        const zz = s.z0 + r * s.dz;
        const tile: Tile = {
          i: tiles.length,
          row: rowsOf.length,
          col: k,
          x,
          z: zz,
          real: k === c,
          trusted: false,
          fallAt: null,
          collider: b.collider(
            b.anchor(x, -THICK / 2, zz),
            { type: 'box', hx: PANE / 2, hy: THICK / 2, hz: PANE / 2 },
            { isStatic: true },
          ),
        };
        const touched = () => {
          if (tile.real) {
            if (!tile.trusted && ctx.server) ctx.emit('safe', { i: tile.i });
            return;
          }
          if (tile.fallAt !== null || ctx.now() < 0) return;
          // Gives way at once: the collider goes with the next tick.
          if (ctx.server) ctx.emit('tile', { i: tile.i, at: ctx.now() });
          else breakTile(tile, ctx.now());
        };
        tile.collider.onGround = touched;
        // Landing on its rim counts too (brushing its side from the next pane does not).
        tile.collider.onTouch = (_c, n) => {
          if (n.y > 0.3) touched();
        };
        tiles.push(tile);
        row.push(tile);
      }
      rowsOf.push(row);
    }
  }
  function breakTile(tile: Tile, at: number) {
    if (tile.real || tile.fallAt !== null) return;
    tile.fallAt = at;
    ctx.sfx('break');
  }

  bridge(A);
  const aRows = rowsOf.length;
  const aEnd = A.z0 + (A.rows - 1) * A.dz + PANE / 2;
  b.box(0, -1, (MID.z0 + MID.z1) / 2, 16, 2, MID.z1 - MID.z0, PAL.purple);
  b.hub(0, 0, MID.hubZ, 0.8);
  b.rotor(0, 0.6, MID.hubZ, 4.4, 1, sweep, 0.7);
  bridge(B);
  const bEnd = B.z0 + (B.rows - 1) * B.dz + PANE / 2;
  for (const g of GLOVES)
    glovePuncher(b, {
      x: g.side * GLOVE_REST,
      y: 0.95,
      z: B.z0 + g.row * B.dz,
      side: g.side,
      w: g.w,
      ph: g.ph,
      reach: 6.4,
      scale: 1.35,
      postTo: -4.8,
    });
  b.box(0, -1, (bEnd + 0.4 + 98) / 2, 18, 2, 98 - bEnd - 0.4, PAL.yellow);
  b.finish(0, 0, FINISH_Z);
  b.clouds(0, 50, 60, 36);

  b.move((t) => {
    for (const tile of tiles) tile.collider.enabled = tile.fallAt === null || t < tile.fallAt;
  });

  if (b.view) {
    const view = b.view;
    // Glass panes (instanced, coloured per pane), their frames, and shards of the broken ones.
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
    const shardGeo = view.own(new THREE.BoxGeometry(0.7, 0.12, 0.7));
    const shards = view.own(
      new THREE.InstancedMesh(shardGeo, view.plain('#d9f3ff', { roughness: 0.05 }, 'glass'), tiles.length * SHARDS),
    );
    shards.frustumCulled = false;
    b.group.add(shards);
    // Girders along both sides (scenery, well outside the panes).
    for (const [z0, z1, half] of [
      [A.z0 - PANE / 2, aEnd, ((A.cols - 1) / 2) * A.dx + PANE / 2 + 1.1],
      [B.z0 - PANE / 2, bEnd, ((B.cols - 1) / 2) * B.dx + PANE / 2 + 1.1],
    ] as const)
      for (const sx of [-1, 1])
        b.box(sx * half, -1.1, (z0 + z1) / 2, 0.5, 0.7, z1 - z0 + 1.2, '#7d86ad', { noCollide: true, surface: 'metal' });

    const glass = new THREE.Color('#bfe9ff');
    const safe = new THREE.Color('#8ceaa2');
    const c = new THREE.Color();
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
    for (const tile of tiles) inst.setColorAt(tile.i, glass);
    b.anim((t) => {
      for (const tile of tiles) {
        const gone = tile.fallAt !== null && t >= tile.fallAt;
        c.copy(tile.trusted ? safe : glass);
        inst.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(tile.x, -THICK / 2, tile.z));
        frames.setMatrixAt(tile.i, gone ? hidden : m.makeTranslation(tile.x, -THICK / 2, tile.z));
        inst.setColorAt(tile.i, c);
        for (let k = 0; k < SHARDS; k++) {
          const f = gone ? t - tile.fallAt! : -1;
          if (f < 0 || f > 2.5) {
            shards.setMatrixAt(tile.i * SHARDS + k, hidden);
            continue;
          }
          const s = spread[k]!;
          p.set(tile.x + s.ox * (1 + f * 1.5), -THICK / 2 + s.up * f - 14 * f * f, tile.z + s.oz * (1 + f * 1.5));
          q.setFromEuler(eu.set(f * s.spin, f * s.spin * 0.7, f * s.spin * 0.3));
          shards.setMatrixAt(tile.i * SHARDS + k, m.compose(p, q, one));
        }
      }
      inst.instanceMatrix.needsUpdate = true;
      frames.instanceMatrix.needsUpdate = true;
      shards.instanceMatrix.needsUpdate = true;
      if (inst.instanceColor) inst.instanceColor.needsUpdate = true;
    });
  }

  const firstRow = (zz: number) => rowsOf.findIndex((r) => (r[0]?.z ?? 0) > zz + 0.5);
  const midJump = (bot: BotView) => {
    const pp = bot.body.pos;
    const r = Math.hypot(pp.x, pp.z - MID.hubZ);
    if (r > 5.6 || bot.t <= 0) return false;
    const eta = armContactEta(bot, sweep(bot.t), 0.9, 1, 0, MID.hubZ);
    return eta > 0.08 && eta < 0.22;
  };

  return {
    spawns,
    killY: -12,
    finish: { z: FINISH_Z, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: MID.z0 + 1, p: new THREE.Vector3(-5.5, 0.1, MID.z0 + 1.6) },
    ],
    onEvent(name, data) {
      const d = TileEvent.safeParse(data);
      if (!d.success) return;
      const tile = tiles[d.data.i];
      if (!tile) return;
      if (name === 'safe' && tile.real) tile.trusted = true;
      else if (name === 'tile' && !tile.real && d.data.at !== undefined) {
        // The server's time wins over the local guess (the collider follows it).
        if (tile.fallAt === null) breakTile(tile, d.data.at);
        else tile.fallAt = d.data.at;
      }
    },
    bot(bot, out) {
      initBot(bot);
      const p = bot.body.pos;
      const ri = firstRow(p.z);
      // Off the bridges: to the next one (hopping the sweeper on the mid platform), or to the line.
      if (ri < 0 || p.z > bEnd + 0.5 || (ri === aRows && p.z < MID.z1 - 0.5)) {
        const tz = ri < 0 || p.z > bEnd ? 98 : ri === aRows ? MID.z1 + 0.4 : A.z0 - PANE / 2 - 0.3;
        steer(bot, ri === aRows ? (bot.mem.off ?? 0) * 3 - 3.5 : (bot.mem.off ?? 0) * 3, tz, out, bot.mem.spd);
        if (midJump(bot) && bot.body.grounded) out.jump = true;
        humanize(bot, out, { precise: ri === aRows });
        unstick(bot, out);
        return;
      }
      const row = rowsOf[ri]!;
      const inB = ri >= aRows;
      const chosen = row.find((t) => t.col === bot.mem.col);
      if (bot.mem.row !== ri || !chosen || chosen.fallAt !== null) {
        bot.mem.row = ri;
        const cur = bot.mem.col ?? Math.floor(row.length / 2);
        const open = row.filter((t) => t.fallAt === null && Math.abs(t.col - cur) <= 1);
        const choices = open.length ? open : row.filter((t) => t.fallAt === null);
        const known = choices.find((t) => t.trusted);
        const real = choices.find((t) => t.real);
        // Nobody has stood on this row yet: a guess (a good eye sometimes spots the right pane).
        const eye = 0.1 + (bot.mem.skill ?? 0.7) * 0.12;
        const pick = known ?? (real && bot.rng() < eye ? real : choices[Math.floor(bot.rng() * choices.length)]);
        bot.mem.col = pick?.col ?? cur;
        // Weighing it up: longer when it is a guess.
        bot.mem.wait = bot.t + (known ? 0.05 : 0.3 + bot.rng() * 0.6);
      }
      const target = row.find((t) => t.col === bot.mem.col) ?? row[0]!;
      const standing = bot.body.grounded && bot.body.groundCol !== null;
      if (bot.t < (bot.mem.wait ?? 0) && standing) {
        out.mx = 0;
        out.mz = 0;
        return;
      }
      // A glove row ahead: step onto it only when the glove has pulled back for a while.
      const glove = inB ? GLOVES.find((g) => g.row === ri - aRows) : undefined;
      if (glove && standing && [0, 0.3, 0.6, 0.9].some((dt) => Math.abs(gloveX(glove, bot.t + dt)) < GLOVE_REST - 1.5)) {
        out.mx = 0;
        out.mz = 0;
        return;
      }
      // Line up first (on our own pane, or at the platform edge), then step straight across: cutting
      // the corner would cross the hole of a pane that already broke.
      const on = tiles.find((t) => t.collider === bot.body.groundCol);
      const exitX = on ? Math.max(on.x - 0.95, Math.min(on.x + 0.95, target.x)) : target.x;
      const exitZ = (on ? on.z + PANE / 2 : target.z - PANE / 2 - 0.4) - 0.45;
      const aligned = Math.abs(p.x - exitX) < 0.35 || p.z > exitZ + 0.2;
      if (standing && !aligned && p.z < target.z - PANE / 2)
        steer(bot, exitX, Math.min(exitZ, Math.max(p.z, exitZ - 1)), out, 0.7);
      else steer(bot, target.x, target.z + 0.3, out, bot.mem.spd);
      humanize(bot, out, { precise: true });
    },
  };
});
