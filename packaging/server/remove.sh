#!/bin/sh
# Before removal (deb prerm remove, rpm %preun on erase): the service off.
if [ -d /run/systemd/system ]; then
  systemctl disable --now fallbeans.service >/dev/null 2>&1 || true
fi
