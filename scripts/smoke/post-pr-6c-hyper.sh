#!/usr/bin/env bash
# Post-PR-6c Hyper-only smoke (not official benchmark, no claims).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

COMMIT="$(git rev-parse --short HEAD)"
echo "=== post-PR-6c Hyper smoke @ $COMMIT ==="

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

UPSTREAM_PORT="$(pick_port)"
LISTEN_PORT="$(pick_port)"

cleanup() {
  [[ -n "${UP_PID:-}" ]] && kill "$UP_PID" 2>/dev/null || true
  [[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true
  rm -f "$CFG"
}
trap cleanup EXIT

# Mock upstream (echo + health)
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
    body = "upstream-ok" if "/api/" in req else "echo"
    resp = (
        f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n{body}"
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
routes = ["site", "api"]

[[route]]
name = "site"
match = { path = "/site" }
root = "benchmarks/scenarios/payloads/www"
index = "index.html"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UPSTREAM_PORT}"
timeout_ms = 5000
EOF

RUST_LOG=error "${ROOT}/target/debug/exyonq" serve -c "$CFG" > /tmp/exyonq-smoke-serve.log 2>&1 &
SRV_PID=$!

BASE="http://127.0.0.1:${LISTEN_PORT}"
PASS=0
WATCH=0
FAIL=0

ready=0
for _ in $(seq 1 20); do
  if curl -sf -o /dev/null "$BASE/health" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.25
done
if [[ "$ready" -ne 1 ]]; then
  echo "FAIL  server did not become ready"
  tail -20 /tmp/exyonq-smoke-serve.log || true
  exit 1
fi

check() {
  local name="$1" expect="$2"
  shift 2
  if "$@"; then
    echo "PASS  $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL  $name (expected $expect)"
    FAIL=$((FAIL + 1))
  fi
}

check_watch() {
  local name="$1" reason="$2"
  shift 2
  if "$@"; then
    echo "PASS  $name"
    PASS=$((PASS + 1))
  else
    echo "WATCH $name — $reason"
    WATCH=$((WATCH + 1))
  fi
}

# Static Hyper
check "static GET /site/index.html" "200" \
  bash -c 'curl -sf -o /tmp/smoke-static-body -w "%{http_code}" "$1/site/index.html" | grep -qx 200' _ "$BASE"
check "static GET body" "non-empty" \
  bash -c 'test -s /tmp/smoke-static-body'

check "static HEAD /site/index.html" "200" \
  bash -c 'curl -sf -I -o /dev/null -w "%{http_code}" -X HEAD "$1/site/index.html" | grep -qx 200' _ "$BASE"

# P1/P2/P3 static assets (Hyper or wire depending on path; flags OFF default)
check "P1 GET /site/1k.bin" "200" \
  bash -c 'curl -sf -o /dev/null -w "%{http_code}" "$1/site/1k.bin" | grep -qx 200' _ "$BASE"

check_watch "P2 GET /site/64k.bin" "host may use blocking/sendfile on Linux only" \
  bash -c 'curl -sf -o /dev/null -w "%{http_code}" "$1/site/64k.bin" | grep -qx 200' _ "$BASE"

check_watch "P3 GET /site/1m.bin" "host may use sendfile/Linux only" \
  bash -c 'curl -sf -o /dev/null -w "%{http_code}" "$1/site/1m.bin" | grep -qx 200' _ "$BASE"

# Proxy Hyper
check "proxy GET /api/health" "200 + upstream-ok" \
  bash -c 'curl -sf "$1/api/health" | grep -qx upstream-ok' _ "$BASE"

check "proxy HEAD /api/health" "200" \
  bash -c 'curl -sf -I -o /dev/null -w "%{http_code}" -X HEAD "$1/api/health" | grep -qx 200' _ "$BASE"

check "proxy POST /api/submit" "200" \
  bash -c 'curl -sf -o /dev/null -w "%{http_code}" -X POST "$1/api/submit" -d x=1 | grep -qx 200' _ "$BASE"

# Wire legacy raw GET /api/*
WIRE_OUT="$(mktemp)"
printf 'GET /api/wire HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' \
  | nc -w 3 127.0.0.1 "$LISTEN_PORT" >"$WIRE_OUT" || true
check "wire GET /api/*" "200 upstream body" \
  bash -c 'grep -q "HTTP/1.1 200" "$1" && grep -q "upstream-ok\\|echo" "$1"' _ "$WIRE_OUT"

echo ""
echo "=== summary @ $COMMIT ==="
echo "PASS=$PASS WATCH=$WATCH FAIL=$FAIL"
if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
exit 0
