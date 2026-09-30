import { MAPS } from '../games';
import { MAP_AUDITS } from './maps';
import { GLOBAL_AUDITS } from './systems';
import type { Audit, AuditCtx, AuditOut, AuditReport, AuditResult, Finding } from './types';

/** Audits that are slow (bots play whole rounds): only in full runs unless asked for by name. */
const SLOW = new Set(['balance']);

export const AUDIT_NAMES = [...MAP_AUDITS, ...GLOBAL_AUDITS].map((a) => a.name);

export interface RunOpts {
  maps?: string[];
  only?: string[];
  skip?: string[];
  quick?: boolean;
  seed?: number;
  onResult?: (r: AuditResult) => void;
}

async function runOne(audit: Audit, map: string, ctx: AuditCtx): Promise<AuditResult> {
  const findings: Finding[] = [];
  const metrics: AuditResult['metrics'] = {};
  const add =
    (severity: Finding['severity']) =>
    (msg: string, extra: Omit<Finding, 'severity' | 'msg'> = {}) =>
      findings.push({ severity, msg, ...extra });
  const out: AuditOut = { error: add('error'), warn: add('warn'), info: add('info'), metric: (k, v) => (metrics[k] = v) };
  const t0 = performance.now();
  try {
    if (audit.perMap) await audit.run(map, ctx, out);
    else await audit.run(ctx, out);
  } catch (e) {
    findings.push({ severity: 'error', msg: `audit crashed: ${e instanceof Error ? (e.stack ?? e.message) : String(e)}` });
  }
  return { audit: audit.name, map, ms: Math.round(performance.now() - t0), findings, metrics };
}

export async function runAudits(o: RunOpts = {}): Promise<AuditReport> {
  const quick = o.quick ?? false;
  const ctx: AuditCtx = { quick, seed: o.seed ?? 11 };
  const picked = (a: Audit) =>
    (o.only?.length ? o.only.includes(a.name) : !(quick && SLOW.has(a.name))) && !o.skip?.includes(a.name);
  const maps = MAPS.map((m) => m.meta.id).filter((id) => !o.maps?.length || o.maps.includes(id));
  const unknown = o.maps?.filter((id) => !MAPS.some((m) => m.meta.id === id)) ?? [];
  const t0 = performance.now();
  const results: AuditResult[] = [];
  const push = (r: AuditResult) => {
    results.push(r);
    o.onResult?.(r);
  };
  for (const id of unknown)
    push({ audit: 'maps', map: id, ms: 0, findings: [{ severity: 'error', msg: `unknown map ${id}` }], metrics: {} });
  for (const a of GLOBAL_AUDITS) if (picked(a)) push(await runOne(a, '*', ctx));
  for (const map of maps) for (const a of MAP_AUDITS) if (picked(a)) push(await runOne(a, map, ctx));
  const all = results.flatMap((r) => r.findings);
  return {
    build: process.env.FB_BUILD ?? 'local',
    at: new Date().toISOString(),
    quick,
    ms: Math.round(performance.now() - t0),
    summary: {
      errors: all.filter((f) => f.severity === 'error').length,
      warnings: all.filter((f) => f.severity === 'warn').length,
      infos: all.filter((f) => f.severity === 'info').length,
      audits: results.length,
    },
    results,
  };
}

/** Plain-text report: one line per finding, grouped by map and audit, plus key metrics. */
export function formatReport(r: AuditReport, opts: { metrics?: boolean; infos?: boolean } = {}): string {
  const lines: string[] = [];
  const icon = { error: '✖', warn: '⚠', info: '·' } as const;
  for (const res of r.results) {
    const shown = res.findings.filter((f) => opts.infos || f.severity !== 'info');
    const m = Object.entries(res.metrics);
    if (!shown.length && !(opts.metrics && m.length)) continue;
    lines.push(`${res.map === '*' ? '[global]' : res.map} / ${res.audit} (${res.ms} ms)`);
    for (const f of shown) {
      const where = [f.t !== undefined ? `t=${f.t}` : '', f.at ? `at ${f.at.join(' ')}` : ''].filter(Boolean).join(' ');
      lines.push(`  ${icon[f.severity]} ${f.msg}${where ? `  [${where}]` : ''}`);
    }
    if (opts.metrics && m.length) lines.push(`    ${m.map(([k, v]) => `${k}=${v}`).join('  ')}`);
  }
  const s = r.summary;
  lines.push(
    `${s.errors} errors, ${s.warnings} warnings, ${s.infos} notes · ${s.audits} audits in ${(r.ms / 1000).toFixed(1)} s${r.quick ? ' (quick)' : ''}`,
  );
  return lines.join('\n');
}
