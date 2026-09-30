/** Browser launch settings for the client bench: the real GPU, not a software fallback. */

/** Windows: the installed Edge; elsewhere Playwright's Chromium (PW_CHANNEL overrides). */
export const channel = process.env.PW_CHANNEL ?? (process.platform === 'win32' ? 'msedge' : undefined);

/** ANGLE backend per OS (d3d11 elsewhere than Windows silently falls back to SwiftShader). */
const angle = process.platform === 'win32' ? 'd3d11' : process.platform === 'darwin' ? 'metal' : 'gl';

export const gpuArgs = [
  '--ignore-gpu-blocklist',
  '--enable-gpu',
  `--use-angle=${angle}`,
  '--disable-background-timer-throttling',
  '--disable-renderer-backgrounding',
];
