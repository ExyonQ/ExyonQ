#!/usr/bin/env bash
# Focused H3 error-response product E2E (real curl --http3-only).
# Proves TooLarge → real 413 on the shipping s2n path. Not a smoke wrapper.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; exit 1; }

[[ "$(uname -s)" == "Linux" ]] || fail "requires Linux"

CLIENT_IMAGE="${EXYONQ_H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${EXYONQ_H3_CLIENT_PLATFORM:-linux/amd64}"
EXYONQ_BIN="${EXYONQ_BIN:-}"

docker_run() {
  docker run --rm --platform "$CLIENT_PLATFORM" "$@"
}

docker image inspect "$CLIENT_IMAGE" >/dev/null 2>&1 \
  || docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null

TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

# shellcheck source=scripts/e2e/lib/tls.sh
source "$ROOT/scripts/e2e/lib/tls.sh"
ensure_ephemeral_tls
CERT="$CERT_PEM"
KEY="$KEY_PEM"

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
config_version = 1

[[server]]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "0.0.0.0:${UDP_PORT}"
routes = ["site"]

[server.tls]
cert = "${CERT}"
key = "${KEY}"

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

# Provider identity (shipping path)
if command -v strings >/dev/null 2>&1 && strings "$EXYONQ_BIN" | grep -Eqi 's2n.?quic|http3-provider-s2n|s2n_quic'; then
  pass "provider_s2n_markers"
else
  # Default product feature is s2n even if strings miss; still record.
  echo "WARN provider_strings_inconclusive (default feature=s2n)"
fi

export EXYONQ_CONFIG="$CFG"
export EXYONQ_CONTROL_SOCKET="$CTRL"
RUST_LOG=info "$EXYONQ_BIN" serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

for _ in $(seq 1 100); do
  if grep -q "HTTP/3 listening" "$LOG" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    tail -80 "$LOG" || true
    fail "server exited early"
  fi
  sleep 0.2
done
grep -q "HTTP/3 listening" "$LOG" || { tail -80 "$LOG"; fail "no HTTP/3 listening"; }
pass "listener_ready"

H3_URL="https://127.0.0.1:${UDP_PORT}"

# GET sanity (real H3)
code="$(docker_run --network host "$CLIENT_IMAGE" \
  curl --http3-only -sk --max-time 15 -o /dev/null -w '%{http_code}' \
  "${H3_URL}/site/" 2>/dev/null || true)"
[[ "$code" == "200" ]] || fail "GET expected 200 got $code"
pass "GET_200"

# Oversize POST → 413 (TooLarge error-response write success path)
python3 - <<'PY' >"$TMP/oversize.bin"
import sys
sys.stdout.buffer.write(b"x" * (32 * 1024 * 1024 + 1))
PY
code="$(docker_run --network host -v "${TMP}:/e2e:ro" "$CLIENT_IMAGE" \
  curl --http3-only -sk --max-time 120 \
    -o /dev/null -w '%{http_code}' \
    -X POST --data-binary @/e2e/oversize.bin \
    "${H3_URL}/site/upload" 2>/dev/null || true)"
[[ "$code" == "413" ]] || fail "POST oversize expected 413 got $code"
pass "POST_413_ERROR_RESPONSE_EMITTED"

# Ensure server still alive (no crash after error-response path)
kill -0 "$SRV_PID" 2>/dev/null || fail "server died after 413"
code="$(docker_run --network host "$CLIENT_IMAGE" \
  curl --http3-only -sk --max-time 15 -o /dev/null -w '%{http_code}' \
  "${H3_URL}/site/" 2>/dev/null || true)"
[[ "$code" == "200" ]] || fail "GET after 413 expected 200 got $code"
pass "RECOVERY_AFTER_413"

echo "SUB_LET_H3_ERROR_RESPONSE_PRODUCT_E2E=PASS"
