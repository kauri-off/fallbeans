# Fall Beans — notes for Claude

Browser party game: Bun server (authoritative 120 Hz sim, WebTransport + WebSocket), three.js + Preact client. The UI and player-facing text are Russian; code, comments and tooling output are English.

Open to everyone, no site PIN. Players meet in rooms: the page opens at the room list, anyone may create one room (they are its
host whenever they are in it; a stand-in hosts while they are away) and be in one room at a time; private rooms ask for a 4-digit
PIN only their host sees. A player is an identity token in localStorage (`fb_id`), so two tabs are one player (`?profile=b` in dev
gives a tab its own).

## Rules

- Git: the repo is on GitHub (kauri-off/fallbeans, `master`). Do not commit, push or open PRs unless asked.
- Line endings are LF (`.gitattributes`). Biome (TS/JSON/CSS) and rustfmt (`.rs`) format on every edit (hook in
  `.claude/settings.json` → `scripts/hooks/format-edited.ts`).
- Deploy only when asked: `cd rust && cargo xtask deploy` (the Rust server; `rust/README.md`, "Деплой"). The TS version is
  no longer deployed: it was taken off the host on 2026-10-01 (`bun run deploy` and `deploy/` are gone from this branch).
  There is no "the game is updating" step yet: connected clients just lose the connection.
- The server simulation must stay deterministic: no `Math.random`/`Date.now` in sim or map logic (use `b.rng`, the seed, sim time). The determinism audit and the replay test catch violations.

## Verify changes

```sh
bun run check          # tsc + biome + vitest (includes the quick audits: 0 errors AND 0 warnings)
bun run audit          # all audits incl. multi-seed bot balance; --quick, --only a,b, map ids, --metrics
bun run bench          # server cost per map vs bench/baseline.json; --profile for hot functions
bun run assets         # validate models (gltf-validator, budgets, names the code needs)
```

## Look inside a running game (dev server: `bun run dev`; it keeps a permanent room `dev`, `?room=dev`)

- Browser: always use the Playwright MCP (`mcp__playwright__*`) — not the in-app preview pane or ad-hoc scripts. Open
  `http://localhost:5173/fallbeans/?room=dev` (`?practice=<map>` a practice round, no query = room list), then drive the game
  with `browser_evaluate` against `window.__fallbeans`, e.g. `await p.dev({c:'start', games:['door-dash'], bots:3})` with
  `const p = window.__fallbeans`; `browser_take_screenshot` to look, `browser_console_messages` for errors.
- Probe (`src/client/debug/probe.ts`): `state/snapshot/time/body/beans/colliders/world/net/render/memory/hud/logs/errors/msgs`,
  `input.hold({x,z}|{f,r}, ms)/press('jump')/walkTo(x,z)`, `camera.set(eye, look)/free()`, `shot(true)` (no UI), `dev(cmd)`,
  `frames(n)`, `waitFor(fn)`, `profile.run()/passes()/ablate()/scene()`, `sections()`, `gpu('passes')`,
  `fx({shadows, ao, grade, msaa, smaa, temporal, motes})` (graphics features on/off, as in the settings),
  `upscale('off'|'ultra'|'quality'|'balanced'|'performance')` (FSR 1), `lod({enabled, force, bias})`, `decorate({crown, tail})`,
  `rooms()/createRoom(title, private)/joinRoom(id, pin)/leaveRoom()`, `chat(text)/chatLog()`.
- Dev commands (`DevCmdSchema` in `src/shared/protocol.ts`, only with `--dev`): skipIntro, warp, endRound, start {games, bots, rounds},
  lobby, rate {k} (0 pauses), step, teleport, goto {spawn|finish|checkpoint i}, bot {near}, bots {on}, kill, knock, grab, seed.
- Server state without a browser: `curl "http://127.0.0.1:7777/fallbeans/api/debug/state?format=text"` — also `health`,
  `logs?level=warn`, `errors` (client reports), `trace?id=3&s=10`, `replay?i=0|current`, `audit?map=x&only=a`; `&room=<id or number>`
  picks a room (default: the first, `dev`). Page: `/fallbeans/debug/`.
- Bugs in a round: `bun run trace <map> [--stuck] [--bot n --from t --to t]` (headless bots), `bun run replay [--i 0] [--bot id]`
  (re-simulates the last real rounds of the dev server exactly), `bun run stress` (fake clients with latency/loss).

## Where things are

`src/shared` protocol, codec, rules, prof · `src/sim` physics, world, builder, bots, nav · `src/games/*` maps (`meta.ts`, `map.ts`) ·
`src/server` main, config, auth (tickets, identities), debugApi, diag · `src/server/net` gateway (hello, routing), http, webtransport ·
`src/server/rooms` hub (room list, create/join/leave, who is where), room (players, host, bots, chat, game flow, dev commands),
players, clock, roomDebug, arena (simulation, recording), director, awards, replay · `src/client` game, net, `debug/`
(capture, probe, profiler, gpuTimer, debug page), `ui/` App, `home/` (room list), `menu/` (Esc menu: room, host setup, dev),
`hud/` (HUD, chat, controls line, name tags) · `src/audit` audits (maps.ts, systems.ts, run.ts) · `scripts/` CLI tools ·
`blender/` source models + `export.py` (`bun run assets --export [--dry-run]`: modifiers applied, AO baked per model in
Cycles and attached as each material's occlusion map, compressed with ffmpeg from `ffmpeg-master-latest-win64-gpl/` or $FFMPEG; new models: add an `A_<name>` collection and the
name to `MODEL_NAMES`). Client rendering: `src/client/game/renderer.ts` (presets, shadows, feature toggles), `postfx.ts` (the post
pipeline: scene/MSAA, composite, SMAA, TAA, output), `xegtao.ts` (AO), `fsr.ts` (FSR 1 EASU + RCAS), `lod.ts` (M0–M6 + dithered cross-fade), `materials.ts` (surfaces, patterns, fade chunk),
`scenery.ts` + `decor.ts` (clouds, islands, themed set pieces, the ground below). Lighting: the sun is the only light (one
shadow map, no cascades, statics baked: `shadowBake.ts`); ambient light is image-based (`environment.ts`). Looks: `src/sim/looks.ts` (per map in
`defineMap(meta, build, looks)`: palettes, patterns, sky, sun, fog, motes, decor set; a round picks one by seed; visual only).

## Rust port (branch `rogue/port-to-rust`, workspace `rust/`)

The game is being ported to a native Rust + Bevy 0.19 + Lightyear 0.30 client and server. The Rust server already runs in
production (Phase 0, closed 2026-10-01: `jump-club`); the TS version is frozen, off the host, and kept on `master` as the porting source
(a prototype, not a reference: where it behaves badly, the Rust version does better). Read before working on it:

- `plan.md` (Russian): the spec and the phases; section 12 says where we are and what comes next.
- `rust/PHASE0.md`: Phase 0 state, measurements, review of every technical decision, open items.
- `rust/README.md`: layout, commands, code rules, debugging, known issues.
- `rust/deploy/README.md`: the production host (hardware, what else runs there, ports, ufw, nginx, load measured).

Essentials:

- `rust/core/` (fb_shared, fb_sim, fb_maps, fb_arena) is the deterministic simulation: no Bevy, f64, maths only via
  `fb_shared::m`, operation order as three.js; `rust/core/clippy.toml` forbids the rest (it applies to core only).
  `rust/crates/` (fb_net, fb_server, fb_client) is Bevy/Lightyear code.
- Verify with `cd rust && cargo xtask check` (fmt, clippy -D warnings, tests: golden traces against TS, recorded
  determinism hashes, rollback replay). Network changes: `cargo xtask stress --clients 8 --secs 100 --lag 75
  --jitter 15 --loss 0.05` (server + headless clients, compares predictions with the server tick by tick).
- Look at a running build: `cargo xtask dev --clients 2 [--autopilot] [--lag 75]`, or `fb_client --screenshot f.png
  --exit-after 15` against a running `fb_server`. There is no BRP probe yet (Phase 4); read `stats:`/`metrics:` logs.
- Over the real network: `cargo xtask stress --remote --clients 8 --secs 100 --transport udp|ws|auto` runs a probe
  server (this tree's Linux build) on the production host next to the game (UDP 5890, `/fallbeans/ws-probe`) and
  compares the clients run here with it tick by tick. The production service is not touched.
- Porting from TS: the source of truth is TS on `master`; port line by line and cover it with a golden trace
  (`scripts/golden.ts` → `rust/core/fb_arena/tests/golden/`, `cargo xtask golden` re-exports with bun).
- Changing a replicated component or message: bump `PROTOCOL_VERSION` (`rust/core/fb_shared/src/consts.rs`).
- One Lightyear `Server` listens on UDP and WebSocket. The room reads inputs itself (`room::frame_for`: late presses
  happen on the next tick; Lightyear's copy into `ActionState` is off). Inputs go out at 60 Hz with 15 messages of
  redundancy, input margin 3 ticks: all chosen by stress measurements. The server runs every schedule single-threaded (`SingleThreadedExecutor`; the 1-vCPU host lost 15% of its
  core to the multi-threaded executor's hand-offs). `rust/vendor/aeronet_websocket` patches the
  WebSocket server (`TCP_NODELAY`); keep it until aeronet has it, carry it over on aeronet updates.
- No traffic budget anywhere (decided by the author): traffic is measured and reported only. Server target: up
  to 4 rooms of 8 players on the current host (1 vCPU / 0.9 GB, shared). Minimum client: 2 cores / 2 GB RAM. Many players sit behind VPNs that drop UDP/443 (plan.md §5): the game's UDP
  must stay off port 443 and not look like QUIC; WebSocket on 443 is the fallback.
