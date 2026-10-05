# Fall Beans — notes for Claude

Fall Beans is a party game (up to 8 beans; races, survival and points rounds) with a native Rust client and a
120 Hz authoritative server (Bevy 0.19 + Lightyear 0.30). There is no production server: players run their own and
add it to the client's server list. Player-facing text and the docs are Russian; code, comments and tool output are
English. License: AGPL-3.0-or-later.

- `core/` (fb_shared, fb_sim, fb_maps, fb_arena, fb_audit) — the deterministic simulation and its tools: no Bevy,
  f64, no wall clock, no unseeded randomness, no `HashMap`, maths only through `fb_shared::m`.
- `crates/` (fb_proto, fb_net, fb_server, fb_client) — Bevy/Lightyear code.
- `docs/state.md` — the handoff between threads: read it first, rewrite it before you stop.

## Commands

- `cargo xtask check` — fmt, clippy, all tests and quick audits; run before calling a change done.
- `cargo xtask dev [--clients N] [--autopilot] [--fill]` — server plus windowed clients locally.
- `cargo xtask audit [map…] [--quick]` — map audits.
- `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` — after network changes.
- `cargo xtask fuzz-ui` — plays the whole client at random, prints the path to a crash.
- `cargo xtask assets [--export]` — check models (`--export` needs Blender).
- `cargo xtask dist <nsis|appimage|flatpak|deb|rpm>` — build a release package.

## Rules

- Do not commit, push, open PRs or run the `release` workflow unless asked.
- Changing a replicated component or message: bump `PROTOCOL_VERSION` (`core/fb_shared/src/consts.rs`).
- The repository is public: no private hosts, addresses or keys in committed files.
