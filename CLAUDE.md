# Fall Beans — notes for Claude

Browser party game: Bun server (authoritative 120 Hz sim, WebTransport + WebSocket), three.js + Preact client. The UI and player-facing text are Russian; code, comments and tooling output are English.

Open to everyone, no site PIN. Players meet in rooms: the page opens at the room list, anyone may create one room (they are its
host whenever they are in it; a stand-in hosts while they are away) and be in one room at a time; private rooms ask for a 4-digit
PIN only their host sees. A player is an identity token in localStorage (`fb_id`), so two tabs are one player (`?profile=b` in dev
gives a tab its own).

## Rules

- Git: the repo is on GitHub (kauri-off/fallbeans, `master`). Do not commit, push or open PRs unless asked.
- Line endings are LF (`.gitattributes`). Biome formats on every edit (hook in `.claude/settings.json`).
- Deploy only when asked: `bun run deploy` (see README). Protocol changes (`PROTOCOL_VERSION`) make players refresh.
- The server simulation must stay deterministic: no `Math.random`/`Date.now` in sim or map logic (use `b.rng`, the seed, sim time). The determinism audit and the replay test catch violations.

## Verify changes

```sh
bun run check          # tsc + biome + vitest (includes the quick audits: 0 errors AND 0 warnings)
bun run audit          # all audits incl. multi-seed bot balance; --quick, --only a,b, map ids, --metrics
bun run e2e            # browser tests (needs: bun run build:client)
bun run visual         # screenshot per map vs this machine's baselines (visual:update after intended changes)
bun run bench          # server cost per map vs bench/baseline.json; --profile for hot functions
bun run assets         # validate models (gltf-validator, budgets, names the code needs)
```

## Look inside a running game (dev server: `bun run dev`; it keeps a permanent room `dev`, `?room=dev`)

- Headless browser: `bun run probe "<js>" …` runs snippets against `window.__fallbeans` (as `p`) in headless Edge, e.g.
  `bun run probe "await p.dev({c:'start', games:['door-dash'], bots:3})" "await p.dev({c:'skipIntro'})" "p.snapshot()"`.
  It enters the `dev` room (`--room id` another, `--home` stays at the room list, `--practice map` a practice round).
  The in-app browser pane pauses animation frames while hidden or unfocused; prefer `probe` for anything that needs frames.
- Probe (`src/client/debug/probe.ts`): `state/snapshot/time/body/beans/colliders/world/net/render/memory/hud/logs/errors/msgs`,
  `input.hold({x,z}|{f,r}, ms)/press('jump')/walkTo(x,z)`, `camera.set(eye, look)/free()`, `shot(true)`, `dev(cmd)`,
  `frames(n)`, `waitFor(fn)`, `profile.run()/passes()/ablate()/scene()`, `sections()`, `gpu('passes')`,
  `fx({smaa, temporal})` (post effects on/off), `lod({enabled, force, bias})`, `decorate({crown, tail})`,
  `rooms()/createRoom(title, private)/joinRoom(id, pin)/leaveRoom()`, `chat(text)/chatLog()`.
  Snippets can save screenshots mid-run: `await shot('/tmp/a.png')`. Probe/bench pick the GPU backend per OS (scripts/browser.ts).
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
`blender/` source models + `export.py` (`bun run assets --export [--dry-run]`; new models: add an `A_<name>` collection and the
name to `MODEL_NAMES`). Client rendering: `src/client/game/renderer.ts` (presets, passes), `postfx.ts` (scene/MSAA,
temporal AA), `lod.ts` (M0–M3 + dithered cross-fade), `materials.ts` (surfaces, patterns, fade chunk),
`scenery.ts` + `decor.ts` (clouds, islands, themed set pieces, the ground below). Looks: `src/sim/looks.ts` (per map in
`defineMap(meta, build, looks)`: palettes, patterns, sky, sun, fog, motes, decor set; a round picks one by seed; visual only).
