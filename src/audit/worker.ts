/// <reference lib="webworker" />
import { type RunOpts, runAudits } from './run';

/**
 * Runs audits off the game server's event loop (the debug API starts one worker per request):
 * a full audit takes seconds of CPU and would stall every room's simulation.
 */
declare const self: Worker;

self.onmessage = async (e: MessageEvent<Omit<RunOpts, 'onResult'>>) => {
  try {
    self.postMessage({ ok: true, report: await runAudits(e.data) });
  } catch (err) {
    self.postMessage({ ok: false, error: err instanceof Error ? (err.stack ?? err.message) : String(err) });
  }
};
