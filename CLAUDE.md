# Fall Beans — notes for Claude

Fall Beans is a party game (up to 8 beans, races / survival / points rounds, 120 Hz authoritative server): a native
Rust + Bevy 0.19 + Lightyear 0.30 client and server (branch `rogue/port-to-rust`). It started as a browser game in
TypeScript; that version is gone from this branch (it stays on `master` and in git history) — its golden traces in
`core/fb_arena/tests/golden/` are frozen fixtures that can no longer be re-recorded. Version: `0.1.0-alpha`. There is
no production server: players run their own (`.deb`/`.rpm` from the releases, or a build) and add it to the
client's server list. Player-facing text and the docs are Russian; code, comments and tool output are English.

Game rules to keep: open to everyone, no site PIN. Players meet in rooms: the client opens at the server list, then
the room list; anyone may create one room (they are its host whenever they are in it; a stand-in hosts while they
are away) and be in one room at a time; private rooms ask for a 4-digit PIN only their host sees. A player is an
identity token (`fb_id`).

## State: read it first, save it before you stop

- `docs/state.md` — the handoff: date, branch, last commit; **where to start** (1–3 concrete next steps); work
  started and not finished; what is broken. Rewritten whole, never appended.
- `docs/decisions.md` — everything decided or found that the code does not explain: deviations, workarounds,
  temporary hacks (with "Пересмотреть"), rejected options. Entries appended at the bottom, never rewritten; a
  dropped decision gets "→ отменено: date".
- **Start of a thread:** read `docs/state.md`, then what it points to (`README.md`, decisions).
- **Before a thread ends** (task done, the user stops, or the context is getting long) — without being asked:
  rewrite `docs/state.md` and add a `docs/decisions.md` entry for anything new.
- **Do not track completion.** No "done" lists or ✓ marks: the code, tests and git show what is done. There are no
  phases any more.
- `README.md` describes the code as it is (layout, commands, rules, releases, known issues); `deploy/README.md` —
  what a server host needs for `cargo xtask deploy`.

## Rules

- Git: the repo is on GitHub (kauri-off/fallbeans). Do not commit, push, open PRs or run the `release` workflow
  unless asked.
- Line endings are LF (`.gitattributes`). rustfmt formats every edited `.rs` (hook in `.claude/settings.json` →
  `cargo xtask format-hook`); files changed by scripts need `cargo fmt --all`.
- Deploy only when asked, and only to a host the author names: `cargo xtask deploy --host … --domain …`
  (`README.md`, "Деплой"); there are no default hosts. The repository is public: no private hosts, addresses or
  keys in committed files.
- The simulation must stay deterministic: `core/` uses no wall clock, no unseeded randomness, no `HashMap`, maths
  only through `fb_shared::m` (`core/clippy.toml` enforces it).

## Rust: essentials

- `core/` (fb_shared, fb_sim, fb_maps, fb_arena, fb_audit) is the deterministic simulation and its tools: no Bevy,
  f64, operation order as three.js (the golden traces depend on it). `crates/` (fb_proto, fb_net, fb_server,
  fb_client) is Bevy/Lightyear code.
- Verify with `cargo xtask check` (fmt, clippy -D warnings, tests: golden traces, recorded determinism hashes,
  rollback replay, the quick audits with 0 errors and 0 warnings). A change that intentionally alters the
  simulation re-blesses `determinism.txt` and, if it breaks a golden trace on purpose, narrows that trace with a
  `docs/decisions.md` entry and a Rust-side test instead.
- Audits: `cargo xtask audit [map…] [--quick] [--only a,b] [--metrics]`. Models: `cargo xtask assets [--export]`
  (`--export` needs Blender; never in CI — the glb in `assets/models` are committed).
- A new map goes into `fb_maps::GAMES`/`MAPS` and gets audits and Rust tests.
- Network changes: `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` (server + headless
  clients, predictions compared with the server tick by tick); over the real network: `cargo xtask stress --remote
  --host … --domain … --clients 8 --secs 100 --transport udp|ws|auto`.
- Look at a running build without a window: `fb_client --offscreen` (`README.md`, «Отладка»); drive a client with
  `--brp` (`fb/state`, `fb/send`, `fb/dev`, `fb/input`, `fb/ui`); read `stats:`/`metrics:` logs.
- Changing a replicated component or message: bump `PROTOCOL_VERSION` (`core/fb_shared/src/consts.rs`).
- Servers: the client keeps a list (`servers.rs`); a player types a domain, IP or `host:port`, the client finds
  the HTTP API (5887, or https on 443 for a domain) and the server tells the rest: UDP address in the connect
  token, WebSocket URL in the session reply (`--public-ws-url`, else from `Host`/`X-Forwarded-Proto`).
- One Lightyear `Server` listens on UDP and WebSocket. The room reads inputs itself (`play::frame_for`: late presses
  happen on the next tick; Lightyear's copy into `ActionState` is off). Inputs go out at 60 Hz with 15 messages of
  redundancy, input margin 3 ticks: all chosen by stress measurements. The server runs every schedule
  single-threaded (`SingleThreadedExecutor`; a 1-vCPU VPS lost 15% of its core to the multi-threaded executor's
  hand-offs). `vendor/aeronet_websocket` patches the WebSocket server (`TCP_NODELAY`); keep it until aeronet has
  it, carry it over on aeronet updates.
- No traffic budget anywhere (decided by the author): traffic is measured and reported only. Server target: up to 4
  rooms of 8 players in one core (1 vCPU / ~1 GB). Minimum client: 2 cores / 2 GB RAM / OpenGL 3.3. Many players
  sit behind VPNs that drop UDP/443: the game's UDP must stay off port 443 and not look like QUIC; WebSocket on
  443 is the fallback.

## Releases

- `release.yml` (workflow_dispatch only) builds in parallel: `check`, NSIS, AppImage, Flatpak, server `.deb` and
  `.rpm`, then creates GitHub release `v<version>` with `SHA256SUMS`. Packaging logic lives in
  `cargo xtask dist <nsis|appimage|flatpak|deb|rpm>` (files in `packaging/`), so every package builds locally too.
- No signing. The client updates itself from `releases/latest` (`update.rs`): NSIS and AppImage download, check
  the sum, install and restart; Flatpak links to the release page. New release = raise `version` in `Cargo.toml`.
