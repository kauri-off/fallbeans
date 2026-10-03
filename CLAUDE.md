# Fall Beans — notes for Claude

Fall Beans is a party game (up to 8 beans, races / survival / points rounds, 120 Hz authoritative server) being
**rewritten from a browser game into a native Rust + Bevy 0.19 + Lightyear 0.30 client and server** (workspace
`rust/`, branch `rogue/port-to-rust`). The Rust server has run in production since 2026-10-01; the TS version (Bun
server, three.js + Preact client) is frozen, off the host, and kept on `master` as the porting source — a
prototype, not a reference: where it behaves badly, the Rust version does better. Player-facing text and the
port's docs are Russian; code, comments and tool output are English.

Game rules to keep: open to everyone, no site PIN. Players meet in rooms: the client opens at the room list, anyone
may create one room (they are its host whenever they are in it; a stand-in hosts while they are away) and be in one
room at a time; private rooms ask for a 4-digit PIN only their host sees. A player is an identity token (`fb_id`).

## Port state: read it first, save it before you stop

Everything about the port's progress lives in `rust/port/`. Each file has one job:

| File | Holds | Changes |
| --- | --- | --- |
| `state.md` | the handoff: date, branch, last commit, what is uncommitted; one line per phase; **where to start** (1–3 concrete next steps, with enough context to begin cold); work started and not finished; what is broken; what is left of the current phase's gate; leftovers of earlier phases | rewritten whole, never appended |
| `plan.md` | the spec: goals, stack, architecture, requirements, phases with their gates | only when the spec itself changes (with a `decisions.md` entry saying why); no progress marks |
| `decisions.md` | everything decided or found **outside the plan**: deviations, workarounds, temporary hacks (with "Пересмотреть (Фаза N)"), rejected options, TS bugs found | entries appended at the bottom, never rewritten; a dropped decision gets "→ отменено: date" |
| `phases/N.md` | one per phase: measurements worth keeping while it runs; at close, the gate verdict with evidence and what moved to other phases | measurements as taken; verdict at close |

- **Start of a thread:** read `rust/port/state.md`, then what it points to (plan sections, decisions, `rust/README.md`).
- **Before a thread ends** (the task is done, the user stops, or the context is getting long) — without being asked:
  rewrite `state.md` so a fresh thread knows where to start, and add a `decisions.md` entry for anything new that
  the plan does not say.
- **When a phase closes:** write the verdict and the moved items in `phases/N.md`, put the moved items into the
  target phases in `plan.md`, and rewrite `state.md` for the next phase.
- **Do not track completion.** No "done" lists or ✓ marks anywhere: the code, tests and git show what is done.
  Record only what they do not show — where to start, what is half-done, why something is the way it is.
- `rust/README.md` describes the code as it is (layout, commands, rules, known issues); `rust/deploy/README.md` —
  the production host.

## Rules

- Git: the repo is on GitHub (kauri-off/fallbeans, `master`). Do not commit, push or open PRs unless asked.
- Line endings are LF (`.gitattributes`). rustfmt (`.rs`) and Biome (TS/JSON/CSS) format on every edit (hook in
  `.claude/settings.json` → `scripts/hooks/format-edited.ts`); files changed by scripts need `cargo fmt --all`.
- Deploy only when asked: `cd rust && cargo xtask deploy` (`rust/README.md`, "Деплой"). The TS version is not
  deployed any more. There is no "the game is updating" step yet: connected clients just lose the connection.
- The simulation must stay deterministic: `rust/core/` uses no wall clock, no unseeded randomness, no `HashMap`,
  maths only through `fb_shared::m` (`rust/core/clippy.toml` enforces it). The same rule held in TS (`b.rng`, the
  seed, sim time).

## Rust: essentials

- `rust/core/` (fb_shared, fb_sim, fb_maps, fb_arena, fb_audit) is the deterministic simulation and its tools: no
  Bevy, f64, operation order as three.js. `rust/crates/` (fb_net, fb_server, fb_client) is Bevy/Lightyear code.
- Verify with `cd rust && cargo xtask check` (fmt, clippy -D warnings, tests: golden traces against TS, recorded
  determinism hashes, rollback replay, the quick audits with 0 errors and 0 warnings).
- Audits: `cargo xtask audit [map…] [--quick] [--only a,b] [--metrics]`; `cargo xtask audit --vs-ts` runs the TS
  audits with the same libm and requires identical results (bot balance included) apart from wall times.
- Porting from TS: port line by line and cover it with a golden trace (`scripts/golden.ts` →
  `rust/core/fb_arena/tests/golden/`; `cargo xtask golden [map…]` re-exports with bun, TS computing with
  `fb_shared::m` built to WebAssembly). A new map goes into `fb_maps::GAMES`/`MAPS` (order of `src/games/index.ts`)
  and `golden!` in `tests/golden.rs`; check that every section of its pools lands in at least one traced seed.
- Network changes: `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` (server + headless
  clients, predictions compared with the server tick by tick); over the real network: `cargo xtask stress --remote
  --clients 8 --secs 100 --transport udp|ws|auto` (a probe server next to the game on the production host; the
  production service is not touched).
- Look at a running build: `cargo xtask dev --clients 2 [--autopilot] [--lag 75]`, or `fb_client --screenshot f.png
  --exit-after 15` against a running `fb_server`. No BRP probe yet (Phase 4); read `stats:`/`metrics:` logs.
- Changing a replicated component or message: bump `PROTOCOL_VERSION` (`rust/core/fb_shared/src/consts.rs`).
- One Lightyear `Server` listens on UDP and WebSocket. The room reads inputs itself (`room::frame_for`: late presses
  happen on the next tick; Lightyear's copy into `ActionState` is off). Inputs go out at 60 Hz with 15 messages of
  redundancy, input margin 3 ticks: all chosen by stress measurements. The server runs every schedule
  single-threaded (`SingleThreadedExecutor`; the 1-vCPU host lost 15% of its core to the multi-threaded executor's
  hand-offs). `rust/vendor/aeronet_websocket` patches the WebSocket server (`TCP_NODELAY`); keep it until aeronet
  has it, carry it over on aeronet updates.
- No traffic budget anywhere (decided by the author): traffic is measured and reported only. Server target: up to 4
  rooms of 8 players on the current host (1 vCPU / 0.9 GB, shared). Minimum client: 2 cores / 2 GB RAM. Many players
  sit behind VPNs that drop UDP/443 (`rust/port/plan.md` §5): the game's UDP must stay off port 443 and not look
  like QUIC; WebSocket on 443 is the fallback.

## The TS version (porting source)

Bun ≥ 1.4. Run it to compare behaviour or numbers with the port:

```sh
bun run check          # tsc + biome + vitest (includes the quick audits)
bun run audit          # all audits incl. multi-seed bot balance; --quick, --only a,b, map ids, --metrics
bun run bench          # server cost per map vs bench/baseline.json (saved on Windows)
bun run dev            # server + Vite, permanent room `dev`: http://localhost:5173/fallbeans/?room=dev
```

- Browser: always the Playwright MCP (`mcp__playwright__*`), driving `window.__fallbeans` (probe:
  `src/client/debug/probe.ts`; dev commands: `DevCmdSchema` in `src/shared/protocol.ts`, server `--dev` only).
  Server state without a browser: `curl "http://127.0.0.1:7777/fallbeans/api/debug/state?format=text"` (also
  `logs`, `trace`, `replay`, `audit`). Rounds headless: `bun run trace <map>`, `bun run replay`.
- Where things are: `src/shared` protocol, codec, rules · `src/sim` physics, world, builder, course, props, bots,
  nav, looks · `src/games/*` maps (`meta.ts`, `map.ts`; registry `index.ts`) · `src/server/rooms` hub, room,
  arena, director, awards, replay · `src/server` auth, debugApi · `src/client` game, net, `ui/`, `hud/`, `menu/`,
  rendering in `src/client/game/` (`renderer.ts`, `postfx.ts`, `xegtao.ts`, `fsr.ts`, `lod.ts`, `materials.ts`,
  `scenery.ts`, `decor.ts`, `environment.ts`) · `src/audit` audits · `scripts/` CLI tools (`golden.ts`,
  `golden-math.ts` for the port) · `blender/` source models + `export.py` (`bun run assets --export`).
