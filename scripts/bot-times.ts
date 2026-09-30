/**
 * Plays every map with server bots and prints finish times (races) or survival (other games).
 *   bun scripts/bot-times.ts [map-id…]
 */
import { MAPS } from '../src/games';
import { ServerArena } from '../src/server/rooms/arena';

const where = process.argv.includes('--where');
const only = process.argv.slice(2).filter((a) => !a.startsWith('--'));
for (const mod of MAPS) {
  if (only.length && !only.includes(mod.meta.id)) continue;
  for (const seed of [11, 23]) {
    const fin: number[] = [];
    const outs: number[] = [];
    let falls = 0;
    const spots = new Map<string, number>();
    const warnings: string[] = [];
    const ids = [1, 2, 3, 4, 5, 6, 7, 8];
    const a = new ServerArena({
      id: 1,
      kind: 'round',
      module: mod,
      seed,
      startAt: 0,
      participants: ids,
      now: -500,
      hooks: {
        onFinish: (_id, t) => fin.push(t),
        onKo: (k) => {
          if (k.out) outs.push(a.time);
          else {
            falls++;
            const p = a.pawns.get(k.id);
            if (p) {
              const key = `z${Math.round(p.progress / 3) * 3} ${k.cause}`;
              spots.set(key, (spots.get(key) ?? 0) + 1);
            }
          }
        },
        onEvent: () => {},
        onScore: () => {},
        onSnapshot: () => {},
        warn: (m) => warnings.push(m),
      },
    });
    ids.forEach((id, i) => {
      a.addPawn(id, true, i);
    });
    for (let t = -500; t < mod.meta.duration * 1000; t += 50) {
      a.advance(t);
      if ([...a.pawns.values()].every((p) => p.status !== 'play')) break;
    }
    const f = (xs: number[]) => xs.map((x) => x.toFixed(0)).join(' ');
    console.log(
      `${mod.meta.id.padEnd(14)} seed ${seed}: ${mod.meta.genre === 'race' ? `fin [${f(fin)}]` : `out [${f(outs)}]`} falls ${falls} (limit ${mod.meta.duration})${warnings.length ? ` WARN ${warnings[0]}` : ''}`,
    );
    if (where)
      console.log(
        '   falls at (max progress, cause):',
        [...spots]
          .sort((x, y) => y[1] - x[1])
          .slice(0, 8)
          .map(([k, v]) => `${k}×${v}`)
          .join(', '),
      );
    a.dispose();
  }
}
