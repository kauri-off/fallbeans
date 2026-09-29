import * as THREE from 'three';
import type { Collider } from '../../client/engine/physics';
import { PAL } from '../../client/world/builder';
import { steer, unstick } from '../../client/world/bots';
import { defineMap } from '../../client/world/map';
import { AtEvent } from '../../shared/game';
import meta from './meta';

interface Tile {
  i: number;
  row: number;
  col: number;
  x: number;
  z: number;
  real: boolean;
  trusted: boolean;
  fallAt: number | null;
  mesh: THREE.Mesh;
  collider: Collider;
  requested: boolean;
}

export default defineMap(meta, (b, ctx) => {
  const spawns = b.startArea(0);
  b.box(0, -1, 12.2, 18, 2, 10.4, PAL.purple);

  const tiles: Tile[] = [];
  const rowsOf: Tile[][] = [];
  const warn = new THREE.Color('#ff6b6b');
  const safe = new THREE.Color('#8ceaa2');
  const base = new THREE.Color('#bca4ff');
  const tileGeo = new THREE.BoxGeometry(2.6, 0.6, 2.6);

  function bridge(z0: number, rows: number, cols: number) {
    let c = Math.floor(b.rng() * cols);
    for (let r = 0; r < rows; r++) {
      if (r > 0) c = Math.max(0, Math.min(cols - 1, c + Math.floor(b.rng() * 3) - 1));
      const row: Tile[] = [];
      for (let k = 0; k < cols; k++) {
        const x = (k - (cols - 1) / 2) * 3;
        const z = z0 + r * 3;
        const mesh = new THREE.Mesh(tileGeo, b.ownMaterial(new THREE.MeshStandardMaterial({ color: base, roughness: 0.4 })));
        mesh.position.set(x, -0.3, z);
        mesh.castShadow = mesh.receiveShadow = true;
        b.group.add(mesh);
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
          mesh,
          requested: false,
          collider: b.collider(mesh, { type: 'box', hx: 1.3, hy: 0.3, hz: 1.3 }),
        };
        tile.collider.onGround = (_c, body) => {
          if (tile.real) {
            tile.trusted = true;
            return;
          }
          if (!tile.requested) {
            tile.requested = true;
            ctx.emit('tile', { i: tile.i }, body.actor);
          }
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

  b.update(() => {
    const now = ctx.serverNow();
    for (const t of tiles) {
      const mat = t.mesh.material as THREE.MeshStandardMaterial;
      if (t.fallAt === null) {
        mat.color.copy(t.trusted ? safe : base);
        continue;
      }
      const left = t.fallAt - now;
      if (left > 0) {
        mat.color.copy(base).lerp(warn, 1 - left / 350);
        t.mesh.position.x = t.x + Math.sin(now * 0.09 + t.i) * 0.06;
      } else {
        t.collider.enabled = false;
        const f = -left / 1000;
        t.mesh.position.y = -0.3 - 12 * f * f;
        mat.color.copy(warn);
        t.mesh.visible = f < 2;
      }
    }
  });

  const firstRow = (z: number) => rowsOf.findIndex((r) => (r[0]?.z ?? 0) > z - 1.2);

  return {
    spawns,
    killY: -12,
    finish: { z: 96, y: -1 },
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: 54, p: new THREE.Vector3(0, 0.1, 57) },
    ],
    onEvent(name, data) {
      if (name !== 'at') return;
      const d = AtEvent.safeParse(data);
      if (!d.success) return;
      const t = tiles[d.data.i];
      if (t && !t.real && t.fallAt === null) {
        t.fallAt = d.data.at;
        t.requested = true;
        ctx.sfx('break');
      }
    },
    bot(bot, out) {
      const p = bot.body.pos;
      bot.mem.spd ??= 0.8 + bot.rng() * 0.2;
      const ri = firstRow(p.z);
      if (ri < 0 || p.z > 84) {
        steer(bot, 0, p.z < 60 ? 60 : 100, out, bot.mem.spd);
        unstick(bot, out, 1 / 20);
        return;
      }
      const row = rowsOf[ri]!;
      if (bot.mem.row !== ri) {
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
    },
  };
});
