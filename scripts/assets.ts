/**
 * Checks the 3D models (public/models/*.glb), and rebuilds them from Blender.
 *   bun run assets                 validate + budgets + what the game needs from each model
 *   bun run assets --export [--dry-run]  re-export from blender/fallguys_assets.blend (headless Blender),
 *                                  optimise, check, and replace public/models if everything passes
 *   bun run assets --optimize      optimise the current files in place (lossless: dedup, prune,
 *                                  weld, vertex-cache order; names and hierarchy are kept)
 *
 * Checks: the Khronos glTF validator (errors fail), triangle / size / material budgets per model,
 * every model the code loads exists (MODEL_NAMES), and the node and material names the code looks
 * up are present (bean limbs, eyes, material names that pick surfaces). Exit code 1 on problems.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { type Document, NodeIO } from '@gltf-transform/core';
import { ALL_EXTENSIONS } from '@gltf-transform/extensions';
import { dedup, prune, reorder, weld } from '@gltf-transform/functions';
import validator from 'gltf-validator';
import { MeshoptEncoder } from 'meshoptimizer';
import { MODEL_NAMES } from '../src/sim/builder';

const args = process.argv.slice(2);
const MODELS = 'public/models';
const STAGE = '.build/models';

/** Budgets per model: triangles, file size (KB), materials. */
const BUDGET: Record<string, { tris: number; kb: number; mats: number }> = {
  bean: { tris: 16000, kb: 400, mats: 12 },
  default: { tris: 6000, kb: 120, mats: 8 },
};
/** Names the code looks up in a model (src/client/game/bean.ts, assets.ts, materials.ts). */
const NEEDS: Record<string, { nodes?: string[]; materials?: string[] }> = {
  bean: { nodes: ['ArmL', 'ArmR', 'LegL', 'LegR', 'EyeL', 'EyeR'], materials: ['Body', 'Belly', 'Glint'] },
  cloud: { materials: ['Cloud'] },
  fan: { nodes: ['FanBlades'] },
  flag: { nodes: ['Pennant'], materials: ['Flag'] },
};

const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);

interface ModelReport {
  name: string;
  kb: number;
  tris: number;
  meshes: number;
  nodes: number;
  materials: number;
  errors: string[];
  warnings: string[];
}

function stats(doc: Document) {
  const root = doc.getRoot();
  let tris = 0;
  for (const m of root.listMeshes())
    for (const p of m.listPrimitives()) tris += (p.getIndices()?.getCount() ?? p.getAttribute('POSITION')?.getCount() ?? 0) / 3;
  return {
    tris: Math.round(tris),
    meshes: root.listMeshes().length,
    nodes: root.listNodes().length,
    materials: root.listMaterials().length,
    nodeNames: new Set(root.listNodes().map((n) => n.getName())),
    materialNames: new Set(root.listMaterials().map((m) => m.getName())),
  };
}

async function check(dir: string): Promise<ModelReport[]> {
  const out: ModelReport[] = [];
  for (const name of MODEL_NAMES) {
    const file = join(dir, `${name}.glb`);
    const r: ModelReport = { name, kb: 0, tris: 0, meshes: 0, nodes: 0, materials: 0, errors: [], warnings: [] };
    out.push(r);
    if (!existsSync(file)) {
      r.errors.push('missing (the game loads it: MODEL_NAMES in src/sim/builder.ts)');
      continue;
    }
    const bytes = new Uint8Array(readFileSync(file));
    r.kb = Math.round(bytes.byteLength / 102.4) / 10;
    const v = await validator.validateBytes(bytes, { maxIssues: 50 });
    for (const m of v.issues.messages) {
      const line = `${m.code}: ${m.message}${m.pointer ? ` (${m.pointer})` : ''}`;
      if (m.severity === 0) r.errors.push(line);
      else if (m.severity === 1) r.warnings.push(line);
    }
    const doc = await io.readBinary(bytes);
    const s = stats(doc);
    Object.assign(r, { tris: s.tris, meshes: s.meshes, nodes: s.nodes, materials: s.materials });
    const b = BUDGET[name] ?? BUDGET.default!;
    if (s.tris > b.tris) r.warnings.push(`${s.tris} triangles (budget ${b.tris})`);
    if (r.kb > b.kb) r.warnings.push(`${r.kb} KB (budget ${b.kb})`);
    if (s.materials > b.mats) r.warnings.push(`${s.materials} materials (budget ${b.mats})`);
    for (const n of NEEDS[name]?.nodes ?? [])
      if (!s.nodeNames.has(n)) r.errors.push(`node "${n}" is missing (the code animates it)`);
    for (const n of NEEDS[name]?.materials ?? [])
      if (!s.materialNames.has(n)) r.errors.push(`material "${n}" is missing (the code looks it up)`);
  }
  const extra = existsSync(dir)
    ? readdirSync(dir).filter((f) => f.endsWith('.glb') && !MODEL_NAMES.includes(f.slice(0, -4) as never))
    : [];
  for (const f of extra)
    out.push({ name: f, kb: 0, tris: 0, meshes: 0, nodes: 0, materials: 0, errors: [], warnings: ['not used by the game'] });
  return out;
}

/** Lossless clean-up: shared data merged, unused data dropped, vertices welded and ordered for the GPU cache. */
async function optimize(dir: string) {
  await MeshoptEncoder.ready;
  for (const name of MODEL_NAMES) {
    const file = join(dir, `${name}.glb`);
    if (!existsSync(file)) continue;
    const before = statSync(file).size;
    const doc = await io.read(file);
    await doc.transform(dedup(), prune({ keepLeaves: true, keepAttributes: true }), weld(), reorder({ encoder: MeshoptEncoder }));
    await io.write(file, doc);
    const after = statSync(file).size;
    console.log(`  optimised ${name}: ${Math.round(before / 1024)} → ${Math.round(after / 1024)} KB`);
  }
}

function print(reports: ModelReport[]) {
  console.log('model      KB     tris  meshes nodes mats');
  for (const r of reports) {
    console.log(
      `${r.name.padEnd(9)} ${String(r.kb).padStart(6)} ${String(r.tris).padStart(7)} ${String(r.meshes).padStart(6)} ${String(r.nodes).padStart(5)} ${String(r.materials).padStart(4)}`,
    );
    for (const e of r.errors) console.log(`   ✖ ${e}`);
    for (const w of r.warnings) console.log(`   ⚠ ${w}`);
  }
  const total = reports.reduce((a, r) => a + r.kb, 0);
  console.log(`total ${Math.round(total)} KB`);
}

if (args.includes('--export')) {
  const blender =
    process.env.BLENDER ??
    [
      'C:/Program Files/Blender Foundation/Blender 5.2/blender.exe',
      '/usr/bin/blender',
      '/Applications/Blender.app/Contents/MacOS/Blender',
    ].find((p) => existsSync(p));
  if (!blender) {
    console.error('Blender not found: set BLENDER=/path/to/blender');
    process.exit(2);
  }
  mkdirSync(STAGE, { recursive: true });
  const r = Bun.spawnSync([blender, '-b', 'blender/fallguys_assets.blend', '--python', 'blender/export.py', '--', STAGE], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const log = r.stdout.toString();
  for (const l of log.split('\n')) if (l.startsWith('[export]')) console.log(l);
  if (r.exitCode !== 0) {
    console.error(r.stderr.toString().slice(-2000));
    process.exit(1);
  }
  await optimize(STAGE);
  const reports = await check(STAGE);
  print(reports);
  if (reports.some((x) => x.errors.length)) {
    console.error(`\nNot replacing ${MODELS}: fix the errors above (the exported files are in ${STAGE}).`);
    process.exit(1);
  }
  if (args.includes('--dry-run')) {
    console.log(`\n--dry-run: the new files are in ${STAGE}; ${MODELS} is unchanged.`);
    process.exit(0);
  }
  for (const name of MODEL_NAMES) writeFileSync(join(MODELS, `${name}.glb`), readFileSync(join(STAGE, `${name}.glb`)));
  console.log(`\nreplaced ${MODEL_NAMES.length} models in ${MODELS}`);
} else {
  if (args.includes('--optimize')) await optimize(MODELS);
  const reports = await check(MODELS);
  print(reports);
  process.exit(reports.some((x) => x.errors.length) ? 1 : 0);
}
