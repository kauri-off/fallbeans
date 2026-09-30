import { describe, expect, it } from 'vitest';
import { runAudits } from '../src/audit/run';

/**
 * The quick audits (see src/audit): every map follows the rules, spawns and respawns are safe,
 * moving parts do not pass through the course, the server simulation is deterministic, input and
 * scoring hold up. Warnings count too: fix the map, or make the audit smarter.
 */
describe('audits (quick)', () => {
  it('find no errors or warnings', async () => {
    const r = await runAudits({ quick: true });
    const problems = r.results.flatMap((res) =>
      res.findings.filter((f) => f.severity !== 'info').map((f) => `${res.map}/${res.audit}: ${f.severity} ${f.msg}`),
    );
    expect(problems).toEqual([]);
  }, 60_000);
});
