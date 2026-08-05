#!/bin/sh
set -e
if ! getent group exyonq >/dev/null 2>&1; then
  groupadd --system exyonq || true
fi
if ! getent passwd exyonq >/dev/null 2>&1; then
  useradd --system --gid exyonq --home-dir /var/lib/exyonq --shell /usr/sbin/nologin exyonq || true
fi
install -d -o exyonq -g exyonq -m 0750 /var/lib/exyonq /var/log/exyonq
if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload || true
fi
