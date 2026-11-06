#!/usr/bin/env bash
# F6-WP-01: exact-path deny-all route must return 403 and never execute PHP.
# Real exyonq binary + real PHP-FPM socket; no nginx and no mock upstream.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$BIN" ]]; then
  echo "RESULT=BLOCKED missing=exyonq-binary path=$BIN"
  exit 3
fi

PHP_FPM_BIN="${PHP_FPM_BIN:-}"
if [[ -z "$PHP_FPM_BIN" ]]; then
  for candidate in php-fpm php-fpm8.5 php-fpm8.4 php-fpm8.3 php-fpm8.2 php-fpm8.1; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PHP_FPM_BIN="$(command -v "$candidate")"
      break
    fi
  done
fi
if [[ -z "$PHP_FPM_BIN" ]]; then
  echo "RESULT=BLOCKED missing=php-fpm"
  exit 3
fi

PORT="$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-f6wp01.XXXXXX")"
DOCROOT="$TMP/www"
SOCK="$TMP/php-fpm.sock"
FPM_CONF="$TMP/php-fpm.conf"
CFG="$TMP/exyonq.toml"
mkdir -p "$DOCROOT"
chmod 755 "$TMP" "$DOCROOT"

cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'PHP_OK';
PHP
cat >"$DOCROOT/wp-config.php" <<'PHP'
<?php
file_put_contents(__DIR__ . '/wp-config-executed.marker', 'executed');
http_response_code(500);
echo 'WP_CONFIG_EXECUTED';
PHP
chmod 644 "$DOCROOT/index.php" "$DOCROOT/wp-config.php"

RUN_USER="$(id -un)"
RUN_GROUP="$(id -gn)"
cat >"$FPM_CONF" <<EOF
[global]
error_log = $TMP/php-fpm.log
daemonize = no
[www]
user = $RUN_USER
group = $RUN_GROUP
listen = $SOCK
listen.owner = $RUN_USER
listen.group = $RUN_GROUP
listen.mode = 0666
pm = static
pm.max_children = 2
clear_env = no
EOF

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["php", "wp-config-deny"]

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"

[[route]]
name = "wp-config-deny"
match = { path = "/wp-config.php" }
access = { deny = ["all"] }

[[fcgi_pool]]
name = "php"
address = "${SOCK}"
document_root = "${DOCROOT}"
max_concurrency = 4
EOF

FPM_PID=""
EX_PID=""
cleanup() {
  [[ -n "${EX_PID:-}" ]] && kill "$EX_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

"$PHP_FPM_BIN" -t -y "$FPM_CONF" >/dev/null
"$PHP_FPM_BIN" --nodaemonize --fpm-config "$FPM_CONF" >"$TMP/php-fpm.out" 2>&1 &
FPM_PID=$!
for _ in $(seq 1 100); do
  [[ -S "$SOCK" ]] && break
  sleep 0.1
done
if [[ ! -S "$SOCK" ]]; then
  echo "FAIL: php-fpm socket missing"
  cat "$TMP/php-fpm.out" "$TMP/php-fpm.log" 2>/dev/null || true
  exit 1
fi

RUST_LOG=info "$BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
EX_PID=$!
READY=0
for _ in $(seq 1 100); do
  if [[ "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/index.php" 2>/dev/null || true)" == "200" ]]; then
    READY=1
    break
  fi
  if ! kill -0 "$EX_PID" 2>/dev/null; then
    echo "FAIL: exyonq exited during startup"
    cat "$TMP/exyonq.log" || true
    exit 1
  fi
  sleep 0.1
done
if [[ "$READY" -ne 1 ]]; then
  echo "FAIL: exyonq readiness timeout"
  cat "$TMP/exyonq.log" || true
  exit 1
fi

check() {
  local path="$1" expected="$2" body="$TMP/body.$RANDOM"
  local code
  code="$(curl -sS -o "$body" -w '%{http_code}' "http://127.0.0.1:${PORT}${path}" || true)"
  if [[ "$code" != "$expected" ]]; then
    echo "FAIL: ${path} expected ${expected}, got ${code}"
    cat "$body" 2>/dev/null || true
    return 1
  fi
  echo "PASS: ${path} -> ${code}"
}

check "/wp-config.php" "403"
check "/wp-config.php?probe=1" "403"
check "/wp-config.php/" "403"
check "/wp-config.php/nested" "403"
check "/wp-config.php.bak" "404"
check "/index.php" "200"

if [[ -e "$DOCROOT/wp-config-executed.marker" ]]; then
  echo "FAIL: wp-config.php was executed; marker exists"
  exit 1
fi
echo "PASS: wp-config.php was not executed"

echo "RESULT=PASS f6-wp01 path-boundary deny-all + real PHP-FPM"
