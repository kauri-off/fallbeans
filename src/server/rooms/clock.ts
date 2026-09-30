/**
 * A room's game time (ms): the real clock, unless a dev command slowed, paused or warped it.
 * Timers, arenas and the clients' clocks all follow game time.
 */
export class GameClock {
  /** Game ms per real ms (dev: slow motion, 0 = paused). */
  rate = 1;
  private timeBase: number;
  private realBase: number;

  constructor(readonly real: () => number) {
    this.realBase = this.timeBase = real();
  }

  now() {
    const real = this.real();
    if (!this.shifted) return real;
    return this.timeBase + (real - this.realBase) * this.rate;
  }

  /** Game time no longer is the real clock: clients joining need a `clock` message. */
  get shifted() {
    return this.rate !== 1 || this.timeBase !== this.realBase;
  }

  /** Game time runs `k` times as fast from now on; returns the game time of the change. */
  setRate(k: number) {
    const now = this.rebase();
    this.rate = k;
    return now;
  }

  /** Starts counting from the current game time (before a rate change or a jump). */
  rebase() {
    const now = this.now();
    this.timeBase = now;
    this.realBase = this.real();
    return now;
  }

  /** Jumps game time forward. */
  skip(ms: number) {
    this.timeBase += ms;
  }
}
