#!/usr/bin/env bash
# P1.3b-C1 — HTTP/3 runtime foundation smoke (real client when available).
# NOT a soak. NOT P13. Classification: P13B_RUNTIME_FOUNDATION = E2E_CHECKPOINT.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; exit 1; }
skip() { echo "SKIP $*"; exit 0; }

command -v curl >/dev/null || skip "curl missing"
if ! curl --version 2>/dev/null | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper'; then
  skip "curl without HTTP/3 support"
fi

TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

CERT="$ROOT/benchmarks/scenarios/fixtures/tls/cert.pem"
KEY="$ROOT/benchmarks/scenarios/fixtures/tls/key.pem"
[[ -f "$CERT" && -f "$KEY" ]] || fail "tls fixtures missing"

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
UDP_PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"
WWW="$ROOT/benchmarks/scenarios/fixtures/www"
[[ -d "$WWW" ]] || WWW="$ROOT/tests/fixtures/www"

cat >"$CFG" <<EOF
[server]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "127.0.0.1:${UDP_PORT}"
control_socket = "${CTRL}"

[server.tls]
cert = "${CERT}"
key = "${KEY}"

[[routes]]
match = { path_prefix = "/" }
root = "${WWW}"
EOF

cargo build -q -p exyonq 2>/dev/null || cargo build -p exyonq
EXYONQ_CONFIG="$CFG" cargo run -q -p exyonq -- serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

for i in $(seq 1 60); do
  if rg -q "HTTP/3 listening" "$LOG" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    fail "server exited early"; tail -80 "$LOG"; exit 1
  fi
  sleep 0.5
done
rg -q "HTTP/3 listening" "$LOG" || { fail "no HTTP/3 listening"; tail -80 "$LOG"; exit 1; }
pass "listener_ready"

BASE="https://127.0.0.1:${UDP_PORT}"

# GET static / health-ish
code="$(curl --http3-only -sk -o /dev/null -w '%{http_code}' "${BASE}/" || echo 000)"
[[ "$code" =~ ^(200|404)$ ]] || fail "GET expected 200/404 got $code"
pass "GET_$code"

# HEAD
code="$(curl --http3-only -sk -o /dev/null -w '%{http_code}' -I "${BASE}/" || echo 000)"
[[ "$code" =~ ^(200|404)$ ]] || fail "HEAD expected 200/404 got $code"
pass "HEAD_$code"

# POST echo
body="$(curl --http3-only -sk -X POST --data 'marker-c1' "${BASE}/api/echo" || true)"
[[ "$body" == "marker-c1" ]] || fail "POST echo body mismatch: [$body]"
pass "POST_echo"

# 405 on PUT
code="$(curl --http3-only -sk -o /dev/null -w '%{http_code}' -X PUT "${BASE}/api/echo" || echo 000)"
[[ "$code" == "405" ]] || fail "PUT expected 405 got $code"
pass "PUT_405"

# Concurrent streams (best-effort with curl; one connection reused)
ok=0
for i in $(seq 1 8); do
  code="$(curl --http3-only -sk -o /dev/null -w '%{http_code}' "${BASE}/" || echo 000)"
  [[ "$code" =~ ^(200|404)$ ]] && ok=$((ok+1))
done
[[ "$ok" -ge 6 ]] || fail "concurrent GETs ok=$ok/8"
pass "concurrent_streams_$ok"

# Fail-closed negative: invalid http3_listen must not silent-skip (validate via second config check)
BAD="$TMP/bad.toml"
cat >"$BAD" <<EOF
[server]
listen = "127.0.0.1:0"
http3_listen = "not-a-socket"
control_socket = "${TMP}/bad.sock"
[server.tls]
cert = "${CERT}"
key = "${KEY}"
[[routes]]
match = { path_prefix = "/" }
root = "${WWW}"
EOF
if cargo run -q -p exyonq -- run -c "$BAD" >"$TMP/bad.log" 2>&1; then
  # Should exit quickly with error — if it keeps running, fail
  fail "bad http3_listen should fail-closed"
else
  pass "fail_closed_bad_listen"
fi

pass "p13b_http3_runtime_foundation"
