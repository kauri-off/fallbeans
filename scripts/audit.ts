/**
 * Runs the audits (src/audit) and prints what they found.
 *   bun run audit [map…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics] [--notes] [--json]
 * The report is also saved to .reports/audit-latest.json. Exit code 1 when there are errors.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { AUDIT_NAMES, formatReport, runAudits } from '../src/audit/run';

const args = process.argv.slice(2);
const opt = (n: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : undefined;
};
const list = (n: string) => opt(n)?.split(',').filter(Boolean);
const maps = args.filter((a, i) => !a.startsWith('--') && !['--only', '--skip', '--seed'].includes(args[i - 1] ?? ''));
const only = list('--only');
const bad = [...(only ?? []), ...(list('--skip') ?? [])].filter((a) => !AUDIT_NAMES.includes(a));
if (bad.length) {
  console.error(`unknown audits: ${bad.join(', ')} (known: ${AUDIT_NAMES.join(', ')})`);
  process.exit(2);
}

const json = args.includes('--json');
const progress = !json && process.stderr.isTTY;
const report = await runAudits({
  ...(maps.length ? { maps } : {}),
  ...(only ? { only } : {}),
  ...(list('--skip') ? { skip: list('--skip')! } : {}),
  quick: args.includes('--quick'),
  seed: Number(opt('--seed') ?? 11),
  onResult: (r) => {
    if (progress) process.stderr.write(`\r\x1b[K  ${r.map} / ${r.audit} ${r.ms} ms`);
  },
});
if (progress) process.stderr.write('\r\x1b[K');
mkdirSync('.reports', { recursive: true });
writeFileSync('.reports/audit-latest.json', JSON.stringify(report, null, 1));
if (json) console.log(JSON.stringify(report, null, 1));
else console.log(formatReport(report, { metrics: args.includes('--metrics'), infos: args.includes('--notes') }));
process.exit(report.summary.errors ? 1 : 0);
