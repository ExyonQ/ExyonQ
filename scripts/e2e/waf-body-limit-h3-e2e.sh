#!/usr/bin/env bash
# Real HTTP/3 E2E: a body over the WAF inspection budget must log
# outcome=waf_body_limit on the access log, same as HTTP/1.1.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_ensure_bins
# shellcheck source=lib/tls.sh
source "$ROOT/scripts/e2e/lib/tls.sh"
ensure_ephemeral_tls

if ! command -v docker >/dev/null 2>&1; then
    echo "FAIL: docker is required for the real HTTP/3 client" >&2
    exit 1
fi
CLIENT_IMAGE="${H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${H3_CLIENT_PLATFORM:-linux/amd64}"
if ! docker image inspect "$CLIENT_IMAGE" >/dev/null 2>&1; then
    docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null
fi

if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$EXYONQ_BIN" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$EXYONQ_BIN" | awk '{print $1}')"
fi
echo "binary=$EXYONQ_BIN binary_sha256=$BIN_SHA256"

WORK="$(mktemp -d "$ROOT/target/waf-body-h3.XXXXXX")"
TCP_PORT="$(e2e_pick_port)"
UDP_PORT="$(e2e_pick_port)"
CTRL="/tmp/waf-body-h3-$$.sock"
PID=""
cleanup() {
    if [[ -n "$PID" ]]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -f "$CTRL"
}
trap cleanup EXIT

mkdir -p "$WORK/www"
printf 'h3-body-limit-static' >"$WORK/www/index.html"
python3 -c 'import sys; sys.stdout.buffer.write(b"A" * 4096)' >"$WORK/payload.bin"

cat >"$WORK/exy.toml" <<EOF
config_version = 1

[http3]
enabled = true
request_body_drain_cap_bytes = 1048576

[waf]
enabled = true
mode = "monitor"
max_inspection_body_bytes = 1024
on_inspection_limit = "log_and_allow"

[[server]]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "0.0.0.0:${UDP_PORT}"
routes = ["upload"]
tls = { cert = "${CERT_PEM}", key = "${KEY_PEM}" }

[[route]]
name = "upload"
match = { path = "/upload" }
root = "$WORK/www"
index = "index.html"
EOF

"$EXYONQ_BIN" validate "$WORK/exy.toml" >"$WORK/validate.log" 2>&1
EXYONQ_ACCESS_LOG=1 EXYONQ_CONFIG="$WORK/exy.toml" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=info \
    "$EXYONQ_BIN" serve -c "$WORK/exy.toml" >"$WORK/server.log" 2>&1 &
PID=$!

for _ in $(seq 1 150); do
    if grep -q 'HTTP/3 listening' "$WORK/server.log" 2>/dev/null; then break; fi
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "FAIL: server exited before the HTTP/3 listener became ready" >&2
        cat "$WORK/server.log" >&2 || true
        exit 1
    fi
    sleep 0.1
done
grep -q 'HTTP/3 listening' "$WORK/server.log" || { echo "FAIL: no HTTP/3 listener" >&2; cat "$WORK/server.log" >&2; exit 1; }

H3_HOST="${H3_CLIENT_HOST:-host.docker.internal}"
if [[ "$H3_HOST" == "127.0.0.1" ]]; then
    H3_NET_ARGS=(--network host)
else
    H3_NET_ARGS=(--add-host=host.docker.internal:host-gateway)
fi
H3_URL="https://${H3_HOST}:${UDP_PORT}"
h3() {
    docker run --rm --platform "$CLIENT_PLATFORM" "${H3_NET_ARGS[@]}" \
        -v "$WORK:/e2e" "$CLIENT_IMAGE" \
        curl --http3-only -sk --max-time 25 "$@"
}

BASE_OUT="$(h3 -o /dev/null -w '%{http_code} %{http_version}' "${H3_URL}/upload/index.html" 2>"$WORK/base.err" || true)"
BASE_CODE="${BASE_OUT%% *}"; BASE_VER="${BASE_OUT##* }"
[[ "$BASE_CODE" == "200" && "$BASE_VER" == "3" ]] || { echo "FAIL: baseline HTTP/3 code=$BASE_CODE version=$BASE_VER" >&2; cat "$WORK/base.err" >&2; exit 1; }

POST_OUT="$(h3 -X POST -H 'Content-Type: text/plain' --data-binary @/e2e/payload.bin \
    -o /dev/null -w '%{http_code} %{http_version}' "${H3_URL}/upload/index.html" 2>"$WORK/post.err" || true)"
POST_CODE="${POST_OUT%% *}"; POST_VER="${POST_OUT##* }"
echo "h3 POST code=$POST_CODE version=$POST_VER"
grep 'event="access"' "$WORK/server.log" || true
grep -q 'EXY-LIMIT-BODY-001' "$WORK/server.log" || { echo "FAIL: engine did not apply the inspection limit" >&2; exit 1; }
grep 'event="access"' "$WORK/server.log" | grep 'method=POST' | grep -q 'outcome=.*waf_body_limit' \
    || { echo "FAIL: HTTP/3 truncated body not logged as waf_body_limit" >&2; exit 1; }

echo "PASS  HTTP/3 POST status=$POST_CODE version=$POST_VER"
echo "PASS  access outcome=waf_body_limit on HTTP/3"
echo "RESULT=PASS WAF body limit HTTP/3"
