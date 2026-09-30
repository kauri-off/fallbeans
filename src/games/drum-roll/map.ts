import * as THREE from 'three';
import type { Waypoint } from '../../sim/bots';
import { type Builder, PAL, type Palette } from '../../sim/builder';
import {
  edgeJump,
  hammerBridges,
  pickSections,
  raceCourse,
  rotorDecks,
  type Segment,
  trampolineGap,
  withRests,
} from '../../sim/course';
import { type BotView, defineMap } from '../../sim/map';
import meta from './meta';

/**
 * A drum along x (rolling you forwards or back) or along z (a log rolling you sideways), turned by
 * `angle(t)`. Pegs (boxes on the surface) knock over whoever they catch.
 */
function drum(
  b: Builder,
  x: number,
  top: number,
  z: number,
  r: number,
  len: number,
  angle: (t: number) => number,
  pal: Palette,
  o: { alongZ?: boolean; pegs?: number } = {},
) {
  const axis = b.anchor(x, top - r, z);
  if (o.alongZ) axis.rotation.y = Math.PI / 2;
  const d = b.cyl(0, 0, 0, r, len, pal, { parent: axis, dynamic: true, rot: [0, 0, Math.PI / 2], seg: 36, tag: 'drum' });
  for (let k = 0; k < 8; k++) {
    const a = (k / 8) * Math.PI * 2;
    b.box(Math.cos(a) * r, 0, Math.sin(a) * r, 0.16, len - 0.2, 0.32, '#ffffff', {
      parent: d.obj,
      noCollide: true,
      castShadow: false,
    });
  }
  // Pegs: bars across the drum that come round and sweep the top.
  for (let k = 0; k < (o.pegs ?? 0); k++) {
    const a = (k / (o.pegs ?? 1)) * Math.PI * 2 + 0.4;
    b.box(Math.cos(a) * (r + 0.2), 0, Math.sin(a) * (r + 0.2), 0.4, len - 1.2, 0.4, PAL.red, {
      parent: d.obj,
      dynamic: true,
      hit: 0.9,
      tag: 'peg',
      rot: [0, a, 0],
    });
  }
  b.move((t) => {
    d.obj.rotation.set(angle(t), 0, Math.PI / 2);
  });
  return d;
}

/** Spin that speeds up and slows down (and may reverse): rate base + amp·sin(w t + ph); the angle is its integral. */
function pulse(base: number, amp: number, w: number, ph: number) {
  const angle = (t: number) => {
    const tt = Math.max(0, t);
    return base * tt - (amp / w) * (Math.cos(tt * w + ph) - Math.cos(ph));
  };
  const rate = (t: number) => (t <= 0 ? 0 : base + amp * Math.sin(t * w + ph));
  return { angle, rate };
}

const pals = [PAL.orange, PAL.teal, PAL.pink, PAL.green];

/**
 * A zig-zag staircase of drums rolling back at you, their speed surging and easing (wait for a slow
 * moment), or a launch pad to a narrow walkway over them (faster, one slip and you are back).
 */
function drumStairs(): Segment {
  return (s) => {
    const { b, rng } = s;
    b.box(0, s.y - 1, s.z + 2, 16, 2, 4, PAL.purple);
    const R = 1.4;
    const stairs = Array.from({ length: 6 }, (_, i) => ({
      x: i % 2 ? 3 : -3,
      z: s.z + 5.8 + i * 4.8,
      top: s.y + 0.3 + i * 0.5,
      spin: pulse(-(1.1 + rng() * 0.3), 0.45 + rng() * 0.35, 0.7 + rng() * 0.5, rng() * 6),
    }));
    stairs.forEach((st, i) => {
      drum(b, st.x, st.top, st.z, R, 5, st.spin.angle, pals[i % 4]!);
    });
    const last = stairs.at(-1)!;
    const endZ = last.z + R + 0.05;
    const endY = last.top - 0.45;
    b.box(0, endY - 1, endZ + 3, 23, 2, 6, PAL.purple);
    // The walkway: launch pads at the sides, up to a narrow beam over the drums.
    for (const sx of [-1, 1]) {
      b.box(sx * 9.5, s.y - 1, s.z + 3.5, 3.5, 2, 3, PAL.yellow);
      b.pad(sx * 9.5, s.y, s.z + 3.4, 1.1, 17);
    }
    const beamY = s.y + 3.5;
    const beamZ0 = s.z + 5.5;
    b.box(9.5, beamY, (beamZ0 + endZ) / 2, 2.2, 1, endZ - beamZ0, PAL.pink);
    b.bonus(9.5, beamY + 0.5, (beamZ0 + endZ) / 2);

    const stairsRoute: Waypoint[] = [{ x: 0, z: s.z + 1.5, w: 1 }];
    stairs.forEach((st) => {
      stairsRoute.push({
        x: st.x > 0 ? 1.3 : -1.3,
        z: st.z,
        w: 0.15,
        speed: 0.9,
        jumpWhen: (bot) => bot.body.pos.z > st.z - 5.6 && bot.body.pos.z < st.z - 4.4,
      });
    });
    stairsRoute.push({
      x: 0,
      z: endZ + 1.5,
      w: 0.5,
      jumpWhen: (bot: BotView) => bot.body.pos.z > last.z + 0.3 && bot.body.pos.z < endZ,
    });
    const beamRoute: Waypoint[] = [
      { x: 6.5, z: s.z + 2.5, w: 0 },
      { x: 9.5, z: s.z + 3.4, w: 0 },
      { x: 9.5, z: beamZ0 + 2, w: 0 },
      { x: 9.5, z: endZ - 0.5, w: 0 },
      { x: 5, z: endZ + 2, w: 0.3 },
    ];
    for (const r of [stairsRoute, beamRoute]) r.push({ x: 0, z: endZ + 3, w: 1 });
    return {
      z: endZ + 6,
      y: endY,
      routes: [stairsRoute, stairsRoute, beamRoute],
      forbidden: (p) => p.y > beamY + 3,
      checkpoint: { from: endZ + 0.5, p: new THREE.Vector3(0, endY + 0.1, endZ + 3) },
    };
  };
}

/** Logs rolling sideways, each on its own rhythm, reversing now and then; run along them and hop the gaps. */
function logRun(n = 5): Segment {
  return (s) => {
    const { b, rng } = s;
    const R = 1.8;
    const L = 9;
    let zz = s.z + 2;
    b.box(0, s.y - 1, s.z + 1, 12, 2, 2, PAL.purple);
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 1 }];
    let edge = s.z + 2;
    for (let i = 0; i < n; i++) {
      const c = zz + 1.4 + L / 2;
      const dir = rng() < 0.5 ? -1 : 1;
      // Some logs reverse (the rate swings through zero), some just surge.
      const reverse = rng() < 0.4;
      const sp = reverse
        ? pulse(0, 1.4 + rng() * 0.4, 0.5 + rng() * 0.3, rng() * 6)
        : pulse(dir * (1 + rng() * 0.5), 0.5, 0.9, rng() * 6);
      drum(b, 0, s.y, c, R, L, sp.angle, pals[(i + 2) % 4]!, { alongZ: true });
      const e = edge;
      route.push({ x: 0, z: c - L / 2 + 1.2, w: 0.1, jumpWhen: (bot) => bot.body.pos.z > e - 1.5 && bot.body.pos.z < e + 0.2 });
      route.push({ x: 0, z: c + L / 2 - 2.2, w: 0 });
      edge = c + L / 2;
      zz = c + L / 2;
    }
    b.bonus(0, s.y + 0.1, s.z + 2 + 1.4 + L + 1.4 + L / 2);
    zz += 1.4;
    b.box(0, s.y - 1, zz + 3, 14, 2, 6, PAL.purple);
    const e = edge;
    route.push({ x: 0, z: zz + 3, w: 1, jumpWhen: (bot) => bot.body.pos.z > e - 1.5 && bot.body.pos.z < e + 0.2 });
    return {
      z: zz + 6,
      y: s.y,
      routes: [route],
      checkpoint: { from: zz + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zz + 3) },
    };
  };
}

/** Big drums rolling towards you with pegs across them: jump each peg as it comes over the top. */
function pegDrums(n = 3): Segment {
  return (s) => {
    const { b, rng } = s;
    const R = 2.2;
    let zz = s.z + 2;
    b.box(0, s.y - 1, s.z + 1, 12, 2, 2, PAL.purple);
    const route: Waypoint[] = [{ x: 0, z: s.z + 1, w: 1 }];
    for (let i = 0; i < n; i++) {
      // Room for the pegs (they stand 0.4 m proud) between drums and platforms.
      const c = zz + 0.55 + R;
      const w = -(0.55 + rng() * 0.25);
      const ph = rng() * 6;
      const pegs = 3;
      const ang = (t: number) => ph + Math.max(0, t) * w;
      drum(b, 0, s.y + 0.2, c, R, 9, ang, pals[i % 4]!, { pegs });
      // Pegs sit at angle β = offset + ang(t) on the drum (β = 0 on top, growing towards +z) and come
      // round to the bot at |w| rad/s: jump just before one reaches it.
      const axisY = s.y + 0.2 - R;
      const margin = 0.7 / (R + 0.2);
      const jumpWhen = (bot: BotView) => {
        const p = bot.body.pos;
        if (Math.abs(p.z - c) > R + 0.6 || bot.t <= 0) return false;
        const phi = Math.atan2(p.z - c, p.y - axisY);
        for (let k = 0; k < pegs; k++) {
          let d = ((k / pegs) * Math.PI * 2 + 0.4 + ang(bot.t) - phi) % (Math.PI * 2);
          if (d < 0) d += Math.PI * 2;
          const eta = (d - margin) / -w;
          if (eta > 0.08 && eta < 0.22) return true;
        }
        return false;
      };
      const gapAt = c - R - 0.55;
      route.push(
        { x: 0, z: c - 0.5, w: 0.2, jumpWhen: (bot) => jumpWhen(bot) || edgeJump(gapAt, 0.8)(bot) },
        { x: 0, z: c + R - 0.3, w: 0.2, jumpWhen },
      );
      zz = c + R + 0.55;
    }
    b.box(0, s.y - 1, zz + 3, 14, 2, 6, PAL.purple);
    route.push({ x: 0, z: zz + 3, w: 1, jumpWhen: edgeJump(zz, 1.2) });
    return {
      z: zz + 6,
      y: s.y,
      routes: [route],
      checkpoint: { from: zz + 0.5, p: new THREE.Vector3(0, s.y + 0.1, zz + 3) },
    };
  };
}

/** A bridge of small rollers turning in alternating directions, with bumpers to dodge. */
function rollerBridge(n = 10): Segment {
  return (s) => {
    const { b, rng } = s;
    const r = 0.55;
    const pitch = 1.3;
    b.box(0, s.y - 1, s.z + 1, 10, 2, 2, PAL.purple);
    for (let i = 0; i < n; i++) {
      const c = s.z + 2 + r + i * pitch;
      const sp = (i % 2 ? 1 : -1) * (2 + rng() * 2);
      drum(b, 0, s.y, c, r, 7, (t) => Math.max(0, t) * sp, pals[i % 4]!);
    }
    const end = s.z + 2 + n * pitch + 0.2;
    for (const [x, f] of [
      [-2, 0.3],
      [2, 0.65],
    ] as const)
      b.bumper(x, s.y + 0.2, s.z + 2 + n * pitch * f, 0.7, 9);
    b.box(0, s.y - 1, end + 3, 14, 2, 6, PAL.purple);
    return {
      z: end + 6,
      y: s.y,
      routes: [
        [
          { x: 0, z: s.z + 2 + n * pitch * 0.5, w: 0.5 },
          { x: 0, z: end + 3, w: 1 },
        ],
      ],
      checkpoint: { from: end + 0.5, p: new THREE.Vector3(0, s.y + 0.1, end + 3) },
    };
  };
}

export default defineMap(meta, (b, ctx) => {
  b.style.pattern = 'dots';
  const middle = pickSections(b.rng, [rotorDecks(1), pegDrums(3), rollerBridge(10), trampolineGap(), hammerBridges(2)], 3);
  return raceCourse(b, ctx, { sections: withRests([drumStairs(), ...middle, logRun(5)]) });
});
