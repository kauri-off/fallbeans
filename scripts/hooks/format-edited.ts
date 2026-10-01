/**
 * Claude Code hook (PostToolUse on Edit/Write, see .claude/settings.json): formats the file that was just
 * edited — Biome for TS/JSON/CSS (and lints it), rustfmt for Rust (rust/rustfmt.toml) — and reports what it
 * could not fix so the next step sees it. Reads the hook payload ({ tool_input: { file_path } }) from stdin.
 */
const input = JSON.parse(await Bun.stdin.text()) as { tool_input?: { file_path?: string } };
const file = input.tool_input?.file_path;
if (!file || /node_modules|[/\\](dist|target)[/\\]/.test(file)) process.exit(0);

function run(name: string, cmd: string[]) {
  const r = Bun.spawnSync(cmd, { stdout: 'pipe', stderr: 'pipe' });
  if (r.exitCode !== 0) {
    // Exit code 2: shown to Claude as feedback on the edit.
    console.error(`${name}: ${file} still has problems:\n${(r.stdout.toString() + r.stderr.toString()).slice(0, 3000)}`);
    process.exit(2);
  }
}

if (/\.(tsx?|json|css)$/.test(file)) {
  run('biome', [process.execPath, 'x', 'biome', 'check', '--write', '--no-errors-on-unmatched', file]);
} else if (/\.rs$/.test(file)) {
  const config = new URL('../../rust/rustfmt.toml', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
  run('rustfmt', ['rustfmt', '--edition', '2024', '--config-path', config, file]);
}

export {};
