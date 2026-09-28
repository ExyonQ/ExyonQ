#!/usr/bin/env bash
# SUB-LET-CTRL-WRITE-FALSE-OK — real Unix control socket product E2E.
# 1) Oversize command → error JSON delivered (ok:false)
# 2) Connect+RST before reply → server observes write failure (warn), no false Ok silence
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
EOF

if [[ -z "$EXYONQ_BIN" ]]; then
  cargo build -q -p exyonq --release --locked
  TD="${CARGO_TARGET_DIR:-$ROOT/target}"
  EXYONQ_BIN="$TD/release/exyonq"
fi
[[ -x "$EXYONQ_BIN" ]] || fail "missing EXYONQ_BIN=$EXYONQ_BIN"

export EXYONQ_CONFIG="$CFG"
export EXYONQ_CONTROL_SOCKET="$CTRL"
RUST_LOG=warn "$EXYONQ_BIN" serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

for _ in $(seq 1 100); do
  if [[ -S "$CTRL" ]]; then
    break
  fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    tail -80 "$LOG" || true
    fail "server exited before control socket appeared"
  fi
  sleep 0.1
done
[[ -S "$CTRL" ]] || fail "control socket missing at $CTRL"

# --- Success path: oversize → error JSON delivered ---
python3 - <<PY
import socket, sys
MAX = 4096
req = b"x" * (MAX + 8) + b"\n"
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect("${CTRL}")
s.sendall(req)
data = s.recv(4096)
s.close()
text = data.decode("utf-8", "replace")
if '"ok":false' not in text and '"ok": false' not in text:
    print("body=", text[:500], file=sys.stderr)
    raise SystemExit("expected ok:false error JSON")
print("oversize_error_json_ok")
PY
pass "OVERSIZE_ERROR_JSON_DELIVERED"

# --- Failure path: trigger empty/error reply then RST so write fails ---
python3 - <<PY
import socket, struct, time
MAX = 4096
req = b"x" * (MAX + 8) + b"\n"
for _ in range(80):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect("${CTRL}")
    s.sendall(req)
    # Real OS RST on close
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    s.close()
    time.sleep(0.02)
print("rst_burst_done")
PY

sleep 0.5
if grep -q "control response write" "$LOG"; then
  pass "CTRL_RESPONSE_WRITE_FAILURE_OBSERVED"
else
  python3 - <<PY
import socket, struct, time
MAX = 4096
req = b"x" * (MAX + 8) + b"\n"
for _ in range(160):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect("${CTRL}")
    s.sendall(req)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    s.close()
    time.sleep(0.01)
PY
  sleep 0.8
  grep -q "control response write" "$LOG" \
    || { tail -120 "$LOG"; fail "expected warn containing control response write"; }
  pass "CTRL_RESPONSE_WRITE_FAILURE_OBSERVED"
fi

# Known-good status still works (exyonqctl path / line protocol)
python3 - <<PY
import socket
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect("${CTRL}")
s.sendall(b"status\n")
data = s.recv(4096)
s.close()
text = data.decode("utf-8", "replace")
assert '"ok":true' in text or '"ok": true' in text, text
assert '"command":"status"' in text or '"command": "status"' in text, text
print("status_ok")
PY
pass "STATUS_STILL_WORKS"

kill -0 "$SRV_PID" 2>/dev/null || fail "server died"
pass "SERVER_ALIVE"
echo "SUB_LET_CTRL_WRITE_FALSE_OK_PRODUCT_E2E=PASS"
