import { type Builder, PAL, type Palette } from './builder';
import type { BotView } from './map';

export interface BallLaneOpts {
  lanes: readonly number[];
  zTop: number;
  yTop: number;
  zBottom: number;
  yBottom: number;
  radius: number;
  speed: (t: number) => number;
  period: number;
  perLane?: number;
  pal?: Palette;
}

export interface BallLanes {
  /** Will a ball pass through the box x ∈ [x0, x1], z ∈ [z0, z1] during the next `horizon` seconds? */
  danger(x0: number, x1: number, z0: number, z1: number, t: number, horizon: number): boolean;
}

/** Balls rolling down a ramp in lanes; position is a pure function of time. */
export function rollingBalls(b: Builder, o: BallLaneOpts): BallLanes {
  const len = o.zTop - o.zBottom;
  const slope = (o.yTop - o.yBottom) / len;
  const cosA = Math.cos(Math.atan(slope));
  const perLane = o.perLane ?? 1;
  const palettes = [PAL.pink, PAL.orange, PAL.purple, PAL.red];
  const balls: { x: number; phase: number }[] = [];
  const ballZ = (phase: number, t: number): number | null => {
    const tt = Math.max(0, t) + phase;
    const s = tt - Math.floor(tt / o.period) * o.period;
    const dist = s * o.speed(t);
    return t > 0 && dist < len ? o.zTop - dist : null;
  };
  o.lanes.forEach((x, li) => {
    for (let k = 0; k < perLane; k++) {
      const phase = b.rng() * o.period + (k * o.period) / perLane;
      balls.push({ x, phase });
      const ball = b.sphere(x, o.yTop + o.radius, o.zTop, o.radius, o.pal ?? palettes[(li + k) % palettes.length]!, {
        dynamic: true,
        hit: 1.1,
        tag: 'ball',
      });
      const obj = ball.obj;
      b.move((t) => {
        const tt = Math.max(0, t) + phase;
        const cycle = Math.floor(tt / o.period);
        const s = tt - cycle * o.period;
        const dist = s * o.speed(t);
        const alive = t > 0 && dist < len;
        // Not solid in the first moments of a cycle: the jump back to the top is not a hit.
        ball.col.enabled = alive && s > 0.1;
        obj.visible = alive;
        if (!alive) return;
        const z = o.zTop - dist;
        const grow = Math.min(1, s / 0.3) * Math.min(1, (len - dist) / 1.5);
        obj.scale.setScalar(Math.max(0.01, grow));
        obj.position.set(x, o.yBottom + (z - o.zBottom) * slope + o.radius / cosA, z);
        obj.rotation.x = -dist / o.radius;
      });
    }
  });
  return {
    danger(x0, x1, z0, z1, t, horizon) {
      const reach = o.radius + 0.7;
      for (const ball of balls) {
        if (ball.x + reach < x0 || ball.x - reach > x1) continue;
        for (let dt = 0; dt <= horizon; dt += 0.1) {
          const z = ballZ(ball.phase, t + dt);
          if (z !== null && z + reach > z0 && z - reach < z1) return true;
        }
      }
      return false;
    },
  };
}

export function yOnRamp(z: number, z0: number, y0: number, z1: number, y1: number) {
  return y0 + ((z - z0) / (z1 - z0)) * (y1 - y0);
}

/** Seconds until a rotor arm (angle(t), angular speed omega, `arms` arms) sweeps over the bot. */
export function sweepEta(bot: BotView, angle: number, omega: number, arms: number, cx = 0, cz = 0): number {
  const phi = Math.atan2(-(bot.body.pos.z - cz), bot.body.pos.x - cx);
  const period = (Math.PI * 2) / arms;
  let d = (phi - angle) % period;
  if (d < 0) d += period;
  return omega > 0 ? d / omega : (period - d) / -omega;
}

/** Seconds until a rotor arm (half thickness `half`) first touches the bot; negative while touching. */
/**
 * A rotor angle that starts at `start`, eases up to `w` rad/s over about `ease` seconds after the start
 * (nobody is hit before they can react) and keeps speeding up by `acc`. `omega` is its angular speed.
 */
export function spinUp(start: number, w: number, acc: number, ease = 1.5) {
  return {
    angle: (t: number) => (t <= 0 ? start : start + (w * t * t) / (t + ease) + acc * t * t),
    omega: (t: number) => (t <= 0 ? 0.2 : Math.max(0.2, (w * t * (t + 2 * ease)) / (t + ease) ** 2 + 2 * acc * t)),
  };
}

export function armContactEta(bot: BotView, angle: number, omega: number, arms: number, cx = 0, cz = 0, half = 0.36): number {
  const r = Math.max(0.5, Math.hypot(bot.body.pos.x - cx, bot.body.pos.z - cz));
  const margin = (half + 0.55) / r / Math.abs(omega);
  const eta = sweepEta(bot, angle, omega, arms, cx, cz);
  const period = (Math.PI * 2) / arms / Math.abs(omega);
  // Just passed: still touching until the arm clears the other side.
  return eta > period - margin ? eta - period - margin : eta - margin;
}

export interface GloveOpts {
  x: number;
  y: number;
  z: number;
  /** −1: comes from the left (punches towards +x), 1: from the right. */
  side: -1 | 1;
  /** Punch rhythm (rad/s) and phase. */
  w: number;
  ph: number;
  /** How far the punch reaches out of its post (m). */
  reach: number;
  scale?: number;
  /** A post under the glove down to this height (none: hangs from nothing, e.g. out of a wall). */
  postTo?: number;
}

/**
 * A boxing glove on a rod that rests in its post and punches out along x now and then: a quick jab
 * and a slower pull back. Returns the glove's x offset from the post over time (for bots).
 */
export function glovePuncher(b: Builder, o: GloveOpts): (t: number) => number {
  const s = o.scale ?? 1.3;
  const out = (t: number) => o.reach * Math.max(0, Math.sin(Math.max(0, t) * o.w + o.ph)) ** 3;
  if (o.postTo !== undefined) {
    const h = o.y - o.postTo + 0.6;
    b.box(o.x + o.side * 1.4 * s, o.postTo + h / 2, o.z, 1.1, h, 1.3, PAL.orange, { noCollide: true });
  }
  const glove = b.anchor(o.x, o.y, o.z);
  b.collider(
    b.anchor(-o.side * 0.56 * s, 0, 0, glove),
    { type: 'box', hx: 0.54 * s, hy: 0.48 * s, hz: 0.48 * s },
    { hit: 1.1, tag: 'glove', sinks: true },
  );
  const model = b.model('glove', glove);
  // The model punches along its +z: turned to punch across.
  model.rotation.y = -o.side * (Math.PI / 2);
  model.scale.setScalar(s);
  b.move((t) => {
    glove.position.x = o.x - o.side * out(t);
  });
  return (t) => o.x - o.side * out(t);
}
