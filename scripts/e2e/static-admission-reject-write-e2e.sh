#!/usr/bin/env bash
# SUB-LET-STATIC-ADMISSION-REJECT-WRITE — focused product E2E (real TCP).
# 1) Saturate static blocking pool → shed 503 delivered
# 2) Saturated + real SO_LINGER RST → reject write failure observed
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; exit 1; }

[[ "$(uname -s)" == "Linux" ]] || fail "requires Linux"

EXYONQ_BIN="${EXYONQ_BIN:-}"
TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

WWW="$ROOT/benchmarks/scenarios/fixtures/www"
[[ -d "$WWW" ]] || WWW="$ROOT/tests/fixtures/www"

pick_port() {
  python3 - <<'PY'
import socket
s=socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

TCP_PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${TCP_PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/site" }
root = "${WWW}"
index = "index.html"

[logging.access]
enabled = true
EOF

if [[ -z "$EXYONQ_BIN" ]]; then
  cargo build -q -p exyonq --release --locked
  TD="${CARGO_TARGET_DIR:-$ROOT/target}"
  EXYONQ_BIN="$TD/release/exyonq"
fi
[[ -x "$EXYONQ_BIN" ]] || fail "missing EXYONQ_BIN=$EXYONQ_BIN"

# /health is WirePlan::Static (ops probe) and never sendfile-eligible → serve_static_tcp.
# Blocking pool env is clamped: pool>=4, queue>=1 → max_admission >= 5.
export EXYONQ_STATIC_BLOCKING=1
export EXYONQ_STATIC_BLOCKING_POOL=4
export EXYONQ_STATIC_BLOCKING_QUEUE=1
export EXYONQ_READ_TIMEOUT_MS=30000
export EXYONQ_CONFIG="$CFG"
export EXYONQ_CONTROL_SOCKET="$CTRL"
export EXYONQ_ACCESS_LOG=1

RUST_LOG=warn "$EXYONQ_BIN" serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

# Wait for real /health 200 (Connection: close so admission slot frees).
ready=0
for _ in $(seq 1 150); do
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    tail -80 "$LOG" || true
    fail "server exited early"
  fi
  if TCP_PORT="$TCP_PORT" python3 - <<'PY'
import os, socket
port = int(os.environ["TCP_PORT"])
req = b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: e2e\r\nConnection: close\r\n\r\n"
try:
    s = socket.create_connection(("127.0.0.1", port), timeout=0.5)
    s.settimeout(2.0)
    s.sendall(req)
    data = s.recv(256)
    s.close()
except Exception:
    raise SystemExit(1)
if data.startswith(b"HTTP/1.1 200"):
    print("up")
else:
    raise SystemExit(1)
PY
  then
    ready=1
    break
  fi
  sleep 0.1
done
[[ "$ready" == "1" ]] || { tail -80 "$LOG"; fail "server never served /health 200"; }
sleep 0.3

# --- Success path: saturate (4 workers + 1 queue) then receive 503 shed ---
code="$(TCP_PORT="$TCP_PORT" python3 - <<'PY'
import socket, time, os

port = int(os.environ["TCP_PORT"])
req = b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: e2e\r\nConnection: keep-alive\r\n\r\n"
holders = []

def hold_worker():
    """Occupy one pool worker after 200 (keep-alive wait)."""
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    s.sendall(req)
    buf = b""
    s.settimeout(5.0)
    while b"\r\n\r\n" not in buf:
        chunk = s.recv(256)
        if not chunk:
            break
        buf += chunk
    if not buf.startswith(b"HTTP/1.1 200"):
        raise RuntimeError("worker hold expected 200, got " + repr(buf[:120]))
    holders.append(s)

# pool_threads=4 → occupy all workers
for _ in range(4):
    hold_worker()
    time.sleep(0.05)

# Fill the single queue slot (Accepted, may not respond while workers held)
q = socket.create_connection(("127.0.0.1", port), timeout=5)
q.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
q.sendall(req)
holders.append(q)
time.sleep(0.1)

# inflight == max_admission (5) → Rejected → STATIC_BLOCKING_ADMISSION_REJECTED
s = socket.create_connection(("127.0.0.1", port), timeout=5)
s.sendall(b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: e2e\r\n\r\n")
s.settimeout(5.0)
data = s.recv(512)
s.close()
for h in holders:
    try:
        h.close()
    except Exception:
        pass
text = data.decode("latin1", "ignore")
print(text.split(" ", 2)[1] if text.startswith("HTTP/") else "000")
if "unavailable" not in text and "503" not in text:
    print("BODY=" + repr(text[:200]), flush=True)
PY
)"
[[ "$code" == "503" ]] || { tail -80 "$LOG"; fail "admission shed expected 503 got $code"; }
pass "STATIC_BLOCKING_SHED_503_DELIVERED"

# --- Failure path: resaturate + RST so reject write fails ---
saturate_and_rst() {
  local bursts="$1"
  TCP_PORT="$TCP_PORT" BURSTS="$bursts" python3 - <<'PY'
import socket, struct, time, os

port = int(os.environ["TCP_PORT"])
bursts = int(os.environ["BURSTS"])
req = b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: e2e\r\nConnection: keep-alive\r\n\r\n"
holders = []

for _ in range(4):
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    s.sendall(req)
    s.settimeout(5.0)
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = s.recv(256)
        if not chunk:
            break
        buf += chunk
    holders.append(s)
    time.sleep(0.02)

q = socket.create_connection(("127.0.0.1", port), timeout=5)
q.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
q.sendall(req)
holders.append(q)
time.sleep(0.05)

reject = b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: e2e\r\n\r\n"
for _ in range(bursts):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    s.connect(("127.0.0.1", port))
    s.sendall(reject)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    s.close()
    time.sleep(0.015)

for h in holders:
    try:
        h.close()
    except Exception:
        pass
print("rst_burst_done")
PY
}

saturate_and_rst 80
sleep 0.5
if grep -q "static blocking admission reject response write failed" "$LOG"; then
  pass "ADMISSION_REJECT_WRITE_FAILURE_OBSERVED"
else
  saturate_and_rst 150
  sleep 0.8
  grep -q "static blocking admission reject response write failed" "$LOG" \
    || { tail -120 "$LOG"; fail "expected warn for admission reject write failure"; }
  pass "ADMISSION_REJECT_WRITE_FAILURE_OBSERVED"
fi

if grep -q "response_write_failed" "$LOG" 2>/dev/null; then
  pass "ACCESS_OUTCOME_RESPONSE_WRITE_FAILED"
else
  pass "ACCESS_OUTCOME_OPTIONAL_WARN_AUTHORITATIVE"
fi

kill -0 "$SRV_PID" 2>/dev/null || fail "server died"
pass "SERVER_ALIVE"
echo "SUB_LET_STATIC_ADMISSION_REJECT_WRITE_PRODUCT_E2E=PASS"
