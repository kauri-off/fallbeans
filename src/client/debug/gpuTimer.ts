/**
 * GPU time by label, with EXT_disjoint_timer_query_webgl2. Only one timer query can run at a time,
 * so labels nest as a stack: begin('shadows') inside 'scene' pauses 'scene' and resumes it after
 * end(). Results arrive a few frames later; `report()` averages the frames that completed.
 */

interface Ext {
  TIME_ELAPSED_EXT: number;
  GPU_DISJOINT_EXT: number;
}

interface Pending {
  q: WebGLQuery;
  label: string;
  frame: number;
}

export class GpuTimer {
  private readonly ext: Ext | null;
  private active: { label: string; q: WebGLQuery } | null = null;
  private stack: string[] = [];
  private pending: Pending[] = [];
  private frame = 0;
  private frameTotals = new Map<number, Map<string, number>>();
  private disjointFrames = new Set<number>();
  private sums = new Map<string, number>();
  private frames = 0;
  /** GPU time of each completed frame (ms), newest last. */
  readonly frameMs: number[] = [];

  constructor(private readonly gl: WebGL2RenderingContext) {
    this.ext = gl.getExtension('EXT_disjoint_timer_query_webgl2') as Ext | null;
  }

  get supported() {
    return !!this.ext;
  }

  begin(label: string) {
    if (!this.ext) return;
    if (this.active) {
      this.stack.push(this.active.label);
      this.endQuery();
    }
    this.startQuery(label);
  }

  end() {
    if (!this.ext) return;
    this.endQuery();
    const parent = this.stack.pop();
    if (parent) this.startQuery(parent);
  }

  /** Call after the frame's work was submitted. */
  endFrame() {
    if (!this.ext) return;
    while (this.active) this.end();
    this.frame++;
    this.poll();
  }

  private startQuery(label: string) {
    const q = this.gl.createQuery();
    if (!q) return;
    this.gl.beginQuery(this.ext!.TIME_ELAPSED_EXT, q);
    this.active = { label, q };
  }

  private endQuery() {
    if (!this.active) return;
    this.gl.endQuery(this.ext!.TIME_ELAPSED_EXT);
    this.pending.push({ q: this.active.q, label: this.active.label, frame: this.frame });
    this.active = null;
  }

  private poll() {
    const gl = this.gl;
    if (gl.getParameter(this.ext!.GPU_DISJOINT_EXT)) for (const p of this.pending) this.disjointFrames.add(p.frame);
    while (this.pending.length) {
      const p = this.pending[0]!;
      if (!gl.getQueryParameter(p.q, gl.QUERY_RESULT_AVAILABLE)) break;
      const ms = (gl.getQueryParameter(p.q, gl.QUERY_RESULT) as number) / 1e6;
      gl.deleteQuery(p.q);
      this.pending.shift();
      let f = this.frameTotals.get(p.frame);
      if (!f) this.frameTotals.set(p.frame, (f = new Map()));
      f.set(p.label, (f.get(p.label) ?? 0) + ms);
    }
    // Frames whose queries all came back.
    const oldestPending = this.pending[0]?.frame ?? this.frame;
    for (const [frame, totals] of this.frameTotals) {
      if (frame >= oldestPending) continue;
      this.frameTotals.delete(frame);
      if (this.disjointFrames.delete(frame)) continue;
      let sum = 0;
      for (const [label, ms] of totals) {
        this.sums.set(label, (this.sums.get(label) ?? 0) + ms);
        sum += ms;
      }
      this.frames++;
      this.frameMs.push(sum);
      if (this.frameMs.length > 240) this.frameMs.shift();
    }
    // Results that never arrive (lost context): do not pile up.
    if (this.pending.length > 400) this.dispose();
  }

  /** Average GPU ms per frame for each label, largest first, since the last reset. */
  report(): { frames: number; total: number; labels: { label: string; ms: number; share: number }[] } {
    const total = [...this.sums.values()].reduce((a, b) => a + b, 0);
    const n = Math.max(1, this.frames);
    const r = (v: number) => Math.round(v * 1000) / 1000;
    return {
      frames: this.frames,
      total: r(total / n),
      labels: [...this.sums]
        .sort((a, b) => b[1] - a[1])
        .map(([label, ms]) => ({ label, ms: r(ms / n), share: total ? Math.round((ms / total) * 1000) / 10 : 0 })),
    };
  }

  reset() {
    this.sums.clear();
    this.frames = 0;
    this.frameMs.length = 0;
  }

  dispose() {
    for (const p of this.pending) this.gl.deleteQuery(p.q);
    this.pending = [];
    this.frameTotals.clear();
    this.active = null;
    this.stack = [];
  }
}
