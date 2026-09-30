import * as THREE from 'three';
import { z } from 'zod';
import { aimLanding, arenaBrain, BOT_DT, humanize, initBot, navTo, steer, unstick } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { type BotInput, type BotView, defineMap } from '../../sim/map';
import { armContactEta, spinUp } from '../../sim/props';
import meta from './meta';

const ARENA_R = 15;
const TOWER_R = 2.5;
const TOWER_H = 4;
const ISLAND_X = 20.5;
const ISLAND_Y = 3.5;
const ISLAND_R = 3.2;
const TRAMP_X = 12.5;
/** The sweeping bars turn round the tower, from here to there (m from the middle). */
const SWEEP_IN = 2.9;
const SWEEP_OUT = 8.2;
/** A star lies this long (s) if nobody takes it; a new one falls about this often. */
const LIFE = 15;
const EVERY = 1.1;
const SPECIAL_EVERY = 11;
const REACH = 1.2;
const SNATCH_RANGE = 2.4;
/** After a star was snatched from somebody, nobody can snatch from them for this long (s). */
const IMMUNE = 1.2;

interface Spot {
  x: number;
  y: number;
  z: number;
  value: number;
  where: 'ground' | 'tower' | 'island';
}

interface Star {
  k: number;
  spot: number;
  at: number;
  until: number;
  value: number;
}

const TakeEvent = z.object({ k: z.number().int().min(0), id: z.number().int() });
const DropEvent = z.object({ from: z.number().int(), to: z.number().int().optional(), n: z.number().int().min(0) });
const SnatchEvent = z.object({ from: z.number().int(), to: z.number().int() });

/**
 * Stars fall on the arena, one about every second (the bigger ones on the tower and the islands now
 * and then): run through them to collect. Knocked off, you lose them all to whoever knocked you;
 * falling by yourself, they are gone. A grab snatches one. Bars sweep round the tower in the middle;
 * trampolines at the rim throw you up to the islands. Where and when stars fall follows from the seed
 * (the same everywhere); who took which one comes from the server.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const rng = b.rng;
    b.cyl(0, -1, 0, ARENA_R, 2, PAL.purple, { freq: 0.3 });
    b.cyl(0, 0.03, 0, ARENA_R + 0.05, 0.1, PAL.yellow, { noCollide: true });
    b.cyl(0, 0.06, 0, SWEEP_OUT + 0.3, 0.1, PAL.blue, { noCollide: true, freq: 0.35 });
    // The tower, with a ladder on either side, and the bars sweeping round it.
    b.cyl(0, TOWER_H / 2, 0, TOWER_R, TOWER_H, PAL.orange, { surface: 'rock' });
    b.ladder(0, 0, TOWER_R, TOWER_H, 0);
    b.ladder(0, 0, -TOWER_R, TOWER_H, Math.PI);
    const dir = rng() < 0.5 ? 1 : -1;
    const SPIN = spinUp(Math.PI / 2 + 0.3, 0.55 + rng() * 0.1, 0.0012);
    const ang = (t: number) => dir * SPIN.angle(t);
    const omega = (t: number) => dir * SPIN.omega(t);
    const sweeper = b.anchor(0, 0.6, 0);
    for (const k of [0, 1]) {
      const pivot = b.anchor(0, 0, 0, sweeper);
      pivot.rotation.y = k * Math.PI;
      const holder = b.anchor(SWEEP_IN, 0, 0, pivot);
      b.model('arm', holder).scale.set(SWEEP_OUT - SWEEP_IN, 1, 1);
      b.collider(
        b.anchor((SWEEP_OUT - SWEEP_IN) / 2, 0, 0, holder),
        { type: 'box', hx: (SWEEP_OUT - SWEEP_IN) / 2, hy: 0.36, hz: 0.36 },
        { hit: 0.45, tag: 'rotor', sweep: true },
      );
    }
    b.move((t) => {
      sweeper.rotation.y = ang(t);
    });
    // Trampolines up to the islands beyond the rim (the big stars fall there).
    for (const sx of [-1, 1]) {
      b.trampoline(sx * TRAMP_X, 0, 0, 1.5, 18);
      b.cyl(sx * ISLAND_X, ISLAND_Y - 0.6, 0, ISLAND_R, 1.2, PAL.green, { surface: 'grass' });
      b.prop('mushroom', sx * (ISLAND_X + 1.6), ISLAND_Y, 1.8, { scale: 0.7 });
    }
    const bumpers = [0.5, 7 / 6, 11 / 6].map((a) => ({ x: Math.cos(a * Math.PI) * 9.9, z: Math.sin(a * Math.PI) * 9.9 }));
    for (const p of bumpers) b.bumper(p.x, 0, p.z, 0.8, 8);
    for (let k = 0; k < 4; k++) b.bonus(Math.cos(k * 1.57 + 0.4) * 11.8, 0, Math.sin(k * 1.57 + 0.4) * 11.8);
    b.clouds(0, 0, 50);

    // Where stars may fall.
    const spots: Spot[] = [];
    for (const [r, n, off] of [
      [5.4, 8, 0.2],
      [10, 12, 0.13],
      [12.8, 14, 0.3],
    ] as const)
      for (let k = 0; k < n; k++) {
        const a = off + (k / n) * Math.PI * 2;
        const x = Math.cos(a) * r;
        const zz = Math.sin(a) * r;
        if (Math.abs(x) > TRAMP_X - 2.2 && Math.abs(zz) < 2.4) continue;
        if (bumpers.some((p) => Math.hypot(p.x - x, p.z - zz) < 2.6)) continue;
        spots.push({ x, y: 0, z: zz, value: 1, where: 'ground' });
      }
    const towerSpot = spots.length;
    spots.push({ x: 0, y: TOWER_H, z: 0, value: 2, where: 'tower' });
    for (const sx of [-1, 1]) spots.push({ x: sx * ISLAND_X, y: ISLAND_Y, z: 0, value: 3, where: 'island' });

    // When and where they fall: from the seed, identical on the server and every client.
    const stars: Star[] = [];
    const busy = spots.map(() => -1);
    const put = (spot: number, at: number, life: number) => {
      stars.push({ k: 0, spot, at, until: at + life, value: spots[spot]!.value });
      busy[spot] = at + life;
    };
    const ground = spots.flatMap((s, i) => (s.where === 'ground' ? [i] : []));
    const special = spots.flatMap((s, i) => (s.where !== 'ground' ? [i] : []));
    for (let t = 0.4; t < meta.duration - 1; t += t < 3 ? 0.35 : EVERY * (0.75 + rng() * 0.5)) {
      const free = ground.filter((i) => busy[i]! <= t);
      if (free.length) put(free[Math.floor(rng() * free.length)]!, t, LIFE);
    }
    for (let t = 7 + rng() * 3; t < meta.duration - 4; t += SPECIAL_EVERY * (0.85 + rng() * 0.3)) {
      const free = special.filter((i) => busy[i]! <= t);
      if (free.length) put(free[Math.floor(rng() * free.length)]!, t, LIFE + 5);
    }
    stars.sort((a, c) => a.at - c.at);
    stars.forEach((s, i) => {
      s.k = i;
    });
    const bySpot = spots.map((_, i) => stars.filter((s) => s.spot === i));
    const taken = new Map<number, { id: number; at: number }>();
    const live = (s: Star, t: number) => t >= s.at && t < s.until && !taken.has(s.k);
    const starAt = (spot: number, t: number) => bySpot[spot]!.find((s) => live(s, t)) ?? null;

    // Stars on the course (client): one per spot, falling in, bobbing, taken with a pop.
    if (b.view) {
      const objs = spots.map((sp) => {
        const g = b.anchor(sp.x, sp.y, sp.z);
        const star = b.model('star', g);
        star.scale.setScalar(sp.value > 1 ? 1 + sp.value * 0.25 : 1.1);
        const ring = new THREE.Mesh(
          b.view!.own(new THREE.RingGeometry(0.55, 0.8, 32)),
          b.view!.own(
            new THREE.MeshBasicMaterial({
              color: sp.value > 1 ? '#ff9f4a' : '#ffd23f',
              transparent: true,
              opacity: 0.55,
              side: THREE.DoubleSide,
              depthWrite: false,
              toneMapped: false,
            }),
          ),
        );
        ring.rotation.x = -Math.PI / 2;
        ring.position.y = 0.05;
        g.add(ring);
        g.traverse((o) => {
          o.userData.dynamic = true;
        });
        g.userData.cat = 'decor';
        g.visible = false;
        return { g, star, ring };
      });
      b.anim((t) => {
        spots.forEach((sp, i) => {
          const o = objs[i]!;
          const s = bySpot[i]!.find((x) => t >= x.at - 0.5 && t < x.until && (taken.get(x.k)?.at ?? 1e9) + 0.3 > t);
          o.g.visible = !!s;
          if (!s) return;
          const got = taken.get(s.k);
          const fall = Math.max(0, s.at - t) / 0.5;
          const pop = got ? Math.min(1, (t - got.at) / 0.3) : 0;
          const fade = Math.min(1, (s.until - t) / 1.5);
          o.star.position.y = 0.35 + fall * fall * 9 + Math.sin(t * 2.6 + i) * 0.12;
          o.star.rotation.y = t * 2 + i;
          o.star.scale.setScalar((sp.value > 1 ? 1 + sp.value * 0.25 : 1.1) * Math.max(0.01, fade * (1 + pop * 0.8) * (1 - pop)));
          o.ring.visible = fall <= 0 && !got;
          o.ring.scale.setScalar(1 + Math.sin(t * 3 + i) * 0.08);
        });
      });
    }

    // What happened to me lately (client): a line on the HUD for a few seconds.
    let flash: { text: string; until: number } | null = null;
    const say = (text: string) => {
      flash = { text, until: ctx.now() + 3 };
    };
    const immune = new Map<number, number>();
    const badges = new Map<number, number>();
    b.anim(() => {
      for (const id of ctx.participants) {
        const n = ctx.score(id);
        if (badges.get(id) === n) continue;
        badges.set(id, n);
        ctx.decorate(id, { badge: n > 0 ? `⭐${n}` : '' });
      }
    });
    let first = 0;

    // Bots.
    const sweepJump = (bot: BotView) => {
      const p = bot.body.pos;
      const r = Math.hypot(p.x, p.z);
      if (bot.t <= 0 || r < SWEEP_IN - 0.8 || r > SWEEP_OUT + 0.8 || p.y > 1) return false;
      const eta = armContactEta(bot, ang(bot.t), omega(bot.t), 2);
      return eta > 0.1 && eta < 0.15 + (bot.mem.react ?? 0.2) * 0.3;
    };
    const safe = (x: number, zz: number) => {
      const r = Math.hypot(x, zz);
      return r > TOWER_R + 1 && r < ARENA_R - 2.5;
    };
    const wander = arenaBrain({ radius: 11, safe, jumpWhen: sweepJump });
    /** How the bot gets to a spot, and how much that costs (m, roughly). */
    const costTo = (bot: BotView, sp: Spot) => {
      const p = bot.body.pos;
      if (sp.where === 'ground') return Math.hypot(sp.x - p.x, sp.z - p.z);
      if (sp.where === 'tower') return Math.hypot(p.x, Math.abs(p.z) - TOWER_R - 0.8) + 6;
      return Math.hypot(Math.sign(sp.x) * TRAMP_X - p.x, p.z) + 9;
    };
    const goTo = (bot: BotView, sp: Spot, out: BotInput) => {
      const p = bot.body.pos;
      if (sp.where === 'ground') {
        navTo(bot, sp.x, sp.z, out, 1, 0.4);
        return;
      }
      if (sp.where === 'tower') {
        const side = p.z >= 0 ? 1 : -1;
        const fz = side * (TOWER_R + 0.9);
        if (Math.hypot(p.x, p.z - fz) < 0.8) steer(bot, 0, 0, out);
        else navTo(bot, 0, fz + side * 0.5, out, 1, 0.3);
        return;
      }
      const tx = Math.sign(sp.x) * TRAMP_X;
      if (Math.hypot(p.x - tx, p.z) < 2) steer(bot, tx, 0, out);
      else navTo(bot, tx, 0, out, 1, 0.5);
    };

    return {
      spawns: b.ringSpawns(8, 11.5, 0.1, Math.PI / 8),
      killY: -10,
      faceCenter: true,
      view: new THREE.Vector3(0, 2, 0),
      tick(t) {
        if (t < 0) return;
        while (first < stars.length && stars[first]!.until <= t) first++;
        for (let i = first; i < stars.length; i++) {
          const s = stars[i]!;
          if (s.at > t) break;
          if (!live(s, t)) continue;
          const sp = spots[s.spot]!;
          for (const [id, body] of ctx.bodies()) {
            if (body.inPortal) continue;
            const dy = body.pos.y - sp.y;
            if (dy < -0.8 || dy > 2) continue;
            if (Math.hypot(body.pos.x - sp.x, body.pos.z - sp.z) > REACH + 0.3 * body.size) continue;
            ctx.emit('star', { k: s.k, id });
            ctx.setScore(id, ctx.score(id) + s.value);
            break;
          }
        }
      },
      onFall(id, by) {
        const n = ctx.score(id);
        if (n <= 0) return;
        const to = by !== null && by !== id && ctx.bodies().has(by) ? by : null;
        ctx.setScore(id, 0);
        if (to !== null) ctx.setScore(to, ctx.score(to) + n);
        ctx.emit('drop', to !== null ? { from: id, to, n } : { from: id, n });
      },
      onGrab(actor, target) {
        const t = ctx.now();
        if (t < 0 || ctx.score(target) <= 0 || (immune.get(target) ?? -1) > t) return;
        const a = ctx.bodies().get(actor);
        const v = ctx.bodies().get(target);
        if (!a || !v) return;
        if (Math.hypot(a.pos.x - v.pos.x, a.pos.z - v.pos.z) > SNATCH_RANGE || Math.abs(a.pos.y - v.pos.y) > 2) return;
        immune.set(target, t + IMMUNE);
        ctx.setScore(target, ctx.score(target) - 1);
        ctx.setScore(actor, ctx.score(actor) + 1);
        ctx.emit('snatch', { from: target, to: actor });
      },
      onEvent(name, data) {
        const me = ctx.me();
        if (name === 'star') {
          const d = TakeEvent.safeParse(data);
          if (!d.success || !stars[d.data.k] || taken.has(d.data.k)) return;
          taken.set(d.data.k, { id: d.data.id, at: ctx.now() });
          if (d.data.id === me) ctx.sfx('pickup');
        } else if (name === 'drop') {
          const d = DropEvent.safeParse(data);
          if (!d.success) return;
          if (d.data.from === me) {
            ctx.sfx('steal');
            say(d.data.to !== undefined ? `Вас сбили: ${d.data.n} ⭐ у соперника!` : `Вы упали: ${d.data.n} ⭐ сгорели!`);
          } else if (d.data.to === me) {
            ctx.sfx('steal');
            say(`+${d.data.n} ⭐ со сбитого соперника!`);
          }
        } else if (name === 'snatch') {
          const d = SnatchEvent.safeParse(data);
          if (!d.success) return;
          immune.set(d.data.from, ctx.now() + IMMUNE);
          if (d.data.from === me) {
            ctx.sfx('steal');
            say('У вас выхватили звезду!');
          } else if (d.data.to === me) {
            ctx.sfx('pickup');
            say('+1 ⭐ — выхватили!');
          }
        }
      },
      hud() {
        const f = flash as { text: string; until: number } | null;
        if (f && ctx.now() < f.until) return f.text;
        return `Ваши звёзды: ${ctx.score(ctx.me())} · Q / ПКМ — выхватить звезду`;
      },
      bot(bot, out) {
        initBot(bot);
        const body = bot.body;
        const p = body.pos;
        const t = bot.t;
        const mine = ctx.score(bot.id);
        // On a ladder: keep climbing.
        if (body.state === 'ladder') {
          steer(bot, 0, 0, out);
          return;
        }
        // Thrown up by a trampoline: land on the island.
        if (!body.grounded && bot.mem.fly !== undefined && body.state === 'normal') {
          aimLanding(bot, bot.mem.fly * ISLAND_X, ISLAND_Y, 0, out);
          return;
        }
        if (body.grounded) bot.mem.fly = undefined;
        else if (Math.abs(p.x) > TRAMP_X - 2 && Math.abs(p.z) < 2 && body.vel.y > 12) {
          bot.mem.fly = Math.sign(p.x);
          aimLanding(bot, bot.mem.fly * ISLAND_X, ISLAND_Y, 0, out);
          return;
        }
        // Up on an island or the tower: its star, then back down.
        const onIsland = p.y > ISLAND_Y - 0.6 && Math.abs(p.x) > ARENA_R;
        const onTower = p.y > TOWER_H - 0.6 && Math.hypot(p.x, p.z) < TOWER_R + 0.3;
        if (onIsland || onTower) {
          const spot = onTower ? towerSpot : towerSpot + (p.x < 0 ? 1 : 2);
          const sp = spots[spot]!;
          if (starAt(spot, t)) steer(bot, sp.x, sp.z, out);
          else if (onIsland) {
            steer(bot, Math.sign(p.x) * 11, 0, out);
            if (Math.abs(p.x) < ISLAND_X - ISLAND_R + 1.3 && body.grounded) out.jump = true;
          } else {
            const a = bot.mem.ph ?? 0;
            steer(bot, Math.cos(a) * 6, Math.sin(a) * 6, out);
          }
          humanize(bot, out, { precise: true });
          return;
        }
        if (!body.grounded) {
          humanize(bot, out, { fun: false });
          return;
        }
        // Somebody with a pile of stars close by: go and take it off them (the pushy ones).
        const aggro = bot.mem.aggro ?? 0.3;
        let hunt: BotView['others'][number] | undefined;
        if (aggro > 0.45 && mine < 4 && t > 4) {
          let best = 2;
          for (const o of bot.others) {
            const n = ctx.score(o.id);
            const d = Math.hypot(o.pos.x - p.x, o.pos.z - p.z);
            if (o.down || n <= best || d > 9 || Math.abs(o.pos.y - p.y) > 1) continue;
            best = n;
            hunt = o;
          }
        }
        if (hunt) {
          const d = Math.hypot(hunt.pos.x - p.x, hunt.pos.z - p.z);
          navTo(bot, hunt.pos.x + hunt.vel.x * 0.3, hunt.pos.z + hunt.vel.z * 0.3, out, 1, 1);
          out.grab = d < 1.8;
          if (d > 2 && d < 3.4 && bot.rng() < (0.3 + aggro) * BOT_DT * 3) out.dive = true;
          if (sweepJump(bot)) out.jump = true;
          humanize(bot, out, { rough: false, fun: false, avoid: false });
          unstick(bot, out);
          return;
        }
        // The best star for the effort (a careful bot with a pile keeps off the rim and the islands).
        let target = bot.mem.tk !== undefined ? stars[bot.mem.tk] : undefined;
        if (!target || !live(target, t) || t > (bot.mem.tkUntil ?? 0)) {
          target = undefined;
          let best = 0;
          for (let i = first; i < stars.length; i++) {
            const s = stars[i]!;
            if (s.at > t + 0.3) break;
            if (!live(s, Math.max(t, s.at))) continue;
            const sp = spots[s.spot]!;
            if (sp.where === 'island' && (mine > 2 || (bot.mem.skill ?? 0.7) < 0.55)) continue;
            if (mine >= 4 && Math.hypot(sp.x, sp.z) > 11.5 && sp.where === 'ground') continue;
            let score = s.value / (costTo(bot, sp) + 3);
            // Someone else is closer to it: less worth going for.
            for (const o of bot.others)
              if (Math.hypot(o.pos.x - sp.x, o.pos.z - sp.z) < Math.hypot(p.x - sp.x, p.z - sp.z) - 1) score *= 0.6;
            score *= 0.85 + bot.rng() * 0.3;
            if (score > best) {
              best = score;
              target = s;
            }
          }
          bot.mem.tk = target?.k;
          bot.mem.tkUntil = t + 2 + bot.rng() * 1.5;
        }
        if (!target) {
          wander(bot, out);
          return;
        }
        goTo(bot, spots[target.spot]!, out);
        if (sweepJump(bot)) out.jump = true;
        humanize(bot, out, { fun: false, rough: mine < 3, avoid: true });
        unstick(bot, out);
      },
    };
  },
  ['starlight', 'neon', 'circus'],
);
