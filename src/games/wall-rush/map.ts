import * as THREE from 'three';
import { humanize, initBot, steer } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import type { Collider } from '../../sim/physics';
import meta from './meta';

type Seg = 'solid' | 'gap' | 'low' | 'high';
const SEGS = 5;
const SEG_W = 4;
const START_Z = -22;
const END_Z = 16;

interface Wall {
  at: number;
  speed: number;
  segs: Seg[];
  group: THREE.Object3D;
  cols: Collider[];
}

export default defineMap(meta, (b) => {
  b.style.pattern = 'waves';
  b.box(0, -1, 0, 20, 2, 16, PAL.blue, { freq: 0.3 });
  b.box(0, 0.02, -7.6, 20, 0.05, 0.6, PAL.red, { noCollide: true });
  b.box(0, 0.02, 7.6, 20, 0.05, 0.6, PAL.red, { noCollide: true });
  for (const sx of [-1, 1]) b.box(sx * 10.4, 0.6, 0, 0.8, 1.2, 16, PAL.pink);

  const walls: Wall[] = [];
  let at = 0.5;
  for (let k = 0; k < 40 && at < meta.duration; k++) {
    const segs: Seg[] = Array.from({ length: SEGS }, () => 'solid');
    const passCount = k < 3 ? 2 : k < 8 && b.rng() < 0.25 ? 2 : 1;
    for (let n = 0; n < passCount; n++) {
      const kinds: Seg[] = k < 2 ? ['gap'] : ['gap', 'low', 'high', 'low'];
      segs[Math.floor(b.rng() * SEGS)] = kinds[Math.floor(b.rng() * kinds.length)]!;
    }
    if (!segs.some((s) => s !== 'solid')) segs[0] = 'gap';
    const speed = 3.8 + Math.min(5.6, k * 0.33);
    const group = b.anchor(0, 0, START_Z);
    group.visible = false;
    const pal = [PAL.orange, PAL.purple, PAL.green, PAL.pink][k % 4]!;
    const cols = segs.flatMap((s, i) => {
      const x = (i - (SEGS - 1) / 2) * SEG_W;
      if (s === 'gap') return [];
      if (s === 'solid') return [b.box(x, 2, 0, SEG_W, 4, 1, pal, { parent: group, dynamic: true }).col];
      if (s === 'low') return [b.box(x, 0.55, 0, SEG_W, 1.1, 1, PAL.yellow, { parent: group, dynamic: true }).col];
      return [b.box(x, 3, 0, SEG_W, 2, 1, pal, { parent: group, dynamic: true }).col];
    });
    walls.push({ at, speed, segs, group, cols });
    at += Math.max(1.7, 4.4 - k * 0.19);
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
  b.clouds(0, 0, 40);

  const spawns = Array.from({ length: 8 }, (_, i) => new THREE.Vector3(-7 + i * 2, 0.1, 1));

  return {
    spawns,
    killY: -8,
    view: new THREE.Vector3(0, 2, 0),
    bot(bot, out) {
      initBot(bot);
      const p = bot.body.pos;
      const t = bot.t;
      bot.mem.home ??= -3 + bot.rng() * 6;
      const next = walls.find((w) => t >= w.at - 1 && wallZ(w, t) < p.z + 0.6);
      if (!next) {
        steer(bot, p.x, 2, out, bot.mem.spd);
        humanize(bot, out, {});
        return;
      }
      const wi = walls.indexOf(next);
      if (bot.mem.wall !== wi) {
        bot.mem.wall = wi;
        const options = next.segs.map((s, i) => ({ s, i })).filter((o) => o.s !== 'solid');
        const skill = bot.rng();
        const choice =
          skill < 0.85
            ? options.reduce((a, o) => (Math.abs(o.i - 2 - p.x / SEG_W) < Math.abs(a.i - 2 - p.x / SEG_W) ? o : a))
            : options[0];
        bot.mem.seg = choice?.i ?? 0;
      }
      const seg = next.segs[bot.mem.seg ?? 0];
      const tx = ((bot.mem.seg ?? 0) - (SEGS - 1) / 2) * SEG_W;
      const dz = p.z - wallZ(next, t);
      steer(bot, tx, Math.max(-5, Math.min(5, bot.mem.home ?? 0)), out, bot.mem.spd);
      if (seg === 'low' && dz < 1.6 && dz > 0.4 && bot.body.grounded && Math.abs(p.x - tx) < SEG_W / 2) out.jump = true;
      humanize(bot, out, { precise: dz < 4, fun: dz > 6 });
    },
  };
});
