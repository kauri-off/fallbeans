#!/bin/sh
# After install or upgrade (deb postinst, rpm %post): the secret once, then the service on.
set -e
ENV=/etc/fallbeans/fallbeans.env
if [ ! -f "$ENV" ]; then
  mkdir -p /etc/fallbeans
  umask 077
  cat >"$ENV" <<CONF
# Fall Beans server. A new FB_SECRET gives every player a new identity: keep it.
FB_SECRET=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
# The name players see in their server lists.
FB_NAME=
# Extra flags for fb_server (fb_server --help), e.g.: --public-host 203.0.113.5 --udp-port 6000
FB_ARGS=
CONF
fi
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  systemctl enable fallbeans.service >/dev/null 2>&1 || true
  systemctl restart fallbeans.service || true
fi
