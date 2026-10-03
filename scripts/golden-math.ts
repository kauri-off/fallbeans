/**
 * Math.hypot as V8 computes it (scaled, Kahan-summed), as fb_shared::m::hypot does. Bun (JavaScriptCore)
 * hands two arguments to the platform libm, so its traces would differ between OSes in the last bit.
 * Imported first by scripts/golden.ts.
 */
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
