import { TICK_MS } from './consts';

/** Input margin the lead aims for: inputs should reach the server this many ticks early. */
export const MARGIN_TARGET = 2;
/** Snapshots over which margins are judged (about 3 s). */
const WINDOW = 90;
/** The margin percentile steered on: rare spikes (a Wi-Fi hiccup) are left to input resends. */
const PERCENTILE = 0.1;
/**
 * Largest single raise (ms): a margin far below zero is a jump (a stalled tab or server, a clock
 * correction), not the link, and must not leave the lead high for a long time.
 */
const MAX_RAISE = 150;
/** Share of the excess margin given back per snapshot (above target: back within a second or two). */
const GIVE_BACK = 0.05;

/** Margins needed before judging (about 0.3 s). */
const MIN_SAMPLES = 10;

/**
 * How far ahead of the server clock a client predicts and sends its inputs (ms), steered by the
 * input margins the server reports with each snapshot (Snapshot.own.margin): the 10th percentile
 * of the last few seconds should be MARGIN_TARGET ticks. Short of it: up by the shortfall, and the
 * margins start over once the raise can show in them (a round trip later; those in between reflect
 * the old lead). Above it: down in proportion to the excess (quickly when far above, gently near the
 * target). Errors of the clock estimate and asymmetric routes (VPNs) cancel out: only the arrival at
 * the server counts.
 */
export class LeadControl {
  private readonly margins: number[] = [];
  /** Margins before this time (ms) reflect a lead since raised: ignored. */
  private settleUntil = 0;

  constructor(public lead: number) {}

  /** A snapshot's margin; `rtt` (ms) and `now` (ms, any monotonic clock). Returns the new lead. */
  update(margin: number, rtt: number, now: number): number {
    if (now < this.settleUntil) return this.lead;
    this.margins.push(margin);
    if (this.margins.length > WINDOW) this.margins.shift();
    if (this.margins.length < MIN_SAMPLES) return this.lead;
    const sorted = [...this.margins].sort((a, b) => a - b);
    const low = sorted[Math.floor(sorted.length * PERCENTILE)]!;
    if (low < MARGIN_TARGET) {
      this.lead += Math.min(MAX_RAISE, (MARGIN_TARGET - low) * TICK_MS);
      this.margins.length = 0;
      this.settleUntil = now + rtt + 50;
    } else if (low > MARGIN_TARGET) this.lead -= (low - MARGIN_TARGET) * TICK_MS * GIVE_BACK;
    this.lead = Math.max(TICK_MS, Math.min(this.lead, Math.max(TICK_MS * 4, rtt / 2 + 400)));
    return this.lead;
  }

  /** The worst margin of the window (debug). */
  get worst(): number {
    return this.margins.length ? Math.min(...this.margins) : 0;
  }
}
