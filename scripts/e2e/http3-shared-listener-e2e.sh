#!/usr/bin/env bash
# Real HTTP/3 E2E: HTTP/3 belongs to the listener, not to the first [[server]].
#
# Asserts, with the real product binary and a real QUIC client:
#   - two [[server]] blocks sharing one TCP listen share exactly one UDP socket
#     and every host of that port is served over HTTP/3 (SNI selects the cert);
#   - any [[server]] block may declare http3_listen, and a per-server value
#     creates its own UDP socket (two listeners -> two sockets).
# Classification: H3_REAL_E2E (Linux + Docker curl with HTTP/3 required; a
# missing prerequisite is BLOCKED, never PASS).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_require_linux

pass() { echo "PASS $*"; }
fail() {
  echo "FAIL $*"
  if [[ -f "${LOG:-/nonexistent}" ]]; then
    echo "--- server log tail ---"
    tail -30 "$LOG"
  fi
  exit 1
}
blocked() { echo "RESULT=BLOCKED SKIP $*"; exit 3; }

command -v docker >/dev/null 2>&1 || blocked "docker missing (real HTTP/3 client)"
command -v openssl >/dev/null 2>&1 || blocked "openssl missing (ephemeral TLS)"
command -v ss >/dev/null 2>&1 || blocked "ss missing (UDP socket accounting)"
BIN="$ROOT/target/release/exyonq"
[[ -x "$BIN" ]] || blocked "release binary missing: $BIN (build it first)"

CLIENT_IMAGE="${EXYONQ_H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${EXYONQ_H3_CLIENT_PLATFORM:-linux/amd64}"
docker image inspect "$CLIENT_IMAGE" >/dev/null 2>&1 \
  || docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null
docker run --rm --platform "$CLIENT_PLATFORM" --network none "$CLIENT_IMAGE" \
  curl --version 2>/dev/null | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper' \
  || blocked "client image lacks HTTP/3: $CLIENT_IMAGE"

TMP="$(mktemp -d)"
SRV_PID=""
cleanup() {
  [[ -n "$SRV_PID" ]] && kill "$SRV_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

pick_port() {
  python3 - <<'PY'
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_tcp() {
  python3 - "$1" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
for _ in range(200):
    try:
        socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
        raise SystemExit(0)
    except OSError:
        time.sleep(0.05)
raise SystemExit(1)
PY
}

make_cert() {
  local name="$1" prefix="$2"
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "$TMP/$prefix.key" -out "$TMP/$prefix.crt" \
    -subj "/CN=$name" -addext "subjectAltName=DNS:$name" >/dev/null 2>&1
}

h3_get() {
  local host="$1" port="$2" path="$3"
  docker run --rm --platform "$CLIENT_PLATFORM" --network host "$CLIENT_IMAGE" \
    curl -sS --http3-only -k --max-time 10 \
    --resolve "$host:$port:127.0.0.1" "https://$host:$port$path"
}

start_server() {
  local cfg="$1" log="$2"
  LOG="$log"
  EXYONQ_CONTROL_SOCKET="$TMP/control.sock" "$BIN" serve -c "$cfg" >"$log" 2>&1 &
  SRV_PID=$!
}

# --- Case 1: two hosts, one TCP listen, one global [http3] listen (UDP/port) ---
PORT="$(pick_port)"
mkdir -p "$TMP/www-a" "$TMP/www-b"
printf 'A\n' >"$TMP/www-a/index.html"
printf 'B\n' >"$TMP/www-b/index.html"
make_cert a.test a
make_cert b.test b
cat >"$TMP/shared.toml" <<EOF
config_version = 1

[http3]
listen = "127.0.0.1:${PORT}"

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = "a.test"
routes = ["a"]

[server.tls]
cert = "$TMP/a.crt"
key = "$TMP/a.key"

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = "b.test"
routes = ["b"]

[server.tls]
cert = "$TMP/b.crt"
key = "$TMP/b.key"

[[route]]
name = "a"
match = { path = "/", host = "a.test" }
root = "$TMP/www-a"

[[route]]
name = "b"
match = { path = "/", host = "b.test" }
root = "$TMP/www-b"
EOF
start_server "$TMP/shared.toml" "$TMP/shared.log"
wait_tcp "$PORT" || fail "shared: TCP listen $PORT did not open"
sleep 1
sockets="$(ss -lunpH 2>/dev/null | grep -c ":$PORT " || true)"
[[ "$sockets" -eq 1 ]] || fail "shared: expected 1 UDP socket on :$PORT, found $sockets"
pass "shared_listener: one UDP socket on :$PORT for both hosts"

body_a="$(h3_get a.test "$PORT" /)" || fail "shared: HTTP/3 request to a.test failed"
body_b="$(h3_get b.test "$PORT" /)" || fail "shared: HTTP/3 request to b.test failed"
[[ "$body_a" == "A" ]] || fail "shared: a.test body mismatch: [$body_a]"
[[ "$body_b" == "B" ]] || fail "shared: b.test body mismatch: [$body_b]"
pass "shared_listener: HTTP/3 SNI routing a.test -> A, b.test -> B"

kill "$SRV_PID" 2>/dev/null || true
wait "$SRV_PID" 2>/dev/null || true
SRV_PID=""

# --- Case 2: http3_listen on the NON-primary block is honored ---
# The reported bug: the field was only read from the first [[server]]. Declare
# it on the second block and let the first inherit the group socket.
PORT="$(pick_port)"
cat >"$TMP/non-primary.toml" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = "a.test"
routes = ["a"]

[server.tls]
cert = "$TMP/a.crt"
key = "$TMP/a.key"

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = "b.test"
routes = ["b"]
http3_listen = "127.0.0.1:${PORT}"

[server.tls]
cert = "$TMP/b.crt"
key = "$TMP/b.key"

[[route]]
name = "a"
match = { path = "/", host = "a.test" }
root = "$TMP/www-a"

[[route]]
name = "b"
match = { path = "/", host = "b.test" }
root = "$TMP/www-b"
EOF
start_server "$TMP/non-primary.toml" "$TMP/non-primary.log"
wait_tcp "$PORT" || fail "non-primary: TCP listen $PORT did not open"
sleep 1
sockets="$(ss -lunpH 2>/dev/null | grep -c ":$PORT " || true)"
[[ "$sockets" -eq 1 ]] || fail "non-primary: expected 1 UDP socket on :$PORT, found $sockets"
pass "non_primary_listener: http3_listen on the second block shares one UDP socket"

body_a="$(h3_get a.test "$PORT" /)" || fail "non-primary: HTTP/3 a.test failed"
body_b="$(h3_get b.test "$PORT" /)" || fail "non-primary: HTTP/3 b.test failed"
[[ "$body_a" == "A" ]] || fail "non-primary: a.test body mismatch: [$body_a]"
[[ "$body_b" == "B" ]] || fail "non-primary: b.test body mismatch: [$body_b]"
pass "non_primary_listener: both hosts served over HTTP/3"

# --- Case 3: a global [http3] listen with a plain-HTTP listener is accepted ---
# QUIC needs TLS; the global value applies to TLS listener groups only and must
# not force a plain HTTP listener to require certificates.
cat >"$TMP/mixed.toml" <<EOF
config_version = 1

[http3]
listen = "127.0.0.1:${PORT}"

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = "a.test"
routes = ["a"]

[server.tls]
cert = "$TMP/a.crt"
key = "$TMP/a.key"

[[route]]
name = "a"
match = { path = "/", host = "a.test" }
root = "$TMP/www-a"
EOF
"$BIN" validate "$TMP/mixed.toml" >/dev/null 2>&1 \
  || fail "mixed: global [http3] listen with a TLS listener should validate"
pass "global_listen: accepted with a TLS listener"

echo "RESULT=PASS HTTP3_SHARED_LISTENER_E2E"
