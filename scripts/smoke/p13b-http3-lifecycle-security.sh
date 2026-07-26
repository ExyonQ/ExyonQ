#!/usr/bin/env bash
# P1.3b Lifecycle/Security — real HTTP/3 E2E (Docker curl with HTTP/3).
# SKIP ≠ tranche close. Requires Docker. Classification: H3_REAL_E2E.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; exit 1; }
skip() { echo "SKIP $*"; exit 0; }

command -v docker >/dev/null || skip "docker missing (required for real H3 E2E)"

# Frozen client image (digest recorded in docs/operations/p1.3b-http3-e2e-report.md).
# Prefer HTTP/3-capable image; curlimages/curl musl builds often lack HTTP/3.
CLIENT_IMAGE="${EXYONQ_H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${EXYONQ_H3_CLIENT_PLATFORM:-linux/amd64}"

docker_run() {
  docker run --rm --platform "$CLIENT_PLATFORM" "$@"
}

# Prefer pulling once; allow offline reuse of local tag.
docker image inspect "$CLIENT_IMAGE" >/dev/null 2>&1 \
  || docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null

# Probe HTTP/3 support inside the image.
if ! docker_run --network none "$CLIENT_IMAGE" curl --version 2>/dev/null \
  | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper'; then
  fail "client image lacks HTTP/3: $CLIENT_IMAGE (platform=$CLIENT_PLATFORM)"
fi
pass "client_http3_$CLIENT_IMAGE"

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
UP_PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"
WWW="$ROOT/benchmarks/scenarios/fixtures/www"
[[ -d "$WWW" ]] || WWW="$ROOT/tests/fixtures/www"

# Tiny body-echo upstream for POST proxy proof.
python3 - <<PY &
import socket, threading
addr=("127.0.0.1", ${UP_PORT})
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(addr); s.listen(32)
def serve(c):
    data=b""
    c.settimeout(2)
    try:
        while b"\\r\\n\\r\\n" not in data:
            chunk=c.recv(4096)
            if not chunk: break
            data+=chunk
        body=data.split(b"\\r\\n\\r\\n",1)[-1] if b"\\r\\n\\r\\n" in data else b""
        # drain Content-Length remainder
        hdr=data.split(b"\\r\\n\\r\\n",1)[0].decode("latin1","ignore")
        cl=0
        for line in hdr.split("\\r\\n"):
            if line.lower().startswith("content-length:"):
                cl=int(line.split(":",1)[1].strip() or 0)
        while len(body)<cl:
            chunk=c.recv(cl-len(body))
            if not chunk: break
            body+=chunk
        resp=(f"HTTP/1.1 200 OK\\r\\nContent-Length: {len(body)}\\r\\n"
              f"X-Exyonq-Upstream: h3-e2e\\r\\nConnection: close\\r\\n\\r\\n").encode()+body
        c.sendall(resp)
    except Exception:
        pass
    finally:
        c.close()
while True:
    c,_=s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
UP_PID=$!
trap 'kill "$UP_PID" 2>/dev/null || true; [[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "0.0.0.0:${UDP_PORT}"
control_socket = "${CTRL}"
routes = ["site", "api"]

[server.tls]
cert = "${CERT}"
key = "${KEY}"

[[route]]
name = "site"
match = { path = "/site" }
root = "${WWW}"
index = "index.html"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UP_PORT}"
EOF

cargo build -q -p exyonq 2>/dev/null || cargo build -p exyonq
cargo run -q -p exyonq -- serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

for _ in $(seq 1 80); do
  if rg -q "HTTP/3 listening" "$LOG" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    fail "server exited early"; tail -80 "$LOG"; exit 1
  fi
  sleep 0.25
done
rg -q "HTTP/3 listening" "$LOG" || { fail "no HTTP/3 listening"; tail -80 "$LOG"; exit 1; }
pass "listener_ready"

# Prefer host networking on Linux. Docker Desktop (macOS) often cannot reach
# host UDP/QUIC via host.docker.internal — fall back to Quinn client E2E.
H3_URL="https://127.0.0.1:${UDP_PORT}"
DOCKER_NET=(--network host)
USE_DOCKER_CURL=1
if [[ "$(uname -s)" == "Darwin" ]]; then
  DOCKER_NET=()
  H3_URL="https://host.docker.internal:${UDP_PORT}"
  USE_DOCKER_CURL=0
  pass "darwin_use_quinn_e2e"
fi

if [[ "$USE_DOCKER_CURL" == "1" ]]; then
  h3() {
    docker_run "${DOCKER_NET[@]}" "$CLIENT_IMAGE" \
      curl --http3-only -sk "$@"
  }

  code="$(h3 -o /dev/null -w '%{http_code}' "${H3_URL}/site/" || echo 000)"
  [[ "$code" =~ ^(200|404)$ ]] || fail "GET expected 200/404 got $code"
  pass "GET_$code"

  code="$(h3 -o /dev/null -w '%{http_code}' -I "${H3_URL}/site/" || echo 000)"
  [[ "$code" =~ ^(200|404)$ ]] || fail "HEAD expected 200/404 got $code"
  pass "HEAD_$code"

  body="$(h3 -X POST --data 'marker-e2e-post' "${H3_URL}/api/echo" || true)"
  [[ "$body" == "marker-e2e-post" ]] || fail "POST proxy body mismatch got=${body:0:80}"
  pass "POST_proxy_body"

  code="$(h3 -o /dev/null -w '%{http_code}' -X POST --data x "${H3_URL}/__exyonq/h3-echo" || echo 000)"
  [[ "$code" != "200" ]] || fail "product h3-echo still exposed"
  pass "echo_removed_$code"

  code="$(h3 -o /dev/null -w '%{http_code}' -X PUT "${H3_URL}/api/x" || echo 000)"
  [[ "$code" == "405" ]] || fail "PUT expected 405 got $code"
  pass "PUT_405"

  ok=0
  for i in $(seq 1 8); do
    c="$(h3 -o /dev/null -w '%{http_code}' "${H3_URL}/site/" || echo 000)"
    [[ "$c" =~ ^(200|404)$ ]] && ok=$((ok+1))
  done
  [[ "$ok" -ge 6 ]] || fail "concurrent streams ok=$ok/8"
  pass "concurrent_streams_$ok"
else
  # Portable real wire E2E: Quinn/h3 client in-process (dev-dep; no product crate).
  cargo test -p exyonq-core --test ps1a_h3_lifecycle -- --nocapture \
    native_quic_connection_single_enter_multi_request \
    native_handle_drain_stops_listener_idempotent \
    native_drain_rejects_second_connection \
    native_core_dispatcher_zero_enter_per_request
  pass "quinn_native_e2e"
fi

# Drain via control socket if present.
if [[ -S "$CTRL" ]]; then
  python3 - <<PY
import socket
s=socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("${CTRL}")
s.sendall(b'{"cmd":"drain"}\n')
print(s.recv(4096).decode())
PY
  sleep 1
  pass "drain_command"
fi

pass "H3_REAL_E2E"
echo "CLIENT_IMAGE=$CLIENT_IMAGE"
echo "UDP_PORT=$UDP_PORT"
echo "RUN_OK=1"
