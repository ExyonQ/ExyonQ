#!/usr/bin/env bash
# K0.4 Core Smoke Matrix — not official benchmark, no claims.
# Hyper + wire paths; sendfile engagement verified on Linux when flags set.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OS="$(uname -s)"
ARCH="$(uname -m)"
COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
EPOLL_STATIC="${EXYONQ_EPOLL_STATIC:-0}"
EPOLL_SENDFILE="${EXYONQ_EPOLL_SENDFILE:-0}"
K0_SMOKE_PROFILE="${K0_SMOKE_PROFILE:-default}"
if [[ "$EPOLL_STATIC" == "1" && "$EPOLL_SENDFILE" == "1" ]]; then
  K0_SMOKE_PROFILE="epoll_sendfile"
fi
K0_HOST_LABEL="${K0_HOST_LABEL:-local}"
echo "=== K0.4 Core Smoke Matrix @ $COMMIT host=$K0_HOST_LABEL os=$OS arch=$ARCH profile=$K0_SMOKE_PROFILE ==="

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

UPSTREAM_PORT="$(pick_port)"
DEAD_PORT="$(pick_port)"
LISTEN_PORT="$(pick_port)"
CTRL_SOCK="$(mktemp -u /tmp/exyonq-k0.XXXX.sock)"
LOG="/tmp/exyonq-k0-smoke.log"
RESULTS="/tmp/k0-smoke-results-${K0_HOST_LABEL}-${K0_SMOKE_PROFILE}.txt"

PASS=0
FAIL=0
SKIP=0
NA=0

cleanup() {
  [[ -n "${UP_PID:-}" ]] && kill "$UP_PID" 2>/dev/null || true
  [[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true
  rm -f "$CFG" "$CTRL_SOCK"
}
trap cleanup EXIT

record() {
  local status="$1" id="$2" detail="$3"
  printf '%s\t%s\t%s\n' "$status" "$id" "$detail" >>"$RESULTS"
  case "$status" in
    PASS) echo "PASS  $id — $detail"; PASS=$((PASS + 1)) ;;
    FAIL) echo "FAIL  $id — $detail"; FAIL=$((FAIL + 1)) ;;
    SKIP) echo "SKIP  $id — $detail"; SKIP=$((SKIP + 1)) ;;
    N/A)  echo "N/A   $id — $detail"; NA=$((NA + 1)) ;;
  esac
}

# Mock upstream
python3 - "$UPSTREAM_PORT" <<'PY' &
import socket, sys
port = int(sys.argv[1])
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(64)
while True:
    c, _ = s.accept()
    data = c.recv(4096)
    req = data.decode("latin-1", errors="replace")
    if req.startswith("HEAD "):
        body = ""
        cl = "0"
    else:
        body = "upstream-ok" if "/api/" in req else "echo"
        cl = str(len(body))
    resp = (
        f"HTTP/1.1 200 OK\r\nContent-Length: {cl}\r\nConnection: close\r\n\r\n{body}"
    )
    c.sendall(resp.encode())
    c.close()
PY
UP_PID=$!
sleep 0.2

CFG="$(mktemp)"
cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site", "api", "api_down"]

[[route]]
name = "site"
match = { path = "/site" }
root = "benchmarks/scenarios/payloads/www"
index = "index.html"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[route]]
name = "api_down"
match = { path = "/api-down" }
upstream = "dead"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UPSTREAM_PORT}"
timeout_ms = 2000

[[upstream]]
name = "dead"
target = "http://127.0.0.1:${DEAD_PORT}"
timeout_ms = 500
EOF

: >"$RESULTS"
RUST_LOG=error EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
  EXYONQ_EPOLL_STATIC="$EPOLL_STATIC" EXYONQ_EPOLL_SENDFILE="$EPOLL_SENDFILE" \
  "${ROOT}/target/debug/exyonq" serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

BASE="http://127.0.0.1:${LISTEN_PORT}"
ready=0
for _ in $(seq 1 30); do
  if curl -sf -o /dev/null "$BASE/health" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.25
done
if [[ "$ready" -ne 1 ]]; then
  echo "FAIL  server not ready"
  tail -30 "$LOG" || true
  exit 1
fi

metrics_counter() {
  local name="$1"
  local raw val
  raw="$(curl -sf "$BASE/metrics" 2>/dev/null | tr -d '\r')"
  val="$(printf '%s\n' "$raw" | sed -n "s/^${name} \\([0-9][0-9]*\\)\$/\\1/p" | head -1)"
  if [[ -n "$val" ]]; then
    echo "$val"
  elif printf '%s\n' "$raw" | grep -q "^${name} "; then
    echo "0"
  else
    echo "MISSING"
  fi
}

snap_counters() {
  MET_COMPLETE="$(metrics_counter exyonq_epoll_sendfile_complete_total)"
  MET_ERROR="$(metrics_counter exyonq_epoll_sendfile_error_total)"
  MET_FALLBACK="$(metrics_counter exyonq_epoll_sendfile_fallback_total)"
  MET_TERM503="$(metrics_counter exyonq_epoll_sendfile_terminal_503_total)"
  MET_UNKNOWN="$(metrics_counter exyonq_epoll_sendfile_unknown_fd_event_total)"
}

snap_counters
INIT_COMPLETE="$MET_COMPLETE"
INIT_ERROR="$MET_ERROR"
INIT_FALLBACK="$MET_FALLBACK"
INIT_TERM503="$MET_TERM503"
INIT_UNKNOWN="$MET_UNKNOWN"

# --- Static GET sizes ---
for spec in "1k:1024" "64k:65536" "1m:1048576"; do
  file="${spec%%:*}"
  want="${spec##*:}"
  out="/tmp/k0-${file}.body"
  code="$(curl -sf -o "$out" -w '%{http_code}' "$BASE/site/${file}.bin" || echo 000)"
  got="$(wc -c <"$out" | tr -d ' ')"
  if [[ "$code" == "200" && "$got" == "$want" ]]; then
    record PASS "static-get-${file}" "200 ${got}B"
  else
    record FAIL "static-get-${file}" "code=$code bytes=$got want=$want"
  fi
done

# --- Static HEAD ---
for file in 1k.bin 64k.bin 1m.bin index.html; do
  code="$(curl -sf -I -o /dev/null -w '%{http_code}' -X HEAD "$BASE/site/$file" || echo 000)"
  cl="$(curl -sf -I -X HEAD "$BASE/site/$file" 2>/dev/null | awk -F': ' 'tolower($1)=="content-length"{print $2; exit}' | tr -d '\r')"
  want=""
  case "$file" in
    1k.bin) want=1024 ;;
    64k.bin) want=65536 ;;
    1m.bin) want=1048576 ;;
  esac
  if [[ "$code" != "200" ]]; then
    record FAIL "static-head-${file}" "code=$code"
  elif [[ -n "$want" && "$cl" != "$want" ]]; then
    record FAIL "static-head-${file}" "200 cl=$cl want=$want"
  else
    record PASS "static-head-${file}" "200 cl=${cl:-?}"
  fi
done

# --- Wire static ---
wire_content_length() {
  awk 'BEGIN{IGNORECASE=1} /^Content-Length:/ {gsub(/\r/,"",$2); print $2; exit}' "$1"
}

wire_check() {
  local path="$1" want_bytes="$2"
  local out="/tmp/k0-wire.body"
  printf 'GET %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$path" \
    | nc -w 5 127.0.0.1 "$LISTEN_PORT" >"$out" || true
  if grep -q "HTTP/1.1 404" "$out"; then
    record FAIL "wire-static-${path##*/}" "404 on wire static"
    return
  fi
  if ! grep -q "HTTP/1.1 200" "$out"; then
    record FAIL "wire-static-${path##*/}" "no 200 in response"
    return
  fi
  if [[ -n "$want_bytes" ]]; then
    local cl
    cl="$(wire_content_length "$out")"
    if [[ "$cl" == "$want_bytes" ]]; then
      record PASS "wire-static-${path##*/}" "200 cl=${cl}"
    elif [[ "$cl" == "9" && "$want_bytes" != "9" ]]; then
      record FAIL "wire-static-${path##*/}" "wire NOT_FOUND shim cl=9 want=$want_bytes"
    else
      record FAIL "wire-static-${path##*/}" "cl=$cl want=$want_bytes"
    fi
  else
    record PASS "wire-static-${path##*/}" "200"
  fi
}

wire_check "/site/1k.bin" "1024"
wire_check "/site/64k.bin" "65536"
wire_check "/site/1m.bin" "1048576"

# --- Proxy ---
code="$(curl -sf -o /tmp/k0-proxy.body -w '%{http_code}' "$BASE/api/health" || echo 000)"
body="$(cat /tmp/k0-proxy.body 2>/dev/null || true)"
if [[ "$code" == "200" && "$body" == "upstream-ok" ]]; then
  record PASS "proxy-get-api" "200 upstream-ok"
else
  record FAIL "proxy-get-api" "code=$code body=$body"
fi

pcode="$(curl -sf -I -o /dev/null -w '%{http_code}' -X HEAD "$BASE/api/health" || echo 000)"
if [[ "$pcode" == "200" ]]; then
  record PASS "proxy-head-api" "200"
else
  record FAIL "proxy-head-api" "code=$pcode"
fi

WIRE_PROXY="$(mktemp)"
printf 'GET /api/wire HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' \
  | nc -w 5 127.0.0.1 "$LISTEN_PORT" >"$WIRE_PROXY" || true
if grep -q "HTTP/1.1 200" "$WIRE_PROXY" && grep -q "upstream-ok" "$WIRE_PROXY"; then
  record PASS "wire-proxy-api" "200 upstream-ok"
else
  record FAIL "wire-proxy-api" "unexpected wire proxy response"
fi

# --- Health / metrics / 404 ---
hcode="$(curl -sf -o /dev/null -w '%{http_code}' "$BASE/health" || echo 000)"
[[ "$hcode" == "200" ]] && record PASS "health" "200" || record FAIL "health" "code=$hcode"

mcode="$(curl -s -o /tmp/k0-metrics.txt -w '%{http_code}' "$BASE/metrics")"
if [[ "$OS" != "Linux" ]]; then
  if [[ "$mcode" == "200" ]] && grep -q "exyonq_epoll_sendfile_complete_total" /tmp/k0-metrics.txt; then
    record PASS "metrics" "200 sendfile counters present"
  else
    record N/A "metrics" "wire /metrics + sendfile scrape is Linux-only (got code=$mcode)"
  fi
elif [[ "$mcode" == "200" ]] && grep -q "exyonq_epoll_sendfile_complete_total" /tmp/k0-metrics.txt; then
  record PASS "metrics" "200 sendfile counters present"
else
  record FAIL "metrics" "code=$mcode or missing counters"
fi

ncode="$(curl -s -o /dev/null -w '%{http_code}' "$BASE/unknown")"
[[ "$ncode" == "404" ]] && record PASS "unknown-404" "404" || record FAIL "unknown-404" "code=$ncode"

# --- Upstream down (controlled 502) ---
dcode="$(curl -sf -o /dev/null -w '%{http_code}' "$BASE/api-down/ping" 2>/dev/null || true)"
if [[ -z "$dcode" ]]; then
  dcode="$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api-down/ping" || echo 000)"
fi
if [[ "$dcode" == "502" ]]; then
  record PASS "upstream-down" "502 controlled"
else
  record FAIL "upstream-down" "code=$dcode want=502"
fi

# --- Reload fail-closed ---
if [[ ! -S "$CTRL_SOCK" ]]; then
  record FAIL "reload-fail-closed" "control socket missing"
else
  gen_before="$(printf 'status\n' | nc -U -w 2 "$CTRL_SOCK" | python3 -c 'import sys,json; print(json.load(sys.stdin)["generation"])')"
  cp "$CFG" "${CFG}.bak"
  echo 'not valid toml [[[' >"$CFG"
  reload_line="$(printf 'reload\n' | nc -U -w 2 "$CTRL_SOCK" || true)"
  ok="$(printf '%s' "$reload_line" | python3 -c 'import sys,json; d=json.loads(sys.stdin.read().strip()); print(d.get("ok"))' 2>/dev/null || echo false)"
  gen_after="$(printf 'status\n' | nc -U -w 2 "$CTRL_SOCK" | python3 -c 'import sys,json; print(json.load(sys.stdin)["generation"])')"
  mv "${CFG}.bak" "$CFG"
  h2="$(curl -sf -o /dev/null -w '%{http_code}' "$BASE/health" || echo 000)"
  if [[ "$ok" == "False" && "$gen_before" == "$gen_after" && "$h2" == "200" ]]; then
    record PASS "reload-fail-closed" "ok=false gen=$gen_before health=200"
  else
    record FAIL "reload-fail-closed" "ok=$ok gen_before=$gen_before gen_after=$gen_after health=$h2"
  fi
fi

# --- Sendfile engagement ---
wire_get() {
  local path="$1"
  printf 'GET %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$path" \
    | nc -w 5 127.0.0.1 "$LISTEN_PORT" >/dev/null || true
}

snap_counters
AFTER_STATIC_COMPLETE="$MET_COMPLETE"

if [[ "$OS" == "Linux" && "$K0_SMOKE_PROFILE" == "epoll_sendfile" ]]; then
  c0="$(metrics_counter exyonq_epoll_sendfile_complete_total)"
  wire_get "/site/1k.bin"
  c1="$(metrics_counter exyonq_epoll_sendfile_complete_total)"
  if [[ "$c1" != "MISSING" && ( "$c0" == "$c1" || ( "$c0" == "MISSING" && "$c1" == "0" ) ) ]]; then
    record PASS "sendfile-1k-not-engaged" "complete $c0 -> $c1 (wire 1k)"
  else
    record FAIL "sendfile-1k-not-engaged" "complete $c0 -> $c1"
  fi
  wire_get "/site/64k.bin"
  c2="$(metrics_counter exyonq_epoll_sendfile_complete_total)"
  if [[ "$c2" != "MISSING" && "$c2" -gt "$c1" ]]; then
    record PASS "sendfile-64k-engaged" "complete $c1 -> $c2"
  else
    record FAIL "sendfile-64k-engaged" "complete $c1 -> $c2"
  fi
  wire_get "/site/1m.bin"
  c3="$(metrics_counter exyonq_epoll_sendfile_complete_total)"
  if [[ "$c3" != "MISSING" && "$c3" -gt "$c2" ]]; then
    record PASS "sendfile-1m-engaged" "complete $c2 -> $c3"
  else
    record FAIL "sendfile-1m-engaged" "complete $c2 -> $c3"
  fi
elif [[ "$OS" == "Linux" ]]; then
  if [[ "$AFTER_STATIC_COMPLETE" == "$INIT_COMPLETE" || ( "$INIT_COMPLETE" == "0" && "$AFTER_STATIC_COMPLETE" == "0" ) ]]; then
    record PASS "sendfile-1k-not-engaged" "complete stable $INIT_COMPLETE (flags off)"
  elif [[ "$INIT_COMPLETE" == "MISSING" && "$AFTER_STATIC_COMPLETE" == "0" ]]; then
    record PASS "sendfile-1k-not-engaged" "complete=0 flags off"
  else
    record FAIL "sendfile-1k-not-engaged" "complete $INIT_COMPLETE -> $AFTER_STATIC_COMPLETE flags off"
  fi
  record N/A "sendfile-64k-engaged" "profile=default; run epoll_sendfile profile"
  record N/A "sendfile-1m-engaged" "profile=default; run epoll_sendfile profile"
else
  if [[ "$AFTER_STATIC_COMPLETE" == "$INIT_COMPLETE" || "$AFTER_STATIC_COMPLETE" == "0" || "$AFTER_STATIC_COMPLETE" == "MISSING" ]]; then
    record PASS "sendfile-1k-not-engaged" "complete unchanged ($INIT_COMPLETE) — epoll N/A on $OS"
  else
    record FAIL "sendfile-1k-not-engaged" "complete moved $INIT_COMPLETE -> $AFTER_STATIC_COMPLETE"
  fi
  record N/A "sendfile-64k-engaged" "Linux-only epoll sendfile"
  record N/A "sendfile-1m-engaged" "Linux-only epoll sendfile"
fi

# --- Counter sanity (no unexpected growth on happy path) ---
snap_counters
FINAL_COMPLETE="$MET_COMPLETE"
FINAL_ERROR="$MET_ERROR"
FINAL_FALLBACK="$MET_FALLBACK"
FINAL_TERM503="$MET_TERM503"
FINAL_UNKNOWN="$MET_UNKNOWN"

if [[ "$OS" != "Linux" ]]; then
  record N/A "counters-happy-path" "sendfile counters not exported on $OS wire path"
else
  counter_ok=1
  for pair in "error:$INIT_ERROR:$FINAL_ERROR" "fallback:$INIT_FALLBACK:$FINAL_FALLBACK" \
               "terminal_503:$INIT_TERM503:$FINAL_TERM503" "unknown_fd:$INIT_UNKNOWN:$FINAL_UNKNOWN"; do
    name="${pair%%:*}"
    rest="${pair#*:}"
    before="${rest%%:*}"
    after="${rest##*:}"
    if [[ "$before" == "MISSING" || "$after" == "MISSING" ]]; then
      record FAIL "counter-${name}" "metric missing (before=$before after=$after)"
      counter_ok=0
    elif [[ "$after" -gt "$before" ]]; then
      record FAIL "counter-${name}" "increased $before -> $after on happy path"
      counter_ok=0
    fi
  done
  [[ "$counter_ok" -eq 1 ]] && record PASS "counters-happy-path" "error/fallback/terminal_503/unknown_fd stable"
fi

echo ""
echo "=== K0.4 summary @ $COMMIT ($OS) ==="
echo "PASS=$PASS FAIL=$FAIL SKIP=$SKIP N/A=$NA"
echo ""
echo "--- counters final ---"
echo "complete=$FINAL_COMPLETE error=$FINAL_ERROR fallback=$FINAL_FALLBACK terminal_503=$FINAL_TERM503 unknown_fd=$FINAL_UNKNOWN"
echo ""
echo "--- matrix (TSV) ---"
cat "$RESULTS"

echo ""
if [[ "$FAIL" -gt 0 ]]; then
  echo "server log tail:"
  tail -25 "$LOG" || true
  exit 1
fi
exit 0
