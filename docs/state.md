# Состояние: с чего продолжать

Перезаписывается целиком в конце каждого треда. Только то, чего не видно в коде и git: где остановились, что
начато и не закончено, что сломано, с чего начать.

**5 октября 2026**, ветка `rogue/port-to-rust` (PR #11), последний коммит `fb3af03` (паника поля ввода), поверх
него — незакоммиченные этапы 1–3 плана падений (ниже). Версия `0.1.0-alpha`.

TS-версия убрана из ветки (осталась в `master`), фаз больше нет. Раскладка — Cargo workspace в корне репозитория.
Постоянного сервера нет: игроки добавляют свой сервер в список на главном экране (`README.md`, «Игра по локальной
сети»).

## Незакоммиченные изменения

Готовое сообщение коммита (один коммит: стенд, сценарии и monkey — новые файлы целиком). `cargo xtask check`
проходит (171 тест, клиентские — около минуты). `xtask stress --release` (8 клиентов, 100 с, lag 75, jitter 15,
loss 0.05) после правки связи — ok, 0 опозданий; у 6 из 8 клиентов по одной прежней «необъяснённой» расходимости
около 230–245 тика (известная странность ниже). `--link-timeout` касается только стенда — `stress` после него не
гонялся.

```
Catch crashes before runtime: client test harness, scenarios, monkey, crash reports

- Remove .github/workflows/ci.yml: GitHub only builds releases (release.yml keeps its check job)
- crates/clippy.toml bans EditableText::clear and editor_mut (allow-invalid for crates without
  bevy text); ui::set_field_text is the one #[expect] exception, so a dead path fails check
- clippy::string_slice workspace-wide; rewrite the 4 byte-slicing sites (fb_client net.rs
  default_ws, fb_server http.rs host_name, xtask stress.rs num_after/num_before/shifts)
- fb_net: logbook moved here from fb_server (+ non-blocking tail()); errors::ErrorPolicyPlugin:
  Bevy system/command errors panic in debug and tests, log "ECS ERROR" in release (every sub-app);
  xtask stress fails on "ECS ERROR" in a client or server log
- fb_client crash.rs: a panic hook writes crashes/crash-<secs>.txt in the profile dir (build, OS,
  thread, message, location, backtrace, last 200 log lines) and a `last` marker, keeps 10;
  the home screen shows the report's path on the next start (LastCrash, settings::dir)
- fb_server is a library (lib.rs: fb_server::app(opts, log)) with a thin main.rs
- fb_client main.rs: build(app, opts, noop) builds the whole client; with a noop RenderCreation
  (tests) no winit, no audio, no updater, no pipelined rendering, no stored settings, no crash hook
- fb_client harness.rs (tests only): wgpu noop device named as a GPU type (tier T0-T2, fixed
  preset), a real --dev --solo server in the same process on per-process ports, any number of
  clients stepped with it (add_peer, as_peer); press enabled buttons (Activate on Act), type into
  and erase fields (KeyboardInput with focus), move sliders (ValueChange), send dev commands, wait
  for arenas, stop stepping the server for a while (server_away); Drop fails the test on pipelines
  that failed to build
- fb_client scenarios.rs: chat lines in a row, rename in the lobby menu (reproduces fb3af03 with
  the fix reverted), the server list, a private room with a wrong and a right PIN and a second
  player taking over as host, the owner leaving the lobby, a newcomer watching a round in play,
  abort mid-round, leaving a room and coming back (mid-round, from the podium), practice from the
  room list and from a room, every setting and a key rebound mid-round with the graphics presets
  switched on the fly, reconnecting after the server was away, a round of every map to the
  podium on T2, one on T0
- fb_client monkey.rs: random play by seed on the harness, two clients on one server: buttons on
  screen (not Quit or the updater's), keys tapped and held, text (Cyrillic, ZWJ emoji, flags,
  combining marks, RTL, invisible, too long; per character and as one IME commit), sliders, the
  mouse, the host's dev commands, the server away; a failure prints the seed and the last actions.
  Two 25 s seeds in check; finds fb3af03 with its fix reverted (4 seeds of 6 within 2 minutes)
- harness: hold, type_at_once, click, look, fields, knobs, typing; the server gets
  --link-timeout 8: one thread steps server and clients, a busy machine stalled both past
  netcode's 3 s and the owner took the host role back on reconnect (a flaky scenario)
- fb_server: hidden --link-timeout (seconds of silence in the connect token, both transports)
- xtask fuzz-ui --secs N [--seed S] [--jobs J]: J seeds in parallel, logs in target/fuzz-ui, the
  panic and the path of each failed seed
- Fix crash: net::hang_up replaces the 5 Disconnect triggers sent from Update (practice in/out,
  leaving the server, back to UDP, reject, updating); the link is disconnected in First, when no
  system has commands queued on the beans and map that go with it (an insert on a despawned bean
  panicked: face::attach in practice_and_back); a link that went down is despawned in Last, after
  its disconnect packets went out, so the server lets the player go at once
- face::attach spawns the face parts in one command on the bean (queue_silenced): nothing is
  attached to a bean despawned before the frame's commands are applied (unit test)
- Fix: a field rebuilt while focused keeps the keyboard (ui::keep_focus): after Enter with a wrong
  server address or PIN the player had to click into the field again
- Fix chat: Enter opens it while the window holds the input focus (Bevy 0.19's default)
- Fix: a hidden text field gives the keyboard back to the game (ui::keep_focus): the menu's name
  field focused as a round started, or the room title after Enter created the room, kept the focus
  (keys typed into the hidden field, Gate off, Esc ignored, the mouse still turned the camera);
  scenarios a_round_starts_while_typing_a_name, a_room_created_with_enter
- Workspace dependency wgpu =29.0.4; fb_client dev-dependencies fb_server, wgpu (noop)
- Docs: CLAUDE.md (state.md format, GitHub, crash rule, fuzz-ui), README.md, decisions.md
- Docs: Intel Gen9 GPU hang on Vulkan (workaround INTEL_DEBUG=reemit, not built in) and
  --backend gl on Wayland in README known issues; the hang hunt in decisions.md
```

## План: падения ловятся до запуска

Почему и все этапы — `decisions.md`, «Падения ловятся до запуска: стратегия и этапы»; стенд — «Клиент целиком в
`cargo test`», «Стенд: несколько клиентов…», monkey — «Monkey на стенде клиента». Этапы 1–3 — в незакоммиченных
изменениях. Дальше:

4. `stress` с полной логикой клиента (+ «Прервать игру», выход/вход, уход хоста). Сценарии и monkey уже
   закрывают эти действия на стенде (без задержек и потерь сети); решить, нужен ли этап целиком — вероятно,
   хватит флага задержки/потерь у сервера стенда.
5. Линты `indexing_slicing`, `unwrap_used`, `expect_used`, `panic` в `crates/` по модулям.
6. Property-тесты ядра (proptest только в dev-зависимостях).
7. `cargo xtask smoke` на программном GPU (lavapipe, llvmpipe `--backend gl`) — локально: noop не проверяет
   драйверы и GLSL для GL.

## Начать с

1. **Закоммитить** (сообщение выше), когда автор попросит.
2. Погонять `cargo xtask fuzz-ui --secs 600 --jobs 4` подольше (на текущем коде 6 зёрен × 3 мин и 2 × 20 с — без
   падений); упавшее зерно → сценарий в `scenarios.rs` → исправление. Геймпада в monkey нет (стенд не шлёт его
   событий).
3. Этап 5 (линты по модулям) или 7 (smoke на программном GPU) — по выбору автора.
4. Прежнее, после падений: аудит (`docs/audit/proposals.md`: 1.2 PIN по комнате, 1.3 спам Start, 5.6 геймпад в
   меню); первый релиз (`release.yml` целиком ещё не запускался; не проверены на раннерах NSIS, `choco install
   nsis`, `sysctl` для bubblewrap, AppImage в `ubuntu:22.04`; запуск только по просьбе автора); автообновление
   вживую — только на втором релизе (`0.1.0-alpha.2` против `0.1.0-alpha`); экран серверов автор смотрит сам.

## Не закончено и известные странности

- Иконка окна не задана (Bevy рисует свою): в Windows на панели задач и у exe нет иконки игры, ярлыки NSIS — с
  иконкой. На Linux иконка берётся из desktop-файла по app id `io.github.kauri_off.fallbeans`.
- `cargo xtask assets --export` пересобирает модели из Blender (AO вшивается серым PNG, у TS был JPEG — файлы
  чуть больше); закоммиченные модели — ещё из TS-конвейера, заменять не обязательно.
- `cargo xtask dist flatpak` добавляет пользовательский remote flathub (`--if-not-exists`).
- Перезапуск на другом графическом API при падении инициализации не сделан: API выбирается флагом или в настройках.
- Intel Gen9 + Mesa 26.2 (anv), Vulkan: зависание GPU через ~5 с раунда; обход `INTEL_DEBUG=reemit`, в игру не
  встроен (`decisions.md`, «Зависание GPU на Intel Gen9»). `--backend gl` на Wayland падает (Bevy #22220).
  `DeviceLost` клиент не объясняет игроку — выходит молча.
- Окно не в фокусе на машине автора рисуется на 20 FPS (композитор): кадр мерить с окном на переднем плане.
- В соло-игре раунд кончается, как только единственный человек финишировал или выбыл: зрителя проверять двумя
  клиентами.
- Стресс с ботами в dev-сборке не проходит порог тика: гонять с `--release`. Первый прогон стресса сразу после
  сборки иногда ловит общее замирание на 0,1–0,6 с — повторить.
- Replay теряет операцию над пешкой, вошедшей между теми же тиками (`decisions.md`).
- Не разобрано: `jump-club` сид 1 — разница 5,6e-17 в золотом следе. JS-точные `max`/`min` (аудит) её не задели: в
  закоммиченных строках следа (каждые 120 тиков) её не видно, а потиковый след уже не записать.
- CI на GitHub нет (`decisions.md`): `check` на Windows (детерминизм между ОС) — только локально у автора;
  стенд клиента на Windows не запускался.
- `DevCmd::Start` со списком игр не смотрит на `rounds` (`decisions.md`, «Сценарии этапа 2…»).
- Не измерен самый долгий кадр входа в раунд на слабом клиенте: дольше 3 с — сервер отключит игрока
  (`decisions.md`, «Стенд: таймаут связи 8 с»).
- Стенд: в логе тестов клиента `Could not set global logger` (ERROR) у всех клиентов процесса, кроме первого,
  `server_late_input_mismatch` от lightyear_debug и `Settings registry not found` (настройки в тестах не хранятся)
  — безвредно. Сервер стенда нельзя перезапустить в том же процессе (HTTP-поток держит порт): только
  `server_away`.
- Отчёт о падении в релизной сборке — без имён функций в backtrace (`strip = true` в `dist`).
- Стресс после аудита: у каждого клиента одна «необъяснённая» расходимость через ≈200 тиков после старта (в пределах
  порога); с прежней сборкой не сравнивалось.
- Логи и трассы пишутся синхронно из главного цикла.
- Сессия по https через свой прокси со своим CA не проходит: `ureq` проверяет по встроенным корням.
- `pkill -f fb_client` убивает и вызывающую оболочку: гасить `pkill -x fb_client` / `pkill -x fb_server`.
- Не сделано из прежних планов: `xtask bench` (секции, p99, память), `xtask trace / replay / new-game`, проверка
  графики на железе автора (60 FPS на «Низком» на HD 520/Vega 3, 144 на «Высоком» на GTX 1660), SSAO/TAA на T2,
  смена пресета на лету, проход через портал, LOD на глаз.

## Проверка без окон

Автор не хочет окон и скриншотов на экране и смотрит интерфейс сам в своей игре: правки — сборкой и `check`, без
запусков. Если графику всё же надо проверить снимком — `fb_client --offscreen` (`README.md`, «Отладка»).
