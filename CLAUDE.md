Fall Beans is a party game (up to 8 beans; races, survival and points rounds) with a native Rust client and a
120 Hz authoritative server (Bevy 0.19 + Lightyear 0.30). There is no production server: players run their own and
add it to the client's server list. Player-facing text and the docs are Russian; code, comments and tool output are
English.

- `core/` (fb_shared, fb_sim, fb_maps, fb_arena, fb_audit) — the deterministic simulation and its tools: no Bevy,
  f64, no wall clock, no unseeded randomness, no `HashMap`, maths only through `fb_shared::m`.
- `crates/` (fb_proto, fb_net, fb_server, fb_client) — Bevy/Lightyear code.

## Commands

- `cargo xtask check [-v]` — fmt, clippy, all tests and quick audits; run before calling a change done. Full output
  in `target/check.log`; failing `scenarios::` tests (wall-clock) are retried once and reported as flaky.
- `cargo xtask play [--dist] [-- client args]` — the author's local test: dev server and one client from its menu
  (`perf` profile; `--dist`: the packages' client).
- `cargo xtask doctor` — what this machine lacks for the build, the upscalers and each package.
- `cargo xtask setup [--dist]` — the pinned SDKs and tools of `toolchain/deps.toml` (their versions change only
  there) into `target/sdk`; an environment variable (`DLSS_SDK`…) overrides them.
- `cargo xtask dev [--clients N] [--autopilot] [--fill] [--trace hits,clicks,input]` — server plus windowed clients
  locally; traces into `target/traces`.
- `cargo xtask audit [map…] [--quick]` — map audits.
- `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` — after network changes.
- `cargo xtask perf <run|show|compare|csv>` — render benchmarks.
- `cargo xtask fuzz-ui` — plays the whole client at random, prints the path to a crash.
- `cargo xtask assets [--export]` — check models (`--export` needs Blender).
- `cargo xtask dist <nsis|appimage|flatpak|deb|rpm>` — a package (deb, rpm: the server). The `release` workflow
  builds nsis, AppImage and deb; the AppImage on Ubuntu 22.04 for its old glibc.

## Rules

- Do not commit, push, open PRs or run the `release` workflow unless asked.
- Changing a replicated component or message: bless the wire schema (`FB_BLESS=1 cargo test -p fb_net schema`,
  `crates/fb_net/protocol.txt`). `fb_net::PROTOCOL_VERSION` is a hash of it and of
  `core/fb_arena/tests/determinism.txt`: never set by hand.
- Graphics: Vulkan 1.2+ first (its render thread is ~3× cheaper than wgpu's DX12), DX12 as the Windows fallback
  (`crates/fb_client/src/backend.rs`); no OpenGL.
- Debug traces (`--trace <kind>`) exist only with the `traces` feature (default, never in packages): their code
  lives in `trace` modules (`fb_net::trace`, `fb_server::play::trace`, `fb_client::trace`), the game's code calls
  them under `#[cfg(feature = "traces")]`.
- The repository is public: no private hosts, addresses or keys in committed files.

## Tooling habits

- Dev builds use `dev_features()` of xtask (`fb_client/dynamic`, `fb_server/dynamic`): call cargo through xtask or
  pass the same `--features`, or Bevy rebuilds. `dlss` does not link with `dynamic`; optimized builds have it:
  `perf` (iteration, `play`, `perf run`), `dist` (packages).
- One build at a time: a cold Bevy build takes every core and many GB. Sub-agents in parallel worktrees only read
  and edit code; the lead merges and builds once. Ask the author before builds, runs and performance measurements
  on their machine.
