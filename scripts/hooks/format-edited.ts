/**
 * Claude Code hook (PostToolUse on Edit/Write, see .claude/settings.json): formats and lints the file
 * that was just edited with Biome, and reports what it could not fix so the next step sees it.
 * Reads the hook payload ({ tool_input: { file_path } }) from stdin.
 */
const input = JSON.parse(await Bun.stdin.text()) as { tool_input?: { file_path?: string } };
const file = input.tool_input?.file_path;
if (!file || !/\.(tsx?|json|css)$/.test(file) || /node_modules|[/]dist[/]/.test(file)) process.exit(0);
const r = Bun.spawnSync([process.execPath, 'x', 'biome', 'check', '--write', '--no-errors-on-unmatched', file], {
  stdout: 'pipe',
  stderr: 'pipe',
});
if (r.exitCode !== 0) {
  // Exit code 2: shown to Claude as feedback on the edit.
  console.error(`biome: ${file} still has problems:\n${(r.stdout.toString() + r.stderr.toString()).slice(0, 3000)}`);
  process.exit(2);
}

export {};
