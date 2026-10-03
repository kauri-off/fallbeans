# Сервер на хосте

Постоянного сервера у игры нет: играем по локальной сети с машины автора (`../README.md`, «Игра по локальной
сети»). Здесь — что нужно, чтобы выложить сервер на свой хост через `cargo xtask deploy`, и чем такой хост
ограничивает.

## Что нужно от хоста

- Linux x86-64 с systemd, доступ по SSH с sudo без пароля.
- nginx с https-сайтом домена, в server-блоке которого есть `include /etc/nginx/apps.d/*.conf;`. Сертификат
  должен быть публичным (Let's Encrypt и т. п.): клиент проверяет https сессии по встроенным корням, а не по
  системным.
- Публичный IPv4 (уходит в connect token как адрес UDP).
- На машине, откуда деплой: цель `x86_64-unknown-linux-musl` (`rustup target add x86_64-unknown-linux-musl`;
  на Windows — в WSL, `--wsl-distro`, плюс `build-essential cmake musl-tools`).

Хост задаётся каждый раз, умолчаний нет: `--host user@ip`, `--domain`, при необходимости `--key` и `--public-ip`
(если SSH-цель — не IP), или переменные `DEPLOY_HOST`, `DEPLOY_KEY`, `DEPLOY_DOMAIN`, `DEPLOY_PUBLIC_IP`.

## Что ставится

| | |
| --- | --- |
| Служба | `fallbeans.service` (`deploy/fallbeans.service`): `DynamicUser`, песочница systemd, `MemoryMax=300M`, `TasksMax=64`, `Restart=always` |
| Бинарник | `/opt/fallbeans/releases/<время>/fb_server` (статический musl), `/opt/fallbeans/current` → текущий; хранятся три последних, в каждом `VERSION` (коммит, `-dirty` при незакоммиченных правках) |
| Запуск | `fb_server --udp-port 5888 --ws-addr 127.0.0.1 --ws-port 5889 --http-addr 127.0.0.1 --http-port 5887 --public-host <IP> --maintenance-file /run/fallbeans-updating --metrics-every 60`, лог — `journalctl -u fallbeans` |
| Секреты | `/etc/fallbeans.env` (root, 600): `FB_SECRET` (identity, ключ netcode, cookie), `FB_DEBUG_KEY` (вход в debug API); делается установщиком один раз |
| Порты | UDP 5888 наружу (игра); TCP 127.0.0.1:5889 (WebSocket) и 127.0.0.1:5887 (HTTP API) за nginx: `wss://<домен>/fallbeans/ws`, `https://<домен>/fallbeans/api/…`, `/fallbeans/health` |
| nginx | `/etc/nginx/apps.d/fallbeans.conf`, `fallbeans.ws`, `fallbeans.http` (`deploy/nginx/`): WebSocket без буферизации, `tcp_nodelay`, час без трафика до разрыва; HTTP с `X-Real-IP` |
| ufw | если включён: 5888/udp (игра), 5890/udp (проба) |
| Проба | `~/fb-probe/` SSH-пользователя: сервер `cargo xtask stress --remote`, UDP 5890, TCP 127.0.0.1:5891 за `/fallbeans/ws-probe` и 127.0.0.1:5892 за `/fallbeans/probe/`; запускается только на время прогона |

## Нагрузка (замеры Фазы 0 на VPS: 1 vCPU KVM, Xeon Cascade Lake, ~1 ГБ RAM; одна комната, `jump-club`)

| | CPU (одно ядро) | Память |
| --- | --- | --- |
| Без игроков | 7% (до однопоточного исполнителя — 15% и 9 300 переключений контекста в секунду) | 17 МБ |
| 8 игроков | 17% (до правки: 26–32%) | 16–17 МБ |

Тик комнаты там: p99 0,2–0,5 мс, максимум 1–5 мс при сборке раунда (на машине автора в 3–4 раза быстрее).
Трафик на игрока: 13,4 КБ/с к нему, 6,7 КБ/с от него (4 × 8 игроков — ~430 КБ/с наружу). Остаток CPU в простое —
цикл 240 Гц, системы Lightyear и шаг пустой арены.

## Как проверить

```sh
ssh … 'systemctl status fallbeans; journalctl -u fallbeans -n 50'
ssh … 'P=$(pgrep -x fb_server); grep -E "VmRSS|Threads|ctxt" /proc/$P/status'   # память, потоки, переключения
curl -s https://<домен>/fallbeans/health                                         # версия, сборка, updating, комнаты, игроки
cargo xtask stress --remote --host … --domain … --clients 8 --secs 100           # игра через настоящую сеть
```
