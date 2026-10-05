# Состояние: с чего продолжать

Перезаписывается целиком в конце каждого треда. Только то, чего не видно в коде и git: где остановились, что
начато и не закончено, что сломано, с чего начать.

**5 октября 2026**, ветка `master`, последний коммит `95801a0`; поверх него — незакоммиченная переделка логов (ниже).
Версия `0.1.0-alpha`. Последний TS-коммит помечен тегом `ts-final`.

Постоянного сервера нет: игроки добавляют свой сервер в список на главном экране (`README.md`, «Свой сервер»).
У автора сервер стоит из `.deb` (systemd, логи в journald).

## Незакоммиченные изменения

Логи переделаны под вопрос «что пошло не так» (автор: «постоянно телепортирует назад», «если краш — где логи»).
Предлагаемое сообщение коммита:

```
Logs that explain what went wrong, on disk and in F8 reports

- client: a log file a run in the profile's logs/ (newest 10), crash reports there too (crash-<time>.txt),
  a GPU error (DeviceLost) writes one as well; settings and the crash note open the folder
- client: `stats:` only with --stats-every (headless: every second, stress reads it); a `net:` summary a
  minute, and lines at once for corrections of the own bean (rubber-banding), unpredicted respawns, clock
  jumps, long frames, loss and slow round trips, each with the connection's state
- client: F8 saves report-<time>.txt (connection, corrections, the bean's last 10 s by tick, log tail)
- client: the same connection failure is logged once and then every tenth time; disconnect reasons
- server: `input gap` per player (with how late the newest input was), `still no input`, disconnect reason
  and RTT, panics with a backtrace, room crashes with phase/tick/players; no `metrics:` while empty
- dist: strip only debuginfo, so backtraces name functions
```

Проверено: `cargo xtask check`; сервер и `--headless`-клиент с `--lag/--jitter/--loss` (строки `input gap`, `clock
set`, `packet loss`, `slow round trip`; детектор поправок — с временно нулевыми порогами); файл лога —
`fb_client --offscreen --profile logtest` (профиль удалён). Окно, кнопка «Открыть папку с логами» и F8 вживую не
проверялись (автор смотрит сам).

## Начать с

1. Дальше гонять `cargo xtask fuzz-ui --secs 600 --jobs 4`; упавшее зерно → сценарий в `scenarios.rs` →
   исправление. Геймпада в monkey нет (стенд не шлёт его событий).
2. Автообновление вживую — на втором релизе (`version` → `0.1.0-alpha.2`, `release` по просьбе автора).
3. `docs/audit/` автор пока не трогает (там же нерешённые предложения: 1.2 PIN по комнате, 1.3 спам Start, 5.6
   геймпад в меню).

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
- Отчёт о падении в релизной сборке — с именами функций, но без строк (`strip = "debuginfo"` в `dist`); на Windows
  имена берутся из PDB, которого в установщике нет, — там, вероятно, по-прежнему без имён.
- Пороги в логах подобраны на глаз (`watch.rs`: поправка от 0,5 м; `stats.rs`: кадр от 250 мс, потери от 5%,
  RTT от 250 мс; сервер: разрыв ввода от 0,2 с) — подправить по первым реальным логам.
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
