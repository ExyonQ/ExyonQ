#!/usr/bin/env bash
# NON_AUTHORITATIVE_DIAGNOSTIC_ONLY / NOT_GATE_CLOSING under EXYONQ_NO_SMOKE_POLICY.
# Depth class (audit 2026-08-02) recorded in .exyonq-local/tmp/no-smoke-audit-20260802/REPORT.md.
# Do not use this script alone to close capability / release / benchmark admission.
# Plan 08 PR5-B operational closure — Linux real PHP-FPM smoke (not CI, not benchmark).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: Linux only (got $(uname -s))"
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

PHP_FPM_BIN="${PHP_FPM_BIN:-}"
if [[ -z "$PHP_FPM_BIN" ]]; then
  for candidate in php-fpm php-fpm8.3 php-fpm8.2 php-fpm8.1; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PHP_FPM_BIN="$(command -v "$candidate")"
      break
    fi
  done
fi

if [[ -z "$PHP_FPM_BIN" || ! -x "$PHP_FPM_BIN" ]]; then
  echo "SKIP: php-fpm not installed"
  exit 0
fi

EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$EXYONQ_BIN" ]]; then
  echo "Building exyonq (debug)..."
  cargo build -p exyonq --bin exyonq
fi

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

EXYONQ_FCGI_SMOKE_PORT="${EXYONQ_FCGI_SMOKE_PORT:-$(pick_port)}"
KEEP_TMP="${KEEP_TMP:-0}"

HOST_LABEL="$(uname -n 2>/dev/null || hostname)"
ARCH_LABEL="$(uname -m)"

TMP=""
FPM_PID=""
EXYONQ_PID=""
DOCROOT=""
SOCKET_PATH=""
CFG=""
FPM_CFG=""
FPM_LOG=""
EXYONQ_LOG=""
BASE=""

PASS=0
FAIL=0

record_pass() {
  echo "PASS  $1"
  PASS=$((PASS + 1))
}

record_fail() {
  echo "FAIL  $1"
  FAIL=$((FAIL + 1))
  return 1
}

stop_exyonq() {
  if [[ -n "${EXYONQ_PID:-}" ]] && kill -0 "$EXYONQ_PID" 2>/dev/null; then
    kill "$EXYONQ_PID" 2>/dev/null || true
    wait "$EXYONQ_PID" 2>/dev/null || true
  fi
  EXYONQ_PID=""
}

stop_fpm() {
  if [[ -n "${FPM_PID:-}" ]] && kill -0 "$FPM_PID" 2>/dev/null; then
    kill "$FPM_PID" 2>/dev/null || true
    wait "$FPM_PID" 2>/dev/null || true
  fi
  FPM_PID=""
}

cleanup() {
  local code=$?
  stop_exyonq
  stop_fpm
  if [[ "$KEEP_TMP" != "1" && -n "${TMP:-}" && -d "$TMP" ]]; then
    rm -rf "$TMP"
  fi
  if [[ $code -ne 0 ]]; then
    echo "--- smoke failed; logs ---"
    [[ -n "${FPM_LOG:-}" && -f "$FPM_LOG" ]] && tail -n 80 "$FPM_LOG" || true
    [[ -n "${EXYONQ_LOG:-}" && -f "$EXYONQ_LOG" ]] && tail -n 80 "$EXYONQ_LOG" || true
  fi
}
trap cleanup EXIT INT TERM

wait_for_file() {
  local path="$1"
  local timeout_sec="$2"
  local i=0
  while [[ $i -lt $timeout_sec ]]; do
    if [[ -e "$path" ]]; then
      return 0
    fi
    if [[ -n "${FPM_PID:-}" ]] && ! kill -0 "$FPM_PID" 2>/dev/null; then
      record_fail "php-fpm exited before $path appeared"
    fi
    sleep 0.2
    i=$((i + 1))
  done
  record_fail "timed out waiting for $path"
}

wait_for_http() {
  local url="$1"
  local timeout_sec="$2"
  local i=0
  while [[ $i -lt $timeout_sec ]]; do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${EXYONQ_PID:-}" ]] && ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      record_fail "exyonq exited before HTTP ready ($url)"
    fi
    sleep 0.2
    i=$((i + 1))
  done
  record_fail "timed out waiting for HTTP $url"
}

http_code() {
  curl -sS -o "$2" -w '%{http_code}' --max-time "${3:-10}" "$1"
}

write_exyonq_config() {
  local max_concurrency="${1:-16}"
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:$EXYONQ_FCGI_SMOKE_PORT"
routes = ["php", "php-query", "php-hold", "php-slow"]

[[route]]
name = "php"
match = { path = "/index.php" }
fastcgi = "php"

[[route]]
name = "php-query"
match = { path = "/query.php" }
fastcgi = "php"

[[route]]
name = "php-hold"
match = { path = "/hold.php" }
fastcgi = "php"

[[route]]
name = "php-slow"
match = { path = "/slow.php" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "$SOCKET_PATH"
max_concurrency = $max_concurrency
EOF
}

fpm_run_as() {
  if [[ "$(id -u)" == "0" ]]; then
    echo "www-data"
  else
    id -un
  fi
}

fpm_run_group() {
  if [[ "$(id -u)" == "0" ]]; then
    echo "www-data"
  else
    id -gn
  fi
}

start_fpm() {
  local max_children="${1:-2}"
  local fpm_user fpm_group
  fpm_user="$(fpm_run_as)"
  fpm_group="$(fpm_run_group)"
  cat >"$FPM_CFG" <<EOF
[global]
error_log = $FPM_LOG
daemonize = no

[www]
user = $fpm_user
group = $fpm_group
listen = $SOCKET_PATH
listen.owner = $fpm_user
listen.group = $fpm_group
listen.mode = 0600
pm = static
pm.max_children = $max_children
clear_env = no
EOF
  chmod -R a+rX "$DOCROOT"
  rm -f "$SOCKET_PATH"
  "$PHP_FPM_BIN" --nodaemonize --fpm-config "$FPM_CFG" >>"$FPM_LOG" 2>&1 &
  FPM_PID=$!
  wait_for_file "$SOCKET_PATH" 30
}

start_exyonq() {
  : >"$EXYONQ_LOG"
  export EXYONQ_FCGI_DOCUMENT_ROOT="$DOCROOT"
  "$EXYONQ_BIN" serve --config "$CFG" >>"$EXYONQ_LOG" 2>&1 &
  EXYONQ_PID=$!
  BASE="http://127.0.0.1:$EXYONQ_FCGI_SMOKE_PORT"
  wait_for_http "$BASE/index.php" 30
}

init_fixture() {
  local tag="$1"
  stop_exyonq
  stop_fpm
  TMP="$(mktemp -d)"
  chmod a+rwX "$TMP"
  DOCROOT="$TMP/www"
  SOCKET_PATH="$TMP/php-fpm.sock"
  CFG="$TMP/exyonq-${tag}.toml"
  FPM_CFG="$TMP/php-fpm-${tag}.conf"
  FPM_LOG="$TMP/php-fpm-${tag}.log"
  EXYONQ_LOG="$TMP/exyonq-${tag}.log"
  mkdir -p "$DOCROOT"
}

case_200_success() {
  echo "=== CASE 200 ==="
  init_fixture "200"

  cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'exyonq-php-fpm-real';
PHP

  cat >"$DOCROOT/query.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo $_GET['name'] ?? 'missing';
PHP

  write_exyonq_config 16
  start_fpm 2
  start_exyonq

  local headers body status ct resp
  headers="$(curl -fsS -D - "$BASE/index.php" -o "$TMP/body-200.txt")"
  status="$(echo "$headers" | head -n1 | awk '{print $2}')"
  ct="$(echo "$headers" | tr -d '\r' | awk -F': ' 'tolower($1)=="content-type"{print $2; exit}')"
  resp="$(cat "$TMP/body-200.txt")"

  [[ "$status" == "200" ]] || record_fail "200 status got $status"
  [[ "$resp" == "exyonq-php-fpm-real" ]] || record_fail "200 body got $resp"
  [[ "$ct" == *"text/plain"* ]] || record_fail "200 content-type got $ct"

  resp="$(curl -fsS "$BASE/query.php?name=exyonq")"
  [[ "$resp" == "exyonq" ]] || record_fail "query string got $resp"

  record_pass "200 success + query string"
}

case_502_backend_down() {
  echo "=== CASE 502 ==="
  init_fixture "502"

  cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'ok';
PHP

  write_exyonq_config 16
  start_fpm 2
  start_exyonq

  local code
  code="$(http_code "$BASE/index.php" "$TMP/pre-502.txt" 5)"
  [[ "$code" == "200" ]] || record_fail "502 fixture pre-check expected 200 got $code"

  stop_fpm
  sleep 0.3
  code="$(http_code "$BASE/index.php" "$TMP/body-502.txt" 5)"
  [[ "$code" == "502" ]] || record_fail "502 backend down expected 502 got $code"

  record_pass "502 socket down (not 503)"
}

case_503_saturation() {
  echo "=== CASE 503 ==="
  init_fixture "503"

  local started="$TMP/hold-started"
  local release="$TMP/hold-release"
  rm -f "$started" "$release"

  cat >"$DOCROOT/hold.php" <<PHP
<?php
file_put_contents('$started', '1');
while (!file_exists('$release')) {
    usleep(10000);
}
header('Content-Type: text/plain');
echo 'released';
PHP

  cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'ok';
PHP

  write_exyonq_config 1
  start_fpm 1
  start_exyonq

  local code_a code_b code_c t0 t1 elapsed
  : >"$TMP/body-a.txt"
  curl -sS -o "$TMP/body-a.txt" --max-time 30 "$BASE/hold.php" &
  local pid_a=$!

  local i=0
  while [[ $i -lt 100 ]]; do
    if [[ -f "$started" ]]; then
      break
    fi
    sleep 0.05
    i=$((i + 1))
  done
  [[ -f "$started" ]] || record_fail "503 hold script never started"

  t0="$(date +%s%N)"
  code_b="$(http_code "$BASE/index.php" "$TMP/body-b.txt" 5)"
  t1="$(date +%s%N)"
  elapsed=$(( (t1 - t0) / 1000000 ))
  [[ "$code_b" == "503" ]] || record_fail "503 saturation expected 503 got $code_b"
  [[ "$elapsed" -lt 3000 ]] || record_fail "503 saturation too slow (${elapsed}ms)"

  touch "$release"
  wait "$pid_a" || true
  local body_a
  body_a="$(cat "$TMP/body-a.txt" 2>/dev/null || true)"
  [[ "$body_a" == "released" ]] || record_fail "503 request A expected released got '$body_a'"

  code_c="$(http_code "$BASE/index.php" "$TMP/body-c.txt" 5)"
  [[ "$code_c" == "200" ]] || record_fail "503 recovery C expected 200 got $code_c"

  record_pass "503 saturation A=released B=503 C=200 (B ${elapsed}ms)"
}

case_504_timeout_recovery() {
  echo "=== CASE 504 ==="
  init_fixture "504"

  cat >"$DOCROOT/slow.php" <<'PHP'
<?php
usleep(2000000);
header('Content-Type: text/plain');
echo 'slow-done';
PHP

  cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'recovered';
PHP

  write_exyonq_config 1
  start_fpm 1
  export EXYONQ_FCGI_DELEGATE_TIMEOUT_MS=800
  start_exyonq

  local code_slow code_recover
  code_slow="$(http_code "$BASE/slow.php" "$TMP/body-504.txt" 5)"
  [[ "$code_slow" == "504" ]] || record_fail "504 slow path expected 504 got $code_slow"

  sleep 2.5
  code_recover="$(http_code "$BASE/index.php" "$TMP/body-recover.txt" 5)"
  [[ "$code_recover" == "200" ]] || record_fail "504 recovery expected 200 got $code_recover"
  local body_recover
  body_recover="$(cat "$TMP/body-recover.txt")"
  [[ "$body_recover" == "recovered" ]] || record_fail "504 recovery body got $body_recover"

  unset EXYONQ_FCGI_DELEGATE_TIMEOUT_MS
  record_pass "504 delegate timeout + recovery 200"
}

echo "=== plan08-fcgi-real-smoke PR5-B closure ==="
echo "host=$HOST_LABEL arch=$ARCH_LABEL php-fpm=$PHP_FPM_BIN"

case_200_success
case_502_backend_down
case_503_saturation
case_504_timeout_recovery

echo "=== SUMMARY pass=$PASS fail=$FAIL host=$HOST_LABEL arch=$ARCH_LABEL ==="
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
echo "PASS: plan08-fcgi-real-smoke PR5-B complete"
exit 0
