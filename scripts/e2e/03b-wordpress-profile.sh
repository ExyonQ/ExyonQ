#!/usr/bin/env bash
# AUTHORITATIVE FastCGI PHP application profile for Core Regression Gate.
# Real php-fpm + real PHP front-controller app (WordPress-shaped routing contract).
# Not a product WordPress plugin test; not a mock HTTP peer.
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

PHP_FPM_BIN=""
for c in php-fpm php-fpm8.3 php-fpm8.2 php-fpm8.1; do
  if command -v "$c" >/dev/null 2>&1; then PHP_FPM_BIN="$(command -v "$c")"; break; fi
done
if [[ -z "$PHP_FPM_BIN" ]]; then
  echo "RESULT=BLOCKED missing=php-fpm"
  exit 3
fi

# Non-privileged pool identity — same policy as Plan 08 suite (do not hard-code www-data).
fpm_run_as() {
  if [[ "$(id -u)" == "0" ]]; then
    if id -u www-data >/dev/null 2>&1; then
      echo "www-data"
    elif id -u nginx >/dev/null 2>&1; then
      echo "nginx"
    else
      echo "RESULT=BLOCKED missing=unprivileged-php-fpm-user"
      exit 3
    fi
  else
    id -un
  fi
}
fpm_run_group() {
  if [[ "$(id -u)" == "0" ]]; then
    local u
    u="$(fpm_run_as)"
    id -gn "$u"
  else
    id -gn
  fi
}

FPM_USER="$(fpm_run_as)"
FPM_GROUP="$(fpm_run_group)"
echo "PHP_FPM_POOL_USER=$FPM_USER PHP_FPM_POOL_GROUP=$FPM_GROUP"

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d /tmp/exyonq-cg-wp.XXXXXX)"
DOCROOT="$TMP/www"
mkdir -p "$DOCROOT/wp-admin"
chmod -R a+rX "$TMP"

# Real PHP application (front controller + wp-admin script).
cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=UTF-8');
$path = $_SERVER['REQUEST_URI'] ?? '/';
$path = parse_url($path, PHP_URL_PATH) ?: '/';
echo "WP_FRONT:" . $path;
PHP
cat >"$DOCROOT/wp-admin/index.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=UTF-8');
echo "WP_ADMIN_OK";
PHP

SOCK="/tmp/exyonq-cg-fpm-$$.sock"
rm -f "$SOCK"
CTRL_SOCK="/tmp/exyonq-cg-wp-ctrl-$$.sock"
rm -f "$CTRL_SOCK"
FPM_CFG="$TMP/php-fpm.conf"
FPM_LOG="$TMP/php-fpm.log"
cat >"$FPM_CFG" <<EOF
[global]
error_log = $FPM_LOG
daemonize = no

[www]
user = $FPM_USER
group = $FPM_GROUP
listen = $SOCK
listen.owner = $FPM_USER
listen.group = $FPM_GROUP
listen.mode = 0666
pm = static
pm.max_children = 2
clear_env = no
EOF

# Real PHP-FPM config validation before process start (host-available flag).
set +e
"$PHP_FPM_BIN" -t -y "$FPM_CFG" >"$TMP/fpm-test.stdout" 2>"$TMP/fpm-test.stderr"
FPM_TEST_EC=$?
set -e
if [[ "$FPM_TEST_EC" -ne 0 ]]; then
  echo "FAIL: php-fpm config preflight (-t) exit=$FPM_TEST_EC"
  cat "$TMP/fpm-test.stderr" || true
  exit 1
fi
e2e_record_pass "php-fpm pool config preflight (-t)"

"$PHP_FPM_BIN" --nodaemonize --fpm-config "$FPM_CFG" >"$TMP/fpm.stdout" 2>"$TMP/fpm.stderr" &
FPM_PID=$!

CFG="$TMP/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["wp-front", "wp-admin"]

[[route]]
name = "wp-front"
match = { path = "/index.php" }
fastcgi = "php"

[[route]]
name = "wp-admin"
match = { path = "/wp-admin/index.php" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
# Address is a filesystem path (not unix: prefix); match Plan 08 suite.
address = "${SOCK}"
document_root = "${DOCROOT}"
max_concurrency = 4
EOF

SRV_LOG="$TMP/exyonq.log"
PID=""
cleanup() {
  [[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -f "$SOCK" "$CTRL_SOCK"
  rm -rf "$TMP"
}
trap cleanup EXIT

# Wait for FPM socket
for _ in $(seq 1 40); do
  [[ -S "$SOCK" ]] && break
  sleep 0.1
done
[[ -S "$SOCK" ]] || {
  echo "FAIL: php-fpm socket missing"
  cat "$TMP/fpm.stderr" || true
  exit 1
}
e2e_record_pass "php-fpm started with WIP pool socket"

EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
  "$EXYONQ_BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
PID=$!

READY=0
for _ in $(seq 1 80); do
  if curl -sf -o /dev/null "http://127.0.0.1:${PORT}/index.php"; then READY=1; break; fi
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "FAIL: exyonq exited"
    cat "$SRV_LOG" || true
    exit 1
  fi
  sleep 0.25
done
[[ "$READY" -eq 1 ]] || {
  echo "FAIL: readiness timeout"
  cat "$SRV_LOG" || true
  exit 1
}

FRONT="$(curl -sS "http://127.0.0.1:${PORT}/index.php")"
[[ "$FRONT" == WP_FRONT:/* ]] || {
  echo "FAIL: front controller body: $FRONT"
  exit 1
}
e2e_record_pass "wordpress-profile front controller PHP"

ADMIN="$(curl -sS "http://127.0.0.1:${PORT}/wp-admin/index.php")"
[[ "$ADMIN" == "WP_ADMIN_OK" ]] || {
  echo "FAIL: wp-admin body: $ADMIN"
  exit 1
}
e2e_record_pass "wordpress-profile wp-admin PHP"

e2e_finish "03b-wordpress-profile"
