#!/usr/bin/env bash
# F6-WP-02: real FastCGI saturation envelope (no nginx, no mock upstream).
# 20 simultaneous slow PHP-FPM requests:
#   max_concurrency=16 -> 16x200 + 4x503, one actionable concurrency log per 503
#   max_concurrency=40 -> 20x200, no saturation log
set -euo pipefail

if [[ "$(uname -s)" != "Linux" && "${ALLOW_LOCAL_E2E:-0}" != "1" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s) LOCAL_ITERATION_ONLY"
  exit 2
fi

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

pick_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

PORT16="$(pick_port)"
PORT40="$(pick_port)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-f6wp02.XXXXXX")"
DOCROOT="$TMP/www"
SOCK="$TMP/php-fpm.sock"
FPM_CONF="$TMP/php-fpm.conf"
mkdir -p "$DOCROOT"
chmod 755 "$TMP" "$DOCROOT"

cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo 'PHP_OK';
PHP
cat >"$DOCROOT/slow.php" <<'PHP'
<?php
usleep(2_000_000);
header('Content-Type: text/plain');
echo 'SLOW_OK';
PHP
chmod 644 "$DOCROOT/index.php" "$DOCROOT/slow.php"

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
pm.max_children = 20
clear_env = no
EOF

write_cfg() {
  local port="$1" capacity="$2" path="$3"
  cat >"$path" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${port}"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"

[modules.metrics]
enabled = true
path = "/metrics"
health_path = "/exyonq-metrics-health"

[[fcgi_pool]]
name = "php"
address = "${SOCK}"
document_root = "${DOCROOT}"
max_concurrency = ${capacity}
max_connections = ${capacity}
total_timeout_ms = 10000
checkout_timeout_ms = 100
EOF
}
write_cfg "$PORT16" 16 "$TMP/ex16.toml"
write_cfg "$PORT40" 40 "$TMP/ex40.toml"

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

start_ex() {
  local port="$1" cfg="$2" log="$3"
  EXYONQ_CONTROL_SOCKET="$TMP/control-${port}.sock" RUST_LOG=info "$BIN" serve --config "$cfg" >"$log" 2>&1 &
  EX_PID=$!
  local ready=0
  for _ in $(seq 1 100); do
    if [[ "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/exyonq-metrics-health" 2>/dev/null || true)" == "200" ]]; then
      ready=1
      break
    fi
    if ! kill -0 "$EX_PID" 2>/dev/null; then
      echo "FAIL: exyonq exited during startup on port ${port}"
      cat "$log" || true
      exit 1
    fi
    sleep 0.1
  done
  if [[ "$ready" -ne 1 ]]; then
    echo "FAIL: exyonq readiness timeout on port ${port}"
    cat "$log" || true
    exit 1
  fi
}

stop_ex() {
  if [[ -n "${EX_PID:-}" ]]; then
    kill "$EX_PID" 2>/dev/null || true
    wait "$EX_PID" 2>/dev/null || true
    EX_PID=""
  fi
}

run_batch() {
  local port="$1"
  python3 - "$port" <<'PY'
import http.client
import json
import sys
from concurrent.futures import ThreadPoolExecutor

port = int(sys.argv[1])

def one(_):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
    try:
        conn.request("GET", "/slow.php")
        response = conn.getresponse()
        response.read()
        return response.status
    finally:
        conn.close()

with ThreadPoolExecutor(max_workers=20) as pool:
    codes = list(pool.map(one, range(20)))
print(json.dumps({str(code): codes.count(code) for code in sorted(set(codes))}, sort_keys=True))
PY
}

assert_counts() {
  local counts="$1" expected_200="$2" expected_503="$3"
  python3 - "$counts" "$expected_200" "$expected_503" <<'PY'
import json
import sys

counts = json.loads(sys.argv[1])
expected_200 = int(sys.argv[2])
expected_503 = int(sys.argv[3])
assert counts.get("200", 0) == expected_200, counts
assert counts.get("503", 0) == expected_503, counts
PY
}

assert_metrics() {
  local port="$1" pool="$2"
  local metrics
  metrics="$(curl -sS "http://127.0.0.1:${port}/metrics")"
  printf '%s\n' "$metrics" | grep -Fq "exyonq_fcgi_pool_responses_total{pool=\"${pool}\",status=\"200\"} ${3}"
  printf '%s\n' "$metrics" | grep -Fq "exyonq_fcgi_pool_responses_total{pool=\"${pool}\",status=\"503\"} ${4}"
  printf '%s\n' "$metrics" | grep -Fq "exyonq_fcgi_pool_saturation_rejections_total{pool=\"${pool}\"} ${5}"
  printf '%s\n' "$metrics" | grep -Fq "exyonq_fcgi_pool_inflight_current{pool=\"${pool}\"} 0"
}

start_ex "$PORT16" "$TMP/ex16.toml" "$TMP/ex16.log"
COUNTS16="$(run_batch "$PORT16")"
assert_counts "$COUNTS16" 16 4
SAT_LINES="$(grep -c 'timeout_kind=concurrency' "$TMP/ex16.log" || true)"
if [[ "$SAT_LINES" -lt 4 ]]; then
  echo "FAIL: expected >=4 actionable concurrency logs, got ${SAT_LINES}"
  cat "$TMP/ex16.log" || true
  exit 1
fi
grep -q 'queue=disabled' "$TMP/ex16.log"
assert_metrics "$PORT16" php 16 4 4
echo "PASS: max_concurrency=16 -> ${COUNTS16}; ${SAT_LINES} actionable 503 logs; per-pool metrics OK"
stop_ex

start_ex "$PORT40" "$TMP/ex40.toml" "$TMP/ex40.log"
COUNTS40="$(run_batch "$PORT40")"
assert_counts "$COUNTS40" 20 0
if grep -q 'timeout_kind=concurrency' "$TMP/ex40.log"; then
  echo "FAIL: unexpected saturation log at max_concurrency=40"
  cat "$TMP/ex40.log" || true
  exit 1
fi
assert_metrics "$PORT40" php 20 0 0
echo "PASS: max_concurrency=40 -> ${COUNTS40}; no saturation log; per-pool metrics OK"
stop_ex

echo "RESULT=PASS f6-wp02 bounded fail-fast envelope + real PHP-FPM"
