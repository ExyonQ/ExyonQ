#!/usr/bin/env bash
# AUTHORITATIVE FastCGI + real PHP-FPM E2E (includes POST).
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

ROOT="$E2E_ROOT"
set +e
bash "$ROOT/scripts/e2e/suites/fastcgi-php-fpm-suite.sh"
PLAN08_EC=$?
set -e
if [[ "$PLAN08_EC" -eq 2 ]]; then
  echo "RESULT=NOT_APPLICABLE from fastcgi-php-fpm-suite"
  exit 2
fi
if [[ "$PLAN08_EC" -eq 3 ]]; then
  echo "RESULT=BLOCKED from fastcgi-php-fpm-suite"
  exit 3
fi
if [[ "$PLAN08_EC" -ne 0 ]]; then
  echo "FAIL: fastcgi-php-fpm-suite exit=$PLAN08_EC"
  exit "$PLAN08_EC"
fi
e2e_record_pass "fastcgi-php-fpm-suite (GET/query/502/503/504)"

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d)"
chmod a+rwX "$TMP"
DOCROOT="$TMP/www"
SOCK="$TMP/php-fpm.sock"
mkdir -p "$DOCROOT"
cat >"$DOCROOT/post.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'post-body=' . file_get_contents('php://input');
PHP
chmod -R a+rX "$DOCROOT"

if [[ "$(id -u)" == "0" ]]; then
  RUN_USER=www-data
  RUN_GROUP=www-data
else
  RUN_USER="$(id -un)"
  RUN_GROUP="$(id -gn)"
fi

cat >"$TMP/php-fpm.conf" <<EOF
[global]
error_log = $TMP/fpm.log
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

cat >"$TMP/exyonq.toml" <<EOF
config_version = 1
[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["php"]
[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"
[[fcgi_pool]]
name = "php"
address = "$SOCK"
document_root = "$DOCROOT"
max_concurrency = 4
EOF

FPM_PID=""; EX_PID=""
cleanup() {
  [[ -n "${EX_PID:-}" ]] && kill "$EX_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

rm -f "$SOCK"
"$PHP_FPM_BIN" --nodaemonize --fpm-config "$TMP/php-fpm.conf" >"$TMP/fpm.out" 2>&1 &
FPM_PID=$!
for _ in $(seq 1 50); do [[ -S "$SOCK" ]] && break; sleep 0.1; done
if [[ ! -S "$SOCK" ]]; then
  e2e_record_fail "php-fpm socket missing"
  cat "$TMP/fpm.out" "$TMP/fpm.log" 2>/dev/null || true
  e2e_finish "fastcgi-php-fpm-e2e"
  exit 1
fi

RUST_LOG=info "$EXYONQ_BIN" serve --config "$TMP/exyonq.toml" >"$TMP/ex.log" 2>&1 &
EX_PID=$!
for _ in $(seq 1 80); do
  if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('$PORT'))); s.close()" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$EX_PID" 2>/dev/null; then
    e2e_record_fail "exyonq exited during POST fixture start"
    cat "$TMP/ex.log" >&2 || true
    e2e_finish "fastcgi-php-fpm-e2e"
    exit 1
  fi
  sleep 0.1
done

CODE="$(curl -sS -o "$TMP/post.out" -w '%{http_code}' -X POST \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data 'alpha=1&beta=two' "http://127.0.0.1:${PORT}/post.php" || true)"
RESP="$(cat "$TMP/post.out" 2>/dev/null || true)"
if [[ "$CODE" == "200" && "$RESP" == "post-body=alpha=1&beta=two" ]]; then
  e2e_record_pass "POST body forwarded to real PHP-FPM"
else
  e2e_record_fail "POST code=$CODE body='$RESP'"
  cat "$TMP/ex.log" >&2 || true
  cat "$TMP/fpm.out" "$TMP/fpm.log" 2>/dev/null || true
fi

# Missing script contract: must not succeed as 200 with empty/wrong body.
MISS="$(curl -sS -o "$TMP/miss.out" -w '%{http_code}' \
  "http://127.0.0.1:${PORT}/definitely-missing-${RANDOM}.php" || true)"
if [[ "$MISS" =~ ^(404|403|502)$ ]]; then
  e2e_record_pass "missing script → $MISS (not 200)"
else
  e2e_record_fail "missing script unexpected code=$MISS body='$(head -c 80 "$TMP/miss.out" 2>/dev/null || true)'"
fi

e2e_finish "fastcgi-php-fpm-e2e"
