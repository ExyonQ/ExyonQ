#!/usr/bin/env bash
# OL-05 regression: HTTP/3 GET with a body must inspect the body before the
# /site/ static shortcut. The client is the real curl-http3 image; the server
# is the real exyonq binary with TLS and HTTP/3 enabled. A baseline bodyless
# GET must be 200; the body-bearing GET must be 403 from the WAF.
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
    if ! docker pull --platform "$CLIENT_PLATFORM" "$CLIENT_IMAGE" >/dev/null; then
        echo "FAIL: cannot obtain HTTP/3 client image $CLIENT_IMAGE" >&2
        exit 1
    fi
fi
CLIENT_DIGEST="$(docker image inspect --format '{{index .RepoDigests 0}}' "$CLIENT_IMAGE" 2>/dev/null || true)"
if [[ -z "$CLIENT_DIGEST" || "$CLIENT_DIGEST" == "<no value>" ]]; then
    echo "FAIL: HTTP/3 client image has no immutable repository digest" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$EXYONQ_BIN" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$EXYONQ_BIN" | awk '{print $1}')"
fi
echo "binary=$EXYONQ_BIN binary_sha256=$BIN_SHA256"

WORK="$(mktemp -d "$ROOT/target/ol05-h3-get-body.XXXXXX")"
TCP_PORT="$(e2e_pick_port)"
UDP_PORT="$(e2e_pick_port)"
CTRL="/tmp/ol05-h3-get-body-$$.sock"
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
printf 'h3-get-body-static' >"$WORK/www/index.html"
printf 'var=1234 OR 1=1' >"$WORK/payload.txt"

cat >"$WORK/exy.toml" <<EOF
config_version = 1

[http3]
enabled = true
request_body_drain_cap_bytes = 1048576

[waf]
enabled = true
mode = "block"
max_inspection_body_bytes = 1048576

[[server]]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "0.0.0.0:${UDP_PORT}"
routes = ["site"]
tls = { cert = "${CERT_PEM}", key = "${KEY_PEM}" }

[[route]]
name = "site"
match = { path = "/site" }
root = "$WORK/www"
index = "index.html"
EOF

"$EXYONQ_BIN" validate "$WORK/exy.toml" >"$WORK/validate.log" 2>&1
EXYONQ_CONFIG="$WORK/exy.toml" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=info \
    "$EXYONQ_BIN" serve -c "$WORK/exy.toml" >"$WORK/server.log" 2>&1 &
PID=$!

for _ in $(seq 1 150); do
    if rg -q 'HTTP/3 listening' "$WORK/server.log" 2>/dev/null; then
        break
    fi
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "FAIL: server exited before the HTTP/3 listener became ready" >&2
        cat "$WORK/server.log" >&2 || true
        exit 1
    fi
    sleep 0.1
done
if ! rg -q 'HTTP/3 listening' "$WORK/server.log" 2>/dev/null; then
    echo "FAIL: no HTTP/3 listener in server log" >&2
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

H3_URL="https://host.docker.internal:${UDP_PORT}"
h3() {
    docker run --rm --platform "$CLIENT_PLATFORM" \
        --add-host=host.docker.internal:host-gateway \
        -v "$WORK:/e2e" "$CLIENT_IMAGE" \
        curl --http3-only -sk --max-time 25 "$@"
}

BASE_OUT="$(h3 -o /dev/null -w '%{http_code} %{http_version}' \
    "${H3_URL}/site/index.html" 2>"$WORK/h3-base.err" || true)"
BASE_CODE="${BASE_OUT%% *}"
BASE_VER="${BASE_OUT##* }"
if [[ "$BASE_CODE" != "200" || "$BASE_VER" != "3" ]]; then
    echo "FAIL: baseline HTTP/3 GET code=$BASE_CODE version=$BASE_VER" >&2
    cat "$WORK/h3-base.err" >&2 || true
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

ATTACK_OUT="$(h3 -X GET -H 'Content-Type: application/x-www-form-urlencoded' \
    --data-binary @/e2e/payload.txt -o /dev/null -w '%{http_code} %{http_version}' \
    "${H3_URL}/site/index.html" 2>"$WORK/h3-attack.err" || true)"
ATTACK_CODE="${ATTACK_OUT%% *}"
ATTACK_VER="${ATTACK_OUT##* }"
if [[ "$ATTACK_CODE" != "403" ]]; then
    echo "FAIL: HTTP/3 GET body was not blocked (code=$ATTACK_CODE version=$ATTACK_VER)" >&2
    cat "$WORK/h3-attack.err" >&2 || true
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

echo "PASS  baseline HTTP/3 GET status=200 version=3"
echo "PASS  HTTP/3 GET body status=403 version=3"
echo "PASS  WAF blocked the body before the /site/ static shortcut"
echo "PASS  h3_client_image=$CLIENT_DIGEST"
echo "RESULT=PASS OL-05 HTTP/3 GET body"
echo "RUN=$WORK"
