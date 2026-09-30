/**
 * Audits check the game's content and systems without a browser: maps (rules, spawns, clipping,
 * reachability, balance), physics feel, determinism, input handling and budgets. Each audit returns
 * findings (problems, by severity) and metrics (numbers worth tracking). Run them with
 * `bun run audit`, from tests (quick subset), or on the debug page.
 */

export type Severity = 'error' | 'warn' | 'info';

export interface Finding {
  severity: Severity;
  msg: string;
  /** World position the finding is about. */
  at?: [number, number, number];
  /** Sim time (s). */
  t?: number;
  data?: unknown;
}

export interface AuditResult {
  audit: string;
  /** Map id, or '*' for audits that are not about one map. */
  map: string;
  ms: number;
  findings: Finding[];
  metrics: Record<string, number | string | boolean>;
}

export interface AuditCtx {
  /** Quick mode: fewer seeds, shorter runs, coarser sampling (tests, debug page). */
  quick: boolean;
  seed: number;
}

export interface MapAudit {
  name: string;
  /** Runs once per map. */
  perMap: true;
  run(mapId: string, ctx: AuditCtx, out: AuditOut): void | Promise<void>;
}

export interface GlobalAudit {
  name: string;
  perMap: false;
  run(ctx: AuditCtx, out: AuditOut): void | Promise<void>;
}

export type Audit = MapAudit | GlobalAudit;

export interface AuditOut {
  error(msg: string, extra?: Omit<Finding, 'severity' | 'msg'>): void;
  warn(msg: string, extra?: Omit<Finding, 'severity' | 'msg'>): void;
  info(msg: string, extra?: Omit<Finding, 'severity' | 'msg'>): void;
  metric(name: string, value: number | string | boolean): void;
}

export interface AuditReport {
  build: string;
  at: string;
  quick: boolean;
  ms: number;
  summary: { errors: number; warnings: number; infos: number; audits: number };
  results: AuditResult[];
}

export const r3 = (v: number) => Math.round(v * 1000) / 1000;
export const r1 = (v: number) => Math.round(v * 10) / 10;
export const v3 = (v: { x: number; y: number; z: number }): [number, number, number] => [r3(v.x), r3(v.y), r3(v.z)];
