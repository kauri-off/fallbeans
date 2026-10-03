/**
 * Preload for scripts/golden.ts (`bun --preload ./scripts/golden-math.ts scripts/golden.ts`, as
 * `cargo xtask golden` runs it): the TS side computes with the same maths as Rust, so the traces
 * differ only where the port does.
 *   - Math.hypot as V8 computes it (scaled, Kahan-summed), as fb_shared::m::hypot does. Bun (JavaScriptCore)
 *     hands two arguments to the platform libm, so its traces would differ between OSes in the last bit.
 *   - Math.sin, cos, atan2, atan, exp and pow from fb_shared::m compiled to WebAssembly
 *     (scripts/golden-libm.wasm, built by `cargo xtask golden` from rust/tools/golden_libm): JavaScriptCore's
 *     differ from Rust's libm in the last bit of a few percent of values, and rounds drift apart.
 *   - `r() ** 1.1` in sim/bots.ts made a Math.pow call: the `**` operator cannot be replaced, and a
 *     whole exponent is computed the same way on both sides anyway (repeated squaring), a fraction is not.
 */
import { readFileSync } from 'node:fs';

const wasm = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(new URL('./golden-libm.wasm', import.meta.url))), {})
  .exports as Record<string, (...a: number[]) => number>;
const fn = (name: string) => {
  const f = wasm[name];
  if (typeof f !== 'function') throw new Error(`golden-libm.wasm has no ${name}: rebuild it with cargo xtask golden`);
  return f;
};
Math.sin = fn('fb_sin');
Math.cos = fn('fb_cos');
Math.atan2 = fn('fb_atan2');
Math.atan = fn('fb_atan');
Math.exp = fn('fb_exp');
Math.pow = fn('fb_pow');

Math.hypot = (...args: number[]): number => {
  let max = 0;
  let nan = false;
  const abs = args.map((a) => {
    const v = Math.abs(+a);
    if (Number.isNaN(v)) nan = true;
    else if (v > max) max = v;
    return v;
  });
  if (max === Number.POSITIVE_INFINITY) return Number.POSITIVE_INFINITY;
  if (nan) return Number.NaN;
  if (max === 0) return 0;
  let sum = 0;
  let comp = 0;
  for (const a of abs) {
    const n = a / max;
    const summand = n * n - comp;
    const pre = sum + summand;
    comp = pre - sum - summand;
    sum = pre;
  }
  return Math.sqrt(sum) * max;
};

/** Source rewrites (exact text, checked): non-integer powers written with `**`. */
const REWRITES: Record<string, [string, string][]> = {
  'src/sim/bots.ts': [['r() ** 1.1', 'Math.pow(r(), 1.1)']],
};

let rewritten = 0;

Bun.plugin({
  name: 'golden-math',
  setup(build) {
    build.onLoad({ filter: /src\/sim\/bots\.ts$/ }, async (args) => {
      let text = await Bun.file(args.path).text();
      for (const [file, list] of Object.entries(REWRITES)) {
        if (!args.path.replaceAll('\\', '/').endsWith(file)) continue;
        for (const [from, to] of list) {
          if (!text.includes(from)) throw new Error(`golden-math: "${from}" is no longer in ${file}`);
          text = text.replace(from, to);
          rewritten++;
        }
      }
      return { contents: text, loader: 'ts' };
    });
  },
});

(globalThis as { __goldenMath?: () => number }).__goldenMath = () => rewritten;
