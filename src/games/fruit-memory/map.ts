import * as THREE from 'three';
import { shuffle } from '../../shared/rng';
import { humanize, initBot, navTo } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import meta from './meta';

export const FRUITS = ['🍎', '🍌', '🍇', '🍉', '🍓', '🍍'] as const;
const FRUIT_BG = ['#ffd6d6', '#fff4c2', '#e7d6ff', '#d8f5d0', '#ffd6e8', '#ffe9c2'];
const GRID = 5;
const TILE = 3.3;
const STEP = 3.6;
const HIDE = 2;
const REVEAL = 3;
const DROP = 2.5;
const RESTORE = 1.5;

type PhaseName = 'wait' | 'look' | 'hide' | 'reveal' | 'drop' | 'restore';

interface Round {
  start: number;
  look: number;
  fruit: number[];
  target: number;
}

interface Tile {
  x: number;
  z: number;
  obj: THREE.Object3D;
  col: Collider;
}

export default defineMap(meta, (b, ctx) => {
  b.box(0, -3, -14, 20, 2, 4, PAL.purple);
  const tiles: Tile[] = [];
  const half = (GRID - 1) / 2;
  for (let i = 0; i < GRID; i++)
    for (let k = 0; k < GRID; k++) {
      const x = (i - half) * STEP;
      const z = (k - half) * STEP;
      const p = b.box(x, -0.5, z, TILE, 1, TILE, '#ffffff', {
        dynamic: true,
        material: b.view?.own(new THREE.MeshStandardMaterial()),
      });
      tiles.push({ x, z, obj: p.obj, col: p.col });
    }

  // The whole schedule follows from the seed: server and clients agree without events.
  const rounds: Round[] = [];
  let start = 2;
  for (let r = 0; start < meta.duration; r++) {
    const look = Math.max(2.5, 6 - r * 0.5);
    const kinds = Math.min(FRUITS.length, 3 + r);
    const target = Math.floor(b.rng() * kinds);
    const fruit = shuffle(
      Array.from({ length: tiles.length }, (_, i) => (i < 3 ? target : Math.floor(b.rng() * kinds))),
      b.rng,
    );
    rounds.push({ start, look, fruit, target });
    start += look + HIDE + REVEAL + DROP + RESTORE;
  }

  function phaseAt(t: number): { round: Round | null; phase: PhaseName; left: number } {
    for (const r of rounds) {
      let s = t - r.start;
      if (s < 0) break;
      for (const [phase, len] of [
        ['look', r.look],
        ['hide', HIDE],
        ['reveal', REVEAL],
        ['drop', DROP],
        ['restore', RESTORE],
      ] as const) {
        if (s < len) return { round: r, phase, left: len - s };
        s -= len;
      }
    }
    return { round: null, phase: 'wait', left: 0 };
  }

  b.move((t) => {
    const { round, phase, left } = phaseAt(t);
    tiles.forEach((tile, i) => {
      const falls = !!round && phase === 'drop' && round.fruit[i] !== round.target;
      tile.col.enabled = !falls;
      const drop = falls
        ? DROP - left
        : phase === 'restore' && round && round.fruit[i] !== round.target
          ? Math.max(0, left - 0.5)
          : 0;
      tile.obj.position.y = -0.5 - Math.min(10, 6 * drop * drop);
    });
  });

  if (b.view) {
    const view = b.view;
    const tex = FRUITS.map((f, i) => view.own(view.emojiTexture(f, FRUIT_BG[i]!)));
    const blank = view.own(view.emojiTexture('', '#f4f1ff'));
    const board = new THREE.Mesh(view.own(new THREE.PlaneGeometry(9, 9)), view.own(new THREE.MeshBasicMaterial({ map: blank })));
    board.position.set(0, 6, -14);
    b.group.add(board);
    const question = view.own(view.emojiTexture('❓', '#fff4d6'));
    let lastKey = '';
    b.anim((t) => {
      const { round, phase } = phaseAt(t);
      const key = `${rounds.indexOf(round as Round)}:${phase}`;
      if (key === lastKey) return;
      lastKey = key;
      tiles.forEach((tile, i) => {
        if (!(tile.obj instanceof THREE.Mesh)) return;
        const mat = tile.obj.material as THREE.MeshStandardMaterial;
        const f = round?.fruit[i];
        mat.map = phase === 'look' && f !== undefined ? tex[f]! : blank;
        mat.needsUpdate = true;
      });
      const bm = board.material;
      bm.map = round && (phase === 'reveal' || phase === 'drop') ? tex[round.target]! : phase === 'hide' ? question : blank;
      bm.needsUpdate = true;
      if (phase === 'reveal') ctx.sfx('warn');
    });
  }
  b.clouds(0, 0, 40);

  return {
    spawns: b.ringSpawns(8, 4, 0.1, Math.PI / 8),
    killY: -8,
    view: new THREE.Vector3(0, 1, 0),
    hud() {
      const { round, phase, left } = phaseAt(ctx.now());
      if (!round) return ctx.now() < 0 ? null : 'Приготовьтесь…';
      if (phase === 'look') return `Запоминайте, где какой фрукт: ${Math.ceil(left)}`;
      if (phase === 'hide') return 'Где что лежало?..';
      if (phase === 'reveal') return `Встаньте на ${FRUITS[round.target]} — ${Math.ceil(left)}`;
      if (phase === 'drop') return `${FRUITS[round.target]}!`;
      return null;
    },
    bot(bot, out) {
      initBot(bot);
      const { round, phase } = phaseAt(bot.t);
      const ri = round ? rounds.indexOf(round) : -1;
      // Takes a moment to read the board and decide.
      if (round && (phase === 'reveal' || phase === 'drop') && bot.mem.round !== ri) {
        bot.mem.seenAt ??= bot.t;
        if (bot.t - (bot.mem.seenAt ?? 0) < (bot.mem.react ?? 0.2) * 3) {
          humanize(bot, out, {});
          return;
        }
        bot.mem.seenAt = undefined;
        bot.mem.round = ri;
        const skill = 0.55 + ((bot.id * 37) % 40) / 100;
        const good = tiles.map((_, i) => i).filter((i) => round.fruit[i] === round.target);
        const any = tiles.map((_, i) => i);
        const pool = bot.rng() < skill ? good : any;
        const p = bot.body.pos;
        pool.sort((a, c) => Math.hypot(tiles[a]!.x - p.x, tiles[a]!.z - p.z) - Math.hypot(tiles[c]!.x - p.x, tiles[c]!.z - p.z));
        bot.mem.tile = pool[Math.floor(bot.rng() * Math.min(2, pool.length))] ?? 12;
      }
      const choosing = phase === 'reveal' || phase === 'drop';
      // Between rounds: mill about the middle (a little apart from each other).
      const idle = tiles[12]!;
      const w = bot.t * 0.4 + (bot.mem.ph ?? 0);
      const target = choosing ? tiles[bot.mem.tile ?? 12] : null;
      if (target) navTo(bot, target.x + (bot.mem.off ?? 0) * 0.6, target.z, out, bot.mem.spd ?? 1, 0.5);
      else navTo(bot, idle.x + Math.sin(w) * 3 + (bot.mem.off ?? 0) * 2, idle.z + Math.cos(w * 0.8) * 3, out, 0.6, 0.8);
      humanize(bot, out, { fun: !choosing, rough: !choosing });
    },
  };
});
