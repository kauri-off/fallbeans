# Состояние: с чего продолжать

Перезаписывается целиком в конце каждого треда. Только то, чего не видно в коде и git: где остановились, что
начато и не закончено, что сломано, с чего начать.

**5 октября 2026**, ветка `rogue/port-to-rust` (PR #11 в `master`, сливает автор руками), последний коммит
`8e65c0a` (стенд клиента, сценарии, fuzz-ui, отчёты о падениях), поверх него — незакоммиченная подготовка к слиянию
(ниже). Версия `0.1.0-alpha`. Последний TS-коммит `master` помечен тегом `ts-final` (запушен).

Постоянного сервера нет: игроки добавляют свой сервер в список на главном экране (`README.md`, «Свой сервер»).

## Незакоммиченные изменения

Автор просил не коммитить; сообщение готово:

```
Split README, drop the decisions log, license under AGPL-3.0-or-later

- README.md: rewritten for players and server hosts (install, controls, own server, build from
  source, player-visible known issues, license); the developer manual moves to docs/development.md
  (layout, commands, rules, debugging, releases, deploy, internal known issues)
- docs/decisions.md removed (in git history): why something was decided goes into the commit
  message or a one-line comment; CLAUDE.md, development.md and comments no longer point to it;
  the emoji font rebuild command moves into assets/fonts/emoji.txt
- LICENSE: GNU AGPL v3 text from gnu.org; license = "AGPL-3.0-or-later" in [workspace.package],
  inherited by every crate; fb_server rpm license; metainfo project_license
- Packages carry LICENSE: client staging (NSIS installs and uninstalls it, AppImage, Flatpak in
  /app/share/licenses), deb in /usr/share/doc/fallbeans-server, rpm as a doc file;
  release-notes.md names the license and the source archives
- The TS version is referred to by tag ts-final instead of the master branch (CLAUDE.md,
  development.md, golden trace loader comment, deploy/README.md)
```

`cargo xtask check` после правок проходит; `dist deb|rpm|nsis|flatpak` с LICENSE локально не собирались
(нет `cargo-deb`, `cargo-generate-rpm`, `makensis`, `flatpak-builder`, musl-цели) — проверятся первым релизом.

## Начать с

1. Автор сливает PR #11 руками. После слияния: в `state.md` убрать ветку, CLAUDE.md уже не называет ветку.
2. Первый релиз `0.1.0-alpha` с `master` — только по просьбе автора (`release.yml` целиком ещё не запускался; не
   проверены на раннерах NSIS, `choco install nsis`, `sysctl` для bubblewrap, AppImage в `ubuntu:22.04`, LICENSE в
   пакетах). Автообновление вживую — только на втором релизе (`0.1.0-alpha.2` против `0.1.0-alpha`).
3. `docs/audit/` автор пока не трогает (там же нерешённые предложения: 1.2 PIN по комнате, 1.3 спам Start, 5.6
   геймпад в меню).
4. Погонять `cargo xtask fuzz-ui --secs 600 --jobs 4` подольше; упавшее зерно → сценарий в `scenarios.rs` →
   исправление. Геймпада в monkey нет (стенд не шлёт его событий).

## План: падения ловятся до запуска

Этапы 1–3 (стенд, сценарии, monkey) — в `8e65c0a`. Дальше, по выбору автора:

4. `stress` с полной логикой клиента (+ «Прервать игру», выход/вход, уход хоста). Сценарии и monkey уже закрывают
   эти действия на стенде (без задержек и потерь сети); вероятно, хватит флага задержки/потерь у сервера стенда.
5. Линты `indexing_slicing`, `unwrap_used`, `expect_used`, `panic` в `crates/` по модулям.
6. Property-тесты ядра (proptest только в dev-зависимостях).
7. `cargo xtask smoke` на программном GPU (lavapipe, llvmpipe `--backend gl`) — локально: noop не проверяет
   драйверы и GLSL для GL.

## Не закончено и известные странности

Проблемы, описанные в `README.md` и `docs/development.md` («Известные проблемы»), здесь не повторяются.

- Иконка окна не задана (Bevy рисует свою): в Windows на панели задач и у exe нет иконки игры, ярлыки NSIS — с
  иконкой. На Linux иконка берётся из desktop-файла по app id `io.github.kauri_off.fallbeans`.
- Обход зависания Intel Gen9 (`INTEL_DEBUG=reemit`) в игру не встроен; `DeviceLost` клиент игроку не объясняет —
  выходит молча.
- `cargo xtask assets --export` пересобирает модели из Blender (AO вшивается серым PNG, у TS был JPEG — файлы
  чуть больше); закоммиченные модели — ещё из TS-конвейера, заменять не обязательно.
- `cargo xtask dist flatpak` добавляет пользовательский remote flathub (`--if-not-exists`).
- В соло-игре раунд кончается, как только единственный человек финишировал или выбыл: зрителя проверять двумя
  клиентами.
- Стресс с ботами в dev-сборке не проходит порог тика: гонять с `--release`. Первый прогон стресса сразу после
  сборки иногда ловит общее замирание на 0,1–0,6 с — повторить. У каждого клиента одна «необъяснённая»
  расходимость через ≈200–245 тиков после старта (в пределах порога); с прежней сборкой не сравнивалось.
- Replay теряет операцию над пешкой, вошедшей между теми же тиками.
- CI на GitHub нет: `check` на Windows (детерминизм между ОС) — только локально у автора; стенд клиента на Windows
  не запускался.
- `DevCmd::Start` со списком игр не смотрит на `rounds`.
- Не измерен самый долгий кадр входа в раунд на слабом клиенте: дольше 3 с — сервер отключит игрока (у стенда
  таймаут связи 8 с, `--link-timeout`).
- Стенд: в логе тестов клиента `Could not set global logger` (ERROR) у всех клиентов процесса, кроме первого,
  `server_late_input_mismatch` от lightyear_debug и `Settings registry not found` — безвредно. Сервер стенда нельзя
  перезапустить в том же процессе (HTTP-поток держит порт): только `server_away`.
- Отчёт о падении в релизной сборке — без имён функций в backtrace (`strip = true` в `dist`).
- Логи и трассы пишутся синхронно из главного цикла.
- `pkill -f fb_client` убивает и вызывающую оболочку: гасить `pkill -x fb_client` / `pkill -x fb_server`.
- Лицензия AGPL: сервер не показывает игрокам ссылку на свои исходники (§13 обязывает того, кто изменил сервер);
  SPDX-заголовков в файлах нет — лицензия задана `LICENSE` и `Cargo.toml`.
- Не сделано из прежних планов: `xtask bench` (секции, p99, память), `xtask trace / replay / new-game`, проверка
  графики на железе автора (60 FPS на «Низком» на HD 520/Vega 3, 144 на «Высоком» на GTX 1660), SSAO/TAA на T2,
  проход через портал, LOD на глаз.

## Проверка без окон

Автор не хочет окон и скриншотов на экране и смотрит интерфейс сам в своей игре: правки — сборкой и `check`, без
запусков. Если графику всё же надо проверить снимком — `fb_client --offscreen` (`docs/development.md`, «Отладка»).
