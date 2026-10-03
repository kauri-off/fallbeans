# Fall Beans на Rust

Перенос игры на Rust + Bevy + Lightyear. **С чего продолжать — [`port/state.md`](port/state.md).** ТЗ и фазы —
[`port/plan.md`](port/plan.md), решения вне плана — [`port/decisions.md`](port/decisions.md), отчёты фаз — [`port/phases/`](port/phases/);
сервер на своём хосте — [`deploy/README.md`](deploy/README.md). Этот файл — как работать с кодом.

## Что нужно

- Rust stable ≥ 1.95 (`rustup`), компоненты `rustfmt` и `clippy`.
- Linux: `libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev` (как в CI).
- Bun — только для `cargo xtask golden`, `cargo xtask assets` и `cargo xtask audit --vs-ts`: они берут данные из TS-версии.

Первая сборка долгая (Bevy). В `dev` профиле зависимости собираются с `opt-level = 3`, свой код — с 1.

## Устройство

```
rust/
  core/            детерминированное ядро: без Bevy, f64, математика только через fb_shared::m
    clippy.toml    правила детерминизма (действуют только на core/*)
    fb_shared      константы, ввод (InputFrame), mulberry32, m (libm, hypot как в V8, round_js)
    fb_sim         физика боба (physics.rs), коллайдеры, World (узлы + movers), Builder, бонусы, SceneDesc,
                   навигация ботов (nav.rs), поведение ботов (bots.rs)
    fb_maps        все карты (по модулю на карту), реестры GAMES/MAPS, director (план раундов игры)
    fb_arena       арена: пешки и боты, захваты, таклы, финиш, чекпоинты, падения, выбывание, бонусы;
                   tick_bodies — общий шаг сервера и предсказания
      tests/       golden.rs (полные раунды карт с ботами против TS-арены), scenarios.rs (ветки физики боба в малых мирах против TS),
                   determinism.rs (записанные хэши), replay.rs (откат = прямой прогон), recording.rs (запись раунда → replay)
    fb_audit       аудиты карт и систем (порт src/audit), harness безголовых раундов; бинарник fb_audit; rayon
      tests/       quick.rs (быстрые аудиты: 0 ошибок и 0 предупреждений)
  crates/          всё на Bevy и Lightyear
    fb_proto       управляющие сообщения (ClientMsg/ServerMsg с проверкой границ), события карты; без Bevy
    fb_net         протокол Lightyear: компоненты, ввод, каналы, форматы передачи (wire.rs),
                   фильтры видимости (комната, владелец), NetSim (флаги имитации сети), NetStats (трафик)
    fb_server      rooms/ (Hub, Room, GameClock, награды, debug — обычный Rust, тесты без сети), auth,
                   play (хаб в ECS: сообщения, ввод, сущности комнат и бобов), net (транспорты, вход),
                   http (axum в своём потоке: сессия и connect token, health, debug API), logbook, metrics, opts
    fb_client      net (сессия по HTTP, подключение, переподключение), session (hello, комната, лобби; --start), game (карта, предсказание,
                   события карты, ввод, автопилот), view (карта, бобы, камера, режим зрителя), hud, brp (BRP-probe), stats, assets, opts
  xtask            cargo xtask <check|golden|audit|assets|dev|stress|deploy>
  deploy/          что ставится на хост: systemd-юнит, nginx (wss), установщик с откатом; README — сам хост
  vendor/          зависимости с нашими правками ([patch.crates-io] в Cargo.toml; см. «Известные проблемы»)
  assets/models    glb для Bevy (делаются из ../public/models: cargo xtask assets)
  assets/fonts     Nunito Bold/Black с кириллицей (из ../public/fonts, port/decisions.md), OFL
```

Игрок входит так: `POST /fallbeans/api/session` (identity и connect token netcode с id игрока внутри) →
соединение → `Hello` → список комнат или сразу комната (`--room`, тренировка `--practice`). У
каждой комнаты на сервере сущность `Round` (её текущая арена: лобби, раунд или подиум) и по сущности на боба в игре;
ссылке игрока видны только сущности его комнаты (`RoomTag`/`InRoom`). Сервер строит карту от сида без сцены; клиент
строит ту же карту со сценой (`SceneDesc`) и сверяет хэш коллайдеров (`Round.static_hash`). По сети идут только
бобы (`BodyFull` владельцу, `RemotePose` остальным), события карты с тиком и управляющие сообщения.

## Команды

```sh
cargo xtask check                 # fmt --check, clippy -D warnings, тесты — перед каждой сдачей работы
cargo xtask dev --clients 2       # сервер --dev и два окна в комнате dev, игра --map; --autopilot, --fill (боты на пустые места), --lag/--jitter/--loss, --seed, --release
cargo xtask stress --clients 8 --secs 100 --lag 75 --jitter 15 --loss 0.05
cargo xtask stress --clients 32 --rooms 4 --secs 100 --lag 75 --jitter 15 --loss 0.05   # 4 комнаты по 8
cargo xtask stress --release --clients 16 --rooms 4 --secs 100 --client-arg=--fill   # 4 комнаты по 4 игрока и 4 бота
cargo xtask stress --release --limit-server --clients 32 --rooms 4 --secs 100 --lag 75 --jitter 15 --loss 0.05   # ворота Фазы 3: сервер как на 1 vCPU
cargo xtask stress --transport auto --secs 110 --client-arg=--udp-blocked=20   # UDP → WS при входе и обратно на UDP
cargo xtask stress --transport ws --lag 75 --jitter 15   # по WebSocket (потери не задавать: TCP не теряет)
cargo xtask stress --remote --host … --domain … --clients 8 --secs 100 --transport udp|ws|auto   # через реальную сеть до хоста
cargo xtask golden                # переснять следы из TS и сверить (после изменений в TS-физике или карте)
cargo xtask audit [карта…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics] [--notes] [--json]
cargo xtask audit --vs-ts         # те же аудиты TS (с этой libm) и сверка: всё, кроме замеров времени, должно совпасть
cargo xtask assets                # переэкспортировать модели и проверить загрузку в Bevy
cargo xtask deploy --host … --domain …   # выложить сервер на свой хост (только по просьбе автора, см. «Деплой»)
```

Бинарники напрямую (`cargo run -p fb_server -- --help`, `cargo run -p fb_client -- --help`):

- сервер: `--dev` (dev-команды, комната `dev`), `--solo` (игра с одним игроком), `--respawn` (упавший в
  «выживании» возвращается на респаун, а не выбывает; так гоняет `stress`), `--open-rooms a,b` (постоянные
  комнаты), `--seed`, `--intro`, `--maintenance-file` (пока файл есть — «игра обновляется»), `--udp-port`,
  `--ws-addr`, `--ws-port`, `--http-addr`, `--http-port` (5887), `--public-host` (IP для UDP в connect token; без
  него — адрес, по которому клиент спросил HTTP, и без проверки адреса), `--lag/--jitter/--loss`, `--trace файл`,
  `--metrics-every с`, `--exit-after с`; `FB_SECRET` (64 hex) — секрет identity, ключа netcode и cookie
  (без него — случайный на запуск), `FB_DEBUG_KEY` — ключ debug API без `--dev`;
- клиент: `--server`, `--http-url` (по умолчанию `http://<server>:5887/fallbeans`), `--transport auto|udp|ws`,
  `--ws-url`, `--name`, `--token` (identity), `--room`, `--pin`, `--practice карта`, `--color`, `--start карта
  --start-players n --start-rounds n` (хостом стартует игру из раундов этой карты), `--fill` (хост заполняет комнату ботами),
  `--lag/--jitter/--loss`, `--input-margin`, `--udp-blocked с` (проверка `auto`: всё, что приходит по UDP, первые
  N секунд выбрасывается), `--backend vulkan|dx12|gl`, `--headless` (без окна и GPU, автопилот), `--fps`, `--autopilot`, `--trace файл`,
  `--screenshot файл --exit-after с`, `--check-assets`, `--brp [порт]` (BRP-probe, см. «Отладка»).

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
  (`InputFrame::clamped`). Сервер читает `InputBuffer` сам (`play::frame_for`; перенос Lightyear в
  `ActionState` выключен, он выбрасывает поздний ввод): без ввода держит стик и захват `INPUT_HOLD` тиков,
  нажатие, пришедшее до `LATE_TICKS` (250 мс) позже своего тика, выполняет на следующем.
- Настройки ввода (`INPUT_SEND_INTERVAL`, `INPUT_REDUNDANCY` = `LATE_TICKS`) и `--input-margin` подобраны
  замерами (`port/phases/0.md`); менять — с прогоном `xtask stress` (и `--remote`) до и после.
- Один `Server` Lightyear слушает оба транспорта (`net::start_server`: `ServerUdpIo` и `WebSocketServerIo` на одной
  сущности), топология обычная `Server`. Connect token выдаёт HTTP API (`http.rs`): ключ netcode выводится из
  `FB_SECRET`, в user data — id игрока, адрес — публичный UDP (`--public-host`), он же проверяется как
  `additional_expected_addresses`. Токен — на одну попытку (30 с); таймаут соединения в токене: UDP 3 с, WS 10 с.
- HTTP API отвечает из своего потока; всё, что читает комнаты, — задание главному циклу (`http::Api::ask`,
  выполняется в `Update`). Флаг обновления и замеры для `/api/debug/health` идут в поток HTTP через `HttpShared`.
- Сервер однопоточный: все расписания на `SingleThreadedExecutor` (`fb_server/src/main.rs`), фича Bevy
  `multi_threaded` у `fb_server` не включается. На 1 vCPU многопоточный исполнитель тратил 15% ядра в простое на
  передачу систем пулу. Параллелить (комнаты по потокам) — только по замерам Фазы 3.
- Видимость — фильтрами Replicon (`visibility.rs`): комната (`RoomTag` против `InRoom` ссылки), владелец
  (`OwnerOnly`, `OthersOnly`); новые фильтры регистрирует `add_server_filters`.
- Сообщения клиентов сервер читает каждый кадр (`play::receive` в `PreUpdate`): Lightyear выбрасывает
  непрочитанные за кадр, а тик идёт через кадр.
- Логика комнат (`fb_server::rooms`) не знает о сети: на вход — соединения (`ConnId`), `ClientMsg` и ввод
  (`Inputs`), на выход — `Out`. Новое правило комнаты — с тестом в `rooms/tests.rs`.

**Остальное.** Зависимости — в `[workspace.dependencies]`; Bevy, Lightyear и всё, что с ними связано, пинится
точно. Комментарии и вывод инструментов — по-английски, текст для игрока — по-русски. Переводы строк — LF.

## Отладка

- **BRP-probe клиента** (`fb_client --brp [порт]`, по умолчанию 15702, только 127.0.0.1; фича `brp`, в раздаваемых
  сборках выключается): JSON-RPC по HTTP, методы Bevy (`world.query`, `world.get_resources`…) и свои —
  `fb/state` (соединение, комната, лобби, арена с финишем и выбывшими, свой боб, чужие, статус
  `play|finished|out|spectating`), `fb/send` (`ClientMsg` в JSON: `"Start"`, `{"Fill":true}`), `fb/dev` (`DevCmd`:
  `"SkipIntro"`, `{"Goto":{"id":null,"to":"Finish"}}`, `{"Kill":{"id":null}}`), `fb/input` (стик в мировых осях
  и кнопки на `secs` секунд вместо клавиатуры и автопилота: `{"mx":0,"mz":1,"jump":true,"secs":2}`):
  `curl -d '{"jsonrpc":"2.0","id":1,"method":"fb/state"}' 127.0.0.1:15702`.

- **Предсказание.** `cargo xtask stress` пишет трассы (`target/stress/server.trace`, `client-N.trace`: тик, id,
  ввод, позиция) и логи. Таблица показывает по каждому клиенту опоздавший ввод и расхождения по причинам;
  «прочие» печатаются тиками — смотреть строки этих тиков в трассах сервера и клиента.
- **Без стресса:** сервер и клиент с `--trace` и теми же флагами сети, сравнение тех же строк.
- **Debug API сервера** (порт `debugApi.ts`): `state`, `health`, `logs?level=&n=`, `trace?room=&id=&s=`,
  `replay?room=&i=` (запись раунда; комнаты пишут раунды только с `--dev`), `maps`; `?format=text` — компактный
  текст. Локально с `--dev` — без ключа: `curl "http://127.0.0.1:5887/fallbeans/api/debug/state?format=text"`.
  Без `--dev` — cookie: `curl -c c.txt "https://…/fallbeans/api/debug/login?key=$FB_DEBUG_KEY"`, затем `-b c.txt`
  (после `deploy` ключ — в `/etc/fallbeans.env` на хосте).
- **Логи.** Клиент раз в секунду пишет `stats:` (транспорт, RTT, джиттер, самый длинный кадр за секунду,
  сдвиги часов Lightyear, откаты, предсказанные тики, трафик, раунд, `MAP HASH MISMATCH` при расхождении
  карты), сервер раз в `--metrics-every` с — `metrics:` (тик p50/p99/max, самый длинный кадр, тики без ввода
  игрока вовремя, трафик, пакеты, CPU — доля одного ядра за окно, RSS). HUD в окне показывает то же. `stress`
  падает на тике p99 > 1 мс (все комнаты вместе), CPU сервера > 60% ядра (`--max-cpu`), RSS > 300 МБ
  (`--max-mem`); `--limit-server` (Linux) запускает сервер одного на последнем ядре (клиенты — на остальных) в
  scope systemd с `MemoryMax=300M` и без swap.
- **Трассы клиента** пишутся только при живом соединении; `R тик` — начало соединения (первого или после
  переподключения): `stress` с этого тика ждёт, пока сервер получит ввод клиента, и не считает этот разрыв
  опозданием.
- **Опоздавший ввод.** Длинный кадр клиента (`frame max`) — затык этой машины; сдвиг часов (`shifts`) — Lightyear
  пересинхронизировал часы и переразметил уже отправленный ввод (см. «Известные проблемы»); ни того ни другого
  — сеть. `stress` выводит оба столбца.
- **Графика.** `fb_client --backend gl --screenshot shot.png --exit-after 15` против запущенного сервера.

## Игра по локальной сети

Постоянного сервера нет: сервер запускается на одной машине, клиенты — на ней и на других в той же сети.

```sh
cargo build --release -p fb_server -p fb_client
target/release/fb_server --dev --public-host <LAN-IP сервера>        # --solo: игра с одним игроком
target/release/fb_client --server 127.0.0.1 --room dev --name A --start jump-club   # на машине сервера
fb_client --server <LAN-IP сервера> --room dev --name B                              # на другой машине
```

Открыть на сервере 5887/tcp (HTTP API), 5888/udp (игра), 5889/tcp (WebSocket). Клиенту нужна папка `assets/`
рядом с бинарником (или `BEVY_ASSET_ROOT`). `--public-host` обязателен, если клиенты обращаются к серверу по
имени: без него в токен уходит IP из запроса, а для имени — 127.0.0.1. https сессии через свой обратный прокси
со своим CA не пройдёт: `ureq` проверяет сертификат по встроенным корням (wss берёт системные).

## Деплой

Только по просьбе автора; своего хоста сейчас нет. `cargo xtask deploy` прогоняет `check`, собирает статический
Linux-бинарник сервера (musl: нужна цель `x86_64-unknown-linux-musl`; на Windows — в WSL, `--wsl-distro`, плюс
`build-essential cmake musl-tools`), пакует его с `deploy/` и по SSH запускает `deploy/remote-install.sh`. Хост
задаётся явно, умолчаний нет: `--host user@ip`, `--domain`, при необходимости `--key`, `--public-ip` (или
`DEPLOY_HOST`, `DEPLOY_KEY`, `DEPLOY_DOMAIN`, `DEPLOY_PUBLIC_IP`); требования к хосту — `deploy/README.md`.

- релиз в `/opt/fallbeans/releases/<время>`, ссылка `current`, три последних релиза хранятся;
- секреты `/etc/fallbeans.env` (`FB_SECRET`, `FB_DEBUG_KEY`) создаются один раз и дальше не трогаются: новый
  секрет даёт всем игрокам новые identity;
- systemd-юнит `fallbeans.service` (`DynamicUser`, `MemoryMax=300M`): UDP 5888 наружу, WebSocket на
  127.0.0.1:5889, HTTP API на 127.0.0.1:5887;
- nginx: `/etc/nginx/apps.d/fallbeans.conf` (https-сайт домена включает `apps.d/*.conf`):
  `wss://…/fallbeans/ws` → 5889, `/fallbeans/api/` и `/fallbeans/health` → 5887, `/fallbeans/ws-probe` → 5891 и
  `/fallbeans/probe/` → 5892 (сервер пробы);
- на время перезапуска — `/run/fallbeans-updating`: старый сервер говорит игрокам «игра обновляется» и отпускает
  их, клиенты ждут, пока `/api/session` не перестанет отвечать `updating`;
- ufw: 5888/udp (игра), 5890/udp (проба);
- проверка: служба жива, слушает оба порта, `/fallbeans/health` отвечает; иначе откат на прежний релиз.

`--pack-only` только собирает архив (`target/deploy/`), `--yes` не спрашивает, `--skip-checks` пропускает `check`.

**Проба** (`stress --remote`, тот же `--host`/`--domain`): та же сборка этого дерева запускается в `~/fb-probe` SSH-пользователя (UDP 5890,
WS 5891 и HTTP 5892 за nginx, `--public-host` — адрес хоста) на время прогона; клиенты идут с этой машины по настоящей сети, трасса сервера скачивается и
сверяется как в локальном `stress`. Службу игры проба не трогает.

## Известные проблемы

- Боты комнаты думают все в одном тике (каждый 6-й, как в TS) и перестраивают маршрут A* (до 2500 узлов,
  ~0,3 мкс на узел на машине автора): тик с несколькими перестройками стоит 0,5–0,8 мс на комнату, в худшем
  случае (восемь неудачных поисков разом) — до ~6 мс (`port/phases/3.md`).
- `vendor/aeronet_websocket`: сервер WebSocket не включал `TCP_NODELAY` (клиент умеет, сервер нет), это давало
  +15 мс RTT на wss. Правка в одну строку (`server/backend.rs`); убрать, когда появится в aeronet, а при обновлении aeronet перенести.
- Пересинхронизация часов Lightyear: если опережение клиента ушло от цели больше чем на
  `SyncConfig::max_error_margin` (10 тиков, 83 мс; скачок RTT, затык клиента), Lightyear сдвигает часы и
  переразмечает буфер ввода. Уже отправленный ввод уходит на другие тики: сервер видит опоздание или
  «переписанную историю» (ошибки `lightyear_debug::input` в логе), клиент откатывается.
- Таймаут netcode — 3 с: остановка TCP-потока внутри VPN (VLESS xhttp) рвёт WS-соединение. На таких путях
  рывки джиттера вызывают серии сдвигов часов (`port/phases/0.md`, «Матрица VPN»); `fb_client --sync-max-error` —
  флаг для опытов с порогом.
- После обрыва клиент берёт новый токен и подключается снова тем же игроком (identity из `/api/session`, место
  держится 30 с), с `auto` обрыв UDP переводит на WS. На WS `auto` раз в минуту проверяет UDP своим соединением
  netcode в отдельном `App` (`fb_client/src/probe.rs`: поднялось за 2 с и держится 5 с) и переходит обратно между
  раундами или вне комнаты — разрыв в лобби вывел бы игрока из него.
- Сервер не может разорвать одного клиента netcode: клиент, которому отказали, уходит сам; не ушедший
  отваливается по таймауту (`Received UDP packet for unknown entity` в логе; `port/decisions.md`).
- Следующий раунд игры собирается в потоке во время результатов (вместе с сеткой ботов); первый раунд игры и
  тренировка — ещё в тике (до 6 мс), их сетка ботов — в тике интро.
- На GL wgpu пишет ~70 ошибок `CubeArray` в секунду; безвредно. SSAO на GL отключён через лимит storage-текстур
  (`fb_client/src/main.rs`).
- Респаун не предсказывается: каждое падение даёт 2–3 отката (как в TS).
- Золотые следы есть только для `jump-club`: лестницы, уступы, порталы, конвейеры, батуты, лёд с TS не сверены.
