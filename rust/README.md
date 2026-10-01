# Fall Beans на Rust

Перенос игры на Rust + Bevy + Lightyear. ТЗ и план по фазам — [`../plan.md`](../plan.md); состояние, замеры и
разбор решений Фазы 0 — [`PHASE0.md`](PHASE0.md). Этот файл — как работать с кодом.

## Что нужно

- Rust stable ≥ 1.95 (`rustup`), компоненты `rustfmt` и `clippy`.
- Linux: `libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev` (как в CI).
- Bun — только для `cargo xtask golden` и `cargo xtask assets`: они берут данные из TS-версии.

Первая сборка долгая (Bevy). В `dev` профиле зависимости собираются с `opt-level = 3`, свой код — с 1.

## Устройство

```
rust/
  core/            детерминированное ядро: без Bevy, f64, математика только через fb_shared::m
    clippy.toml    правила детерминизма (действуют только на core/*)
    fb_shared      константы, ввод (InputFrame), mulberry32, m (libm, hypot как в V8, round_js)
    fb_sim         физика боба (physics.rs), коллайдеры, World (узлы + movers), Builder, бонусы, SceneDesc
    fb_maps        карты (сейчас jump-club) и реестр MAPS
    fb_arena       арена: пешки, бонусы, падения; tick_bodies — общий шаг сервера и предсказания
      tests/       golden.rs (против TS), determinism.rs (записанные хэши), replay.rs (откат = прямой прогон)
  crates/          всё на Bevy и Lightyear
    fb_net         протокол Lightyear: компоненты, ввод, события, форматы передачи (wire.rs),
                   фильтры видимости, NetSim (флаги имитации сети), NetStats (трафик)
    fb_server      net (транспорты, вход), room (раунды, тик), metrics, opts
    fb_client      net, game (карта, предсказание, ввод, автопилот), view, hud, stats, assets, opts
  xtask            cargo xtask <check|golden|assets|dev|stress>
  assets/models    glb для Bevy (делаются из ../public/models: cargo xtask assets)
```

Сервер строит карту от сида без сцены; клиент строит ту же карту со сценой (`SceneDesc`) и сверяет хэш коллайдеров
(`Round.static_hash`). По сети идут только бобы (`BodyFull` владельцу, `RemotePose` остальным) и события карты с тиком.

## Команды

```sh
cargo xtask check                 # fmt --check, clippy -D warnings, тесты — перед каждой сдачей работы
cargo xtask dev --clients 2       # сервер и два окна; --autopilot, --lag/--jitter/--loss, --map, --seed, --release
cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05
cargo xtask stress --transport ws --lag 75 --jitter 15   # по WebSocket (потери не задавать: TCP не теряет)
cargo xtask golden                # переснять следы из TS и сверить (после изменений в TS-физике или карте)
cargo xtask assets                # переэкспортировать модели и проверить загрузку в Bevy
```

Бинарники напрямую (`cargo run -p fb_server -- --help`, `cargo run -p fb_client -- --help`):

- сервер: `--map`, `--seed`, `--intro`, `--udp-port`, `--ws-port`, `--lag/--jitter/--loss`, `--trace файл`,
  `--metrics-every с`, `--exit-after с`;
- клиент: `--server`, `--transport auto|udp|ws`, `--ws-url`, `--id`, `--lag/--jitter/--loss`, `--input-margin`,
  `--backend vulkan|dx12|gl`, `--headless` (без окна и GPU, автопилот), `--fps`, `--autopilot`, `--trace файл`,
  `--screenshot файл --exit-after с`, `--check-assets`.

## Правила

**Ядро (`core/`) детерминировано.** Одинаковые входы дают одинаковые биты на Windows и Linux, клиент и сервер
считают одно и то же.

- Тригонометрия, `exp`, `pow`, `hypot` — только из `fb_shared::m`. Clippy запрещает `f64::sin` и т. п.,
  `mul_add`, `HashMap`/`HashSet`, `Instant`/`SystemTime`. Исключение ставится только в `m.rs`.
- Порядок операций — как в three.js и TS (`fb_sim::math`: `M4`, `add_scaled`, `div_s`…), иначе золотой след
  разойдётся в последних битах. Методы glam (`length`, `normalize`, `lerp`) в симуляции не использовать.
- Время сравнивать с допуском (`k·DT − DT` ≠ `(k − 1)·DT` в последнем бите).
- Никаких случайностей и часов: только `Builder::rng` (сид), время симуляции и события.
- Изменение, которое намеренно меняет симуляцию: перезаписать `determinism.txt`
  (`FB_BLESS=1 cargo test -p fb_arena --test determinism`) и объяснить почему. Если меняется и TS — `cargo xtask golden`.

**Перенос из TS.** Исходник — TS в `master` (`src/sim`, `src/games`, `src/server/rooms/arena.ts`). Переносить
построчно, имена в snake_case, индексы коллайдеров совпадают с TS. Каждый перенесённый кусок закрыт следом
из `scripts/golden.ts` или тестом.

**Сеть (`fb_net`).**

- Поменял состав или формат реплицируемого компонента или сообщения — подними `PROTOCOL_VERSION`
  (`core/fb_shared/src/consts.rs`) и обнови тесты в `wire.rs`.
- Форматы передачи (`wire.rs`) в памяти не используются: типы полной точности, сжатие только при отправке.
  `BodyFull` после округления не должен отличаться от исходного больше порога `body_differs`.
- Настройки ввода (`INPUT_SEND_INTERVAL`, `INPUT_REDUNDANCY`) и `--input-margin` подобраны замерами
  (`PHASE0.md`); менять — с прогоном `xtask stress` до и после.
- Сервер запускает два `Server` Lightyear (UDP и WS), поэтому топология Lightyear `Invalid`
  ([lightyear#1693](https://github.com/cBournhonesque/lightyear/issues/1693)): системы Lightyear, которые от неё
  зависят, не работают. Ввод сервер читает из `InputBuffer` сам (`room::frame_for`). Новые возможности Lightyear
  на сервере проверять на этом.
- Видимость компонентов — фильтрами Replicon (`visibility.rs`); новые фильтры регистрирует `add_server_filters`.

**Остальное.** Зависимости — в `[workspace.dependencies]`; Bevy, Lightyear и всё, что с ними связано, пинится
точно. Комментарии и вывод инструментов — по-английски, текст для игрока — по-русски. Переводы строк — LF.

## Отладка

- **Предсказание.** `cargo xtask stress` пишет трассы (`target/stress/server.trace`, `client-N.trace`: тик, id,
  ввод, позиция) и логи. Таблица показывает по каждому клиенту опоздавший ввод и расхождения по причинам;
  «прочие» печатаются тиками — смотреть строки этих тиков в трассах сервера и клиента.
- **Без стресса:** сервер и клиент с `--trace` и теми же флагами сети, сравнение тех же строк.
- **Логи.** Клиент раз в секунду пишет `stats:` (транспорт, RTT, джиттер, откаты, предсказанные тики, трафик,
  раунд, `MAP HASH MISMATCH` при расхождении карты), сервер раз в 5 с — `metrics:` (тик p50/p99/max, трафик,
  пакеты, CPU, память). HUD в окне показывает то же.
- **Графика.** `fb_client --backend gl --screenshot shot.png --exit-after 15` против запущенного сервера.

## Известные проблемы

- Две `Server` в одном процессе (см. выше); решение — до Фазы 3.
- На GL wgpu пишет ~70 ошибок `CubeArray` в секунду; безвредно. SSAO на GL отключён через лимит storage-текстур
  (`fb_client/src/main.rs`).
- Респаун не предсказывается: каждое падение даёт 2–3 отката (как в TS).
- Переподключение даёт нового игрока (удержание места — Фаза 3); `MAX_PLAYERS` не проверяется.
- Аутентификация netcode — нулевой ключ и id от клиента (только Фаза 0).
- Золотые следы есть только для `jump-club`: лестницы, уступы, порталы, конвейеры, батуты, лёд с TS не сверены.
