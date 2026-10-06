# Audit 2026-10-05 — report

> **Исторический документ (аудит 2026-10-05).** Описывает дерево и версию на тот день; многое с тех пор изменено
> или удалено (`docs/decisions.md`, `cargo xtask golden`, `stress --remote`, `deploy/`, золотые трассы TS, OpenGL).
> Текущее состояние — код, `docs/state.md` и `README.md`.

Total check of the tree `K:\fallbeans-rogue-port-to-rust` (content of branch `rogue/port-to-rust`, version
`0.1.0-alpha`; **no `.git` in this copy** — the diff against the untouched snapshot was taken with
`git diff --no-index`). Plan: [`plan.md`](plan.md). Bigger changes that were **not** applied:
[`proposals.md`](proposals.md). All changes: the commit that adds these files.

Every file was read by one of seven parallel audits (A core sim · B maps/arena/audit · C protocol/net/server ·
D client core · E client render/UI · F tooling/CI/packaging/docs · G dependencies), each finding re-checked here before
it was kept. Sources: docs.rs and upstream source (Bevy 0.19.1, Lightyear 0.30.1, aeronet, tungstenite, axum 0.8, ureq
3, NSIS, GitHub Actions changelogs, freedesktop SDK EOL, RustSec advisory DB via `cargo audit` 0.22.2) and
`K:\GameDevLibrary` (image-pipeline, iquilezles books — for the render findings and proposals 5.x).

## Verification

| Step | Result |
|---|---|
| Baseline `cargo build --workspace --all-targets` (cold) | ok |
| `cargo xtask check` after the fixes (fmt, clippy `-D warnings`, all tests: 19 golden traces, determinism hashes 19 maps × 3 seeds, rollback replay, scenarios, quick audits) | **ok — 127 tests, 0 warnings**; `determinism.txt` and every golden trace unchanged |
| `cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05` (UDP, dev build) | **ok** — 0 late inputs, tick p99 68 µs, CPU 9.4 %, 53 MB |
| same, `--secs 45 --transport ws` | **ok** — 0 late inputs, tick p99 70 µs, CPU 12.8 %, 53 MB |
| Release workflow, NSIS build, Flatpak/AppImage, deploy, Blender export | **not run** (side effects / not available here — no `makensis`) |

## Fixed

### Gameplay bugs
- **Tail tag: tails left the room with their holders** (`core/fb_maps/src/tail_tag.rs`). A holder who disconnected
  kept the tail in the list; if every holder left, nobody could score for the rest of the round. Now (server side) the
  orphaned tail goes to the bean without one with the fewest points and a `tails` event is sent. Test
  `core/fb_arena/tests/tail_tag.rs` (fails on the old code, passes now); goldens unchanged.
- **Drum roll: the "no shortcut" zone covered the whole map** (`core/fb_maps/src/drum_roll.rs`): `y > beam_y + 3`
  with no z bounds, and the course ORs every section's zone — any taller section would have fined everybody standing
  on it. Bounded to the section (`z0 < z < end_z`). Latent today; goldens unchanged.
- **`plan_game` panicked with 0 players** (`core/fb_maps/src/director.rs`): empty pool → `bag.remove(0)`; the server
  calls it with `roster().len()`. Plans as for one player now. Test.

### Security and robustness of the server
- **A modified client could drive another player's bean** (`crates/fb_server/src/play.rs`): Lightyear 0.30 writes an
  input message into whatever entity it names (and adds an `InputBuffer`); other pawns are replicated to everyone.
  Added `add_input_validator(authorize_controlled_targets::<NativeStateSequence<FbInput>>)` (pawns carry
  `ControlledBy`).
- **WebSocket: 16 MB frames / 64 MB messages accepted before authentication** (`crates/fb_server/src/net.rs`) —
  enough to hit the unit's `MemoryMax=300M`. Limited to 64 KiB (a client message is ≈1 KB).
- **HTTP API: no header/idle timeout, no connection cap, 2 MB bodies** (`crates/fb_server/src/http.rs`): `axum::serve`
  gives hyper no timer, so slowloris and idle keep-alive connections lived forever. New listener wrapper: ≤512
  connections, closed after 20 s without traffic either way; body limit 16 KiB (= nginx). Test.
- **`/health` queued unbounded work into the main loop** (anyone may ask; late jobs still ran). The main loop now
  publishes the counts every frame; `/health` reads them and answers 503 when they are older than 2 s (same
  "main loop stuck" signal as before).
- **All WebSocket players shared 127.0.0.1 in the PIN-guess limiter** (behind nginx). The real address
  (`X-Real-IP`, trusted only from loopback) is now sealed into the connect token's user data and used when the link
  itself comes from loopback. IPv6 guessers are counted per /64.
- **Memory growth**: per-bean debug trace never cleared when a bean left (bot churn ≈1 GB/day) — now cleared in
  `Arena::remove_pawn`; with `--metrics-every 0` tick samples piled up (≈40 MB/day) — cleared while metrics are off;
  "bad message" WARN logged per message before rate limiting — at most one per connection per second.
- Removed dead `fb_net::DEV_KEY`, `fb_shared::SNAPSHOT_EVERY`, `tick_to_time`; stale docs (`LATE_TICKS`,
  `--public-host`, `RoomOptions::eliminate`).

### Client
- **Leaving a server left a ghost player for 3–10 s** (`net.rs`): the link was despawned in the same step as the
  disconnect, so the server was never told. The link now lives (marker `Closing`) until `Last`, after the disconnect
  packets went out.
- **IPv6 servers unreachable over UDP**: the client socket was always IPv4; it now matches the token's address family
  (`net::local_addr_for`, also the UDP probe). `--server ::1` built `http://::1:5887` → brackets; `[::1]` without a
  port accepted, broken bracketed hosts rejected. Tests.
- **Self-updater could hang forever / leave junk** (`update.rs`): download timeouts (headers 30 s, `SHA256SUMS` 30 s,
  body 120 s + size ÷ 32 KB/s); the partial file (incl. `$APPIMAGE.new`) is removed on any error; a second click
  during a download is ignored; the temp file name is only the asset's file-name part; `open_url` opens only
  `http(s)://`. Test.
- **Panic on a bad connect token** (`.expect` in `spawn_client`) → error, ask again in 2 s. `--fps 0` panic → range
  1–1000. Podium place 0 underflow. `--profile` now only `[A-Za-z0-9_-]` (it is a path segment).
- **Replayed bonus sounds/feed lines after a reconnect** → only newly taken bonuses (the general case: proposal 4.1).
  "Unknown map" error logged every frame → once.
- Animation phases (run/ladder cycles, propeller) grew without bound → wrap at a full turn.
- Per-frame waste: a debug-format string per tinted map piece per frame (material cache keyed by handle id now); about
  ten UI sections formatting their state into a `String` each frame (now hashed directly); the update box redrawn for
  every downloaded chunk (now per percent).

### Rendering and UI
- **Half-float conversion dropped a carry** (`render/env.rs`): `f16(1.9999)` gave 1.0 — parts of the lighting cube
  were half as bright. Tests.
- **Leaving TAA/SSAO left their companions on the camera** (`render/quality.rs`): texture mip bias −1 (shimmer under
  SMAA/FXAA) and the depth/normal/motion-vector prepasses (the cost the lower preset should save). Removed on every
  preset change; re-adding TAA/SSAO brings them back.
- **"Auto" quality dropped the preset when frames were not slow**: the FPS-limit sleep counted as frame time (30 FPS
  cap → Low) and an unfocused window held at 20 FPS by the compositor counted too.
- Scenery mushrooms reset to scale 1 by the prop animation; model LOD distance ignored scale (clouds went low-poly too
  close); two caches never dropped entries of unloaded models; 420 invisible motes drawn when switched off; warm-up
  missed the dithered LOD shader variant (stutter at the first round).
- UI: "✕" not in the game fonts → "×" (all literals checked against the font tables); the «Игра обновляется…» card had
  no way out → «‹ К серверам» on every card; key rebinding outlived the settings screen (a key pressed in play was
  silently bound).

### Core simulation (no output change)
- Ranking with a NaN key could panic (Rust ≥ 1.81 sort) or split players → NaN ranks last in one group. Test.
- Nav: `partial_cmp().unwrap()` panic path, NaN position mapped to cell 0, A* generation counter overflow; per-decision
  route clone in `nav_to` (20 Hz × bots) removed; compile-time check that `BOT_DT` matches `TICK_RATE / BOT_EVERY`;
  `spec_problems` checks the finish width.
- Tests: golden-trace comparisons could be skipped silently (now every row/world hash must be consumed, lengths
  checked); `GAMES`/`MAPS` consistency test; respawn test stepped one tick twice; messages pointed at the removed
  `cargo xtask golden`. `fb_audit`: unknown options were ignored (`--quik` ran the full set) → exit 2; `fb_audit lobby`
  said "unknown map".

### Tooling, CI, packaging, docs
- **NSIS silent update** waited a fixed 2 s for the game to close; NSIS silently skips files it cannot write → old exe
  with new assets. Now polls up to 30 s until the exe is writable. Added `InstallDirRegKey` (updates of a game
  installed elsewhere went to the default folder as a second copy).
- **`SHA256SUMS` would not match the server packages**: GitHub renames `~` to `.` in uploaded names
  (`0.1.0~alpha`). The release job renames before hashing.
- **GitHub Actions**: least-privilege token (only the release job writes); Node 20 actions bumped (upload-artifact v6,
  download-artifact v7); CI cancels superseded runs (the CI workflow was removed later: only `release.yml` is left).
- **xtask**: shell injection through `--domain` / an `--host` starting with `-` (ssh option) in deploy; ssh/scp could
  hang forever on a dead link (keep-alives added); a server that died at start went unnoticed in `stress`/`dev`;
  `stress --remote` hashed the binary with `sha256sum` (absent on Windows → re-upload every run) → hashed in Rust.
- `Cargo.toml`: `exclude = ["vendor/*"]` (cargo commands inside the vendored crate failed); `tokio` 1.53.1 → 1.53.2
  (lockfile, bug fixes).
- `blender/export.py` default output was the old TS path; `.editorconfig` said 2 spaces for Rust (rustfmt uses 4);
  `.gitattributes` stale comment, `*.ico`/`*.gz` binary; README: false known issue, golden traces exist for all maps,
  `INPUT_REDUNDANCY` value, missing files in lists.

## Approved follow-ups (applied the same day)

The author approved proposals 1.1, 2.1, 4.1, 7.1 and 1.9 (details and the variant chosen: `proposals.md`, each marked
"Applied"); NSIS 3.12 was installed (winget `NSIS.NSIS`, installer hash checked by winget).

| Item | What changed | Verified |
|---|---|---|
| 1.1 | `/api/session` 60/min per address → 429; per address ≤16 connections, ≤3 rooms opened, ≤1 practice room; IPv6 per /64; loopback exempt | 2 new tests; stress |
| 2.1 | JS-exact `m::max/min` (+0/−0 ties), `max_js`/`min_js`; 197 call sites in `core/`; `f64::max/min` disallowed | goldens exact, `determinism.txt` unchanged, release tests |
| 4.1 | client skips map events already applied to the arena; server marks re-sent history (`MapEventMsg::history`) → applied without sounds/feed/notes | build + tests; stress |
| 7.1 | AppImage job: `ubuntu-latest` + `container: ubuntu:22.04` | YAML parsed; runs only on GitHub |
| 1.9 | `PROTOCOL_VERSION` 18 → 19 | — |
| NSIS | `cargo xtask dist nsis` builds `dist/FallBeans-0.1.0-alpha-setup.exe` (23.9 MB, makensis 3.12, no warnings); not installed/run here | built |

After these: `cargo xtask check` ok (130 tests); UDP stress 8 clients / 100 s ok (0 late inputs, tick p99 102 µs,
CPU 10 %, 51 MB). Same lone "unexplained divergence" per client as before (≈1 per client, under the limit).

## Checked and left as is (intentional or fine)
Determinism rules hold in `core/` (no `HashMap`, wall clock, unseeded RNG, platform maths, FMA, f32); three.js
operation order; libm without FMA; identity tokens (HMAC, constant-time, no expiry by design); PIN only to the host,
never logged; `X-Real-IP` trusted only from loopback; rounds always end (finite durations, no humans left,
`MAX_CATCHUP`); all client HTTP off the main thread with timeouts; connection flow has no endless waits; colour spaces
(no double gamma), PBR Neutral tonemapping, reverse-Z; EASU matches `ffx_fsr1.h`; all 19 models present and used;
every screen has an exit; Russian text has no typos; no deprecated Bevy 0.19 APIs (warning-free build); the vendored
aeronet patch is minimal and still needed (upstream 0.21.0 / 0.22.0-rc.1 lack it); dependencies current except
those pinned to Bevy 0.19 (Bevy 0.20 is only rc.2; Lightyear has no release for it); `cargo audit`: one vulnerability
(steamworks, lockfile only — never compiled), unmaintained `paste`, `rustls-pemfile` (lockfile only), `ttf-parser`
(Linux client via winit Wayland decorations — upstream).

## Validation (2026-10-05, in the git repository after PR #11)
Every "Fixed" item above and every approved follow-up was found in the code; `cargo xtask check` passed (171 tests,
0 warnings, quick audits clean). Every open proposal still described the code. The quick ones were then applied (marked
in `proposals.md`): 1.2, 1.3, 1.4, 1.7, 1.8, 2.4, 2.5, 2.7, 2.8 (finiteness), 2.10, 3.2, 5.11 (wording), 7.3 (without
`<releases>`), 7.7, 7.9, 7.10 (version info).

## Known remaining issues
See `proposals.md` — the most important still open: no gamepad/keyboard menu navigation (5.6), `appimagetool`
unpinned (7.2), corrupt settings file (4.2), replays after a catch-up skip (2.2), IPv4-only UDP (1.10).

## Stress note
Both stress runs report, per client, exactly one "unexplained divergence" ≈190–265 ticks after the start (1.4 per
minute against the 30/min limit). It was not compared with the pre-audit build (that would need rebuilding the
snapshot); worth a look with `target/stress` logs on the next network change.
