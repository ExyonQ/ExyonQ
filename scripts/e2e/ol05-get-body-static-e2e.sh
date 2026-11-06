#!/usr/bin/env bash
# OL-05 regression: a GET with a body must reach WAF body inspection even
# when the target path is served by the /site/ static shortcut.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$BIN" | awk '{print $1}')"
fi
echo "binary=$BIN binary_sha256=$BIN_SHA256"
WORK="$(mktemp -d "$ROOT/target/ol05-get-body-static.XXXXXX")"
PORT="$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
PID=""
cleanup() {
    if [[ -n "$PID" ]]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

mkdir -p "$WORK/www"
printf 'get-body-static' >"$WORK/www/index.html"
cat >"$WORK/exy.toml" <<EOF
config_version = 1
[waf]
enabled = true
mode = "block"
max_inspection_body_bytes = 1048576

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/site" }
root = "$WORK/www"
EOF

"$BIN" validate "$WORK/exy.toml" >"$WORK/validate.log" 2>&1
CTRL="/tmp/ol05-get-body-static-$$.sock"
rm -f "$CTRL"
EXYONQ_CONTROL_SOCKET="$CTRL" "$BIN" serve -c "$WORK/exy.toml" >"$WORK/server.log" 2>&1 &
PID=$!

ready=0
for _ in $(seq 1 120); do
    if python3 - "$PORT" <<'PY' 2>/dev/null
import socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), 0.2)
s.close()
PY
    then
        ready=1
        break
    fi
    sleep 0.05
done
if [[ "$ready" -ne 1 ]]; then
    echo "server did not become ready" >&2
    cat "$WORK/server.log" >&2
    exit 1
fi

base_status="$(curl --max-time 5 -sS -o "$WORK/base.body" -w '%{http_code}' \
    "http://127.0.0.1:$PORT/site/index.html")"
if [[ "$base_status" != "200" ]]; then
    echo "baseline GET failed: $base_status" >&2
    cat "$WORK/server.log" >&2
    exit 1
fi

get_status="$(curl --max-time 5 -sS -o "$WORK/get.body" -w '%{http_code}' \
    -X GET -H 'Content-Type: application/x-www-form-urlencoded' \
    --data 'var=1234 OR 1=1' \
    "http://127.0.0.1:$PORT/site/index.html")"
if [[ "$get_status" != "403" ]]; then
    echo "GET body bypassed WAF body inspection: status=$get_status" >&2
    cat "$WORK/get.body" >&2
    cat "$WORK/server.log" >&2
    exit 1
fi

echo "RESULT=PASS OL-05 GET body static shortcut inspected (status=$get_status)"
echo "RUN=$WORK"
