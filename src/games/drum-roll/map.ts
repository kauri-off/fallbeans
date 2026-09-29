import * as THREE from 'three';
import { pathBrain, type Waypoint } from '../../sim/bots';
import { type Builder, PAL, type Palette } from '../../sim/builder';
import { type BotView, defineMap } from '../../sim/map';
import { armContactEta } from '../../sim/props';
import meta from './meta';

/**
 * A drum along x (rolling you forwards or back) or along z (a log rolling you sideways).
 * `spin` is the angular speed (rad/s).
 */
function drum(b: Builder, x: number, top: number, z: number, r: number, len: number, spin: number, pal: Palette, alongZ = false) {
  const axis = b.anchor(x, top - r, z);
  if (alongZ) axis.rotation.y = Math.PI / 2;
  const d = b.cyl(0, 0, 0, r, len, pal, { parent: axis, dynamic: true, rot: [0, 0, Math.PI / 2], seg: 36, tag: 'drum' });
  for (let k = 0; k < 8; k++) {
    const a = (k / 8) * Math.PI * 2;
    b.box(Math.cos(a) * r, 0, Math.sin(a) * r, 0.16, len - 0.2, 0.32, '#ffffff', {
      parent: d.obj,
      noCollide: true,
      castShadow: false,
    });
  }
  b.move((t) => {
    d.obj.rotation.set(Math.max(0, t) * spin, 0, Math.PI / 2);
  });
  return d;
}

// A: the drum staircase, zig-zagging and climbing; the tops roll back towards the start.
const STAIR = Array.from({ length: 5 }, (_, i) => ({ x: i % 2 ? 3 : -3, z: 17.3 + i * 4.8, top: 0.3 + i * 0.5 }));
const STAIR_R = 1.4;
const STAIR_LEN = 5;
/** Start of the platform after the staircase. */
const A_END = { z: STAIR.at(-1)!.z + STAIR_R + 0.05, y: STAIR.at(-1)!.top - 0.45 };

// B: the spinner deck.
const DECK_Z = 52;
const DECK_Y = A_END.y;
const lowAng = (t: number) => (t <= 0 ? 0 : t * 1.25);
const highAng = (t: number) => (t <= 0 ? Math.PI / 2 : Math.PI / 2 - t * 0.8);

// C: logs rolling sideways.
const LOGS = [
  { z: 72, x: 0, spin: 0.9 },
  { z: 82.4, x: 0, spin: -1.05 },
  { z: 92.8, x: 0, spin: 1.2 },
] as const;
const LOG_R = 1.8;
const LOG_LEN = 9;

export default defineMap(meta, (b) => {
  const spawns = b.startArea(0);
  // Reaches right up to the first drum (no gap to fall into).
  b.box(0, -1, 11.3, 16, 2, 8.6, PAL.purple);

  const pals = [PAL.orange, PAL.teal, PAL.pink, PAL.green];
  STAIR.forEach((s, i) => {
    drum(b, s.x, s.top, s.z, STAIR_R, STAIR_LEN, -1.25 - i * 0.08, pals[i % 4]!);
  });
  b.box(0, A_END.y - 1, A_END.z + 3, 23, 2, 6, PAL.purple);

  // B: a round deck with a low sweeper to jump and a high one to duck. Launch pads at the sides
  // lead up to narrow walkways over it: faster, but one slip and you are back at the checkpoint.
  b.cyl(0, DECK_Y - 1, DECK_Z, 8, 2, PAL.blue, { freq: 0.35 });
  b.box(0, DECK_Y - 1, 44.5, 5, 2, 3, PAL.yellow);
  b.hub(0, DECK_Y, DECK_Z, 1);
  b.rotor(0, DECK_Y + 0.6, DECK_Z, 7.6, 2, lowAng, 0.75);
  b.rotor(0, DECK_Y + 2.45, DECK_Z, 7.6, 1, highAng, 0.75);
  for (const sx of [-1, 1]) {
    b.box(sx * 9.5, DECK_Y - 1, A_END.z + 7, 3.5, 2, 2, PAL.yellow);
    b.pad(sx * 9.5, DECK_Y, A_END.z + 6.9, 1.1, 17);
    b.box(sx * 9.5, DECK_Y + 3.5, 54, 1.6, 1, 11, PAL.pink);
  }
  b.box(0, DECK_Y - 1, 63, 23, 2, 6, PAL.purple);

  // C: logs; you run along them while they roll you sideways, and hop across the gaps.
  LOGS.forEach((l, i) => {
    drum(b, l.x, DECK_Y, l.z, LOG_R, LOG_LEN, l.spin, pals[(i + 2) % 4]!, true);
  });
  b.box(0, DECK_Y - 1, 106.8, 18, 2, 18, PAL.yellow);
  b.finish(0, DECK_Y, 106);
  b.clouds(0, 60, 60, 36);

  // --- bots
  const path: Waypoint[] = [{ x: 0, z: 12, w: 2 }];
  STAIR.forEach((s) => {
    // Hop diagonally onto the inner end of each drum, taking off around the crest of the one before.
    path.push({
      x: s.x > 0 ? 1.3 : -1.3,
      z: s.z,
      w: 0.15,
      speed: 0.9,
      jumpWhen: (bot) => bot.body.pos.z > s.z - 5.6 && bot.body.pos.z < s.z - 4.4,
    });
  });
  path.push({
    x: 0,
    z: A_END.z + 1.5,
    w: 0.5,
    jumpWhen: (bot) => bot.body.pos.z > STAIR.at(-1)!.z + 0.3 && bot.body.pos.z < A_END.z,
  });
  // Across the deck, hopping the low arm; never into the high one.
  const deckJump = (bot: BotView) => {
    const p = bot.body.pos;
    const r = Math.hypot(p.x, p.z - DECK_Z);
    if (r > 8 || r < 1.3 || bot.t <= 0) return false;
    const eta = armContactEta(bot, lowAng(bot.t), 1.25, 2, 0, DECK_Z);
    const high = armContactEta(bot, highAng(bot.t), -0.8, 1, 0, DECK_Z);
    return eta > 0.1 && eta < 0.24 && high > 0.8;
  };
  path.push(
    { x: 2.8, z: DECK_Z - 4, w: 0.3, jumpWhen: deckJump },
    { x: 2.8, z: DECK_Z + 4, w: 0.3, jumpWhen: deckJump },
    { x: 0, z: 61, w: 0.5, jumpWhen: deckJump },
  );
  let edge = 65.6;
  for (const l of LOGS) {
    const start = l.z - LOG_LEN / 2;
    const e = edge;
    path.push({ x: l.x, z: start + 1.2, w: 0.1, jumpWhen: (bot) => bot.body.pos.z > e - 1.5 && bot.body.pos.z < e + 0.2 });
    path.push({ x: l.x, z: l.z + LOG_LEN / 2 - 2.2, w: 0 });
    edge = l.z + LOG_LEN / 2;
  }
  path.push({ x: 0, z: 100, w: 1, jumpWhen: (bot) => bot.body.pos.z > edge - 1.5 && bot.body.pos.z < edge + 0.2 });
  path.push({ x: 0, z: 110, w: 3 });

  return {
    spawns,
    killY: -12,
    finish: { z: 106, y: DECK_Y - 1 },
    // On the drum frames or the walkway rails? There are none: only the course counts.
    forbidden: (p) => p.y > DECK_Y + 6.5,
    checkpoints: [
      { z: -100, p: new THREE.Vector3(0, 0.1, 10) },
      { z: A_END.z + 0.5, p: new THREE.Vector3(0, A_END.y + 0.1, A_END.z + 2.5) },
      { z: 61, p: new THREE.Vector3(0, DECK_Y + 0.1, 63) },
    ],
    bot: pathBrain(path),
  };
});
