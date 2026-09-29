/** Traces one bot through a map: bun scripts/bot-trace.ts <map> [seed] [from z] [bot id] */
import { getMap } from '../src/games';
import { ServerArena } from '../src/server/arena';

const [id, seedS, fromS, botS] = process.argv.slice(2);
const mod = getMap(id!)!;
const from = Number(fromS ?? 0);
const who = Number(botS ?? 1);
const a = new ServerArena({
  id: 1,
  kind: 'round',
  module: mod,
  seed: Number(seedS ?? 11),
  startAt: 0,
  participants: [1, 2, 3, 4, 5, 6, 7, 8],
  now: -500,
  hooks: {
    onFinish: (i, t) => i === who && console.log('FIN', t.toFixed(1)),
    onKo: (k) => k.id === who && console.log('KO', a.time.toFixed(2), k.cause),
    onEvent() {},
    onScore() {},
    onSnapshot() {},
    warn: console.warn,
  },
});
for (const [k, i] of [1, 2, 3, 4, 5, 6, 7, 8].entries()) a.addPawn(i, true, k);
let n = 0;
for (let t = -500; t < 60_000 && n < 120; t += 100) {
  a.advance(t);
  const p = a.pawns.get(who)!;
  if (p.status !== 'play') break;
  const b = p.body;
  if (b.pos.z >= from) {
    n++;
    console.log(
      (t / 1000).toFixed(1),
      b.pos
        .toArray()
        .map((v) => v.toFixed(1))
        .join(' '),
      b.state,
      b.grounded ? 'G' : 'A',
      'wp',
      p.bot?.mem.wp,
      'in',
      p.bot?.input.mx.toFixed(2),
      p.bot?.input.mz.toFixed(2),
      p.bot?.input.jump ? 'J' : '',
    );
  }
}
