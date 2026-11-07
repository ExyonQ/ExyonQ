#!/usr/bin/env bash
# F6-WP-03: execute the published WordPress-shaped config from a temporary copy.
# Only deployment values (port, socket, document roots) are substituted; route
# shape, cache_headers and the wp-config deny boundary are taken from the file.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" && "${ALLOW_LOCAL_E2E:-0}" != "1" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s) LOCAL_ITERATION_ONLY"
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
SOURCE_CFG="$ROOT/packaging/release/conf/wordpress.toml"
if [[ ! -x "$BIN" || ! -f "$SOURCE_CFG" ]]; then
  echo "RESULT=BLOCKED missing=exyonq-or-source-config"
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
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-f6wp03.XXXXXX")"
DOCROOT="$TMP/www"
SOCK="$TMP/php-fpm.sock"
CFG="$TMP/wordpress.toml"
FPM_CONF="$TMP/php-fpm.conf"
mkdir -p "$DOCROOT/wp-includes" "$DOCROOT/wp-content"
chmod 755 "$TMP" "$DOCROOT" "$DOCROOT/wp-includes" "$DOCROOT/wp-content"

cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'PHP_OK';
PHP
cat >"$DOCROOT/wp-config.php" <<'PHP'
<?php
file_put_contents(__DIR__ . '/wp-config-executed.marker', 'executed');
echo 'WP_CONFIG_EXECUTED';
PHP
printf 'body { color: #123; }\n' >"$DOCROOT/wp-includes/app.css"
printf 'console.log("ok");\n' >"$DOCROOT/wp-content/app.js"
chmod 644 "$DOCROOT/index.php" "$DOCROOT/wp-config.php" \
  "$DOCROOT/wp-includes/app.css" "$DOCROOT/wp-content/app.js"

cat >"$DOCROOT/.htaccess" <<'HTACCESS'
DirectoryIndex index.php
RewriteEngine On
RewriteBase /
RewriteRule ^index\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
HTACCESS
chmod 644 "$DOCROOT/.htaccess"

if [[ "$(id -u)" == "0" ]]; then
  RUN_USER="${FPM_USER:-www-data}"
  RUN_GROUP="${FPM_GROUP:-www-data}"
else
  RUN_USER="$(id -un)"
  RUN_GROUP="$(id -gn)"
fi
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
pm.max_children = 4
clear_env = no
EOF

python3 - "$SOURCE_CFG" "$CFG" "$PORT" "$DOCROOT" "$SOCK" <<'PY'
from pathlib import Path
import sys
src, dst, port, docroot, sock = sys.argv[1:]
text = Path(src).read_text()
text = text.replace('listen = "0.0.0.0:8080"', f'listen = "127.0.0.1:{port}"')
text = text.replace('address = "/run/php/php-fpm.sock"', f'address = "{sock}"')
text = text.replace('/var/www/html/wp-includes', f'{docroot}/wp-includes')
text = text.replace('/var/www/html/wp-content', f'{docroot}/wp-content')
text = text.replace('/var/www/html', docroot)
Path(dst).write_text(text)
PY

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

EXYONQ_CONTROL_SOCKET="$TMP/control.sock" RUST_LOG=info \
  "$BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
EX_PID=$!
READY=0
for _ in $(seq 1 100); do
  if [[ "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/wp-includes/app.css" 2>/dev/null || true)" == "200" ]]; then
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

check_status() {
  local path="$1" expected="$2"
  local code
  code="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}${path}" || true)"
  [[ "$code" == "$expected" ]] || {
    echo "FAIL: ${path} expected ${expected}, got ${code}"
    exit 1
  }
  echo "PASS: ${path} -> ${code}"
}

check_cache() {
  local path="$1" hdr="$TMP/headers.$RANDOM"
  local code
  code="$(curl -sS -D "$hdr" -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}${path}" || true)"
  [[ "$code" == "200" ]] || { echo "FAIL: ${path} status ${code}"; exit 1; }
  tr -d '\r' <"$hdr" | grep -Fqi 'cache-control: public, max-age=2592000, immutable' || {
    echo "FAIL: ${path} missing 30-day immutable Cache-Control"
    cat "$hdr"
    exit 1
  }
  echo "PASS: ${path} -> 200 + 30-day immutable Cache-Control"
}

check_cache "/wp-includes/app.css"
check_cache "/wp-content/app.js"
check_status "/wp-config.php" "403"
check_status "/wp-config.php?probe=1" "403"
check_status "/wp-config.php/" "403"
check_status "/wp-config.php/nested" "403"
# Not on the deny boundary: the overlay sends this missing file to index.php.
check_status "/wp-config.php.bak" "200"
check_status "/index.php" "200"
check_status "/2026/10/hello/" "200"

if [[ -e "$DOCROOT/wp-config-executed.marker" ]]; then
  echo "FAIL: wp-config.php was executed"
  exit 1
fi
echo "PASS: wp-config.php was not executed"
echo "RESULT=PASS f6-wp03 published config copy + real PHP-FPM"
