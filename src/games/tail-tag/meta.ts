import { z } from 'zod';
import { defineGame } from '../../shared/game';

export const TailsEvent = z.object({ ids: z.array(z.number().int()) });

interface TailState {
  tails: Set<number>;
  immune: Map<number, number>;
  held: Map<number, number>;
  lastTick: number;
  lastSync: number;
}

const STEAL_RANGE = 2.6;

export default defineGame({
  id: 'tail-tag',
  title: 'Хвостики',
  genre: 'points',
  rules: 'points',
  desc: 'У половины игроков есть хвосты. Хватайте (Q/ПКМ) чужой хвост и не отдавайте свой!',
  goal: 'Держите хвост как можно дольше',
  duration: 75,
  minPlayers: 2,
  events: { steal: z.object({ from: z.number().int() }) },
  server: {
    init(ctx): TailState {
      const ids = [...ctx.participants].sort(() => ctx.rng() - 0.5);
      const n = Math.max(1, Math.min(ids.length - 1, Math.ceil(ids.length / 2)));
      const tails = new Set(ids.slice(0, n));
      ctx.emit('tails', { ids: [...tails] });
      return { tails, immune: new Map(), held: new Map(), lastTick: ctx.now(), lastSync: 0 };
    },
    tick(ctx, st) {
      const now = ctx.now();
      const dt = (now - st.lastTick) / 1000;
      st.lastTick = now;
      if (ctx.roundTime() < 0) return;
      for (const id of st.tails) st.held.set(id, (st.held.get(id) ?? 0) + dt);
      if (now - st.lastSync > 1000) {
        st.lastSync = now;
        for (const [id, v] of st.held) if (Math.floor(v) !== ctx.score(id)) ctx.setScore(id, Math.floor(v));
      }
    },
    on: {
      steal(ctx, st, from, { from: victim }) {
        if (ctx.roundTime() < 0 || st.tails.has(from) || !st.tails.has(victim)) return;
        if ((st.immune.get(victim) ?? 0) > ctx.now()) return;
        const active = ctx.active();
        if (!active.includes(from) || !active.includes(victim)) return;
        const a = ctx.position(from);
        const b = ctx.position(victim);
        if (!a || !b || Math.hypot(a[0] - b[0], a[2] - b[2]) > STEAL_RANGE || Math.abs(a[1] - b[1]) > 2) return;
        st.tails.delete(victim);
        st.tails.add(from);
        st.immune.set(from, ctx.now() + 1500);
        ctx.emit('tails', { ids: [...st.tails] }, from);
      },
    },
  },
});
