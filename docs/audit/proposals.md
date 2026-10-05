# Audit 2026-10-05 — proposals (changes not applied)

Everything here was found by the total audit (`docs/audit/plan.md`, results in `docs/audit/report.md`) and **not**
applied because it changes the simulation (golden traces / determinism hashes), gameplay rules, the network
protocol or tuning, the look of the game, or needs a decision, a measurement or a run on real hardware / a real host.

Each entry: **problem → impact → variants → recommendation**. Priority: 🔴 high · 🟠 medium · 🟢 low.
Paths are relative to the repository root.

---

## 1. Server: security and abuse

### 1.1 🔴 One script can occupy the whole server (unlimited identities)
> **Applied 2026-10-05 (approved by the author):** both variants. `/api/session` 60/min per address → 429; per address 16 connections, 3 rooms opened, 1 practice room (`auth::address_key`: IPv6 per /64, loopback exempt — dev servers and stress runs). Tests `budgets_per_address_spare_this_machine`, `one_address_cannot_take_the_whole_server`.

**Problem.** `POST /api/session` issues a new identity and connect token to anyone, without limits
(`crates/fb_server/src/http.rs`, `session`). With fresh identities one client can hold all 16 rooms (`MAX_ROOMS`, one
per identity), all 3 practice rooms (`MAX_PRACTICE_ROOMS`) and all 256 netcode slots (≈50 tokens/s: links without a
hello are closed only after 5 s).
**Impact.** Denial of service for every player on a public server with one cheap script.
**Variants.**
1. *Rate-limit `/api/session` per address* (X-Real-IP aware — the address is already computed by `client_ip`; IPv6 per
   /64 like the new `auth::guesser`), e.g. 30 requests/min → `429`. The client already treats any non-2xx as a failed
   attempt and retries after 2 s, so no client change.
   ```rust
   // http.rs, session(): before issuing the token
   let ip = client_ip(peer.remote, &headers).0;
   if !api.session_limiter.lock().unwrap().allow(&ip.to_string(), now_ms()) {
       return respond(json!({ "error": "busy" }), false, StatusCode::TOO_MANY_REQUESTS);
   }
   ```
   (`Limiter` needs a per-instance budget instead of the hard-coded 5/30.)
2. *Per-address caps in the hub*, using the real address now sealed into the token (audit fix F5): live sessions
   (e.g. 16), rooms created (e.g. 3, `creator_ip` on `Room`), practice rooms (1).
   ```rust
   // hub.rs create(), after the `owned` check
   let ip = self.sessions[&conn].ip.clone();
   if self.listed().filter(|(_, r)| r.creator_ip.as_deref() == Some(&ip)).count() >= ROOMS_PER_IP {
       return self.deny(conn, None, DenyReason::Limit, "С этого адреса открыто слишком много комнат");
   }
   ```
**Recommendation.** Both. Variant 2 changes the rules ("anyone may create one room") for players sharing an address
(LAN party behind one NAT, a VPN exit): the numbers need the author's choice.

### 1.2 🟠 PIN entry can be locked server-wide
**Problem.** `auth::Limiter` has a global cap of 30 guesses/min for the whole server, and correct PINs count too. Six
addresses guessing lock every private-room join on the server; the debug-login limiter can be exhausted the same way.
**Variants.** 1) Second bucket per room (`allow(ip, scope, now)`), count only failed guesses. 2) Keep a global cap but
only for failures and much higher (e.g. 300/min).
**Recommendation.** 1 (with a test in `rooms/tests.rs`).

### 1.3 🟠 Start/Abort (and Access) spam stalls the shared tick
**Problem.** `Start` builds the round's arena inside the tick (up to ≈6 ms); Start and Abort are allowed at the room's
60 msg/s. A host alternating them costs ≈180 ms per second of the single-threaded loop that all rooms share. Toggling
`Access` (new PIN each time) is similar but cheap.
**Variants.** 1) Per-room cooldown on starting a game (2 s, `last_start: u64`), deny with a toast. 2) A separate budget
(≈5/s) for state-changing messages (Start, Abort, AddBot, RemoveBot, Fill, Access, Playlist).
**Recommendation.** 1, with a test.

### 1.4 🟠 WebSocket handshake without timeout (vendored aeronet)
**Problem.** `vendor/aeronet_websocket/src/server/backend.rs` spawns a session entity and task on every TCP accept,
before the WebSocket handshake, and the handshake has no timeout. On a directly exposed 5889 (LAN packages, no nginx)
silent TCP connections pile up entities, tasks and file descriptors. `max_write_buffer_size` is `usize::MAX`.
**Variants.** 1) Wrap `accept_hdr_async_with_config` in `tokio::time::timeout(Duration::from_secs(10), …)`; optional
cap of concurrent sessions. 2) Leave the vendor patch minimal and rely on nginx in front (document: never expose 5889
directly).
**Recommendation.** 1 — the vendored patch grows by ~5 lines; also send it upstream together with `TCP_NODELAY`.

### 1.5 🟢 Debug key in the URL
`GET …/api/debug/login?key=…` puts the key into nginx access logs and shell history. Accept `POST` with the key in
the body (or `Authorization: Bearer`), keep `GET` only under `--dev`, update README «Отладка».

### 1.6 🟢 Broadcast amplification of lobby updates
`Name`, `Outfit`, `Color` each send a full lobby to every member, `Emote` broadcasts; one player at 60 msg/s →
≈480 lobby messages/s. Coalesce lobby sends to once per tick with a dirty flag (keep welcome ordering), or a lower
budget for these messages.

### 1.7 🟢 Chat keeps invisible / bidi characters
`fb_shared::text::sanitize_chat` only replaces control characters: bidi overrides, zero-width and tag characters reach
everyone's chat (names and titles already drop them with `is_other`). Separately, names lose the ZWJ, which breaks
emoji sequences (👨‍👩‍👧). Filter `is_other` in chat except ZWJ between emoji; allow ZWJ in names the same way.
Player-visible: author's call.

### 1.8 🟢 Bean ids above 65535
`check_pid` caps ids at 65535, `Room::next_id` only grows: after heavy bot churn the host can no longer remove those
bots or hand the host role. Only the abusing room is affected. Drop the bound in `check_pid` (ids are `u32` on the
wire) or reuse ids.

### 1.9 🟢 `PROTOCOL_VERSION` bump to check
> **Applied 2026-10-05 (approved by the author):** bumped to 19 (it also covers the `history` flag of 4.1).

`docs/decisions.md` records the bump to 18 with the respawn entry; the later entries adding `Hold` and the animation
code in `RemotePose` record none. This tree has no git history to check: run `git log -S PROTOCOL_VERSION` on the branch
and bump to 19 before the first release if 18 predates them. (Since nothing is released yet, bumping anyway is free.)

### 1.10 🟢 Server UDP is IPv4-only
`crates/fb_server/src/net.rs` binds `0.0.0.0` for UDP while the connect token may name an IPv6 address (the client now
opens a socket of the token's family — audit fix). Variants: bind `[::]` dual-stack (`IPV6_V6ONLY=0`; Windows and
Linux both allow it, but socket2 is needed to set the option explicitly); or always put an IPv4 address in the token;
or document "IPv6: WebSocket only". Recommendation: dual-stack, then `cargo xtask stress` and a LAN test.

---

## 2. Simulation (would re-bless or touch determinism)

### 2.1 🟠 `f64::max` / `f64::min` and the sign of zero
> **Applied 2026-10-05 (approved by the author):** `fb_shared::m::max/min` + trait `MinMaxJs` (`max_js`/`min_js`), 197 call sites in `core/`, `m::clamp` too; `f64::max/min` disallowed in `core/clippy.toml`. Goldens exact, `determinism.txt` unchanged (no tested run hits a tie that matters). The jump-club seed-1 5.6e-17 gap is not visible in the committed 120-tick rows, so this neither confirms nor rules it out.

**Problem.** Rust documents that on a ±0 tie `max`/`min` may return either operand ("non-deterministic" w.r.t. the
sign of zero). The simulation has ≈120 such calls (26 `.max` + 17 `.min` in `physics.rs` alone) and the project depends
on the sign of zero (JS semantics, golden traces). Dev (opt-level 1) and release (thin LTO) builds — i.e. client and
server — are not guaranteed to agree. One possible suspect for the unexplained `jump-club` seed 1 golden difference of
5.6e-17 (unverified).
**Variant.** JS-exact `m::max` / `m::min` in `fb_shared::m`, +0 wins a ±0 tie for max (−0 for min), NaN as now:
```rust
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a > b { a } else if b > a { b }
    else if a == b { if a == 0.0 && a.is_sign_negative() { b } else { a } }
    else if a.is_nan() { b } else { a }
}
```
Replace `.max(`/`.min(` throughout `core/`, add `f64::max`/`f64::min` to `disallowed-methods` in `core/clippy.toml`,
run `cargo xtask check`; goldens very likely stay, re-bless `determinism.txt` if anything moves.
**Recommendation.** Do it before the first release (cheap insurance for client/server agreement).

### 2.2 🟠 Replays never match after a server catch-up skip
`room.rs` (≈1210) sets `arena.tick = target - MAX_CATCHUP` when the server falls behind; the recording has no trace of
the skip and `replay` steps every tick (bot RNG draws differ too). Add `Arena::skip_to(k)` that records a `skip` op and
have `replay` apply it. No golden change.

### 2.3 🟠 Scaled colliders
Collision contacts and raycasts work in the collider's local units and the broad-phase radius ignores scale; the only
scaled collider is the rolling ball, so ball hits are slightly off while a ball grows in or shrinks out (TS-faithful,
in the goldens). Variants: 1) debug-assert that collider nodes are unscaled (balls allowed) and leave it; 2) scale a
child model node instead of the collider (re-bless ball maps + `decisions.md` entry). Recommendation: 1 now.

### 2.4 🟢 Degenerate (scale 0) colliders
A collider scaled to 0 inverts to a zero matrix; `contact` then reports a hit with a NaN depth and poisons the bean.
No map does it. Mark the collider degenerate in `Collider::sync` when its inverse is not finite (`contact` → false,
`raycast` → −1). No re-bless.

### 2.5 🟢 `course::Gate::held` while pressed lacks the `max(0, t − at)` clamp
The released branch clamps, the pressed one does not: during client rollback at `t < at` the gate is misplaced
(server unaffected). Add the clamp; check the rollback-replay test and stress.

### 2.6 🟢 Hot-path allocations in the physics step
`Body::step` allocates two `Vec`s per body per tick, so do `snap_down`, `fits`, `grab_ledge`, `push_out`;
`Bodies::ids()` returns a fresh `Vec` (`coop_gate` calls it every tick); `course_brain` / `door_rows` build `Vec`s per
bot decision. A `StepScratch` buffer passed by the caller (signature change in `fb_arena` and `fb_client`;
`thread_local` is unsafe because touch handlers run inside `step`). Measure first with `xtask audit --metrics`.

### 2.7 🟢 Nested `Cx::emit` inside an event handler
Clients would apply the nested event, the server not. Add an `in_event` flag with a `debug_assert`.

### 2.8 🟢 Unbounded grids from bad map data
A non-finite collider extent makes the static grid loop effectively forever; a 1 km floor makes the nav grid allocate
≈650 MB. Add finiteness and size checks to `spec_problems` (and an audit).

### 2.9 🟢 Map-code misuse panics in a running room
`hop_chain` without pads, `ladder` with `y1 < y0`, a vertical `ramp`, `y_on_ramp` / `rolling_balls` with equal z,
`sweep_eta` with ω = 0: add `assert!`s at construction so they fail in tests/audits, not in a room.

### 2.10 🟢 Unused public bot helpers
`path_brain` and `routes_brain` (`fb_sim/src/bots.rs`) have no callers; `routes_brain` panics on an empty list.
Remove, or keep as TS-port helpers with an empty-list guard.

### 2.11 🟢 `reaching` flag lags one tick
`interact` sets it before deciding the hold breaks. Fixing changes bot decisions and the goldens; matches TS → leave
(recorded here so nobody "fixes" it by accident).

---

## 3. Maps and audits

### 3.1 🟠 Lobby and podium get no map audits
CLAUDE.md says every map gets audits; `run_audits` only covers `GAMES` (the game audits give false errors on them:
durations 1e6, empty descriptions, no bot brain on the podium). Add a small audit set for non-game maps: spawn, clip,
spec (without meta / genre / bot rules).

### 3.2 🟢 Map audits reuse one arena across stand tests
In the `respawn` audit time goes 5 → 20 → 0.6·duration → 5 and `world.portal_used` persists between tests. Build a
fresh arena for each `respawn` test.

### 3.3 🟢 Respawn-on-spawn never tested after t = 0
For games that send a fallen bean back to its spawn (tail-tag, star-fall, lobby), spawns are only checked at t = 0;
tail-tag rotors (z = ±11.6) and star-fall sweepers (r 8.2) are never checked against them later. Run `stand_test` on
each spawn at the `respawn` audit's times with ±1 m jitter.

---

## 4. Client

### 4.1 🟠 Event history re-applied on reconnect
> **Applied 2026-10-05 (approved by the author):** a different variant than recommended: the history and `Welcome` travel on different channels, so a rebuild flag set on `Welcome` could race and lose events. Instead the client keeps the (tick, digest) of every event applied to the arena and skips repeats (variant 2), and the server marks re-sent history (`MapEventMsg::history`, protocol 19) so a player joining mid-round applies it silently (no sounds, feed lines or notes). The `Map`-on-Home question is still open.

The `Map` resource is kept while the arena is the same; on reconnect (or leaving and re-entering a room mid-arena) the
server's full event history is applied on top of a world that already has it: past knock-outs re-appear in the feed
with sounds, star-fall "drop" sounds/messages replay, tail-tag grants immunity again (extra rollbacks). (The audit fixed
the bonus case only.)
**Variants.** 1) A stale flag set on `Welcome`; `build_round` then rebuilds even for the same arena (generation++ →
view, camera, specials respawn as on a first join; one map build). 2) Drop duplicate events by (arena, tick, payload).
Also: remove `Map` in `net::close()` / on Home so the old arena is not drawn behind the server list — ask the author
whether that backdrop is wanted.
**Recommendation.** 1.

### 4.2 🟠 Corrupt settings file loses identity, name, outfit, server list
After a parse error bevy_settings starts from defaults and the next save overwrites the file. Rename an unparsable
`settings.toml` to `settings.toml.bad` before loading; replace NaN / infinite values from a hand-edited file with
defaults (clamping does not remove NaN; it reaches the camera and the mixer).

### 4.3 🟢 Face kit built on the main thread
`face.rs::make_kit` draws 10 textures of 512² in software and casts ≈3,200 rays when the first bean appears: a one-off
stall (tens of ms, not measured) on lobby entry. Move to `AsyncComputeTaskPool` or do it at startup.

### 4.4 🟢 Key bindings re-parsed at 120 Hz
`Bindings::keys` parses and allocates every input tick and frame; cache a resource rebuilt when `Bindings` changes.

### 4.5 🟢 Long-session time precision
Animations read `elapsed_secs()` as f32: after ≈1 day of uptime the wobbles step visibly → `elapsed_secs_wrapped()`.
Related: shader time wraps every 3600 s (Bevy) → an hourly jump in clouds, motes, moving patterns.

---

## 5. Rendering and UI (look and feel — need a before/after snapshot)

Sources: `K:\GameDevLibrary\books` (image-pipeline, iquilezles).

| # | Prio | Proposal | Cost / note |
|---|---|---|---|
| 5.1 | 🟠 | **EASU on perceptual colour** (`easu.wgsl`): it filters linear HDR, bright edges dominate. Reversible tonemap around the taps: `tap → c/(1+max3(c))`, output `o/(1−max3(o))`. | 12 divides/pixel. image-pipeline/02, 01 p.2 |
| 5.2 | 🟢 | **`MipBias(log2(scale))` with FSR** (`fsr.rs`, next to `MainPassResolutionOverride`): ≈ −0.58 at 0.67. Order after the audit fix that removes MipBias on preset change. | free; image-pipeline/03 (FSR1 value from memory — verify with AMD's guide) |
| 5.3 | 🟢 | **Mipmaps for portal discs and emoji boards** (single-level 256², sparkle at distance): CPU mip chain averaged in linear. | +33 % of 256 KB; iquilezles/05 |
| 5.4 | 🟢 | **Integer hash in `sky.wgsl`**: `fract(sin(x)*43758)` with arguments up to 1e6 (stars) / 1e5 (clouds) patterns or flickers on iGPU `sin` → PCG `u32` hash (GL 3.3 OK). Changes the stars/clouds look. | iquilezles/02 |
| 5.5 | 🟢 | **Name tags lag one frame** (`tags.rs`): UI layout runs before transform propagation → place tags after `place_camera`, before layout, from `Transform`s (confirm beans are root entities). | — |
| 5.6 | 🟠 | **No keyboard / gamepad navigation of menus**: a gamepad player cannot press «Начать игру», pick a room or change settings → `TabGroup` / `TabIndex` + focus style; directional navigation later. | UX work |
| 5.7 | 🟢 | **AO checkbox** shows "on" for Medium/Low/FSR where AO is never applied → disable it there; on High with AA off SSAO is noisy without TAA → tie to AA or higher SSAO quality. | — |
| 5.8 | 🟢 | **`props::dress` parent walk** (≤12 parents per mesh per frame) → a "not a prop" marker; verify glTF scenes are fully parented when first seen. | needs a run |
| 5.9 | 🟢 | **LOD levels copy the full vertex buffer 6×** (`lod.rs`) → `meshopt::optimize_vertex_fetch` per level. | 19 models |
| 5.10 | 🟢 | **SMAA preset on Medium** (iGPU default) is `High`; SMAA ≈3× FXAA → try `SmaaPreset::Medium` after measuring on HD 520 / Vega 3. | measure |
| 5.11 | 🟢 | **Wording**: «Отдать хоста» → «Сделать хостом»; «Качание камеры…» → «Тряска камеры…»; move hard-coded strings in `hud.rs` (keys line) and the dev tab in `ui/menu.rs` into `text.rs`. | — |

---

## 6. Dependencies and toolchain

| # | Prio | Proposal |
|---|---|---|
| 6.1 | 🟢 | **Bevy 0.20 / Lightyear**: wait. Bevy 0.20 is rc.2 (2026-09-28); Lightyear's port (PR #1760) and replicon's (#768) are open. The move is one step for Bevy, Lightyear, replicon, aeronet 0.22, wgpu 30, glam 0.33. Breaking areas that hit this code: UI Em/Rem units and `TextFont` default size, `BorderRadius`, deprecated `Button`/`Interaction`, flat pointer events, `Font::from_bytes`, `Tonemapping` moved, WESL instead of naga_oil (FSR and surface WGSL), `ViewDepthTexture`, generic `Extract`, exclusive systems, observer generics, query iterators returning `Result`, `NextState::set_if_neq` renamed. glam 0.33 changed scalar paths (`FloatExt::lerp`, recip→division): bump glam first and run `cargo xtask check`. Large effort. |
| 6.3 | 🟢 | `cargo audit` in CI with `--ignore RUSTSEC-2026-0121` (steamworks, lockfile-only via Lightyear's optional Steam) and a reason. Other lockfile-only/unmaintained: paste, rustls-pemfile; ttf-parser (Linux client, via winit Wayland decorations — upstream). |
| 6.4 | 🟢 | One TLS crypto library: aws-lc-rs comes only from the vendored aeronet's rustls defaults, ring from ureq/rcgen. Give rustls only `ring` in the vendor crate and install the ring provider in `session/mod.rs` → aws-lc-sys and `cmake` leave both builds (musl server jobs need cmake only for it). Test `cargo xtask stress --remote --transport ws`. |
| 6.5 | 🟢 | CI job on the MSRV (`rust-version = 1.95`); CI builds only on stable. |
| 6.6 | 🟢 | hmac 0.13 / sha2 0.11 — hold until tungstenite moves to digest 0.11 (else two digest stacks); sysinfo 0.39 with 6.1; `wgpu-types` pin could relax to `"29"`. |

---

## 7. Packaging, CI, deploy

| # | Prio | Proposal |
|---|---|---|
| 7.1 | ✅ | **Applied 2026-10-05:** `runs-on: ubuntu-latest` + `container: ubuntu:22.04` (bare image: the job installs ca-certificates, curl, git, build-essential, pkg-config first). Not run yet — check on the next release run. Was: **AppImage job on `ubuntu-22.04`**: GitHub retires the image (brownouts since 2026-09-17, removed 2027-04-17). Build on `ubuntu-latest` with `container: ubuntu:22.04` (same glibc baseline); alternatives: `debian:bookworm` container, or accept 24.04's glibc. |
| 7.2 | 🟠 | **`appimagetool` unpinned and unverified** (downloaded from `continuous`); it also aborts if an AppStream validator is installed and the metadata fails → pin a release + sha256; `--no-appstream` or 7.3. |
| 7.3 | 🟢 | **Metainfo** lacks `<developer>`, `<releases>`, `<project_license>` (licence: author's choice), first paragraph < 80 chars. |
| 7.4 | 🟢 | **Flatpak runtime 25.08** (supported to ≈2027-09); 26.08 has newer Mesa (new GPUs) — needs a local build and launch check. |
| 7.5 | 🟢 | **systemd hardening** for both units: `SystemCallFilter=@system-service ~@privileged @resources`, `SystemCallErrorNumber=EPERM`, `ProtectProc=invisible`, `UMask=0077`, `PrivateUsers=yes`; check whether `AF_NETLINK` is needed. Verify with `systemd-analyze security` and a stress run on a host. |
| 7.6 | 🟢 | **Pin packaging tools** (`cargo-deb`, `cargo-generate-rpm`, NSIS from choco) or use `taiki-e/install-action`. |
| 7.7 | 🟢 | **Licence files of the vendored crate**: `vendor/aeronet_websocket` (MIT/Apache) is published without its LICENSE files → copy them from upstream. |
| 7.9 | 🟢 | **`remote-install.sh` nginx check** passes on a commented-out include → check the output of `nginx -T`. |
| 7.10 | 🟢 | **Installer**: no version info on `setup.exe` (`VIProductVersion`/`VIAddVersionKey`); uninstall keeps the settings folder (document it); after the new 30 s wait for the game to close: carry on (current) or abort silently. |
| 7.11 | 🟢 | **CI**: `push` + `pull_request` both run → PR branches build twice (filter `push` to the main branches); if the smoke stress run on the dev build flakes, give it `--release` or a looser tick limit. |
