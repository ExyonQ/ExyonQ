#!/usr/bin/env bash
# Plan 11 tranche D — Linux real PHP-FPM + htaccess overlay e2e (DirectoryIndex + front controller).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s)"
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
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
  echo "RESULT=BLOCKED missing=php-fpm"
  exit 3
fi

EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$EXYONQ_BIN" ]]; then
  echo "Building exyonq (debug)..."
  cargo build -p exyonq --bin exyonq
fi

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

EXYONQ_HTACCESS_E2E_PORT="${EXYONQ_HTACCESS_E2E_PORT:-$(pick_port)}"
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
  elif [[ "$KEEP_TMP" == "1" && -n "${TMP:-}" ]]; then
    echo "KEEP_TMP=1 artifacts at $TMP"
  fi
  if [[ $code -ne 0 ]]; then
    echo "--- e2e failed; logs ---"
    [[ -n "${FPM_LOG:-}" && -f "$FPM_LOG" ]] && tail -n 80 "$FPM_LOG" || true
    [[ -n "${EXYONQ_LOG:-}" && -f "$EXYONQ_LOG" ]] && tail -n 80 "$EXYONQ_LOG" || true
  fi
}
trap cleanup EXIT INT TERM

wait_for_file() {
  local path="$1"
  local timeout_ms="$2"
  local elapsed=0
  local step=50
  while [[ $elapsed -lt $timeout_ms ]]; do
    if [[ -e "$path" ]]; then
      return 0
    fi
    if [[ -n "${FPM_PID:-}" ]] && ! kill -0 "$FPM_PID" 2>/dev/null; then
      record_fail "php-fpm exited before $path appeared"
    fi
    sleep 0.05
    elapsed=$((elapsed + step))
  done
  record_fail "timed out waiting for $path"
}

wait_for_http_code() {
  local url="$1"
  local expected="$2"
  local timeout_ms="${3:-10000}"
  local elapsed=0
  local step=75
  while [[ $elapsed -lt $timeout_ms ]]; do
    local code
    code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 "$url" 2>/dev/null || echo "000")"
    if [[ "$code" == "$expected" ]]; then
      return 0
    fi
    if [[ -n "${EXYONQ_PID:-}" ]] && ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      record_fail "exyonq exited while waiting for HTTP $expected on $url (got $code)"
    fi
    sleep 0.075
    elapsed=$((elapsed + step))
  done
  record_fail "timed out waiting for HTTP $expected on $url (last=$code)"
}

poll_json_field() {
  local url="$1"
  local field="$2"
  local expected="$3"
  local timeout_ms="${4:-10000}"
  local elapsed=0
  local step=75
  while [[ $elapsed -lt $timeout_ms ]]; do
    local body
    body="$(curl -fsS --max-time 2 "$url" 2>/dev/null || true)"
    if [[ -n "$body" ]]; then
      local got
      got="$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('$field',''))" <<<"$body" 2>/dev/null || true)"
      if [[ "$got" == "$expected" ]]; then
        echo "$body"
        return 0
      fi
    fi
    sleep 0.075
    elapsed=$((elapsed + step))
  done
  record_fail "poll_json_field $field=$expected on $url (last body: ${body:-empty})"
}

poll_redirect_location() {
  local url="$1"
  local expected_code="$2"
  local expected_location="$3"
  local timeout_ms="${4:-10000}"
  local elapsed=0
  local step=75
  while [[ $elapsed -lt $timeout_ms ]]; do
    local headers code location
    headers="$(curl -sS -D - -o /dev/null --max-time 2 "$url" 2>/dev/null || true)"
    code="$(echo "$headers" | head -n1 | awk '{print $2}')"
    location="$(echo "$headers" | tr -d '\r' | awk -F': ' 'tolower($1)=="location"{print $2; exit}')"
    if [[ "$code" == "$expected_code" && "$location" == "$expected_location" ]]; then
      return 0
    fi
    sleep 0.075
    elapsed=$((elapsed + step))
  done
  record_fail "poll_redirect $url expected $expected_code Location:$expected_location (last code=$code loc=$location)"
}

http_code_and_body() {
  local url="$1"
  local body_file="$2"
  curl -sS -o "$body_file" -w '%{http_code}' --max-time 10 "$url"
}

write_base_htaccess() {
  cat >"$DOCROOT/.htaccess" <<'HTACCESS'
DirectoryIndex index.php index.html

<IfModule mod_rewrite.c>
RewriteEngine On
RewriteBase /
RewriteRule ^index\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
</IfModule>
HTACCESS
}

write_exyonq_config() {
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:$EXYONQ_HTACCESS_E2E_PORT"
routes = ["php-site"]

[[route]]
name = "php-site"
match = { path = "/" }
fastcgi = "php"
htaccess = "overlay"

[[fcgi_pool]]
name = "php"
address = "$SOCKET_PATH"
document_root = "$DOCROOT"
max_concurrency = 4
EOF
}

write_fixture() {
  mkdir -p "$DOCROOT/assets"
  write_base_htaccess

  cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: application/json');
echo json_encode([
    'script_name' => $_SERVER['SCRIPT_NAME'] ?? '',
    'script_filename' => $_SERVER['SCRIPT_FILENAME'] ?? '',
    'request_uri' => $_SERVER['REQUEST_URI'] ?? '',
    'query_string' => $_SERVER['QUERY_STRING'] ?? '',
    'request_method' => $_SERVER['REQUEST_METHOD'] ?? '',
], JSON_UNESCAPED_SLASHES);
PHP

  echo 'plain-existing-file' >"$DOCROOT/existing.txt"

  echo 'asset-index' >"$DOCROOT/assets/index.html"
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
pm.max_children = 4
clear_env = no
EOF
  chmod -R a+rX "$DOCROOT"
  rm -f "$SOCKET_PATH"
  "$PHP_FPM_BIN" --nodaemonize --fpm-config "$FPM_CFG" >>"$FPM_LOG" 2>&1 &
  FPM_PID=$!
  wait_for_file "$SOCKET_PATH" 15000
}

start_exyonq() {
  : >"$EXYONQ_LOG"
  unset EXYONQ_FCGI_DOCUMENT_ROOT || true
  "$EXYONQ_BIN" serve --config "$CFG" >>"$EXYONQ_LOG" 2>&1 &
  EXYONQ_PID=$!
  BASE="http://127.0.0.1:$EXYONQ_HTACCESS_E2E_PORT"
  wait_for_http_code "$BASE/" 200 15000
}

init_stack() {
  stop_exyonq
  stop_fpm
  TMP="$(mktemp -d)"
  chmod a+rwX "$TMP"
  DOCROOT="$TMP/www"
  SOCKET_PATH="$TMP/php-fpm.sock"
  CFG="$TMP/exyonq.toml"
  FPM_CFG="$TMP/php-fpm.conf"
  FPM_LOG="$TMP/php-fpm.log"
  EXYONQ_LOG="$TMP/exyonq.log"
  mkdir -p "$DOCROOT"
  write_fixture
  write_exyonq_config
  start_fpm
  start_exyonq
}

append_redirect() {
  local status="$1"
  local from="$2"
  local to="$3"
  printf '\nRedirect %s %s %s\n' "$status" "$from" "$to" >>"$DOCROOT/.htaccess"
}

replace_redirect() {
  local status="$1"
  local from="$2"
  local to="$3"
  write_base_htaccess
  append_redirect "$status" "$from" "$to"
}

case_directory_index_root() {
  echo "=== CASE A: DirectoryIndex GET / ==="
  local body
  body="$(poll_json_field "$BASE/" script_name "/index.php" 10000)"
  python3 -c "import json,sys; d=json.load(sys.stdin); assert d['request_uri']=='/', d" <<<"$body" \
    || record_fail "A request_uri expected / got $body"
  [[ "$body" == *"index.php"* ]] || record_fail "A script_filename missing index.php"
  record_pass "DirectoryIndex GET / -> index.php"
}

case_front_controller() {
  echo "=== CASE B: front controller GET /posts/hello?page=2 ==="
  local body
  body="$(poll_json_field "$BASE/posts/hello?page=2" script_name "/index.php" 10000)"
  python3 -c "import json,sys; d=json.load(sys.stdin);
assert d['request_uri']=='/posts/hello?page=2', d
assert d['query_string']=='page=2', d" <<<"$body" || record_fail "B uri/query mismatch: $body"
  record_pass "front controller preserves REQUEST_URI and QUERY_STRING"
}

case_existing_file_bypass() {
  echo "=== CASE C: existing file GET /existing.txt ==="
  local code body
  code="$(http_code_and_body "$BASE/existing.txt" "$TMP/c-body.txt")"
  body="$(cat "$TMP/c-body.txt")"
  if python3 -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get('script_name')=='/index.php' else 1)" <<<"$body" 2>/dev/null; then
    record_fail "C front controller incorrectly routed to index.php: $body"
  fi
  echo "CASE C actual: HTTP $code body=${body:0:120}"
  record_pass "existing file bypasses front controller (HTTP $code, not index.php JSON)"
}

case_existing_directory() {
  echo "=== CASE D: directory GET /assets/ ==="
  local code body
  code="$(http_code_and_body "$BASE/assets/" "$TMP/d-body.txt")"
  body="$(cat "$TMP/d-body.txt")"
  [[ "$code" == "200" ]] || record_fail "D expected 200 got $code"
  [[ "$body" == "asset-index" ]] || record_fail "D body got $body"
  record_pass "directory DirectoryIndex static fallback assets/index.html"
}

case_redirect_reload() {
  echo "=== CASE E: redirect overlay 302 ==="
  append_redirect 302 /legacy /new-location
  poll_redirect_location "$BASE/legacy" 302 /new-location 10000
  record_pass "redirect 302 after watcher reload"

  echo "=== CASE F: redirect generational change 301 ==="
  replace_redirect 301 /legacy /final-location
  poll_redirect_location "$BASE/legacy" 301 /final-location 10000
  record_pass "redirect 301 after generational reload"
}

case_invalid_preserve() {
  echo "=== CASE G: invalid htaccess preserves prior generation ==="
  cp "$DOCROOT/.htaccess" "$TMP/htaccess-good-backup"
  cat >>"$DOCROOT/.htaccess" <<'INVALID'

Redirect 999 /legacy /broken
INVALID
  poll_redirect_location "$BASE/legacy" 301 /final-location 10000 \
    || record_fail "G prior redirect lost after invalid compile"
  cp "$TMP/htaccess-good-backup" "$DOCROOT/.htaccess"
  wait_for_http_code "$BASE/" 200 10000
  record_pass "invalid htaccess compile failure preserves prior overlay"
}

echo "=== plan11-htaccess-front-controller-e2e ==="
echo "host=$HOST_LABEL arch=$ARCH_LABEL php-fpm=$PHP_FPM_BIN port=$EXYONQ_HTACCESS_E2E_PORT"

init_stack
case_directory_index_root
case_front_controller
case_existing_file_bypass
case_existing_directory
case_redirect_reload
case_invalid_preserve

echo "=== SUMMARY pass=$PASS fail=$FAIL host=$HOST_LABEL arch=$ARCH_LABEL ==="
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
echo "PASS: plan11-htaccess-front-controller-e2e complete"
exit 0
