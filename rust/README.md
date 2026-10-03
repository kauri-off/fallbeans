# Fall Beans на Rust

Перенос игры на Rust + Bevy + Lightyear. ТЗ и план по фазам — [`../plan.md`](../plan.md); итог, замеры и
разбор решений Фазы 0 — [`PHASE0.md`](PHASE0.md); прод-сервер — [`deploy/README.md`](deploy/README.md). Этот файл —
как работать с кодом.

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
    fb_sim         физика боба (physics.rs), коллайдеры, World (узлы + movers), Builder, бонусы, SceneDesc,
                   навигация ботов (nav.rs), поведение ботов (bots.rs)
    fb_maps        карты (сейчас jump-club) и реестр MAPS
    fb_arena       арена: пешки и боты, захваты, таклы, финиш, чекпоинты, падения, выбывание, бонусы;
                   tick_bodies — общий шаг сервера и предсказания
      tests/       golden.rs (полные раунды карт с ботами против TS-арены), scenarios.rs (ветки физики боба в малых мирах против TS),
                   determinism.rs (записанные хэши), replay.rs (откат = прямой прогон)
  crates/          всё на Bevy и Lightyear
    fb_net         протокол Lightyear: компоненты, ввод, события, форматы передачи (wire.rs),
                   фильтры видимости, NetSim (флаги имитации сети), NetStats (трафик)
    fb_server      net (транспорты, вход), room (раунды, тик), metrics, opts
    fb_client      net, game (карта, предсказание, ввод, автопилот), view, hud, stats, assets, opts
  xtask            cargo xtask <check|golden|assets|dev|stress|deploy>
  deploy/          что ставится на хост: systemd-юнит, nginx (wss), установщик с откатом; README — сам хост
  vendor/          зависимости с нашими правками ([patch.crates-io] в Cargo.toml; см. «Известные проблемы»)
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
cargo xtask stress --remote --clients 8 --secs 100 --transport udp|ws|auto   # через реальную сеть до хоста
cargo xtask golden                # переснять следы из TS и сверить (после изменений в TS-физике или карте)
cargo xtask assets                # переэкспортировать модели и проверить загрузку в Bevy
cargo xtask deploy                # выложить сервер на прод (только по просьбе автора, см. «Деплой»)
```

Бинарники напрямую (`cargo run -p fb_server -- --help`, `cargo run -p fb_client -- --help`):

- сервер: `--map`, `--seed`, `--intro`, `--udp-port`, `--ws-addr`, `--ws-port`, `--lag/--jitter/--loss`,
  `--trace файл`, `--metrics-every с`, `--exit-after с`;
- клиент: `--server`, `--transport auto|udp|ws`, `--ws-url`, `--id`, `--lag/--jitter/--loss`, `--input-margin`,
  `--backend vulkan|dx12|gl`, `--headless` (без окна и GPU, автопилот), `--fps`, `--autopilot`, `--trace файл`,
  `--screenshot файл --exit-after с`, `--check-assets`.

## Правила

**Ядро (`core/`) детерминировано.** Одинаковые входы дают одинаковые биты на Windows и Linux, клиент и сервер
считают одно и то же.

- Тригонометрия, `exp`, `pow`, `hypot` — только из `fb_shared::m` (нужна другая функция — обёртка над `libm`
  туда же). Clippy (`core/clippy.toml`)
  запрещает `f64::sin` и прочую математику платформенной libm, `mul_add`, `f64::round` и `signum` (ведут себя
  не как `Math.round`/`Math.sign`: `m::round_js`, `m::sign`), `f32`, `HashMap`/`HashSet`, `Instant`/`SystemTime`.
  Исключение ставится только в `m.rs`.
- Порядок операций — как в three.js и TS (`fb_sim::math`: `M4`, `add_scaled`, `div_s`…), иначе золотой след
  разойдётся в последних битах. Методы glam с другим порядком (`length`, `normalize`, `lerp`, `distance`,
  `DQuat::from_euler`) тоже запрещены.
- Время сравнивать с допуском (`k·DT − DT` ≠ `(k − 1)·DT` в последнем бите).
- Никаких случайностей и часов: только `Builder::rng` (сид), время симуляции и события.
- Изменение, которое намеренно меняет симуляцию: перезаписать `determinism.txt`
  (`FB_BLESS=1 cargo test -p fb_arena --test determinism`) и объяснить почему. Если меняется и TS — `cargo xtask golden`.

**Перенос из TS.** Исходник — TS в `master` (`src/sim`, `src/games`, `src/server/rooms/arena.ts`). Переносить
построчно, имена в snake_case, индексы коллайдеров совпадают с TS. Каждый перенесённый кусок закрыт следом
из `scripts/golden.ts` или тестом. TS — прототип, а не эталон: где он ведёт себя плохо, Rust-версия делает
лучше (так сделаны ввод и `BodyFull`); такое отступление записывается, а след для этой ветки заменяется
тестом на стороне Rust.

**Сеть (`fb_net`).**

- Поменял состав или формат реплицируемого компонента или сообщения — подними `PROTOCOL_VERSION`
  (`core/fb_shared/src/consts.rs`) и обнови тесты в `wire.rs`.
- Форматы передачи (`wire.rs`) в памяти не используются: типы полной точности, сжатие только при отправке.
  `BodyFull` идёт точно (всё, что читает шаг физики; в f32 только `land_impact`, размер выводится из бонуса):
  `body_differs` откатывает на любое отличие, и округление при передаче давало бы лишние откаты.
- Ввод: стик и захват — удерживаемые, прыжок и нырок — нажатия (один тик на нажатие; клиент защёлкивает их
  между тиками, `game::latch_presses`). Стик ужимается до длины 127 одинаково на клиенте и сервере
  (`InputFrame::clamped`). Сервер читает `InputBuffer` сам (`room::frame_for`; перенос Lightyear в
  `ActionState` выключен, он выбрасывает поздний ввод): без ввода держит стик и захват `INPUT_HOLD` тиков,
  нажатие, пришедшее до `LATE_TICKS` (250 мс) позже своего тика, выполняет на следующем.
- Настройки ввода (`INPUT_SEND_INTERVAL`, `INPUT_REDUNDANCY` = `LATE_TICKS`) и `--input-margin` подобраны
  замерами (`PHASE0.md`); менять — с прогоном `xtask stress` (и `--remote`) до и после.
- Один `Server` Lightyear слушает оба транспорта (`net::start_server`: `ServerUdpIo` и `WebSocketServerIo` на одной
  сущности), топология обычная `Server`. Проверка адреса в connect token выключена (`server_addr_check`): с
  нулевым ключом она пускала только localhost; вернуть с токенами Фазы 3.
- Сервер однопоточный: все расписания на `SingleThreadedExecutor` (`fb_server/src/main.rs`), фича Bevy
  `multi_threaded` у `fb_server` не включается. На 1 vCPU многопоточный исполнитель тратил 15% ядра в простое на
  передачу систем пулу. Параллелить (комнаты по потокам) — только по замерам Фазы 3.
- Видимость компонентов — фильтрами Replicon (`visibility.rs`); новые фильтры регистрирует `add_server_filters`.

**Остальное.** Зависимости — в `[workspace.dependencies]`; Bevy, Lightyear и всё, что с ними связано, пинится
точно. Комментарии и вывод инструментов — по-английски, текст для игрока — по-русски. Переводы строк — LF.

## Отладка

- **Предсказание.** `cargo xtask stress` пишет трассы (`target/stress/server.trace`, `client-N.trace`: тик, id,
  ввод, позиция) и логи. Таблица показывает по каждому клиенту опоздавший ввод и расхождения по причинам;
  «прочие» печатаются тиками — смотреть строки этих тиков в трассах сервера и клиента.
- **Без стресса:** сервер и клиент с `--trace` и теми же флагами сети, сравнение тех же строк.
- **Логи.** Клиент раз в секунду пишет `stats:` (транспорт, RTT, джиттер, самый длинный кадр за секунду,
  сдвиги часов Lightyear, откаты, предсказанные тики, трафик, раунд, `MAP HASH MISMATCH` при расхождении
  карты), сервер раз в `--metrics-every` с — `metrics:` (тик p50/p99/max, самый длинный кадр, тики без ввода
  игрока вовремя, трафик, пакеты, CPU, память). HUD в окне показывает то же.
- **Опоздавший ввод.** Длинный кадр клиента (`frame max`) — затык этой машины; сдвиг часов (`shifts`) — Lightyear
  пересинхронизировал часы и переразметил уже отправленный ввод (см. «Известные проблемы»); ни того ни другого
  — сеть. `stress` выводит оба столбца.
- **Графика.** `fb_client --backend gl --screenshot shot.png --exit-after 15` против запущенного сервера.

## Деплой

Только по просьбе автора. `cargo xtask deploy` прогоняет `check`, собирает статический Linux-бинарник сервера
(musl; на Windows — в WSL, `--wsl-distro`, нужны rustup с целью `x86_64-unknown-linux-musl` и
`build-essential cmake musl-tools`), пакует его с `deploy/` и по SSH (`--host`, `--key`, по умолчанию
`deploy@168.113.157.12` и `~/.ssh/cybershield_deploy`) запускает `deploy/remote-install.sh`:

- релиз в `/opt/fallbeans/releases/<время>`, ссылка `current`, три последних релиза хранятся;
- systemd-юнит `fallbeans.service` (`DynamicUser`, `MemoryMax=300M`): UDP 5888 наружу, WebSocket на
  127.0.0.1:5889;
- nginx: `/etc/nginx/apps.d/fallbeans.conf` (сайт из репозитория SharedServer включает `apps.d/*.conf`):
  `wss://…/fallbeans/ws` → 5889, `/fallbeans/ws-probe` → 5891 (сервер пробы);
- ufw: 5888/udp (игра), 5890/udp (проба);
- проверка: служба жива и слушает оба порта; иначе откат на прежний релиз.

`--pack-only` только собирает архив (`target/deploy/`), `--yes` не спрашивает, `--skip-checks` пропускает `check`.

**Проба** (`stress --remote`): та же сборка этого дерева запускается в `~/fb-probe` пользователя deploy (UDP 5890,
WS 5891 за nginx) на время прогона; клиенты идут с этой машины по настоящей сети, трасса сервера скачивается и
сверяется как в локальном `stress`. Прод-службу проба не трогает.

## Известные проблемы

- Прод-хост маленький: 1 vCPU и 0,9 ГБ RAM, делится с другими службами (`deploy/README.md`); сервер рассчитан
  на 4 комнаты по 8 игроков на нём, это ещё не проверено стрессом (Фаза 3).
- `vendor/aeronet_websocket`: сервер WebSocket не включал `TCP_NODELAY` (клиент умеет, сервер нет), это давало
  +15 мс RTT на wss. Правка в одну строку (`server/backend.rs`); убрать, когда появится в aeronet, а при обновлении aeronet перенести.
- Пересинхронизация часов Lightyear: если опережение клиента ушло от цели больше чем на
  `SyncConfig::max_error_margin` (10 тиков, 83 мс; скачок RTT, затык клиента), Lightyear сдвигает часы и
  переразмечает буфер ввода. Уже отправленный ввод уходит на другие тики: сервер видит опоздание или
  «переписанную историю» (ошибки `lightyear_debug::input` в логе), клиент откатывается.
- Таймаут netcode — 3 с: остановка TCP-потока внутри VPN (VLESS xhttp) рвёт WS-соединение. На таких путях
  рывки джиттера вызывают серии сдвигов часов (`PHASE0.md`, «Матрица VPN»); `fb_client --sync-max-error` —
  флаг для опытов с порогом.
- Клиент не переподключается после обрыва на WS и с `--transport udp`; с `auto` обрыв UDP переводит на WS.
  Переподключение даёт нового игрока (удержание места — Фаза 3).
- Нет механизма «игра обновляется»: при деплое клиенты просто теряют соединение (Фаза 3 / 7).
- На GL wgpu пишет ~70 ошибок `CubeArray` в секунду; безвредно. SSAO на GL отключён через лимит storage-текстур
  (`fb_client/src/main.rs`).
- Респаун не предсказывается: каждое падение даёт 2–3 отката (как в TS).
- `MAX_PLAYERS` не проверяется.
- Аутентификация netcode — нулевой ключ и id от клиента, проверка адреса в токене выключена (только Фаза 0).
- Золотые следы есть только для `jump-club`: лестницы, уступы, порталы, конвейеры, батуты, лёд с TS не сверены.
