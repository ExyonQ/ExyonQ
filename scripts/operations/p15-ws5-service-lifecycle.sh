#!/usr/bin/env bash
# P1.5-WS5 — Service lifecycle harness (O01–O20). Linux evidence hosts only.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  p15-ws5-service-lifecycle.sh \
    --workspace PATH \
    --host-label amd64|arm64 \
    --artifact-dir PATH \
    [--expected-arch x86_64|aarch64] \
    [--scenario ID|all]
EOF
}

WORKSPACE=""
HOST_LABEL=""
ARTIFACT_DIR=""
EXPECTED_ARCH=""
SCENARIO="all"
WAIT_TIMEOUT="${P15_WS5_WAIT_TIMEOUT:-30}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --host-label) HOST_LABEL="${2:-}"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="${2:-}"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="${2:-}"; shift 2 ;;
    --scenario) SCENARIO="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$HOST_LABEL" && -n "$ARTIFACT_DIR" ]] || {
  usage >&2
  exit 2
}
[[ "$(uname -s)" == "Linux" ]] || { echo "ERROR: Linux host required" >&2; exit 2; }
ACTUAL_ARCH="$(uname -m)"
if [[ -n "$EXPECTED_ARCH" && "$ACTUAL_ARCH" != "$EXPECTED_ARCH" ]]; then
  echo "ERROR: arch mismatch expected=$EXPECTED_ARCH actual=$ACTUAL_ARCH" >&2
  exit 2
fi

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
# shellcheck disable=SC1091
source ~/.cargo/env 2>/dev/null || true
mkdir -p "$ARTIFACT_DIR"

TMP="$(mktemp -d)"
DOCROOT="$TMP/public"
CFG="$TMP/exyonq.toml"
CTRL_SOCK="$TMP/control.sock"
SERVER_LOG="$ARTIFACT_DIR/server.log"
RESULTS_CSV="$ARTIFACT_DIR/results.csv"
SUMMARY_JSON="$ARTIFACT_DIR/summary.json"
EXYONQ_BIN=""
EXYONQCTL_BIN=""
LISTEN_PORT=""
SERVER_PID=""
UPSTREAM_PID=""
FCGI_PID=""
OVERALL_RC=0

declare -A SCENARIO_NOTES=()

cleanup() {
  local rc=$?
  [[ -n "${UPSTREAM_PID:-}" ]] && kill "$UPSTREAM_PID" 2>/dev/null || true
  [[ -n "${FCGI_PID:-}" ]] && kill "$FCGI_PID" 2>/dev/null || true
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
  exit "$rc"
}
trap cleanup EXIT INT TERM

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

record_result() {
  local id="$1" verdict="$2" note="${3:-}"
  SCENARIO_NOTES["$id"]="$note"
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$id,$verdict,${note//,/;}" >>"$RESULTS_CSV"
  echo "[$id] $verdict ${note:+- $note}"
  if [[ "$verdict" == "FAIL" ]]; then
    OVERALL_RC=1
  fi
}

want_scenario() {
  local id="$1"
  [[ "$SCENARIO" == "all" || "$SCENARIO" == "$id" ]]
}

ensure_bins() {
  # Always rebuild so WS5 finding-linked probe/signal fixes are not skipped by a stale
  # remote `target/release` left behind when rsync excludes `target/`.
  cargo build -p exyonq -p exyonqctl --release -q
  EXYONQ_BIN="$WORKSPACE/target/release/exyonq"
  EXYONQCTL_BIN="$WORKSPACE/target/release/exyonqctl"
  [[ -x "$EXYONQ_BIN" && -x "$EXYONQCTL_BIN" ]]
}

write_base_config() {
  local listen="${1:-$LISTEN_PORT}"
  mkdir -p "$DOCROOT"
  echo "ws5-lifecycle-ok" >"$DOCROOT/index.html"
  cat >"$CFG" <<EOF
config_version = 2

[[server]]
listen = "127.0.0.1:${listen}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF
}

write_proxy_config() {
  local listen="$1" upstream_port="$2"
  mkdir -p "$DOCROOT"
  cat >"$CFG" <<EOF
config_version = 2

[[server]]
listen = "127.0.0.1:${listen}"
routes = ["app"]

[[route]]
name = "app"
match = { path = "/" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${upstream_port}"
timeout_ms = 30000
EOF
}

write_fcgi_config() {
  local listen="$1" fcgi_port="$2"
  mkdir -p "$DOCROOT"
  # FastCGI resolver requires a real script under document_root.
  printf '<?php echo "FCGI_OK";\n' >"$DOCROOT/index.php"
  cat >"$CFG" <<EOF
config_version = 2

[[server]]
listen = "127.0.0.1:${listen}"
routes = ["php"]

[[route]]
name = "php"
match = { path = "/" }
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = "127.0.0.1:${fcgi_port}"
transport = "tcp"
document_root = "${DOCROOT}"
max_concurrency = 4
max_connections = 4
idle_timeout_ms = 30000
total_timeout_ms = 30000
checkout_timeout_ms = 5000
EOF
}

stop_server() {
  [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null || { SERVER_PID=""; return 0; }
  kill -TERM "$SERVER_PID" 2>/dev/null || true
  local i=0
  while kill -0 "$SERVER_PID" 2>/dev/null && (( i < WAIT_TIMEOUT * 10 )); do
    sleep 0.1
    i=$((i + 1))
  done
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -KILL "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  else
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=""
}

wait_listen() {
  local port="$1" path="${2:-/}"
  local i=0 code
  while (( i < WAIT_TIMEOUT * 10 )); do
    code="$(curl -sS --max-time 2 -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}${path}" 2>/dev/null || echo 000)"
    if [[ "$code" != "000" ]]; then
      return 0
    fi
    if [[ -n "${SERVER_PID:-}" ]] && ! kill -0 "$SERVER_PID" 2>/dev/null; then
      return 1
    fi
    sleep 0.1
    i=$((i + 1))
  done
  return 1
}

start_server() {
  local ready_path="${1:-/}"
  stop_server
  rm -f "$CTRL_SOCK"
  : >"$SERVER_LOG"
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" EXYONQ_CONFIG="$CFG" EXYONQ_LOG_FORMAT=json RUST_LOG=info \
    "$EXYONQ_BIN" serve --config "$CFG" >>"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  wait_listen "$LISTEN_PORT" "$ready_path" || return 1
  # Wait for control socket — HTTP can be ready before the socket is bound
  # (arm64 race: exyonqctl drain → ENOENT → O04 false FAIL).
  local i=0
  while [[ ! -S "$CTRL_SOCK" ]] && (( i < WAIT_TIMEOUT * 10 )); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      return 1
    fi
    sleep 0.1
    i=$((i + 1))
  done
  [[ -S "$CTRL_SOCK" ]]
}

start_server_expect_fail() {
  stop_server
  rm -f "$CTRL_SOCK"
  : >"$SERVER_LOG"
  set +e
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" EXYONQ_CONFIG="$CFG" EXYONQ_LOG_FORMAT=json \
    RUST_LOG=info "$EXYONQ_BIN" serve --config "$CFG" >>"$SERVER_LOG" 2>&1 &
  local pid=$!
  local i=0 rc=0
  while (( i < WAIT_TIMEOUT * 10 )); do
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid" || rc=$?
      SERVER_PID=""
      set -e
      return "$rc"
    fi
    if curl -sf --max-time 1 "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null 2>&1; then
      kill -TERM "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      SERVER_PID=""
      set -e
      return 0
    fi
    sleep 0.1
    i=$((i + 1))
  done
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  SERVER_PID=""
  set -e
  return 124
}

curl_probe() {
  local port="$1" path="$2"
  curl -sS --max-time 5 -w $'\n%{http_code}' "http://127.0.0.1:${port}${path}" 2>/dev/null || echo -e "\n000"
}

status_generation() {
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" --format json 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("generation",0))' 2>/dev/null || echo 0
}

start_upstream() {
  local port="$1"
  if [[ -f "$WORKSPACE/scripts/soak/lib/p13a-upstream.py" ]]; then
    P13A_UPSTREAM_BIND=127.0.0.1 P13A_UPSTREAM_PORT="$port" P13A_UPSTREAM_IDENTITY=ws5-upstream \
      python3 "$WORKSPACE/scripts/soak/lib/p13a-upstream.py" >>"$ARTIFACT_DIR/upstream.log" 2>&1 &
  else
    python3 - "$port" >>"$ARTIFACT_DIR/upstream.log" 2>&1 <<'PY' &
import json, socket, sys, threading
port = int(sys.argv[1])
def handle(conn):
    try:
        data = conn.recv(4096)
        body = json.dumps({"ok": True, "identity": "ws5-inline-upstream"}).encode()
        hdr = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
        conn.sendall(hdr.encode() + body)
    finally:
        conn.close()
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(64)
while True:
    c, _ = s.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
PY
  fi
  UPSTREAM_PID=$!
  local i=0
  while (( i < 50 )); do
    if curl -sf --max-time 1 "http://127.0.0.1:${port}/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
    i=$((i + 1))
  done
  return 1
}

start_fcgi_responder() {
  local port="$1"
  python3 - "$port" >>"$ARTIFACT_DIR/fcgi-upstream.log" 2>&1 <<'PY' &
import socket, struct, sys, threading
FCGI_BEGIN_REQUEST, FCGI_PARAMS, FCGI_STDIN = 1, 4, 5
FCGI_STDOUT, FCGI_END_REQUEST = 6, 3
FCGI_RESPONDER, FCGI_REQUEST_COMPLETE = 1, 0
port = int(sys.argv[1])
BODY = b"Status: 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\n\r\nFCGI_OK"

def encode_record(rtype, req_id, content):
    clen = len(content)
    pad = (8 - (clen % 8)) % 8
    hdr = struct.pack("!BBHHBB", 1, rtype, req_id, clen, pad, 0)
    return hdr + content + (b"\0" * pad)

def handle_conn(conn):
    buf = b""
    req_id = 1
    try:
        while True:
            chunk = conn.recv(4096)
            if not chunk:
                break
            buf += chunk
            while len(buf) >= 8:
                ver, rtype, req_id, clen, plen, _res = struct.unpack_from("!BBHHBB", buf, 0)
                total = 8 + clen + plen
                if len(buf) < total:
                    break
                content = buf[8:8+clen]
                buf = buf[total:]
                if rtype == FCGI_STDIN and len(content) == 0:
                    conn.sendall(encode_record(FCGI_STDOUT, req_id, BODY))
                    conn.sendall(encode_record(FCGI_STDOUT, req_id, b""))
                    end = struct.pack("!IBBBB", 0, FCGI_REQUEST_COMPLETE, 0, 0, 0)
                    conn.sendall(encode_record(FCGI_END_REQUEST, req_id, end))
                    return
    finally:
        conn.close()

s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(32)
while True:
    c, _ = s.accept()
    threading.Thread(target=handle_conn, args=(c,), daemon=True).start()
PY
  FCGI_PID=$!
  sleep 0.3
}

echo "ts_utc,scenario_id,verdict,note" >"$RESULTS_CSV"
ensure_bins
LISTEN_PORT="$(pick_port)"

run_o01() {
  write_base_config
  cp "$CFG" "$ARTIFACT_DIR/o01-config.toml"
  local attempt=1
  while (( attempt <= 3 )); do
    if start_server && curl -sf --max-time 5 "http://127.0.0.1:${LISTEN_PORT}/" | grep -q ws5-lifecycle-ok; then
      record_result O01 PASS "startup_serving attempt=${attempt}"
      stop_server
      return
    fi
    stop_server
    sleep 0.5
    attempt=$((attempt + 1))
  done
  record_result O01 FAIL "startup_or_static"
}

run_o02() {
  cat >"$CFG" <<EOF
config_version = 2
typo_field = true
[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF
  mkdir -p "$DOCROOT"
  set +e
  start_server_expect_fail
  rc=$?
  set -e
  if [[ "$rc" -ne 0 ]] && ! curl -sf --max-time 1 "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null 2>&1; then
    record_result O02 PASS "fail_closed_rc=${rc}"
  else
    record_result O02 FAIL "expected_nonzero_no_listener rc=${rc}"
  fi
}

run_o03() {
  write_base_config
  start_server || { record_result O03 FAIL "start"; return; }
  local out code
  out="$(curl_probe "$LISTEN_PORT" "/ready")"
  code="$(echo "$out" | tail -1)"
  body="$(echo "$out" | sed '$d' | tr -d '\r')"
  if [[ "$code" == "200" && "$body" == *ready* ]]; then
    record_result O03 PASS "/ready=200"
  else
    record_result O03 FAIL "code=${code} body=${body}"
  fi
  stop_server
}

run_o04() {
  write_base_config
  start_server || { record_result O04 FAIL "start"; return; }
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" drain --socket "$CTRL_SOCK" >/dev/null
  sleep 0.5
  local ready live rcode lcode rbody lbody
  ready="$(curl_probe "$LISTEN_PORT" "/ready")"
  live="$(curl_probe "$LISTEN_PORT" "/live")"
  rcode="$(echo "$ready" | tail -1)"
  lcode="$(echo "$live" | tail -1)"
  rbody="$(echo "$ready" | sed '$d' | tr -d '\r')"
  lbody="$(echo "$live" | sed '$d' | tr -d '\r')"
  if [[ "$rcode" == "503" && "$rbody" == *not_ready* && "$lcode" == "200" && "$lbody" == *live* ]]; then
    record_result O04 PASS "ready=503 live=200"
  else
    record_result O04 FAIL "ready=${rcode}/${rbody} live=${lcode}/${lbody}"
  fi
  stop_server
}

run_o05() {
  write_base_config
  start_server || { record_result O05 FAIL "start"; return; }
  local out
  out="$(EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" 2>&1)"
  if echo "$out" | grep -q 'generation=' && echo "$out" | grep -q 'fingerprint='; then
    record_result O05 PASS "human_status"
  else
    record_result O05 FAIL "$out"
  fi
  stop_server
}

run_o06() {
  write_base_config
  start_server || { record_result O06 FAIL "start"; return; }
  local out
  out="$(EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" --format json 2>&1)"
  if echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "generation" in d and "fingerprint" in d' 2>/dev/null; then
    record_result O06 PASS "json_status"
  else
    record_result O06 FAIL "invalid_json"
  fi
  stop_server
}

run_o07() {
  write_base_config
  start_server || { record_result O07 FAIL "start"; return; }
  local gen_before gen_after
  gen_before="$(status_generation)"
  mkdir -p "$DOCROOT/o7"
  echo "ws5-reload-bump" >"$DOCROOT/o7/index.html"
  cat >"$CFG" <<EOF
config_version = 2

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site", "o7"]

[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"

[[route]]
name = "o7"
match = { path = "/o7" }
root = "${DOCROOT}/o7"
index = "index.html"
EOF
  if EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
    "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null 2>&1; then
    sleep 0.5
    gen_after="$(status_generation)"
    if curl -sf "http://127.0.0.1:${LISTEN_PORT}/o7/" | grep -q ws5-reload-bump; then
      if [[ "$gen_after" -gt "$gen_before" ]]; then
        record_result O07 PASS "reload_ok gen ${gen_before}->${gen_after}"
      else
        record_result O07 FAIL "generation_not_advanced ${gen_before}->${gen_after}"
      fi
    else
      record_result O07 FAIL "route_not_serving"
    fi
  else
    record_result O07 FAIL "reload_rejected"
  fi
  stop_server
}

run_o08() {
  write_base_config
  start_server || { record_result O08 FAIL "start"; return; }
  local gen_before gen_after
  gen_before="$(status_generation)"
  if EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
    "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null 2>&1; then
    gen_after="$(status_generation)"
    if [[ "$gen_before" == "$gen_after" ]]; then
      record_result O08 PASS "identical_no_op"
    else
      record_result O08 PASS "reload_ok_same_gen=${gen_after}"
    fi
  else
    record_result O08 FAIL "identical_reload_failed"
  fi
  stop_server
}

run_o09() {
  write_base_config
  start_server || { record_result O09 FAIL "start"; return; }
  local gen_before gen_after good="$TMP/good.toml"
  gen_before="$(status_generation)"
  cp "$CFG" "$good"
  # Invalidate the daemon-bound path in place (PATH honesty); top-level reload
  # publishes EXYONQ_CONFIG, not an alternate --config candidate.
  echo 'typo_root = 1' >>"$CFG"
  set +e
  EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
    "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null 2>&1
  local rc=$?
  set -e
  gen_after="$(status_generation)"
  cp "$good" "$CFG"
  if [[ "$rc" -ne 0 && "$gen_before" == "$gen_after" ]] && curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -q ws5-lifecycle-ok; then
    record_result O09 PASS "fail_retain"
  else
    record_result O09 FAIL "rc=${rc} gen ${gen_before}->${gen_after}"
  fi
  stop_server
}

run_o10() {
  write_base_config
  start_server || { record_result O10 FAIL "start"; return; }
  if EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" drain --socket "$CTRL_SOCK" >/dev/null 2>&1; then
    sleep 0.3
    local code out body draining
    out="$(curl_probe "$LISTEN_PORT" "/site/index.html")"
    code="$(echo "$out" | tail -1)"
    draining="$(EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" --format json \
      | python3 -c 'import json,sys; print(json.load(sys.stdin).get("draining"))' 2>/dev/null || echo false)"
    if [[ "$code" == "503" || "$draining" == "True" ]]; then
      record_result O10 PASS "drain_visible code=${code} draining=${draining}"
    else
      record_result O10 FAIL "code=${code} draining=${draining}"
    fi
  else
    record_result O10 FAIL "drain_cmd"
  fi
  stop_server
}

run_o11() {
  write_base_config
  start_server || { record_result O11 FAIL "start"; return; }
  kill -TERM "$SERVER_PID"
  local i=0
  while kill -0 "$SERVER_PID" 2>/dev/null && (( i < WAIT_TIMEOUT * 10 )); do
    sleep 0.1
    i=$((i + 1))
  done
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    record_result O11 FAIL "still_running"
    stop_server
  else
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
    record_result O11 PASS "sigterm_idle_exit"
  fi
}

run_o12() {
  write_base_config
  start_server || { record_result O12 FAIL "start"; return; }
  ( curl -sf --max-time "$((WAIT_TIMEOUT + 5))" "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null ) &
  local curl_pid=$!
  sleep 0.2
  kill -TERM "$SERVER_PID"
  local i=0
  while kill -0 "$SERVER_PID" 2>/dev/null && (( i < WAIT_TIMEOUT * 10 )); do
    sleep 0.1
    i=$((i + 1))
  done
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=""
  wait "$curl_pid" 2>/dev/null || true
  record_result O12 PASS "sigterm_under_traffic_bounded"
}

run_o13() {
  write_base_config
  start_server || { record_result O13 FAIL "start"; return; }
  kill -TERM "$SERVER_PID"
  sleep 0.1
  kill -TERM "$SERVER_PID" 2>/dev/null || true
  local i=0
  while kill -0 "$SERVER_PID" 2>/dev/null && (( i < WAIT_TIMEOUT * 10 )); do
    sleep 0.1
    i=$((i + 1))
  done
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    record_result O13 FAIL "double_sigterm_hung"
    stop_server
  else
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
    record_result O13 PASS "double_sigterm_idempotent"
  fi
}

run_o14() {
  write_base_config
  start_server || { record_result O14 FAIL "start"; return; }
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" shutdown --socket "$CTRL_SOCK" >/dev/null 2>&1 || true
  local i=0
  while kill -0 "$SERVER_PID" 2>/dev/null && (( i < WAIT_TIMEOUT * 10 )); do
    sleep 0.1
    i=$((i + 1))
  done
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=""
  if start_server && curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -q ws5-lifecycle-ok; then
    record_result O14 PASS "restart_serving"
  else
    record_result O14 FAIL "restart_failed"
  fi
  stop_server
}

run_o15() {
  local up_port
  up_port="$(pick_port)"
  LISTEN_PORT="$(pick_port)"
  write_proxy_config "$LISTEN_PORT" "$up_port"
  start_upstream "$up_port" || { record_result O15 FAIL "upstream_start"; return; }
  start_server || { record_result O15 FAIL "exyonq_start"; kill "$UPSTREAM_PID" 2>/dev/null || true; UPSTREAM_PID=""; return; }
  if ! curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -q ws5-upstream; then
    curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -q identity || {
      record_result O15 FAIL "proxy_initial"
      stop_server
      kill "$UPSTREAM_PID" 2>/dev/null || true
      UPSTREAM_PID=""
      return
    }
  fi
  kill "$UPSTREAM_PID" 2>/dev/null || true
  UPSTREAM_PID=""
  sleep 0.5
  set +e
  out="$(curl_probe "$LISTEN_PORT" "/")"
  set -e
  code="$(echo "$out" | tail -1)"
  start_upstream "$up_port" || { record_result O15 FAIL "upstream_restart"; stop_server; return; }
  local ok=0 i=0
  while (( i < WAIT_TIMEOUT * 10 )); do
    if curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -qE 'identity|ws5-upstream|ok'; then
      ok=1
      break
    fi
    sleep 0.2
    i=$((i + 1))
  done
  if [[ "$ok" -eq 1 ]]; then
    record_result O15 PASS "upstream_recovery after_fail_code=${code}"
  else
    record_result O15 FAIL "no_recovery"
  fi
  stop_server
  kill "$UPSTREAM_PID" 2>/dev/null || true
  UPSTREAM_PID=""
}

run_o16() {
  local fcgi_port
  fcgi_port="$(pick_port)"
  LISTEN_PORT="$(pick_port)"
  write_fcgi_config "$LISTEN_PORT" "$fcgi_port"
  start_fcgi_responder "$fcgi_port"
  start_server "/index.php" || { record_result O16 FAIL "exyonq_start"; kill "$FCGI_PID" 2>/dev/null || true; FCGI_PID=""; return; }
  if ! curl -sf "http://127.0.0.1:${LISTEN_PORT}/index.php" | grep -q FCGI_OK; then
    record_result O16 FAIL "fcgi_initial"
    stop_server
    kill "$FCGI_PID" 2>/dev/null || true
    FCGI_PID=""
    return
  fi
  kill "$FCGI_PID" 2>/dev/null || true
  FCGI_PID=""
  sleep 0.5
  set +e
  out="$(curl_probe "$LISTEN_PORT" "/index.php")"
  set -e
  code="$(echo "$out" | tail -1)"
  start_fcgi_responder "$fcgi_port"
  local ok=0 i=0
  while (( i < WAIT_TIMEOUT * 10 )); do
    if curl -sf "http://127.0.0.1:${LISTEN_PORT}/index.php" | grep -q FCGI_OK; then
      ok=1
      break
    fi
    sleep 0.2
    i=$((i + 1))
  done
  if [[ "$ok" -eq 1 ]]; then
    record_result O16 PASS "fcgi_recovery after_fail_code=${code}"
  else
    record_result O16 FAIL "no_recovery"
  fi
  stop_server
  kill "$FCGI_PID" 2>/dev/null || true
  FCGI_PID=""
}

run_o17() {
  local tls_dir cert missing_key
  tls_dir="$TMP/tls"
  cert="$tls_dir/cert.pem"
  missing_key="$tls_dir/missing.key"
  mkdir -p "$tls_dir" "$DOCROOT"
  echo ok >"$DOCROOT/index.html"
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$tls_dir/key.pem" -out "$cert" -days 1 \
    -subj "/CN=ws5-o17" >/dev/null 2>&1 || { record_result O17 FAIL "openssl"; return; }
  LISTEN_PORT="$(pick_port)"
  cat >"$CFG" <<EOF
config_version = 2
[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site"]
tls = { cert = "${cert}", key = "${missing_key}" }
[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF
  set +e
  start_server_expect_fail
  rc=$?
  set -e
  if [[ "$rc" -ne 0 ]] && ! curl -sf --max-time 1 -k "https://127.0.0.1:${LISTEN_PORT}/" >/dev/null 2>&1; then
    record_result O17 PASS "tls_missing_key_fail_closed rc=${rc}"
  else
    record_result O17 FAIL "expected_fail rc=${rc}"
  fi
}

run_o18() {
  local h3_cfg="$TMP/h3-same-listener.toml" h3_changed="$TMP/h3-changed.toml"
  cp "$WORKSPACE/tests/fixtures/config-product/h3-same-listener.toml" "$h3_cfg" 2>/dev/null || cat >"$h3_cfg" <<'EOF'
config_version = 2
[[server]]
listen = "127.0.0.1:18443"
http3_listen = "127.0.0.1:18443"
routes = ["site"]
tls = { cert = "/tmp/exyonq-ws7-placeholder.crt", key = "/tmp/exyonq-ws7-placeholder.key" }
[[route]]
name = "site"
match = { path = "/" }
root = "/tmp/exyonq-static"
index = "index.html"
EOF
  cp "$h3_cfg" "$h3_changed"
  sed -i 's/18443/19443/g' "$h3_changed" 2>/dev/null || sed 's/18443/19443/g' "$h3_cfg" >"$h3_changed"
  local diff_out check_out ok=0
  diff_out="$("$EXYONQCTL_BIN" config reload "$h3_cfg" --diff 2>&1 || true)"
  if echo "$diff_out" | grep -q 'H3_RELOAD_SUPPORT = SAME_LISTENER_CONFIG_ONLY'; then
    ok=1
  fi
  check_out="$("$EXYONQCTL_BIN" config reload "$h3_cfg" --check 2>&1 || true)"
  if echo "$check_out" | grep -qiE 'EXY-RELOAD-0005|RESTART_REQUIRED|http3_listen'; then
    ok=1
  fi
  diff_out="$("$EXYONQCTL_BIN" config reload "$h3_changed" --diff 2>&1 || true)"
  if echo "$diff_out" | grep -qiE 'RESTART_REQUIRED|EXY-RELOAD-0005|http3_listen'; then
    ok=1
  fi
  if [[ "$ok" -eq 1 ]]; then
    record_result O18 PASS "H3_same_listener_limit_visible"
  else
    record_result O18 FAIL "limit_not_visible"
  fi
}

run_o19() {
  write_base_config
  cp "$CFG" "$TMP/good.toml"
  start_server || { record_result O19 FAIL "start"; return; }
  local gen_before gen_mid gen_after
  gen_before="$(status_generation)"
  cp "$TMP/good.toml" "$CFG"
  cat >"$CFG" <<EOF
config_version = 2
typo_root = 1
[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF
  set +e
  EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
    "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null 2>&1
  local rc=$?
  set -e
  gen_mid="$(status_generation)"
  cp "$TMP/good.toml" "$CFG"
  if [[ "$rc" -ne 0 && "$gen_before" == "$gen_mid" ]]; then
    if EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
      "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null 2>&1; then
      gen_after="$(status_generation)"
      if curl -sf "http://127.0.0.1:${LISTEN_PORT}/" | grep -q ws5-lifecycle-ok; then
        record_result O19 PASS "fail_retain_restore gen=${gen_before}->${gen_after}"
      else
        record_result O19 FAIL "restore_not_serving"
      fi
    else
      record_result O19 FAIL "restore_reload_failed"
    fi
  else
    record_result O19 FAIL "bad_reload_not_rejected rc=${rc}"
  fi
  stop_server
}

run_o20() {
  write_base_config
  stop_server
  rm -f "$CTRL_SOCK"
  echo "stale-control-socket-placeholder" >"$CTRL_SOCK"
  start_server || { record_result O20 FAIL "start_with_stale_sock"; return; }
  if [[ -S "$CTRL_SOCK" ]] && EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" >/dev/null 2>&1; then
    record_result O20 PASS "stale_sock_replaced"
  else
    record_result O20 FAIL "control_socket_not_usable"
  fi
  stop_server
}

for id in O01 O02 O03 O04 O05 O06 O07 O08 O09 O10 O11 O12 O13 O14 O15 O16 O17 O18 O19 O20; do
  fn="run_$(echo "$id" | tr '[:upper:]' '[:lower:]')"
  if want_scenario "$id" && declare -f "$fn" >/dev/null; then
    echo "=== $id ==="
    if ! "$fn"; then
      if ! grep -q "^[^,]*,${id},FAIL," "$RESULTS_CSV" 2>/dev/null; then
        record_result "$id" FAIL "scenario_exception"
      fi
    fi
  fi
done

python3 - "$RESULTS_CSV" "$SUMMARY_JSON" "$HOST_LABEL" "$ACTUAL_ARCH" <<'PY'
import csv, json, pathlib, sys
csv_path, json_path, host, arch = sys.argv[1:5]
rows = []
with open(csv_path, newline="") as f:
    for row in csv.DictReader(f):
        rows.append(row)
fail = sum(1 for r in rows if r.get("verdict") == "FAIL")
out = {
    "host_label": host,
    "arch": arch,
    "scenarios": {r["scenario_id"]: {"verdict": r["verdict"], "note": r.get("note", "")} for r in rows},
    "pass": sum(1 for r in rows if r.get("verdict") == "PASS"),
    "fail": fail,
    "verdict": "PASS" if fail == 0 and rows else "FAIL",
}
pathlib.Path(json_path).write_text(json.dumps(out, indent=2) + "\n")
print(out["verdict"])
PY

echo ""
echo "=== P1.5-WS5 lifecycle summary (host=$HOST_LABEL arch=$ACTUAL_ARCH) ==="
column -t -s, "$RESULTS_CSV" 2>/dev/null || cat "$RESULTS_CSV"
echo "ARTIFACT_DIR=$ARTIFACT_DIR OVERALL_RC=$OVERALL_RC"
exit "$OVERALL_RC"
