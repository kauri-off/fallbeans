import * as THREE from 'three';
import { humanize, initBot, steer } from '../../sim/bots';
import { PAL, type Palette } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import meta from './meta';

/**
 * Walls rush across the platform, fast, each one different: pieces of every width, low ones to
 * jump, high bars to run under, windows to jump through, full blocks, and gaps. Find your way
 * through (or over) in time, or get swept off.
 */

/** What a stretch of wall is: solid, a gap, low (jump it), a bar overhead (run under), a window (jump through). */
type Kind = 'solid' | 'gap' | 'low' | 'bar' | 'window';
const W = 20;
const START_Z = -58;
const END_Z = 16;
/** Jumpable heights: a jump clears about 1.9 m. */
const LOW_H = 1.0;
const BAR_Y = 1.85;

interface Piece {
  kind: Kind;
  x0: number;
  x1: number;
}

interface Wall {
  at: number;
  speed: number;
  pieces: Piece[];
  group: THREE.Object3D;
  cols: Collider[];
}

export default defineMap(
  meta,
  (b) => {
    const rng = b.rng;
    b.box(0, -1, 0, W, 2, 16, PAL.blue, { freq: 0.3 });
    b.box(0, 0.02, -7.6, W, 0.05, 0.6, PAL.red, { noCollide: true });
    b.box(0, 0.02, 7.6, W, 0.05, 0.6, PAL.red, { noCollide: true });
    for (const sx of [-1, 1]) b.box(sx * (W / 2 + 0.4), 0.6, 0, 0.8, 1.2, 16, PAL.pink);
    for (let k = 0; k < 4; k++) b.bonus(-6 + k * 4, 0, -3 + (k % 2) * 6);

    const walls: Wall[] = [];
    const pals: Palette[] = [PAL.orange, PAL.purple, PAL.green, PAL.pink, PAL.teal];
    let at = 0.6;
    for (let k = 0; k < 90 && at < meta.duration; k++) {
      // Split the width into 3–6 pieces of random widths (at least 2.4 m, room for a bean).
      const n = 3 + Math.floor(rng() * 4);
      const cuts = Array.from({ length: n - 1 }, () => rng()).sort((a, c) => a - c);
      const edges = [0, ...cuts, 1].map((f) => -W / 2 + f * W);
      const pieces: Piece[] = [];
      for (let i = 0; i < n; i++) {
        const x0 = edges[i]!;
        const x1 = edges[i + 1]!;
        if (x1 - x0 < 1.2 && pieces.length) {
          pieces.at(-1)!.x1 = x1;
          continue;
        }
        pieces.push({ kind: 'solid', x0, x1 });
      }
      // Ways through: early walls have two, later ones one (sometimes two); what kind, by chance.
      const ways = k < 4 ? 2 : rng() < 0.3 ? 2 : 1;
      const kinds: Kind[] = k < 2 ? ['gap'] : ['gap', 'low', 'bar', 'window', 'low', 'bar'];
      const wide = pieces.filter((p) => p.x1 - p.x0 >= 2.4);
      for (let w = 0; w < ways && wide.length; w++) {
        const p = wide.splice(Math.floor(rng() * wide.length), 1)[0]!;
        p.kind = kinds[Math.floor(rng() * kinds.length)]!;
      }
      if (!pieces.some((p) => p.kind !== 'solid')) pieces[0]!.kind = 'gap';
      // Three times the old pace: from 11 m/s up to about 19.
      const speed = 11 + Math.min(8, k * 0.22) + rng() * 1.5;
      const group = b.anchor(0, 0, START_Z);
      group.visible = false;
      const pal = pals[k % pals.length]!;
      const thick = 0.8 + rng() * 1.2;
      const cols: Collider[] = [];
      for (const p of pieces) {
        const x = (p.x0 + p.x1) / 2;
        const w = p.x1 - p.x0;
        const add = (y: number, h: number, c: Palette) =>
          cols.push(b.box(x, y + h / 2, 0, w, h, thick, c, { parent: group, dynamic: true, tag: 'wall' }).col);
        if (p.kind === 'solid') add(0, 4.2, pal);
        else if (p.kind === 'low') add(0, LOW_H, PAL.yellow);
        else if (p.kind === 'bar') add(BAR_Y, 4.2 - BAR_Y, pal);
        else if (p.kind === 'window') {
          // A hole to jump through: sill at 0.9 m, lintel at 3 m.
          add(0, 0.9, PAL.yellow);
          add(3, 1.2, pal);
        }
      }
      walls.push({ at, speed, pieces, group, cols });
      at += Math.max(1.5, 3.4 - k * 0.07) + rng() * 0.6;
    }
    const wallZ = (w: Wall, t: number) => START_Z + (t - w.at) * w.speed;
    b.move((t) => {
      for (const w of walls) {
        const z = wallZ(w, t);
        const on = t >= w.at && z < END_Z;
        w.group.visible = on;
        w.group.position.z = on ? z : START_Z;
        for (const c of w.cols) c.enabled = on;
      }
    });
    b.clouds(0, -20, 50);

    const spawns = Array.from({ length: 8 }, (_, i) => new THREE.Vector3(-7 + i * 2, 0.1, 2));

    return {
      spawns,
      killY: -8,
      view: new THREE.Vector3(0, 2, -6),
      bot(bot, out) {
        initBot(bot);
        const p = bot.body.pos;
        const t = bot.t;
        bot.mem.home ??= 1 + bot.rng() * 4;
        // The next wall that has not passed us yet (and is on its way).
        const next = walls.find((w) => t >= w.at - 0.5 && wallZ(w, t) < p.z + 0.8);
        if (!next) {
          steer(bot, p.x, bot.mem.home ?? 2, out, bot.mem.spd);
          humanize(bot, out, {});
          return;
        }
        const wi = walls.indexOf(next);
        if (bot.mem.wall !== wi) {
          bot.mem.wall = wi;
          const options = next.pieces.map((pc, i) => ({ pc, i })).filter((o) => o.pc.kind !== 'solid');
          const mid = (o: (typeof options)[number]) => (o.pc.x0 + o.pc.x1) / 2;
          const skill = bot.rng();
          const choice =
            skill < 0.85 ? options.reduce((a, o) => (Math.abs(mid(o) - p.x) < Math.abs(mid(a) - p.x) ? o : a)) : options[0];
          bot.mem.seg = choice?.i ?? 0;
        }
        const pc = next.pieces[bot.mem.seg ?? 0] ?? next.pieces[0]!;
        const tx = Math.max(pc.x0 + 0.7, Math.min(pc.x1 - 0.7, (pc.x0 + pc.x1) / 2));
        const eta = (p.z - wallZ(next, t)) / next.speed;
        steer(bot, tx, Math.max(-5, Math.min(5, bot.mem.home ?? 2)), out, 1);
        // Over a low wall or through a window: jump just before it arrives.
        const inside = p.x > pc.x0 + 0.4 && p.x < pc.x1 - 0.4;
        if (
          (pc.kind === 'low' || pc.kind === 'window') &&
          inside &&
          bot.body.grounded &&
          eta < 0.2 + (bot.mem.react ?? 0.2) * 0.3 &&
          eta > 0.08
        )
          out.jump = true;
        humanize(bot, out, { precise: eta < 2, fun: eta > 3 });
      },
    };
  },
  ['desert', 'factory', 'castle'],
);
