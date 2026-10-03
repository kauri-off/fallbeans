# Прод-сервер

Состояние на 1 октября 2026 (снято с хоста командами из раздела «Как проверить»). Выкладка — `cargo xtask deploy`
(`../README.md`, «Деплой»); здесь — что это за машина, что на ней ещё живёт и чем она нас ограничивает.

## Машина

| | |
| --- | --- |
| Адрес | `168.113.157.12`, домен `киберщит-социум.рф` (`xn----9sbmkcbiwqrnkr4b1b.xn--p1ai`), часовой пояс Europe/Moscow |
| Доступ | `ssh -i ~/.ssh/cybershield_deploy deploy@168.113.157.12` (только ключ, sudo без пароля) |
| Виртуализация | KVM |
| CPU | 1 vCPU, Intel Xeon (Cascade Lake), AVX2 и AVX-512 есть |
| Память | 961 МБ RAM + swap 512 МБ (файл); в покое занято ~530 МБ и ~230 МБ swap |
| Диск | 15 ГБ (`/dev/vda1`), занято 75% |
| ОС | Ubuntu 24.04.5 LTS, ядро 6.8 |
| Сеть | `eth0` с публичным IPv4, IPv6 не настроен; TCP cubic, буферы сокетов по умолчанию (`rmem_max`/`wmem_max` 208 КБ) |
| RTT от автора | 7–25 мс ICMP, TCP-хендшейк 6–7 мс, RTT Lightyear 16–18 мс (UDP и wss) |

Расчёт сервера — на эту машину: до 4 комнат по 8 игроков, не больше ~60% ядра (`../port/plan.md`, раздел 11); увеличивать
CPU не планируется.

## Что ещё на хосте

- nginx 1.24 — общий сайт из репозитория SharedServer (`C:\Users\yarru\SharedServer`, только локальный git):
  `/etc/nginx/sites-available/shared-site`, проекты подключаются через `/etc/nginx/apps.d/*.conf`. TLS:
  `/etc/ssl/cybershield/{fullchain,privkey}.pem` (до 8 апреля 2027).
- `cybershield.service` (API заявок КиберЩита, node), Docker и containerd, fail2ban, atop.
- Самые большие по памяти: dockerd (~110 МБ), journald (~60 МБ), fail2ban (~40 МБ). Сервер игры — 17 МБ.

## Fall Beans на хосте

| | |
| --- | --- |
| Служба | `fallbeans.service` (`deploy/fallbeans.service`): `DynamicUser`, песочница systemd, `MemoryMax=300M`, `TasksMax=64`, `Restart=always` |
| Бинарник | `/opt/fallbeans/releases/<время>/fb_server` (статический musl), `/opt/fallbeans/current` → текущий; хранятся три последних, в каждом `VERSION` (коммит, `-dirty` при незакоммиченных правках) |
| Запуск | `fb_server --udp-port 5888 --ws-addr 127.0.0.1 --ws-port 5889 --http-addr 127.0.0.1 --http-port 5887 --public-host 168.113.157.12 --maintenance-file /run/fallbeans-updating --metrics-every 60`, лог — `journalctl -u fallbeans` |
| Секреты | `/etc/fallbeans.env` (root, 600): `FB_SECRET` (identity, ключ netcode, cookie), `FB_DEBUG_KEY` (вход в debug API); делается установщиком один раз |
| Порты | UDP 5888 наружу (игра); TCP 127.0.0.1:5889 (WebSocket) и 127.0.0.1:5887 (HTTP API) за nginx: `wss://киберщит-социум.рф/fallbeans/ws`, `https://киберщит-социум.рф/fallbeans/api/…`, `/fallbeans/health` |
| nginx | `/etc/nginx/apps.d/fallbeans.conf`, `fallbeans.ws`, `fallbeans.http` (`deploy/nginx/`): WebSocket без буферизации, `tcp_nodelay`, час без трафика до разрыва; HTTP с `X-Real-IP` |
| ufw | открыты 22/tcp, 80/tcp, 443/tcp, 5888/udp (игра), 5890/udp (проба); 443/udp закрыт (был у TS для WebTransport) |
| Проба | `~/fb-probe/` пользователя deploy: сервер `cargo xtask stress --remote`, UDP 5890, TCP 127.0.0.1:5891 за `/fallbeans/ws-probe` и 127.0.0.1:5892 за `/fallbeans/probe/`; запускается только на время прогона |

## Нагрузка (сервер Фазы 0: одна комната, `jump-club`)

| | CPU (одно ядро) | Память |
| --- | --- | --- |
| Без игроков | 7% (до однопоточного исполнителя — 15% и 9 300 переключений контекста в секунду) | 17 МБ |
| 8 игроков | 17% (до правки: 26–32%) | 16–17 МБ |

Тик комнаты на хосте: p99 0,2–0,5 мс, максимум 1–5 мс при сборке раунда (на машине автора в 3–4 раза быстрее).
Трафик на игрока: 13,4 КБ/с к нему, 6,7 КБ/с от него (4 × 8 игроков — ~430 КБ/с наружу). Остаток CPU в простое —
цикл 240 Гц, системы Lightyear и шаг пустой арены. 4 комнаты по 8 игроков — по оценке ~50% ядра; проверить
стрессом 4 × 8 на хосте в Фазе 3.

## Как проверить

```sh
ssh … 'lscpu; free -m; df -h /; sudo ufw status; systemctl status fallbeans; journalctl -u fallbeans -n 50'
ssh … 'P=$(pgrep -x fb_server); grep -E "VmRSS|Threads|ctxt" /proc/$P/status'   # память, потоки, переключения
curl -s -o /dev/null -w '%{http_code}' --http1.1 -m 3 -H 'Connection: Upgrade' -H 'Upgrade: websocket'   -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ=='   https://xn----9sbmkcbiwqrnkr4b1b.xn--p1ai/fallbeans/ws                      # 101 (curl выйдет по таймауту); без апгрейда — 502
curl -s https://xn----9sbmkcbiwqrnkr4b1b.xn--p1ai/fallbeans/health                     # версия, сборка, updating, комнаты, игроки
cargo xtask stress --remote --clients 8 --secs 100                                # игра через настоящую сеть
```
