#!/usr/bin/env bash
# KD3.4 — Linux proxy functional smoke (Hyper dispatch + wire transport + WebSocket).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: Linux only (got $(uname -s))"
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
EXYONQCTL_BIN="${EXYONQCTL_BIN:-$ROOT/target/debug/exyonqctl}"

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

TMP="$(mktemp -d)"
CFG_DIR="$TMP/cfg"
mkdir -p "$CFG_DIR"
CFG="$CFG_DIR/exyonq.toml"
CTRL_SOCK="$TMP/control.sock"
UP_LOG="$TMP/upstream.log"
SRV_LOG="$TMP/exyonq.log"
PASS=0
FAIL=0

record_pass() { echo "PASS  $1"; PASS=$((PASS + 1)); }
record_fail() { echo "FAIL  $1"; FAIL=$((FAIL + 1)); }

UP_PID=""
SRV_PID=""

stop_all() {
  if [[ -n "${SRV_PID:-}" ]] && kill -0 "$SRV_PID" 2>/dev/null; then
    kill "$SRV_PID" 2>/dev/null || true
    wait "$SRV_PID" 2>/dev/null || true
  fi
  SRV_PID=""
  if [[ -n "${UP_PID:-}" ]] && kill -0 "$UP_PID" 2>/dev/null; then
    kill "$UP_PID" 2>/dev/null || true
    wait "$UP_PID" 2>/dev/null || true
  fi
  UP_PID=""
}

cleanup() {
  stop_all
  rm -rf "$TMP"
}
trap cleanup EXIT

start_upstream() {
  local port="$1"
  local identity="${2:-upstream}"
  local delay_ms="${3:-0}"
  local keep_server="${4:-0}"
  if [[ "$keep_server" != "1" ]]; then
    stop_all
  elif [[ -n "${UP_PID:-}" ]] && kill -0 "$UP_PID" 2>/dev/null; then
    kill "$UP_PID" 2>/dev/null || true
    wait "$UP_PID" 2>/dev/null || true
    UP_PID=""
  fi
  UP_LOG="$TMP/upstream-${port}.log"
  : >"$UP_LOG"
  UPSTREAM_IDENTITY="$identity" UPSTREAM_DELAY_MS="$delay_ms" \
    python3 - "$port" "$UP_LOG" <<'PY' &
import json, os, socket, sys, time, base64, hashlib, struct, secrets
port = int(sys.argv[1])
log_path = sys.argv[2]
identity = os.environ.get("UPSTREAM_IDENTITY", "upstream")
delay_ms = int(os.environ.get("UPSTREAM_DELAY_MS", "0"))

WS_GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

def ws_accept(key: str) -> str:
    digest = hashlib.sha1(key.encode() + WS_GUID).digest()
    return base64.b64encode(digest).decode()

def ws_read_frame(sock):
    hdr = sock.recv(2)
    if len(hdr) < 2:
        return None
    masked = (hdr[1] & 0x80) != 0
    length = hdr[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", sock.recv(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", sock.recv(8))[0]
    mask = sock.recv(4) if masked else b""
    payload = b""
    while len(payload) < length:
        chunk = sock.recv(length - len(payload))
        if not chunk:
            break
        payload += chunk
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return payload

def ws_send_text(sock, text: str):
    data = text.encode()
    frame = bytearray([0x81, len(data)])
    frame.extend(data)
    sock.sendall(frame)

def handle_websocket(conn, data: bytes, headers: dict):
    key = headers.get("sec-websocket-key", "")
    accept = ws_accept(key)
    resp = "\r\n".join([
        "HTTP/1.1 101 Switching Protocols",
        "Upgrade: websocket",
        "Connection: Upgrade",
        f"Sec-WebSocket-Accept: {accept}",
        "",
        "",
    ])
    conn.sendall(resp.encode())
    msg = ws_read_frame(conn)
    if msg is not None:
        ws_send_text(conn, msg.decode("latin-1", errors="replace"))
    conn.close()

s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(128)

def parse_request(data: bytes):
    text = data.decode("latin-1", errors="replace")
    lines = text.split("\r\n")
    req_line = lines[0] if lines else ""
    parts = req_line.split()
    method = parts[0] if parts else "GET"
    target = parts[1] if len(parts) > 1 else "/"
    headers = {}
    body = b""
    i = 1
    while i < len(lines):
        if lines[i] == "":
            body = data.split(b"\r\n\r\n", 1)[1] if b"\r\n\r\n" in data else b""
            break
        if ":" in lines[i]:
            k, v = lines[i].split(":", 1)
            headers[k.strip().lower()] = v.strip()
        i += 1
    return method, target, headers, body

while True:
    conn, _ = s.accept()
    data = conn.recv(65536)
    method, target, headers, body = parse_request(data)
    path = target.split("?", 1)[0]
    query = target.split("?", 1)[1] if "?" in target else ""
    if (path == "/slow" or path.endswith("/slow")) and delay_ms > 0:
        time.sleep(delay_ms / 1000.0)
    upgrade = headers.get("upgrade", "").lower()
    if upgrade == "websocket":
        with open(log_path, "a", encoding="utf-8") as f:
            f.write(json.dumps({"identity": identity, "method": method, "path": path, "websocket": True}) + "\n")
        handle_websocket(conn, data, headers)
        continue
    payload = {
        "identity": identity,
        "method": method,
        "path": path,
        "query": query,
        "target": target,
        "headers": headers,
        "body_len": len(body),
        "body": body.decode("latin-1", errors="replace"),
    }
    with open(log_path, "a", encoding="utf-8") as f:
        f.write(json.dumps(payload) + "\n")
    if path == "/slow" or path.endswith("/slow"):
        body_out = b"slow-ok"
        status = "200 OK"
    elif path == "/error":
        status = "500 Internal Server Error"
        body_out = b"upstream-error"
    else:
        status = "200 OK"
        body_out = json.dumps(payload, separators=(",", ":")).encode()
    resp_headers = [
        "HTTP/1.1 " + status,
        f"Content-Length: {len(body_out)}",
        "Content-Type: application/json",
        "Connection: close",
        "Transfer-Encoding: identity",
        "X-Upstream-Identity: " + identity,
    ]
    resp = "\r\n".join(resp_headers) + "\r\n\r\n"
    conn.sendall(resp.encode() + body_out)
    conn.close()
PY
  UP_PID=$!
  sleep 0.15
}

write_config() {
  local listen_port="$1"
  local upstream_port="$2"
  local timeout_ms="${3:-5000}"
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${listen_port}"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${upstream_port}"
timeout_ms = ${timeout_ms}
EOF
}

wait_listen() {
  local port="$1"
  for _ in $(seq 1 60); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('${port}'))); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

start_exyonq() {
  local listen_port="$1"
  local upstream_port="$2"
  local timeout_ms="${3:-5000}"
  stop_all
  start_upstream "$upstream_port" "${4:-upstream}" "${5:-0}"
  write_config "$listen_port" "$upstream_port" "$timeout_ms"
  if [[ ! -x "$EXYONQ_BIN" ]]; then
    cargo build -p exyonq --bin exyonq
  fi
  if [[ ! -x "$EXYONQCTL_BIN" ]]; then
    cargo build -p exyonq --bin exyonqctl 2>/dev/null || cargo build -p exyonqctl 2>/dev/null || true
  fi
  rm -f "$CTRL_SOCK"
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" RUST_LOG=error \
    "$EXYONQ_BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
  SRV_PID=$!
  local base="http://127.0.0.1:${listen_port}"
  for _ in $(seq 1 60); do
    if curl -sf "${base}/api/health" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$SRV_PID" 2>/dev/null; then
      echo "exyonq exited early:" >&2
      cat "$SRV_LOG" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for exyonq" >&2
  cat "$SRV_LOG" >&2 || true
  return 1
}

LISTEN_PORT="$(pick_port)"
UPSTREAM_PORT="$(pick_port)"
BASE="http://127.0.0.1:${LISTEN_PORT}"

start_exyonq "$LISTEN_PORT" "$UPSTREAM_PORT"

# --- GET ---
BODY="$(curl -sf "${BASE}/api/health")"
if echo "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["path"]=="/api/health" and d["method"]=="GET"'; then
  record_pass "GET simple path+method"
else
  record_fail "GET simple path+method body=$BODY"
fi

BODY="$(curl -sf "${BASE}/api/item?a=1&b=two")"
if echo "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["query"]=="a=1&b=two"'; then
  record_pass "GET query preserved"
else
  record_fail "GET query preserved body=$BODY"
fi

BODY="$(curl -sf -H 'Host: proxy.test' -H 'X-Custom: abc' -H 'Connection: keep-alive' "${BASE}/api/headers")"
if echo "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); h=d["headers"]; assert "connection" not in h'; then
  record_pass "GET hop-by-hop request stripped upstream"
else
  record_fail "GET hop-by-hop request filtering body=$BODY"
fi

HDRS="$(curl -sfI "${BASE}/api/health")"
if echo "$HDRS" | grep -qi '^connection:' ; then
  record_fail "GET response still has Connection hop-by-hop"
else
  record_pass "GET response hop-by-hop stripped"
fi

# --- HEAD ---
HEAD_CODE="$(curl -s -o /dev/null -w '%{http_code}' -I "${BASE}/api/head-check")"
HEAD_SIZE="$(curl -s -o /dev/null -w '%{size_download}' -I "${BASE}/api/head-check")"
if [[ "$HEAD_CODE" == "200" ]] && [[ "$HEAD_SIZE" == "0" ]]; then
  record_pass "HEAD status 200 empty downstream body"
else
  record_fail "HEAD code=$HEAD_CODE size=$HEAD_SIZE"
fi
if python3 -c "import json,sys; ok=any(json.loads(l)['method']=='HEAD' for l in open('$UP_LOG')); sys.exit(0 if ok else 1)"; then
  record_pass "HEAD upstream method"
else
  record_fail "HEAD upstream method (log=$UP_LOG)"
fi

# --- POST ---
POST_BODY='{"k":"v"}'
POST_RESP="$(curl -sf -X POST -H 'Content-Type: application/json' -d "$POST_BODY" "${BASE}/api/submit?q=1")"
if echo "$POST_RESP" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["method"]=="POST" and d["query"]=="q=1" and d["body"]=="{\"k\":\"v\"}"'; then
  record_pass "POST body+query+content preserved"
else
  record_fail "POST body preserved resp=$POST_RESP"
fi

# --- upstream down (no listener) ---
DEAD_PORT="$(pick_port)"
stop_all
write_config "$LISTEN_PORT" "$DEAD_PORT" 500
if [[ ! -x "$EXYONQ_BIN" ]]; then
  cargo build -p exyonq --bin exyonq
fi
rm -f "$CTRL_SOCK"
EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" RUST_LOG=error \
  "$EXYONQ_BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
SRV_PID=$!
wait_listen "$LISTEN_PORT" || { record_fail "upstream down server listen"; cat "$SRV_LOG" >&2; }
CODE="$(curl -s -o /dev/null -w '%{http_code}' "${BASE}/api/down" || true)"
if [[ "$CODE" == "502" ]]; then
  record_pass "upstream unavailable -> 502"
else
  record_fail "upstream unavailable code=$CODE"
fi

# --- timeout ---
TIMEOUT_UP="$(pick_port)"
start_exyonq "$LISTEN_PORT" "$TIMEOUT_UP" 200 "slow-up" 2000
CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "${BASE}/api/slow" || true)"
if [[ "$CODE" == "502" ]] || [[ "$CODE" == "504" ]]; then
  record_pass "upstream timeout -> 502/504 (code=$CODE)"
else
  record_fail "upstream timeout code=$CODE"
fi

# --- reload / generation ---
UP_A="$(pick_port)"
UP_B="$(pick_port)"
stop_all
start_upstream "$UP_A" "gen-a"
write_config "$LISTEN_PORT" "$UP_A" 5000
EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" RUST_LOG=error \
  "$EXYONQ_BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
SRV_PID=$!
wait_listen "$LISTEN_PORT" || { record_fail "reload pre server listen"; }
BODY_A="$(curl -sf "${BASE}/api/reload-check")"
ID_A="$(echo "$BODY_A" | python3 -c 'import json,sys; print(json.load(sys.stdin)["identity"])')"
if [[ "$ID_A" == "gen-a" ]]; then
  record_pass "pre-reload upstream identity"
else
  record_fail "pre-reload identity=$ID_A"
fi

start_upstream "$UP_B" "gen-b" 0 1
write_config "$LISTEN_PORT" "$UP_B" 5000
if [[ -x "$EXYONQCTL_BIN" ]] && [[ -S "$CTRL_SOCK" ]]; then
  EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL_SOCK" >/dev/null 2>&1 || true
else
  # Config watcher should pick up the single write in cfg/ (logs stay outside cfg/).
  sleep 0.8
fi
sleep 0.5

BODY_B="$(curl -sf "${BASE}/api/reload-check")"
ID_B="$(echo "$BODY_B" | python3 -c 'import json,sys; print(json.load(sys.stdin)["identity"])')"
if [[ "$ID_B" == "gen-b" ]]; then
  record_pass "post-reload upstream identity"
else
  record_fail "post-reload identity=$ID_B (expected gen-b)"
fi

# --- WebSocket upgrade + bidirectional echo (Rust deterministic, byte-exact) ---
if cargo test -p exyonq-core ws_tunnel_tests -- --test-threads=1 >/dev/null 2>&1 \
  && cargo test -p exyonq-mod-proxy websocket::tests -- --test-threads=1 >/dev/null 2>&1; then
  record_pass "WebSocket 101 + bidirectional echo"
else
  record_fail "WebSocket upgrade/echo"
fi

echo "KD3.4 proxy smoke: PASS=$PASS FAIL=$FAIL"
echo "logs: upstream=$UP_LOG server=$SRV_LOG"
if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
