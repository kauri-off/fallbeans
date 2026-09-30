import * as THREE from 'three';
import type { Waypoint } from '../../sim/bots';
import { type Builder, PAL } from '../../sim/builder';
import {
  edgeJump,
  hammerBridges,
  movingPlatforms,
  pickSections,
  portalFork,
  raceCourse,
  type Segment,
  slidingGates,
  timedDoors,
  tippingBridge,
  trampolineGap,
  withRests,
} from '../../sim/course';
import { type BotView, defineMap } from '../../sim/map';
import { armContactEta, glovePuncher, rollingBalls, sweepEta, yOnRamp } from '../../sim/props';
import meta from './meta';

/**
 * The climb: a fork up to the first plateau (a ramp with balls, or sliding steps), hammer bridges,
 * a glove-swept plateau with launch pads up to a rotor deck, then more climbing drawn from the seed,
 * and the summit: sliding steps up to the crown, guarded by a sweeper. First to touch it wins.
 */

/** Up to a plateau: a ramp with balls in two lanes (blocks on the strip between), or sliding steps. */
function climbFork(rise = 8): Segment {
  return (s) => {
    const { b, rng } = s;
    const z0 = s.z;
    const top = z0 + 29;
    const y1 = s.y + rise;
    const yR = (zz: number) => yOnRamp(zz, z0, s.y, top, y1);
    // Left: the ramp.
    b.ramp(-5, z0, s.y, top, y1, 7, PAL.blue);
    const ang = Math.atan2(rise, top - z0);
    b.box(-8.9, (s.y + y1) / 2 + 0.6, (z0 + top) / 2, 0.8, 1.2, Math.hypot(top - z0, rise), PAL.pink, { rot: [-ang, 0, 0] });
    const balls = rollingBalls(b, {
      lanes: [-7, -3],
      zTop: top - 1,
      yTop: y1,
      zBottom: z0 + 1,
      yBottom: s.y,
      radius: 1,
      speed: (t) => 8.5 + t * 0.02,
      period: 3.3 + rng() * 0.6,
      perLane: 1,
    });
    const blocks = [z0 + 7, z0 + 15, z0 + 23];
    for (const [i, zb] of blocks.entries())
      b.box(-5, yR(zb) + 0.6, zb, 1.8, 1.8, 1.4, i % 2 ? PAL.orange : PAL.yellow, { rot: [-ang, 0, 0] });
    // Right: sliding steps.
    const steps = [0, 1, 2, 3, 4].map((k) => ({
      z: z0 + 3 + k * 5.4,
      y: s.y + 1.3 + k * ((rise - 1.3) / 4),
      w: 0.8 + rng() * 0.6,
      ph: rng() * 6,
    }));
    const stepX = (st: (typeof steps)[number], t: number) => 5 + Math.sin(t * st.w + st.ph) * 2.2;
    steps.forEach((st, k) => {
      const m = b.box(5, st.y - 0.5, st.z, 3.2, 1, 3, k % 2 ? PAL.orange : PAL.green, { dynamic: true });
      b.move((t) => {
        m.obj.position.x = stepX(st, t);
      });
    });
    b.box(0, y1 - 1, top + 5, 20, 2, 10, PAL.purple);

    const ramp = (lane: number): Waypoint[] => {
      const pts: Waypoint[] = [{ x: -5, z: z0 + 0.5, w: 0 }];
      blocks.forEach((zb, i) => {
        const lx = -5 + (i % 2 ? 1 : -1) * 1.55;
        pts.push({ x: -5, z: zb - 2.3, w: 0 });
        pts.push({
          x: lx,
          z: zb - 0.8,
          w: 0,
          wait: (bot) => !balls.danger(Math.min(-5, lx) - 0.3, Math.max(-5, lx) + 0.3, zb - 2.5, zb + 2.5, bot.t, 0.9),
        });
        pts.push({ x: lx, z: zb + 0.9, w: 0 }, { x: -5, z: zb + 2.3, w: 0 });
      });
      pts.push({ x: lane, z: top + 2, w: 0.3 });
      return pts;
    };
    const stepRoute: Waypoint[] = [{ x: 5, z: z0 - 0.8, w: 0 }];
    let edge = z0;
    for (const st of steps) {
      const e = edge;
      stepRoute.push({
        x: (t) => stepX(st, t),
        z: st.z,
        wait: (bot) => Math.abs(stepX(st, bot.t + 0.55) - bot.body.pos.x) < 1.0,
        jumpWhen: edgeJump(e),
      });
      edge = st.z + 1.5;
    }
    stepRoute.push({ x: 3, z: top + 2, w: 0.3, jumpWhen: edgeJump(edge) });
    const routes = [ramp(-3), ramp(3), stepRoute];
    for (const r of routes) r.push({ x: 0, z: top + 5, w: 1 });
    return {
      z: top + 10,
      y: y1,
      routes,
      forbidden: (p) => p.z > z0 && p.z < top && p.x < -8.4,
      checkpoint: { from: top + 0.5, p: new THREE.Vector3(0, y1 + 0.1, top + 5) },
    };
  };
}

/** A plateau swept by gloves, then launch pads up to a deck with a sweeper. */
function gloveLaunch(rise = 5): Segment {
  return (s) => {
    const { b, rng } = s;
    const z0 = s.z;
    b.box(0, s.y - 1, z0 + 7, 14, 2, 14, PAL.purple);
    const gloves = [
      { z: z0 + 5.3, side: -1 as const },
      { z: z0 + 8.2, side: 1 as const },
    ].map((g) =>
      glovePuncher(b, {
        x: g.side * 9,
        y: s.y + 0.95,
        z: g.z,
        side: g.side,
        w: 1.1 + rng() * 0.4,
        ph: rng() * 6,
        reach: 8.5,
        scale: 1.3,
        postTo: s.y - 6,
      }),
    );
    const padZ = z0 + 10.2;
    b.pad(-3, s.y, padZ, 1.3, 19);
    b.pad(3, s.y, padZ, 1.3, 19);
    const deckY = s.y + rise;
    const deckZ = z0 + 20;
    b.box(0, deckY - 1, deckZ, 14, 2, 10, PAL.pink, { freq: 0.3 });
    b.hub(0, deckY, deckZ, 0.9);
    const sp = 1.2 + rng() * 0.5;
    const ph = rng() * 6;
    const ang = (t: number) => (t <= 0 ? ph : ph + t * sp);
    b.rotor(0, deckY + 0.6, deckZ, 5.8, 2, ang, 0.45);
    b.box(0, deckY - 1, deckZ + 7, 8, 2, 4, PAL.purple);
    const clear = (bot: BotView) =>
      [0, 0.25, 0.5, 0.75, 1].every((dt) => gloves.every((gx) => Math.abs(gx(bot.t + dt)) > 9 - 8.5 * 0.3));
    const route = (x: number): Waypoint[] => [
      { x, z: z0 + 2, w: 0.5 },
      { x, z: padZ, w: 0, wait: clear },
      {
        x: 0,
        z: deckZ + 4.5,
        w: 0.3,
        jumpWhen: (bot) => bot.body.pos.y > deckY - 0.5 && sweepEta(bot, ang(bot.t), sp, 2, 0, deckZ) < 0.15,
      },
      { x: 0, z: deckZ + 7, w: 0.5 },
    ];
    return {
      z: deckZ + 9,
      y: deckY,
      routes: [route(-3), route(3)],
      // Falling off the deck: back before the pads (the deck is all within the rotor's reach).
      checkpoint: { from: z0 + 0.5, p: new THREE.Vector3(0, s.y + 0.1, z0 + 2.5) },
    };
  };
}

/** The summit: three sliding steps, then the crown over a platform guarded by a slow sweeper. */
function summit(b: Builder, z0: number, y0: number) {
  const slideX = (k: number, t: number) => Math.sin(t * (0.8 + k * 0.25) + k * 2) * 1.6;
  b.box(0, y0 - 1, z0 + 1.5, 8, 2, 3, PAL.purple);
  [0, 1, 2].forEach((k) => {
    const st = b.box(0, y0 + 0.5 + k * 1.2, z0 + 5 + k * 3.5, 4, 1, 3, k % 2 ? PAL.orange : PAL.green, { dynamic: true });
    b.move((t) => {
      st.obj.position.x = slideX(k, t);
    });
  });
  const topY = y0 + 4;
  const cz = z0 + 20;
  b.box(0, topY - 1, cz, 14, 2, 10, PAL.yellow);
  const hubZ = cz + 1.5;
  const CW = 1.1;
  const crownAng = (t: number) => (t <= 0 ? 0 : t * CW);
  b.hub(0, topY, hubZ, 0.6);
  b.rotor(0, topY + 0.6, hubZ, 5, 2, crownAng, 0.45);
  const crown = b.model('crown');
  crown.position.set(0, topY + 2.6, cz - 1);
  crown.scale.setScalar(1.6);
  b.anim((t) => {
    crown.rotation.y = t * 1.5;
    crown.position.y = topY + 2.6 + Math.sin(t * 2) * 0.15;
  });
  const route: Waypoint[] = [
    { x: 0, z: z0 + 1.5, w: 0.3 },
    { x: (t) => slideX(0, t), z: z0 + 5, jumpWhen: edgeJump(z0 + 3) },
    { x: (t) => slideX(1, t), z: z0 + 8.5, jumpWhen: edgeJump(z0 + 6.5) },
    { x: (t) => slideX(2, t), z: z0 + 12, jumpWhen: edgeJump(z0 + 10) },
    {
      x: 0,
      z: cz - 1,
      w: 0.2,
      jumpWhen: (bot: BotView) => {
        if (bot.body.pos.z < z0 + 14) return edgeJump(z0 + 13.5)(bot);
        const eta = armContactEta(bot, crownAng(bot.t), CW, 2, 0, hubZ);
        return eta > 0.1 && eta < 0.24;
      },
    },
  ];
  return { finish: { z: cz - 2.4, y: topY - 1, halfWidth: 2.2 }, route };
}

export default defineMap(
  meta,
  (b, ctx) => {
    const more = pickSections(
      b.rng,
      [slidingGates(4, 5), movingPlatforms(5), tippingBridge(6), trampolineGap(), timedDoors(2), portalFork()],
      3,
    );
    return raceCourse(b, ctx, {
      sections: withRests([climbFork(8), hammerBridges(3), gloveLaunch(5), ...more]),
      finishWith: summit,
    });
  },
  ['royal', 'snow', 'castle'],
);
