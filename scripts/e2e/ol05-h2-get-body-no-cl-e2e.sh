#!/usr/bin/env bash
# OL-05 regression: HTTP/2 GET with a body and no Content-Length must not take
# the /site/ static shortcut before WAF inspection. nghttp2 is used with
# --no-content-length, so this exercises the stream-end/body path rather than a
# request with a declared length. The target is the real exyonq binary and the
# real WAF, not a synthetic handler.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_ensure_bins
# shellcheck source=lib/tls.sh
source "$ROOT/scripts/e2e/lib/tls.sh"
ensure_ephemeral_tls

if ! command -v nghttp >/dev/null 2>&1; then
    echo "FAIL: nghttp (nghttp2 client) is required for this E2E" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$EXYONQ_BIN" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$EXYONQ_BIN" | awk '{print $1}')"
fi
echo "binary=$EXYONQ_BIN binary_sha256=$BIN_SHA256"

WORK="$(mktemp -d "$ROOT/target/ol05-h2-get-body-no-cl.XXXXXX")"
PORT="$(e2e_pick_port)"
CTRL="/tmp/ol05-h2-no-cl-$$.sock"
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
printf 'h2-get-body-static' >"$WORK/www/index.html"
printf 'var=1234 OR 1=1' >"$WORK/payload.txt"

cat >"$WORK/exy.toml" <<EOF
config_version = 1

[waf]
enabled = true
mode = "block"
max_inspection_body_bytes = 1048576

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]
tls = { cert = "${CERT_PEM}", key = "${KEY_PEM}" }

[[route]]
name = "site"
match = { path = "/site" }
root = "$WORK/www"
index = "index.html"
EOF

"$EXYONQ_BIN" validate "$WORK/exy.toml" >"$WORK/validate.log" 2>&1
EXYONQ_CONFIG="$WORK/exy.toml" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=error \
    "$EXYONQ_BIN" serve -c "$WORK/exy.toml" >"$WORK/server.log" 2>&1 &
PID=$!

if ! wait_listen "$PORT"; then
    echo "FAIL: server did not listen on TCP ${PORT}" >&2
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

BASE_OUT="$(curl -sk --http2 --max-time 10 -o "$WORK/base.body" -w '%{http_code} %{http_version}' \
    "https://127.0.0.1:${PORT}/site/index.html")"
BASE_CODE="${BASE_OUT%% *}"
BASE_VER="${BASE_OUT##* }"
if [[ "$BASE_CODE" != "200" || "$BASE_VER" != "2" ]]; then
    echo "FAIL: baseline HTTP/2 GET code=$BASE_CODE version=$BASE_VER" >&2
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

# nghttp's verbose output proves both the GET method and absence of a declared
# Content-Length in the request headers before the response is received.
set +e
nghttp --no-verify-peer -v --no-content-length \
    -H ':method: GET' -d "$WORK/payload.txt" \
    "https://127.0.0.1:${PORT}/site/index.html" >"$WORK/nghttp.log" 2>&1
NGHTTP_RC=$?
set -e

STATUS="$(sed -n 's/.*:status: *\([0-9][0-9][0-9]\).*/\1/p' "$WORK/nghttp.log" | head -1)"
# Inspect only the request HEADERS block. The response also has a
# content-length header, so scanning the whole verbose log would be wrong.
SENT_HEADERS="$(awk '/send HEADERS frame/{in_request=1} in_request{print} in_request && /END_HEADERS/{exit}' "$WORK/nghttp.log")"
if printf '%s\n' "$SENT_HEADERS" | grep -qi 'content-length'; then
    echo "FAIL: nghttp sent Content-Length despite --no-content-length" >&2
    printf '%s\n' "$SENT_HEADERS" >&2
    exit 1
fi
if ! grep -q 'send DATA frame' "$WORK/nghttp.log"; then
    echo "FAIL: nghttp did not send a DATA frame for the GET body" >&2
    cat "$WORK/nghttp.log" >&2
    exit 1
fi
if [[ "$STATUS" != "403" ]]; then
    echo "FAIL: HTTP/2 GET body without Content-Length was not blocked (status=$STATUS, nghttp_exit=$NGHTTP_RC)" >&2
    cat "$WORK/nghttp.log" >&2
    cat "$WORK/server.log" >&2 || true
    exit 1
fi

echo "PASS  baseline HTTP/2 GET status=200 version=2"
echo "PASS  HTTP/2 GET body has no Content-Length and sent DATA (status=403)"
echo "PASS  WAF blocked the no-Content-Length body on the /site/ static route"
echo "RESULT=PASS OL-05 HTTP/2 GET body without Content-Length"
echo "RUN=$WORK"
