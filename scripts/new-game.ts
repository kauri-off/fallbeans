/**
 * Scaffolds a new game: bun run new-game <id> "<Название>" [race|survival|points|final]
 * Creates src/games/<id>/{meta.ts,map.ts}; then add the map to src/games/index.ts.
 */
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [id, title = id, genre = 'race'] = process.argv.slice(2);
if (!id || !/^[a-z][a-z0-9-]*$/.test(id)) {
  console.error('usage: bun run new-game <id> "<Название>" [race|survival|points|final]');
  process.exit(1);
}
const rules = { race: 'race', survival: 'survival', points: 'points', final: 'lastStanding' }[genre];
if (!rules) {
  console.error('genre must be race, survival, points or final');
  process.exit(1);
}
const dir = join('src', 'games', id);
if (existsSync(dir)) {
  console.error(`${dir} already exists`);
  process.exit(1);
}
mkdirSync(dir, { recursive: true });
const camel = id.replace(/-(\w)/g, (_, c: string) => c.toUpperCase());

writeFileSync(
  join(dir, 'meta.ts'),
  `import { defineGame } from '../../shared/game';

export default defineGame({
  id: '${id}',
  title: '${title}',
  genre: '${genre}',
  rules: '${rules}',
  desc: 'Описание игры для экрана перед раундом.',
  goal: '${genre === 'race' ? 'Добегите до финиша' : 'Не упадите'}',
  duration: ${genre === 'race' ? 180 : 90},
});
`,
);

const race = genre === 'race';
writeFileSync(
  join(dir, 'map.ts'),
  `import * as THREE from 'three';
import { ${race ? 'pathBrain' : 'arenaBrain'} } from '../../sim/bots';
import { PAL } from '../../sim/builder';
import { defineMap } from '../../sim/map';
import meta from './meta';

// Runs on the server (colliders only) and in the browser (with meshes).
// Motion that affects collisions: b.move((t) => …) as a pure function of time.
// Visual-only animation: b.anim((t, dt) => …).
export default defineMap(meta, (b) => {
${
  race
    ? `  const spawns = b.startArea(0);
  b.box(0, -1, 30, 18, 2, 46, PAL.blue);
  b.box(0, -1, 60, 18, 2, 14, PAL.yellow);
  b.finish(0, 0, 60);
  b.clouds(0, 30, 50, 30);
  return {
    spawns,
    killY: -14,
    finish: { z: 60, y: -1 },
    checkpoints: [{ z: -100, p: new THREE.Vector3(0, 0.1, 2) }],
    bot: pathBrain([{ x: 0, z: 30, w: 3 }, { x: 0, z: 64, w: 3 }]),
  };`
    : `  b.cyl(0, -1, 0, 14, 2, PAL.blue, { freq: 0.3 });
  b.clouds(0, 0, 40);
  return {
    spawns: b.ringSpawns(8, 7, 0.1),
    killY: -8,
    faceCenter: true,
    view: new THREE.Vector3(0, 2, 0),
    bot: arenaBrain({ radius: 10 }),
  };`
}
});
`,
);
console.log(`created ${dir}/meta.ts and map.ts
next: import it in src/games/index.ts (import ${camel} from './${id}/map'; add to MAPS), then bun run check`);
