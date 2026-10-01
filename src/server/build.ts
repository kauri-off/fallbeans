/** Set by scripts/build.ts for production bundles (absent in development and tests). */
declare const __FB_BUILD__: string | undefined;

/**
 * This server's build ("v3.0.0+abc1234.lq2x9k"), the same string the client of the same build was
 * made with; null in development. A page of another build reloads (see Connection.start).
 */
export const BUILD: string | null = typeof __FB_BUILD__ === 'string' ? __FB_BUILD__ : null;
