#!/usr/bin/env bash
# ZF-005 real-flow proof: ExyonQ proxy ↔ independent upstream peer.
# Linux evidence hosts only (Netcup amd64 / Oracle arm64). Forbidden diagnostic-only category excluded.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "LINUX_EVIDENCE_REQUIRED=YES"
  echo "ZF005_REAL_FLOW_PROOF = NOT_LINUX"
  exit 2
fi

ARCH="$(uname -m)"
EV_ROOT="${ZF005_EV_DIR:-$ROOT/.exyonq-local/tmp/zf005-real-flow}"
EV="$EV_ROOT/${ARCH}-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$EV"
echo "ZF005_EV=$EV" | tee "$EV/meta.txt"
echo "ARCH=$ARCH" | tee -a "$EV/meta.txt"
echo "HOST=$(hostname)" | tee -a "$EV/meta.txt"
echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "$EV/meta.txt"
echo "USES_REAL_DATA=YES" | tee -a "$EV/meta.txt"
echo "USES_SIMULATED_DATA=NO" | tee -a "$EV/meta.txt"
echo "USES_KNOWN_TEST_UPSTREAM_RESPONSE=YES" | tee -a "$EV/meta.txt"

pick_port() {
  local start="$1"
  python3 - "$start" <<'PY'
import socket, sys
start = int(sys.argv[1])
for port in range(start, start + 80):
    s = socket.socket()
    try:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("127.0.0.1", port))
    except OSError:
        s.close()
        continue
    s.close()
    print(port)
    raise SystemExit(0)
raise SystemExit("no free port")
PY
}

export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

UPSTREAM_BIN="$ROOT/tools/upstream/target/release/exyonq-upstream"
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/release/exyonq}"

echo "=== build upstream ===" | tee -a "$EV/meta.txt"
cargo build --release --manifest-path "$ROOT/tools/upstream/Cargo.toml" 2>&1 | tee "$EV/build-upstream.log" | tail -20
test -x "$UPSTREAM_BIN"

if [[ ! -x "$EXYONQ_BIN" ]]; then
  echo "=== build exyonq ===" | tee -a "$EV/meta.txt"
  cargo build --release -p exyonq 2>&1 | tee "$EV/build-exyonq.log" | tail -30
fi
test -x "$EXYONQ_BIN"

UP_PORT="$(pick_port 19000)"
LISTEN_PORT="$(pick_port 19100)"
echo "UP_PORT=$UP_PORT LISTEN_PORT=$LISTEN_PORT" | tee -a "$EV/meta.txt"

WWW="$EV/www"
mkdir -p "$WWW"
# Prefer known 1k fixture when present; else peer synthesizes 1024 x bytes.
if [[ -f "$ROOT/benchmarks/scenarios/payloads/www/1k.bin" ]]; then
  cp -f "$ROOT/benchmarks/scenarios/payloads/www/1k.bin" "$WWW/1k.bin"
fi

CFG="$EV/exyonq.toml"
cat >"$CFG" <<TOML
config_version = 1

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UP_PORT}"
timeout_ms = 5000
TOML

BENCH_WWW="$WWW" UPSTREAM_HOST=127.0.0.1 UPSTREAM_PORT="$UP_PORT" \
  "$UPSTREAM_BIN" >"$EV/upstream.log" 2>&1 &
UP_PID=$!
echo "$UP_PID" >"$EV/upstream.pid"

EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$EV/exyonq.sock" \
  "$EXYONQ_BIN" serve --config "$CFG" >"$EV/exyonq.log" 2>&1 &
EX_PID=$!
echo "$EX_PID" >"$EV/exyonq.pid"

cleanup() {
  kill "$EX_PID" "$UP_PID" 2>/dev/null || true
}
trap cleanup EXIT

ready_url() {
  local url="$1" name="$2" n
  for n in $(seq 1 80); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      echo "${name}_ready=YES after=${n}" | tee -a "$EV/meta.txt"
      return 0
    fi
    if ! kill -0 "$UP_PID" 2>/dev/null; then
      echo "upstream died during wait" | tee -a "$EV/meta.txt"
      tail -50 "$EV/upstream.log" || true
      exit 3
    fi
    if ! kill -0 "$EX_PID" 2>/dev/null; then
      echo "exyonq died during wait" | tee -a "$EV/meta.txt"
      tail -80 "$EV/exyonq.log" || true
      exit 3
    fi
    sleep 0.25
  done
  echo "${name}_ready=NO" | tee -a "$EV/meta.txt"
  return 1
}

ready_url "http://127.0.0.1:${UP_PORT}/health" upstream
ready_url "http://127.0.0.1:${LISTEN_PORT}/api/health" exyonq_proxy

MARKER="zf005-$(date -u +%s)-$$"
# Marker travels as query input and must be reflected by the real upstream peer.
CODE_HDR_BODY="$(curl -fsS -D "$EV/echo.hdr" \
  "http://127.0.0.1:${LISTEN_PORT}/api/echo?marker=${MARKER}" -o "$EV/echo.body" -w '%{http_code}')"
echo "echo_via_proxy_code=$CODE_HDR_BODY" | tee -a "$EV/meta.txt"
test "$CODE_HDR_BODY" = "200"
if ! grep -q "$MARKER" "$EV/echo.body"; then
  echo "UPSTREAM_ECHO_MARKER=NOT_PRESENT_IN_BODY" | tee -a "$EV/meta.txt"
  echo "UPSTREAM_REQUEST_ASSERTION=FAIL" | tee -a "$EV/meta.txt"
  cat "$EV/echo.body" | tee -a "$EV/meta.txt" || true
  exit 4
fi
echo "UPSTREAM_ECHO_MARKER=PRESENT" | tee -a "$EV/meta.txt"
echo "UPSTREAM_REQUEST_ASSERTION=PASS" | tee -a "$EV/meta.txt"
# Path-specific observation: must see proxied /api/echo (not only earlier /health).
if ! grep -E 'path=/api/echo|path = /api/echo|/api/echo' "$EV/upstream.log" >/dev/null; then
  echo "UPSTREAM_OBSERVED_REQUEST=NO (missing /api/echo in upstream log)" | tee -a "$EV/meta.txt"
  cat "$EV/upstream.log" | tee -a "$EV/meta.txt" || true
  exit 4
fi
echo "UPSTREAM_OBSERVED_REQUEST=YES (/api/echo in upstream log)" | tee -a "$EV/meta.txt"
echo "UPSTREAM_RECEIVED_REQUEST=YES" | tee -a "$EV/meta.txt"

BODY_FILE="$EV/api.body"
CODE="$(curl -fsS -o "$BODY_FILE" -w '%{http_code}' "http://127.0.0.1:${LISTEN_PORT}/api/")"
LEN="$(wc -c <"$BODY_FILE" | tr -d ' ')"
SHA="$(sha256sum "$BODY_FILE" | awk '{print $1}')"
echo "api_code=$CODE api_len=$LEN api_sha256=$SHA" | tee -a "$EV/meta.txt"
test "$CODE" = "200"
test "$LEN" = "1024"

# Direct upstream body for equality
DIRECT="$EV/api.direct"
curl -fsS -o "$DIRECT" "http://127.0.0.1:${UP_PORT}/api/"
cmp -s "$BODY_FILE" "$DIRECT"
echo "BODY_SHA256_MATCH_DIRECT=YES" | tee -a "$EV/meta.txt"

# Path-specific upstream observation already asserted for /api/echo.

# Failure propagation: kill upstream → expect 5xx
kill "$UP_PID" 2>/dev/null || true
sleep 0.5
FAIL_CODE="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${LISTEN_PORT}/api/" || true)"
echo "failure_propagation_code=$FAIL_CODE" | tee -a "$EV/meta.txt"
case "$FAIL_CODE" in
  502|503|504) echo "FAILURE_PROPAGATION=PASS" | tee -a "$EV/meta.txt" ;;
  *)
    echo "FAILURE_PROPAGATION=FAIL code=$FAIL_CODE" | tee -a "$EV/meta.txt"
    exit 5
    ;;
esac

{
  echo "REAL_EXYONQ_BINARY=YES"
  echo "REAL_CONFIG=YES"
  echo "REAL_EXYONQ_PROCESS=YES"
  echo "REAL_UPSTREAM_PROCESS=YES"
  echo "REAL_TCP_NETWORK=YES"
  echo "REAL_HTTP_PROTOCOL=YES"
  echo "REQUEST_REACHES_UPSTREAM=YES"
  echo "UPSTREAM_OBSERVED_REQUEST=YES"
  echo "RESPONSE_REACHES_CLIENT=YES"
  echo "STATUS_ASSERTION=PASS"
  echo "BODY_ASSERTION=PASS"
  echo "BODY_SHA256=PASS"
  echo "FAILURE_PROPAGATION=PASS"
  echo "ZF005_REAL_FLOW_PROOF=PASS"
} | tee "$EV/verdict.txt"

echo "ZF005_REAL_FLOW_PROOF = PASS"
exit 0
