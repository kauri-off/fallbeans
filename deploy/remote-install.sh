#!/usr/bin/env bash
# Installs a Fall Beans release on the host. Runs as root from an unpacked bundle made by
# scripts/deploy.ts (release.tar.gz, fallbeans.service, nginx/ next to this script).
#
#   DEBUG_KEY=…     optional: set/replace the key of the debug page.
#
# Layout:
#   /opt/fallbeans/bin/bun                   Bun runtime (downloaded once from the official release)
#   /opt/fallbeans/releases/<ts>             releases (www/fallbeans/ static site, server/)
#   /opt/fallbeans/current → releases/<ts>   the live one
#   /etc/fallbeans/env                       FB_SECRET, FB_DEBUG_KEY (root only)
#   /etc/nginx/apps.d/fallbeans.{http,conf,headers}   nginx (included by the shared site)
#   /etc/systemd/system/fallbeans.service    service (DynamicUser, TLS via LoadCredential)
# Nothing changes until the checks pass; a failed release rolls back to the previous one.
set -euo pipefail
cd "$(dirname "$0")"

BASE=/opt/fallbeans
BUN_VERSION=1.4.2
DOMAIN=xn----9sbmkcbiwqrnkr4b1b.xn--p1ai
APPS=/etc/nginx/apps.d
UNIT=/etc/systemd/system/fallbeans.service
ENV_FILE=/etc/fallbeans/env
TS=$(date +%Y%m%d%H%M%S)

say() { echo "==> $*"; }
fail() {
  echo "ОШИБКА: $*" >&2
  exit 1
}

# ------------------------------------------------------------------ checks (read-only)

say "Проверки"
[ "$(id -u)" = 0 ] || fail "запустите через sudo"
[ -f release.tar.gz ] || fail "в пакете нет release.tar.gz"
command -v nginx >/dev/null || fail "nginx не установлен"
[ -L /etc/nginx/sites-enabled/shared-site ] && [ -d $APPS ] ||
  fail "общий сайт nginx не установлен — сначала выложите SharedServer (node deploy.mjs)"
grep -q "apps.d/\*.conf" /etc/nginx/sites-available/shared-site || fail "общий сайт не подключает /etc/nginx/apps.d/*.conf"
[ -f /etc/ssl/cybershield/fullchain.pem ] && [ -f /etc/ssl/cybershield/privkey.pem ] || fail "нет сертификата в /etc/ssl/cybershield"
if ss -Hlnup 'sport = :443' | grep -qv fallbeans && ! systemctl is-active --quiet fallbeans; then
  fail "UDP 443 уже занят другим процессом: $(ss -Hlnup 'sport = :443')"
fi
command -v python3 >/dev/null || fail "нужен python3 (распаковка Bun)"

# ------------------------------------------------------------------ Bun runtime

if ! [ -x $BASE/bin/bun ] || [ "$($BASE/bin/bun --version 2>/dev/null)" != "$BUN_VERSION" ]; then
  say "Bun $BUN_VERSION"
  tmp=$(mktemp -d)
  url=https://github.com/oven-sh/bun/releases/download/bun-v$BUN_VERSION
  curl -fsSL -o "$tmp/bun.zip" "$url/bun-linux-x64.zip"
  curl -fsSL -o "$tmp/SHASUMS256.txt" "$url/SHASUMS256.txt"
  want=$(grep ' bun-linux-x64.zip$' "$tmp/SHASUMS256.txt" | cut -d' ' -f1)
  got=$(sha256sum "$tmp/bun.zip" | cut -d' ' -f1)
  [ -n "$want" ] && [ "$want" = "$got" ] || fail "контрольная сумма Bun не совпадает"
  python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$tmp/bun.zip" "$tmp"
  install -d -m 755 $BASE/bin
  install -m 755 "$tmp/bun-linux-x64/bun" $BASE/bin/bun
  rm -rf "$tmp"
fi

# ------------------------------------------------------------------ secrets

install -d -m 700 /etc/fallbeans
touch $ENV_FILE
chmod 600 $ENV_FILE
setvar() {
  local tmp
  tmp=$(mktemp)
  grep -v "^$1=" $ENV_FILE >"$tmp" || true
  printf '%s=%s\n' "$1" "$2" >>"$tmp"
  install -m 600 "$tmp" $ENV_FILE
  rm -f "$tmp"
}
grep -q '^FB_SECRET=' $ENV_FILE || setvar FB_SECRET "$(openssl rand -hex 32)"
if [ -n "${DEBUG_KEY:-}" ]; then
  say "Ключ страницы отладки обновлён"
  setvar FB_DEBUG_KEY "$DEBUG_KEY"
fi

# ------------------------------------------------------------------ firewall

if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then
  if ! ufw status | grep -qE '^443/udp .*ALLOW'; then
    say "ufw: открываем 443/udp (WebTransport)"
    ufw allow 443/udp comment 'Fall Beans WebTransport' >/dev/null
  fi
fi

# ------------------------------------------------------------------ release

say "Релиз $TS"
R=$BASE/releases/$TS
PREV=$(readlink -f $BASE/current 2>/dev/null || true)
BACKUP=$(mktemp -d)
for f in $UNIT $APPS/fallbeans.http $APPS/fallbeans.conf $APPS/fallbeans.headers; do
  if [ -f "$f" ]; then cp -a "$f" "$BACKUP/"; fi
done
install -d -m 755 $BASE/releases
mkdir -p "$R"
tar -xzf release.tar.gz -C "$R" --no-same-owner
chown -R root:root "$R"
chmod -R go-w "$R"
chmod -R a+rX "$R"
[ -f "$R/server/main.js" ] && [ -f "$R/www/fallbeans/index.html" ] || fail "релиз неполный"

switch_to() { ln -sfn "$1" $BASE/current.next && mv -T $BASE/current.next $BASE/current; }
restore_configs() {
  for f in $UNIT $APPS/fallbeans.http $APPS/fallbeans.conf $APPS/fallbeans.headers; do
    if [ -f "$BACKUP/$(basename "$f")" ]; then cp -a "$BACKUP/$(basename "$f")" "$f"; else rm -f "$f"; fi
  done
  systemctl daemon-reload
}
rollback() {
  echo "ОШИБКА: $1 — возвращаем прежнюю версию" >&2
  restore_configs
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
healthy() {
  for _ in $(seq 1 30); do
    curl -fsS -m 2 http://127.0.0.1:7777/fallbeans/health >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  return 1
}

install -m 644 fallbeans.service $UNIT
install -m 644 nginx/fallbeans.http $APPS/fallbeans.http
install -m 644 nginx/fallbeans.conf $APPS/fallbeans.conf
install -m 644 nginx/fallbeans.headers $APPS/fallbeans.headers
systemctl daemon-reload
nginx -t -q 2>/dev/null || {
  nginx -t || true
  rollback "конфигурация nginx не прошла проверку"
}
switch_to "$R"
systemctl enable fallbeans >/dev/null 2>&1
systemctl restart fallbeans
healthy || {
  journalctl -u fallbeans -n 30 --no-pager >&2 || true
  rollback "сервер игры не отвечает"
}
systemctl reload nginx
rm -rf "$BACKUP"
ls -1dt $BASE/releases/* | tail -n +4 | xargs -r rm -rf
echo "Сервер игры обновлён: $R ($(cat "$R/VERSION" 2>/dev/null || echo '?'))"

# ------------------------------------------------------------------ checks from outside

say "Проверка снаружи"
# nginx reload is asynchronous: give the new workers a moment.
sleep 2
code() { curl -sS -o /dev/null -w '%{http_code}' -m 20 "$1" || true; }
status=0
check() {
  local got
  got=$(code "$2")
  printf '%-34s HTTP %s (ожидается %s)\n' "$1" "$got" "$3"
  [ "$got" = "$3" ] || status=1
}
check "Игра" "https://$DOMAIN/fallbeans/" 200
check "Сервер игры" "https://$DOMAIN/fallbeans/health" 200
check "Сессия" "https://$DOMAIN/fallbeans/api/session" 200
check "Отладка без ключа" "https://$DOMAIN/fallbeans/api/debug/state" 403
check "Сайт КиберЩита" "https://$DOMAIN/" 200
if ss -Hlnup 'sport = :443' | grep -q .; then echo "WebTransport слушает udp/443"; else
  echo "ВНИМАНИЕ: udp/443 не слушается — WebTransport выключен, игра работает через WebSocket"
fi
[ $status = 0 ] && say "Готово" || fail "что-то из проверок не прошло (см. выше)"
