/**
 * Cheap CPU section timer: where the time of a server tick or a client frame goes. Sections are
 * sequential (start() ends the previous one); totals roll over into `last` every `windowMs`.
 */
export interface SectionStat {
  /** Average ms per call and per frame/tick, worst call, share of all measured time (%). */
  avg: number;
  perFrame: number;
  max: number;
  calls: number;
  share: number;
}

export class Sections {
  enabled = true;
  private cur: string | null = null;
  private since = 0;
  private acc = new Map<string, { ms: number; n: number; max: number }>();
  private frames = 0;
  private windowStart = 0;
  private last: Record<string, SectionStat> = {};
  private lastFrames = 0;

  constructor(private readonly windowMs = 5000) {}

  /** Ends the running section (if any) and starts `name`. */
  start(name: string) {
    if (!this.enabled) return;
    const now = performance.now();
    this.close(now);
    this.cur = name;
    this.since = now;
  }

  /** Ends the running section. */
  stop() {
    if (!this.enabled) return;
    this.close(performance.now());
    this.cur = null;
  }

  /** Adds time measured elsewhere to a section. */
  add(name: string, ms: number) {
    if (!this.enabled) return;
    const a = this.acc.get(name) ?? { ms: 0, n: 0, max: 0 };
    a.ms += ms;
    a.n++;
    a.max = Math.max(a.max, ms);
    this.acc.set(name, a);
  }

  /** Marks the end of one frame / tick. */
  frame() {
    if (!this.enabled) return;
    this.frames++;
    const now = performance.now();
    if (!this.windowStart) this.windowStart = now;
    if (now - this.windowStart >= this.windowMs) this.roll(now);
  }

  private close(now: number) {
    if (this.cur === null) return;
    this.add(this.cur, now - this.since);
  }

  private roll(now: number) {
    const total = [...this.acc.values()].reduce((s, a) => s + a.ms, 0) || 1;
    const r2 = (v: number) => Math.round(v * 1000) / 1000;
    this.last = Object.fromEntries(
      [...this.acc]
        .sort((a, b) => b[1].ms - a[1].ms)
        .map(([k, a]) => [
          k,
          {
            avg: r2(a.ms / a.n),
            perFrame: r2(a.ms / Math.max(1, this.frames)),
            max: r2(a.max),
            calls: a.n,
            share: Math.round((a.ms / total) * 1000) / 10,
          },
        ]),
    );
    this.lastFrames = this.frames;
    this.acc.clear();
    this.frames = 0;
    this.windowStart = now;
  }

  /** The last complete window (or the current one if none finished yet). */
  report(): { frames: number; sections: Record<string, SectionStat> } {
    if (!this.lastFrames && this.frames) this.roll(performance.now());
    return { frames: this.lastFrames, sections: this.last };
  }

  reset() {
    this.acc.clear();
    this.frames = 0;
    this.windowStart = 0;
    this.last = {};
    this.lastFrames = 0;
    this.cur = null;
  }
}
