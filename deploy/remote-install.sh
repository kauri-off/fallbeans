#!/usr/bin/env bash
# Installs a Fall Beans server release on the host. Runs as root from the bundle `cargo xtask deploy` made
# (fb_server, fallbeans.service, nginx/ next to this script): remote-install.sh <domain> <public IP>.
#
# Layout:
#   /opt/fallbeans/releases/<ts>/fb_server     releases (a static Linux binary each)
#   /opt/fallbeans/current → releases/<ts>     the live one
#   /etc/systemd/system/fallbeans.service      the service (DynamicUser)
#   /etc/nginx/apps.d/fallbeans.{conf,ws,http} wss://…/fallbeans/ws → 127.0.0.1:5889, https://…/fallbeans/api/
#                                              and /health → 127.0.0.1:5887 (included by the domain's site)
#   /etc/fallbeans.env                         FB_SECRET (identities, connect tokens), FB_DEBUG_KEY; made once
#   /run/fallbeans-updating                    exists while a release goes in: players are told and wait
#   ufw: 5888/udp (the game), 5890/udp (the probe server of `cargo xtask stress --remote`)
# Nothing changes until the checks pass; a failed release rolls back to the previous one.
set -euo pipefail
cd "$(dirname "$0")"

BASE=/opt/fallbeans
DOMAIN=${1:?домен сайта: remote-install.sh <домен> <публичный IP>}
PUBLIC_IP=${2:?публичный IP для UDP: remote-install.sh <домен> <публичный IP>}
APPS=/etc/nginx/apps.d
UNIT=/etc/systemd/system/fallbeans.service
SECRETS=/etc/fallbeans.env
UPDATING=/run/fallbeans-updating
CONFIGS="$UNIT $APPS/fallbeans.conf $APPS/fallbeans.ws $APPS/fallbeans.http"
TS=$(date +%Y%m%d%H%M%S)

say() { echo "==> $*"; }
fail() {
  echo "ОШИБКА: $*" >&2
  exit 1
}

# ------------------------------------------------------------------ checks (read-only)

say "Проверки"
[ "$(id -u)" = 0 ] || fail "запустите через sudo"
[ -f fb_server ] || fail "в пакете нет fb_server"
chmod 755 fb_server # (packed on Windows: tar keeps no mode bits)
./fb_server --help >/dev/null || fail "fb_server не запускается на этом хосте"
command -v nginx >/dev/null || fail "nginx не установлен"
[ -d $APPS ] || fail "нет $APPS: сайт $DOMAIN в nginx должен подключать $APPS/*.conf"
grep -rqs "apps.d/\*.conf" /etc/nginx/ || fail "nginx не подключает $APPS/*.conf (include в server-блоке сайта $DOMAIN)"
for port in 5888 5890; do
  if ss -Hlnu "sport = :$port" | grep -q . && ! systemctl is-active --quiet fallbeans; then
    fail "UDP $port уже занят: $(ss -Hlnup "sport = :$port")"
  fi
done

# ------------------------------------------------------------------ release

say "Релиз $TS"
R=$BASE/releases/$TS
PREV=$(readlink -f $BASE/current 2>/dev/null || true)
BACKUP=$(mktemp -d)
for f in $CONFIGS; do
  if [ -f "$f" ]; then cp -a "$f" "$BACKUP/"; fi
done
install -d -m 755 $BASE/releases "$R"
install -m 755 fb_server "$R/fb_server"
[ -f VERSION ] && install -m 644 VERSION "$R/VERSION"

switch_to() { ln -sfn "$1" $BASE/current.next && mv -T $BASE/current.next $BASE/current; }
rollback() {
  echo "ОШИБКА: $1 — возвращаем прежнюю версию" >&2
  rm -f $UPDATING
  journalctl -u fallbeans -n 30 --no-pager >&2 || true
  for f in $CONFIGS; do
    if [ -f "$BACKUP/$(basename "$f")" ]; then cp -a "$BACKUP/$(basename "$f")" "$f"; else rm -f "$f"; fi
  done
  systemctl daemon-reload
  if [ -n "$PREV" ] && [ -d "$PREV" ]; then
    switch_to "$PREV"
    systemctl restart fallbeans || true
  else
    systemctl stop fallbeans 2>/dev/null || true
  fi
  nginx -t -q && systemctl reload nginx
  rm -rf "$R" "$BACKUP"
  exit 1
}
# Up: the service runs, listens on its UDP port and on the WebSocket port, and its HTTP API answers.
healthy() {
  for _ in $(seq 1 30); do
    if systemctl is-active --quiet fallbeans && ss -Hlnu 'sport = :5888' | grep -q . &&
      ss -Hlnt 'sport = :5889' | grep -q . &&
      curl -sf -m 2 http://127.0.0.1:5887/fallbeans/health | grep -q '"ok":true'; then
      sleep 2
      systemctl is-active --quiet fallbeans && return 0
    fi
    sleep 0.5
  done
  return 1
}

# Made once: a new secret would give every player a new identity.
if [ ! -f $SECRETS ]; then
  say "Секреты: $SECRETS (ключ debug API — FB_DEBUG_KEY там же)"
  umask 077
  printf 'FB_SECRET=%s\nFB_DEBUG_KEY=%s\n' "$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')" \
    "$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')" >$SECRETS
  umask 022
fi
sed "s/@PUBLIC_IP@/$PUBLIC_IP/" fallbeans.service >$UNIT
chmod 644 $UNIT
install -m 644 nginx/fallbeans.conf $APPS/fallbeans.conf
install -m 644 nginx/fallbeans.ws $APPS/fallbeans.ws
install -m 644 nginx/fallbeans.http $APPS/fallbeans.http
systemctl daemon-reload
nginx -t -q 2>/dev/null || {
  nginx -t || true
  rollback "конфигурация nginx не прошла проверку"
}
# The running server tells its players the game is being updated and lets them go; their clients wait for
# the new one (the session API says "updating" until the flag is gone).
touch $UPDATING
sleep 2
switch_to "$R"
systemctl enable fallbeans >/dev/null 2>&1
systemctl restart fallbeans
healthy || rollback "сервер игры не поднялся"
systemctl reload nginx
rm -f $UPDATING
rm -rf "$BACKUP"
echo "Сервер игры обновлён: $R ($(cat "$R/VERSION" 2>/dev/null || echo '?'))"

# ------------------------------------------------------------------ firewall

if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then
  ufw status | grep -qE '^5888/udp .*ALLOW' || ufw allow 5888/udp comment 'Fall Beans' >/dev/null
  ufw status | grep -qE '^5890/udp .*ALLOW' || ufw allow 5890/udp comment 'Fall Beans probe' >/dev/null
fi

ls -1dt $BASE/releases/* | tail -n +4 | xargs -r rm -rf

# ------------------------------------------------------------------ checks from outside

say "Проверка снаружи"
# nginx reload is asynchronous: give the new workers a moment.
sleep 2
status=0
check() {
  local got
  got=$(curl -s -o /dev/null -w '%{http_code}' -m 20 "${@:4}" "$2" 2>/dev/null || true)
  printf '%-34s HTTP %s (ожидается %s)\n' "$1" "$got" "$3"
  [ "$got" = "$3" ] || status=1
}
# A WebSocket handshake (curl stops after the 101; the server then times the half-open link out).
check "WebSocket игры" "https://$DOMAIN/fallbeans/ws" 101 --http1.1 -m 3 \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: ZmFsbGJlYW5zLXByb2JlLTE='
check "Health" "https://$DOMAIN/fallbeans/health" 200
check "Сессия" "https://$DOMAIN/fallbeans/api/session" 200 -X POST -H 'Content-Type: application/json' \
  -d '{"identity":null,"protocol":0,"transport":"udp"}'
ss -Hlnup 'sport = :5888' | grep -q fb_server && echo "UDP 5888 слушает fb_server" || {
  echo "ВНИМАНИЕ: UDP 5888 не слушается"
  status=1
}
[ $status = 0 ] && say "Готово" || fail "что-то из проверок не прошло (см. выше)"
