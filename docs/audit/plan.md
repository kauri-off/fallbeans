# Total audit of Fall Beans — plan

Date: 2026-10-05. Tree: `K:\fallbeans-rogue-port-to-rust` (branch `rogue/port-to-rust` content, **not a git checkout** —
no `.git`). Version `0.1.0-alpha`. Toolchain here: rustc/cargo 1.98.1 stable, Windows 11.

## Inputs taken from the request

- Check **every file**: Rust (≈50 k lines in `core/`, `crates/`, `xtask/`, `vendor/`), tests and golden fixtures,
  CI (`.github/workflows`), packaging (`packaging/`), deploy (`deploy/`), Blender export script (`blender/`), docs
  (`README.md`, `docs/`, `CLAUDE.md`), configs (`Cargo.toml`, `.cargo`, `.claude`, `rustfmt.toml`, `clippy.toml`,
  `.gitattributes`, `.editorconfig`, `.gitignore`).
- Look for: bugs, errors, mistakes, bad/odd decisions, suspended / deprecated / unmaintained technology or APIs.
- **Fix** what can be fixed without changing the game much. **Write down** larger changes in
  `docs/audit/proposals.md` with a detailed description and one or more proposed variants.
- Use sub-agents, official docs and upstream source (crates.io / docs.rs / GitHub for Bevy 0.19, Lightyear 0.30,
  bevy_replicon, aeronet, wgpu, axum, ureq, tokio, clap…), and `K:\GameDevLibrary` (it is a rendering library:
  AA/TAA, tonemapping, upscalers, GI, iq maths/SDF/noise — relevant to `crates/fb_client/src/render`).
- Project rules (CLAUDE.md) still bind: deterministic `core/` (no wall clock, no unseeded RNG, no `HashMap`, maths
  only through `fb_shared::m`, f64, three.js operation order); golden traces are frozen; replicated component or
  message change → bump `PROTOCOL_VERSION`; LF endings; rustfmt; no commits/pushes/deploys/releases; no private
  hosts in files; docs and player text in Russian (this audit's own reports are in English, as asked in chat).

## Baseline

1. Snapshot of the tree (without `target/`) in the session scratchpad → final diff with `git diff --no-index`, so
   every change is reviewable even without git.
2. `cargo build --workspace --all-targets` (cold build; there was no `target/`), then `cargo xtask check`
   (fmt, clippy `-D warnings`, all tests incl. golden traces, determinism hashes, rollback replay, quick audits).
   Record what fails *before* any edit so regressions are attributable.

## Audit split (parallel sub-agents, each owns disjoint files)

| # | Area | Files |
|---|------|-------|
| A | Deterministic core: shared, physics, course, builder, bots, nav, looks | `core/fb_shared`, `core/fb_sim` |
| B | Maps, arena, audits, golden/determinism tests | `core/fb_maps`, `core/fb_arena`, `core/fb_audit` |
| C | Protocol, networking, server (rooms, hub, play, http, auth, metrics, logbook) | `crates/fb_proto`, `crates/fb_net`, `crates/fb_server` |
| D | Client: app, net/session/servers/update/settings, game/beans/camera/audio/brp/probe | `crates/fb_client/src/*.rs` |
| E | Client UI and rendering (render/, ui/, shaders) — with `K:\GameDevLibrary` | `crates/fb_client/src/render`, `crates/fb_client/src/ui`, `assets/` |
| F | Tooling & delivery: xtask, CI, packaging, deploy, vendor patch, blender script, configs, docs | `xtask/`, `.github/`, `packaging/`, `deploy/`, `vendor/`, `blender/`, root files, `README.md`, `docs/` |
| G | Dependency / technology currency: every dependency vs its latest release, RUSTSEC advisories, deprecated APIs (Bevy 0.19, Lightyear 0.30, wgpu 29…), unmaintained crates, GitHub Actions versions | `Cargo.toml`, `Cargo.lock`, workflows |

Each agent reads **every** file of its area, checks the upstream docs/source where an API is in doubt, and:
- applies **safe, local fixes** directly (bugs with clear intent, panics on bad input, wrong error handling,
  resource leaks, off-by-one, dead code, stale comments, deprecated-API swaps with identical behaviour);
- does **not** change simulation results, golden traces, determinism hashes, protocol layout or gameplay feel —
  those go to `proposals.md` instead;
- reports every finding (fixed / proposed / rejected-as-intended) with `file:line`.

Then I (the main thread):
1. Review every applied diff; revert anything risky.
2. `cargo xtask check` → must be green (or no worse than baseline, with each remaining failure explained).
3. Bounded runtime smoke: server + headless clients via `cargo xtask stress` (short, `--release`), and
   `fb_client --offscreen --exit-after` if useful.
4. Write `docs/audit/report.md` (all findings, what was fixed) and `docs/audit/proposals.md` (bigger changes with
   variants); rewrite `docs/state.md`; append to `docs/decisions.md`.

## Never let anything run endlessly or get stuck (hard limit: 30 minutes per run)

- **Every** long command runs under `timeout` (GNU coreutils in Git Bash): builds/clippy/tests `timeout 1800`,
  smoke runs `timeout 300`–`600`, `--kill-after` so a process that ignores SIGTERM is still killed. Never more than
  30 minutes for any single command; if it hits the limit it is reported as a hang, not waited for again.
- Long jobs run in the background (`run_in_background`) and notify on exit; no polling loops, no `sleep` waits.
- Tests: `cargo test` with `RUST_TEST_THREADS` default but each run under `timeout`; a hanging test is found by
  re-running with `-- --list` and per-test `timeout 300 cargo test <name> -- --exact`.
- Game processes: the client only with `--offscreen` plus `--exit-after <secs>` (no window — the author does not
  want windows); the server only through `xtask stress` with `--secs` (it kills the server itself) or under
  `timeout`. After every run, check and kill leftovers: `taskkill /F /IM fb_server.exe`, `taskkill /F /IM
  fb_client.exe` (never `pkill -f`, which kills the calling shell).
- Network: stress only on loopback (`--lag/--jitter/--loss` simulated); no `--remote`, no deploy, no release
  workflow. HTTP fetches by agents have the tool's own timeouts; a fetch that fails is skipped, not retried in a loop.
- Ports: a stuck run can keep 5887/UDP/WS ports busy → check with `netstat -ano` and kill the owner before the next run.
- Cargo lock contention: only one heavy cargo command at a time from the main thread; agents use
  `cargo check -p <crate>` (short) and never `cargo clean`. The format hook (`cargo xtask format-hook`, 60 s
  timeout) needs `xtask` built — it is built in the baseline step first.
- Sub-agents: each gets a bounded scope and is told to stop and report; a sub-agent that has not reported is not
  waited on forever — its area is finished by the main thread.
- Blender (`xtask assets --export`) is **not** run (needs Blender, changes committed models); `xtask dist` and
  `deploy` are not run (packaging/network side effects).

## Deliverables

- Fixed source tree (diff vs the baseline snapshot).
- `docs/audit/report.md` — every finding with status.
- `docs/audit/proposals.md` — larger changes: problem, impact, variants, recommendation.
- Updated `docs/state.md` and `docs/decisions.md`.
