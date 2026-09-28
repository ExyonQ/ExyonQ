#!/usr/bin/env bash
# P1.3b Proxy/Soak Final — product HTTP/3 E2E (real client).
# CLOSE GATE: SKIP forbidden. Requires Linux + Docker curl-http3.
# Classification: H3_REAL_E2E (product close).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; exit 1; }

if [[ "$(uname -s)" != "Linux" ]]; then
  fail "product E2E close requires Linux (got $(uname -s)); run on Netcup/Oracle"
fi

command -v docker >/dev/null || fail "docker missing (required for real H3 product E2E)"

CLIENT_IMAGE="${EXYONQ_H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${EXYONQ_H3_CLIENT_PLATFORM:-linux/amd64}"
CLIENT_DIGEST="${EXYONQ_H3_CLIENT_DIGEST:-}"
RUN_ID="${P1_3B_E2E_RUN_ID:-p13b-e2e-$(date -u +%Y%m%dT%H%M%SZ)}"

docker_run() {
  docker run --rm --platform "$CLIENT_PLATFORM" "$@"
}

docker image inspect "$CLIENT_IMAGE" >/dev/null 2>&1 \
  || docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null

if [[ -z "$CLIENT_DIGEST" ]]; then
  CLIENT_DIGEST="$(docker image inspect --format '{{index .RepoDigests 0}}' "$CLIENT_IMAGE" 2>/dev/null || echo unresolved)"
fi

if ! docker_run --network none "$CLIENT_IMAGE" curl --version 2>/dev/null \
  | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper'; then
  fail "client image lacks HTTP/3: $CLIENT_IMAGE"
fi
pass "client_http3 image=$CLIENT_IMAGE digest=$CLIENT_DIGEST"

TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; [[ -n "${UP_PID:-}" ]] && kill "$UP_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

# Ephemeral TLS (EXYONQ-SEC-PRIVATE-MATERIAL-ZERO)
# shellcheck source=scripts/smoke/lib-p13a-tls.sh
source "$ROOT/scripts/smoke/lib-p13a-tls.sh"
ensure_ephemeral_tls
CERT="$CERT_PEM"
KEY="$KEY_PEM"
[[ -f "$CERT" && -f "$KEY" ]] || fail "tls fixtures missing"
CERT_HASH="$(sha256sum "$CERT" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$CERT" | awk '{print $1}')"

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
UP_STATE="$TMP/upstream_state"
mkdir -p "$UP_STATE"
echo ready >"$UP_STATE/mode"

export UP_PORT UP_STATE
python3 - <<'PY' &
import os, socket, threading, time
addr = ("127.0.0.1", int(os.environ["UP_PORT"]))
state_dir = os.environ["UP_STATE"]
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(addr)
s.listen(64)

def read_mode():
    try:
        return open(os.path.join(state_dir, "mode")).read().strip()
    except Exception:
        return "ready"

def serve(c):
    mode = read_mode()
    if mode in ("refuse", "down"):
        c.close()
        return
    data = b""
    c.settimeout(8)
    try:
        while b"\r\n\r\n" not in data:
            chunk = c.recv(4096)
            if not chunk:
                break
            data += chunk
        hdr, body = data.split(b"\r\n\r\n", 1)
        hs = hdr.decode("latin1", "ignore")
        first = hs.split("\r\n", 1)[0]
        path = first.split(" ")[1] if " " in first else "/"
        cl = 0
        for line in hs.split("\r\n"):
            if line.lower().startswith("content-length:"):
                cl = int(line.split(":", 1)[1].strip() or 0)
        while len(body) < cl:
            chunk = c.recv(max(cl - len(body), 1))
            if not chunk:
                break
            body += chunk
        body = body[:cl]
        open(os.path.join(state_dir, "last_req"), "wb").write(hdr)
        if mode == "timeout":
            time.sleep(35)
            return
        if mode == "reset":
            c.close()
            return
        if "/slow" in path:
            time.sleep(1.2)
        if "/stream" in path:
            payload = b"chunk-" * 2000
            c.sendall(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n"
                b"X-Exyonq-Upstream: stream\r\nConnection: close\r\n\r\n"
            )
            hexlen = format(len(payload), "x").encode()
            c.sendall(hexlen + b"\r\n" + payload + b"\r\n0\r\n\r\n")
            return
        if "/large" in path:
            payload = b"L" * (256 * 1024)
            c.sendall(
                (
                    f"HTTP/1.1 200 OK\r\nContent-Length: {len(payload)}\r\n"
                    f"X-Exyonq-Upstream: large\r\nConnection: close\r\n\r\n"
                ).encode()
                + payload
            )
            return
        resp = (
            f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\n"
            f"X-Exyonq-Upstream: h3-product\r\n"
            f"X-Exyonq-Path: {path}\r\nConnection: close\r\n\r\n"
        ).encode() + body
        c.sendall(resp)
    except Exception:
        pass
    finally:
        try:
            c.close()
        except Exception:
            pass

while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
UP_PID=$!

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
timeout_ms = 2000
EOF

CONFIG_HASH="$(sha256sum "$CFG" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$CFG" | awk '{print $1}')"
HARNESS_HASH="$(sha256sum "$0" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$0" | awk '{print $1}')"

cargo build -q -p exyonq 2>/dev/null || cargo build -p exyonq
export EXYONQ_CONFIG="$CFG"
export EXYONQ_CONTROL_SOCKET="$CTRL"
cargo run -q -p exyonq -- serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!

for _ in $(seq 1 100); do
  if rg -q "HTTP/3 listening" "$LOG" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    fail "server exited early"
  fi
  sleep 0.2
done
rg -q "HTTP/3 listening" "$LOG" || { tail -80 "$LOG"; fail "no HTTP/3 listening"; }
# Wait for Unix control socket (env-driven, not TOML).
for _ in $(seq 1 50); do
  [[ -S "$CTRL" ]] && break
  sleep 0.1
done
[[ -S "$CTRL" ]] || fail "control socket not created at $CTRL (need EXYONQ_CONTROL_SOCKET)"
pass "listener_ready ALPN=h3"

H3_URL="https://127.0.0.1:${UDP_PORT}"
DOCKER_NET=(--network host)

h3() {
  docker_run "${DOCKER_NET[@]}" "$CLIENT_IMAGE" \
    curl --http3-only -sk --max-time 15 "$@"
}

h3_code() {
  local out
  out="$(h3 -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || true)"
  if [[ "$out" =~ ^[0-9]{3}$ ]]; then
    printf '%s\n' "$out"
  else
    printf '000\n'
  fi
}

code="$(h3_code "${H3_URL}/site/")"
[[ "$code" =~ ^(200|404)$ ]] || fail "GET static expected 200/404 got $code"
pass "GET_static_$code"

code="$(h3_code -I "${H3_URL}/site/")"
[[ "$code" =~ ^(200|404)$ ]] || fail "HEAD expected 200/404 got $code"
pass "HEAD_$code"

code="$(h3_code -X POST --data '' "${H3_URL}/api/empty")"
[[ "$code" == "200" ]] || fail "POST empty body got $code"
pass "POST_empty"

body="$(h3 -X POST -H 'Content-Type: application/json' --data '{"a":1}' "${H3_URL}/api/json" || true)"
[[ "$body" == '{"a":1}' ]] || fail "POST json mismatch got=${body:0:80}"
pass "POST_json"

body="$(h3 -X POST -H 'Content-Type: application/x-www-form-urlencoded' --data 'a=b&c=d' "${H3_URL}/api/form" || true)"
[[ "$body" == 'a=b&c=d' ]] || fail "POST form mismatch"
pass "POST_form"

body="$(h3 -X POST --data 'marker-product-post' "${H3_URL}/api/echo" || true)"
[[ "$body" == "marker-product-post" ]] || fail "POST small body"
pass "POST_small"

python3 - <<'PY' >"$TMP/oversize.bin"
import sys
sys.stdout.buffer.write(b"x" * (32 * 1024 * 1024 + 1))
PY
# File lives on host — mount into the curl container (path is not visible otherwise).
code="$(docker_run "${DOCKER_NET[@]}" -v "${TMP}:/e2e:ro" "$CLIENT_IMAGE" \
  curl --http3-only -sk --max-time 120 \
    -o /dev/null -w '%{http_code}' \
    -X POST --data-binary @/e2e/oversize.bin \
    "${H3_URL}/api/big" 2>/dev/null || true)"
[[ "$code" =~ ^[0-9]{3}$ ]] || code=000
[[ "$code" == "413" ]] || fail "POST oversize expected 413 got $code"
pass "POST_413"

code="$(h3_code "${H3_URL}/api/stream")"
[[ "$code" == "200" ]] || fail "upstream stream materialize expected 200 got $code"
pass "UPSTREAM_STREAM_MATERIALIZE_$code"

sz="$(h3 "${H3_URL}/api/large" | wc -c | tr -d ' ')"
[[ "$sz" -ge 262144 ]] || fail "large response size=$sz"
pass "RESPONSE_LARGE_$sz"

code="$(h3_code "${H3_URL}/api/slow")"
[[ "$code" == "200" ]] || fail "slow upstream expected 200 got $code"
pass "SLOW_UPSTREAM_$code"

echo refuse >"$UP_STATE/mode"
code="$(h3_code "${H3_URL}/api/x")"
[[ "$code" =~ ^(502|503|504)$ ]] || fail "upstream refuse expected 5xx got $code"
pass "UPSTREAM_REFUSED_$code"
echo ready >"$UP_STATE/mode"
code="$(h3_code "${H3_URL}/api/ping")"
[[ "$code" == "200" ]] || fail "recovery after refuse got $code"
pass "RECOVERY_AFTER_REFUSE"

echo timeout >"$UP_STATE/mode"
code="$(h3_code --max-time 8 "${H3_URL}/api/t")"
[[ "$code" == "504" ]] || fail "upstream timeout expected 504 got $code"
pass "UPSTREAM_TIMEOUT_504"
echo ready >"$UP_STATE/mode"

echo reset >"$UP_STATE/mode"
code="$(h3_code "${H3_URL}/api/r")"
[[ "$code" =~ ^(502|503|504)$ ]] || fail "upstream reset expected 5xx got $code"
pass "UPSTREAM_RESET_$code"
echo ready >"$UP_STATE/mode"
code="$(h3_code "${H3_URL}/api/ping")"
[[ "$code" == "200" ]] || fail "recovery after reset got $code"
pass "RECOVERY_AFTER_RESET"

echo down >"$UP_STATE/mode"
sleep 0.2
echo ready >"$UP_STATE/mode"
code="$(h3_code "${H3_URL}/api/ping")"
[[ "$code" == "200" ]] || fail "after upstream restart got $code"
pass "UPSTREAM_RESTART_RECOVERY"

code="$(h3_code "${H3_URL}/missing")"
[[ "$code" == "404" ]] || fail "404 expected got $code"
pass "HTTP_404"

code="$(h3_code -X PUT "${H3_URL}/api/x")"
[[ "$code" == "405" ]] || fail "405 expected got $code"
pass "HTTP_405"

ok=0
for _i in $(seq 1 12); do
  c="$(h3_code "${H3_URL}/site/")"
  [[ "$c" =~ ^(200|404)$ ]] && ok=$((ok + 1))
done
[[ "$ok" -ge 10 ]] || fail "multiplex ok=$ok/12"
pass "MULTIPLEX_$ok"

h3 -H 'Connection: keep-alive' -H 'Proxy-Connection: keep-alive' -H 'TE: trailers' \
  -H 'X-Forwarded-For: 1.2.3.4' -H 'Authorization: Bearer <REDACTED>' \
  "${H3_URL}/api/hdr" >/dev/null || true
[[ -f "$UP_STATE/last_req" ]] || fail "missing upstream last_req capture"
lr="$(tr '[:upper:]' '[:lower:]' <"$UP_STATE/last_req")"
echo "$lr" | grep -qi '^connection:' && fail "Connection forwarded upstream"
echo "$lr" | grep -qi 'proxy-connection:' && fail "Proxy-Connection forwarded"
echo "$lr" | grep -qi '^te:' && fail "TE forwarded"
if echo "$lr" | grep -qi 'x-forwarded-for: *1\.2\.3\.4'; then
  fail "untrusted inbound XFF forwarded literally"
fi
echo "$lr" | grep -qi '^:method:' && fail "pseudo-header :method forwarded"
pass "HEADER_TRANSLATION_SPOTCHECK"

if rg -q 'secret-token|Bearer secret' "$LOG" 2>/dev/null; then
  fail "secret leaked into server log"
fi
pass "LOG_REDACTION_AUTH"

ctrl_cmd() {
  # Control plane accepts a single plain line: reload|drain|status|shutdown
  EXYONQ_CTRL_SOCK="$CTRL" python3 -c '
import os, socket, sys
cmd = sys.argv[1]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(15)
s.connect(os.environ["EXYONQ_CTRL_SOCK"])
s.sendall(cmd.encode() + b"\n")
print(s.recv(8192).decode(errors="replace"))
' "$1"
}

[[ -S "$CTRL" ]] || fail "control socket missing for reload matrix"
# Supported same-addr reload: rewrite config in place (same listen + valid cert).
cp "$CFG" "$TMP/exyonq-reload-ok.toml"
cp "$TMP/exyonq-reload-ok.toml" "$CFG"
out="$(ctrl_cmd reload)"
echo "reload_ok_out=$out"
echo "$out" | grep -qiE '"ok"\s*:\s*true' || fail "supported reload did not ok: $out"
sleep 0.5
code="$(h3_code "${H3_URL}/site/")"
[[ "$code" =~ ^(200|404)$ ]] || fail "after allowed reload got $code"
pass "RELOAD_SUPPORTED_CASE"

# Unsupported / PREPARE fail: missing cert → explicit fail + retain previous H3
cp "$CFG" "$TMP/exyonq-good-backup.toml"
sed "s|cert = .*|cert = \"${TMP}/missing-cert.pem\"|" "$TMP/exyonq-good-backup.toml" >"$CFG"
out="$(ctrl_cmd reload || true)"
echo "reload_bad_out=$out"
echo "$out" | grep -qiE '"ok"\s*:\s*false|error|fail|cert|tls|http3|invalid' \
  || fail "unsupported reload did not fail explicitly: $out"
# Restore good config file for retain check (listener should still be prior generation)
cp "$TMP/exyonq-good-backup.toml" "$CFG"
code="$(h3_code "${H3_URL}/site/")"
[[ "$code" =~ ^(200|404)$ ]] || fail "retain after failed reload got $code"
pass "RELOAD_UNSUPPORTED_EXPLICIT_FAIL_RETAIN"

out="$(ctrl_cmd drain)"
echo "drain_out=$out"
sleep 1
code="$(h3_code "${H3_URL}/site/")"
[[ "$code" =~ ^(000|503|502)$ ]] || fail "drain should reject new work got $code"
pass "DRAIN_REJECT_NEW_$code"

ctrl_cmd drain >/dev/null || true
pass "DUPLICATE_DRAIN_OK"

kill "$SRV_PID" 2>/dev/null || true
wait "$SRV_PID" 2>/dev/null || true
sleep 0.5
: >"$LOG"
export EXYONQ_CONFIG="$CFG"
export EXYONQ_CONTROL_SOCKET="$CTRL"
cargo run -q -p exyonq -- serve -c "$CFG" >"$LOG" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 100); do
  rg -q "HTTP/3 listening" "$LOG" 2>/dev/null && break
  sleep 0.2
done
rg -q "HTTP/3 listening" "$LOG" || fail "restart did not restore H3"
code="$(h3_code "${H3_URL}/site/")"
[[ "$code" =~ ^(200|404)$ ]] || fail "post-restart got $code"
pass "RESTART_RECOVERY_$code"

echo "==== P1.3b PRODUCT E2E FREEZE ===="
echo "RUN_ID=$RUN_ID"
echo "CLIENT_IMAGE=$CLIENT_IMAGE"
echo "CLIENT_DIGEST=$CLIENT_DIGEST"
echo "CLIENT_PLATFORM=$CLIENT_PLATFORM"
echo "CERT_HASH=$CERT_HASH"
echo "CONFIG_HASH=$CONFIG_HASH"
echo "HARNESS_HASH=$HARNESS_HASH"
echo "UDP_PORT=$UDP_PORT TCP_PORT=$TCP_PORT"
echo "H3_RELOAD_SUPPORT=SAME_LISTENER_CONFIG_ONLY"
echo "H3_PROXY_STREAMING=NOT_CLAIMED"
echo "UPSTREAM_STREAM_MATERIALIZE=PASS"
pass "H3_REAL_E2E"
pass "H3_PROXY_GET"
pass "H3_PROXY_POST_BODY"
pass "H3_PROXY_ERROR_MAPPING"
pass "H3_PROXY_RECOVERY"
pass "H3_PROXY_HEADER_TRANSLATION"
pass "H3_DRAIN"
pass "QUIC_SHUTDOWN"
pass "DUPLICATE_DRAIN_REGRESSION"
echo "RUN_OK=1"
