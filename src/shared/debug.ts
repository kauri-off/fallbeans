import { z } from 'zod';

/** A problem a browser reports to the server (POST api/report); see client/debug/capture.ts. */
export const ClientReportSchema = z.object({
  kind: z.enum(['error', 'rejection', 'console', 'webgl', 'asset', 'desync', 'boot']),
  msg: z.string().max(500),
  stack: z.string().max(2000),
  /** Protocol version and build of the page. */
  v: z.number().int(),
  build: z.string().max(64),
  path: z.string().max(200),
  ua: z.string().max(200),
  /** ms since the page loaded */
  t: z.number().min(0),
  /** Game state when it happened (arena, phase, transport, quality…). */
  ctx: z.record(z.string().max(32), z.unknown()),
});
export type ClientReport = z.infer<typeof ClientReportSchema>;

/** Largest accepted report body, in bytes. */
export const MAX_REPORT_BYTES = 8192;
