import * as THREE from 'three';
import { arenaBrain, BOT_DT } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotInput, type BotView, defineMap } from '../../sim/map';
import { armContactEta, spinUp } from '../../sim/props';
import meta from './meta';

/**
 * Jump the low bar, stay under the high one. The bars are barriers that knock over whoever they catch
 * (and pass over them: nobody gets dragged along). Direction, the number of bars and how fast they
 * speed up come from the seed.
 */
export default defineMap(
  meta,
  (b) => {
    const rng = b.rng;
    const dir = rng() < 0.5 ? 1 : -1;
    const lowArms = 2;
    const highArms = rng() < 0.35 ? 1 : 2;
    const acc = 0.009 + rng() * 0.005;
    // The low bar starts 10° past a spawn pair and eases in: the first bean it reaches has over a second.
    const LOW = spinUp(-0.26, 1.1 + rng() * 0.15, acc);
    const lowAng = (t: number) => dir * LOW.angle(t);
    const lowOmega = (t: number) => dir * LOW.omega(t);
    const hk = 0.006 + rng() * 0.004;
    const highAng = (t: number) => dir * (Math.PI / 2 - (t <= 0 ? 0 : 0.7 * t + hk * t * t));
    const highOmega = (t: number) => -dir * (0.7 + 2 * hk * t);

    b.cyl(0, -1, 0, 13, 2, PAL.blue, { freq: 0.35 });
    b.cyl(0, 0.03, 0, 13.05, 0.1, PAL.yellow, { noCollide: true });
    b.cyl(0, 0.06, 0, 11.5, 0.1, PAL.blue, { noCollide: true, freq: 0.35 });
    b.hub(0, 0, 0, 1.2);
    b.rotor(0, 0.6, 0, 12.6, lowArms, lowAng, 0.6);
    b.rotor(0, 2.45, 0, 12.6, highArms, highAng, 0.6);
    for (const a of [0, 1, 2, 3]) b.bonus(Math.cos(a * 1.57 + 0.8) * 8, 0, Math.sin(a * 1.57 + 0.8) * 8);
    b.clouds(0, 0, 40);
    const spawns = [25, 65, 115, 155, 205, 245, 295, 335].map((d) => {
      const a = (d * Math.PI) / 180;
      return new THREE.Vector3(Math.cos(a) * 6, 0.1, Math.sin(a) * 6);
    });
    const brain = arenaBrain({
      radius: 8,
      safe: (x, z) => Math.hypot(x, z) > 3.5,
      jumpWhen: (bot) => {
        const t = Math.max(0, bot.t);
        if (t <= 0) return false;
        const eta = armContactEta(bot, lowAng(t), lowOmega(t), lowArms);
        const high = armContactEta(bot, highAng(t), highOmega(t), highArms);
        // How fast the bar really closes in: faster running at it (or near the hub), slower running
        // away from it. Seen between two looks: a quick one would slip through the reaction window, so
        // jump now if it will be too late at the next.
        const m = bot.mem;
        const seen = m.jcT !== undefined && t - m.jcT < 0.2 && (m.jcEta ?? 0) > eta;
        const rate = seen ? Math.max(0.2, ((m.jcEta ?? eta) - eta) / (t - (m.jcT ?? t))) : 1;
        m.jcEta = eta;
        m.jcT = t;
        const when = eta / rate;
        const next = when - BOT_DT;
        // Worse bots react late (and sometimes too late).
        const late = 0.15 + (m.react ?? 0.2) * 0.3;
        return when > 0.1 && (when < late || (next < 0.1 && when < 0.32)) && high > 0.7;
      },
    });
    /**
     * Both bars coming by at about the same time: jumping the low one means meeting the high one in
     * the air. Run along the circle towards the one that comes first: it comes sooner, the other one
     * later, and there is time to deal with each (under the high one standing, over the low one).
     */
    const dodge = (bot: BotView, out: BotInput) => {
      const t = Math.max(0, bot.t);
      const p = bot.body.pos;
      const r = Math.hypot(p.x, p.z);
      if (t <= 0 || r < 2 || bot.body.state !== 'normal') return;
      const eta = armContactEta(bot, lowAng(t), lowOmega(t), lowArms);
      const high = armContactEta(bot, highAng(t), highOmega(t), highArms);
      const lowFirst = eta < high;
      const clash = lowFirst ? high - eta < 0.8 : eta - high < 0.35;
      // Better players spot it sooner.
      const sees = 0.3 + (bot.mem.skill ?? 0.7) * 0.9;
      if (!clash || Math.min(eta, high) > sees || Math.min(eta, high) < 0.08) return;
      const w = lowFirst ? lowOmega(t) : highOmega(t);
      const s = -Math.sign(w);
      out.mx = (s * p.z) / r;
      out.mz = (-s * p.x) / r;
    };
    const bot = (view: BotView, out: BotInput) => {
      brain(view, out);
      dodge(view, out);
    };
    return { spawns, killY: -6, faceCenter: true, view: new THREE.Vector3(0, 3, 0), bot };
  },
  ['neon', 'starlight', 'circus'],
);
