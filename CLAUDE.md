# Fall Beans — notes for Claude

Fall Beans is a party game (up to 8 beans; races, survival and points rounds) with a native Rust client and a
120 Hz authoritative server (Bevy 0.19 + Lightyear 0.30). There is no production server: players run their own and
add it to the client's server list. Player-facing text and the docs are Russian; code, comments and tool output are
English. License: AGPL-3.0-or-later.

- `core/` (fb_shared, fb_sim, fb_maps, fb_arena, fb_audit) — the deterministic simulation and its tools: no Bevy,
  f64, no wall clock, no unseeded randomness, no `HashMap`, maths only through `fb_shared::m`.
- `crates/` (fb_proto, fb_net, fb_server, fb_client) — Bevy/Lightyear code.
- `docs/state.md` — the handoff between threads: read it first, update it before you stop. It must stay true
  after the author commits: no commit hashes, dates, "uncommitted" or "done in this thread" notes (that is git's
  job). Only open state: what is unverified, broken or half-done, and where to start next; drop items once fixed.

## Commands

- `cargo xtask check [-v]` — fmt, clippy, all tests and quick audits; run before calling a change done. Prints only
  diagnostics, failures and a summary line; the full output is in `target/check.log`. A failure of only
  `scenarios::` tests (wall-clock client harness) is retried once and reported as flaky.
- `cargo xtask play [--dist] [-- client args]` — the author's local test: dev server (`--dev --solo`) and one client
  from its menu; `perf` profile, or the packages' exact client with `--dist`.
- `cargo xtask doctor` — what this machine has for the build, the upscalers and each package, with install hints.
- `cargo xtask setup [--dist]` — the pinned SDKs and tools of `toolchain/deps.toml` into `target/sdk`; xtask passes
  their paths on, an environment variable (`DLSS_SDK`…) overrides them. Versions change only in `deps.toml` (the
  `release` workflow runs `setup --dist` too).
- `cargo xtask dev [--clients N] [--autopilot] [--fill]` — server plus windowed clients locally.
- `cargo xtask audit [map…] [--quick]` — map audits.
- `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` — after network changes.
- `cargo xtask fuzz-ui` — plays the whole client at random, prints the path to a crash.
- `cargo xtask assets [--export]` — check models (`--export` needs Blender).
- `cargo xtask dist <nsis|appimage|flatpak|deb|rpm>` — build a package. Releases come from the `release` workflow
  (nsis, AppImage, deb; flatpak and rpm are local only): Linux clients built on a newer distro need its newer glibc
  (the AppImage job builds on Ubuntu 22.04).

## Rules

- Do not commit, push, open PRs or run the `release` workflow unless asked.
- Changing a replicated component or message: bless the wire schema (`FB_BLESS=1 cargo test -p fb_net schema`,
  `crates/fb_net/protocol.txt`, traced from the types). `fb_net::PROTOCOL_VERSION` is a hash of it and of
  `core/fb_arena/tests/determinism.txt`: never set by hand, unchanged by patches that touch neither.
- Graphics: Vulkan 1.2+ first (Windows and Linux: its render thread is ~3× cheaper than wgpu's DX12), DX12 as the
  Windows fallback (`crates/fb_client/src/backend.rs`); no OpenGL.
- The repository is public: no private hosts, addresses or keys in committed files.

## Tooling habits

- Dev-profile builds share one feature set (`dev_features()` in xtask: workspace, `dynamic`): call cargo through
  xtask, or pass the same `--features`, or Bevy rebuilds. No `dlss` with `dynamic` (Bevy's shared library cannot
  link NGX's static library); `check`'s clippy adds `dlss` when the SDK is there. Optimized builds, with DLSS:
  `perf` (iteration, `play`, `perf run`), `dist` (packages).
- One build at a time: a cold Bevy build takes every core and many GB. Sub-agents working in parallel (worktrees
  with their own `target/`) only read and edit code; the lead merges and builds once. Ask the author before builds
  and runs on their machine, and before performance measurements (they may have heavy work running).
