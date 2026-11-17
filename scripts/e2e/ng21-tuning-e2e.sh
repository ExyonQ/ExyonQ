#!/usr/bin/env bash
# NG-21 real E2E: `[tuning].keepalive_timeout_ms` and `[tuning].max_header_bytes`
# against the real binary. Not a mock: the server is started per case and the
# client speaks raw HTTP/1.1 on a real socket.
#
# Cases:
#   1. default geometry: a normal request is served, a 12 KiB header block is not;
#   2. `max_header_bytes = 16384`: the same 12 KiB block is served, 20 KiB is not;
#   3. `keepalive_timeout_ms = 400`: the second request on one socket is served
#      after 100 ms idle and the connection is closed after 900 ms idle;
#   4. `exyonq validate` rejects out-of-range/odd values and accepts the valid one.
#
# Usage: bash scripts/e2e/ng21-tuning-e2e.sh
#   EXYONQ_BIN   measured binary (default target/debug/exyonq)
#   NG21_PORT    listen port (default 19480)
#   NG21_RUN_DIR evidence directory (default .exyonq-local/evidence/ng21-tuning/run-<utc>)
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${NG21_RUN_DIR:-$ROOT/.exyonq-local/evidence/ng21-tuning/run-$STAMP}"
PORT="${NG21_PORT:-19480}"
mkdir -p "$RUN/www"
printf 'tuning-ok' > "$RUN/www/index.html"

PASS=0
FAIL=0
SERVER_PID=""

check() {
    local name=$1 expected=$2 actual=$3
    if [[ "$expected" == "$actual" ]]; then
        echo "PASS $name expected=$expected actual=$actual"
        PASS=$((PASS + 1))
    else
        echo "FAIL $name expected=$expected actual=$actual"
        FAIL=$((FAIL + 1))
    fi
}

cleanup() {
    if [[ -n "$SERVER_PID" ]]; then
        kill "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null
    fi
}
trap cleanup EXIT

[[ -x "$BIN" ]] || { echo "FAIL missing binary: $BIN"; exit 1; }

cat > "$RUN/client.py" <<'PY'
import socket
import sys
import time


def head(pad: int, keepalive: bool) -> bytes:
    lines = [
        "GET /index.html HTTP/1.1",
        "Host: tuning.test",
        f"X-Pad: {'a' * pad}" if pad else "X-Pad: 0",
    ]
    if not keepalive:
        lines.append("Connection: close")
    return ("\r\n".join(lines) + "\r\n\r\n").encode()


def read_status(sock) -> str:
    data = b""
    while b"\r\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            return "EOF"
        data += chunk
    return data.split(b"\r\n")[0].split(b" ")[1].decode()


def once(port: int, pad: int) -> str:
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.sendall(head(pad, keepalive=False))
    try:
        return read_status(sock)
    finally:
        sock.close()


def keepalive(port: int, idle_ms: int) -> str:
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.sendall(head(0, keepalive=True))
    first = read_status(sock)
    time.sleep(idle_ms / 1000.0)
    try:
        sock.sendall(head(0, keepalive=True))
        second = read_status(sock)
    except OSError:
        second = "closed"
    finally:
        sock.close()
    # A clean FIN surfaces as EOF, a reset as OSError: both mean "closed".
    if second == "EOF":
        second = "closed"
    return f"{first}/{second}"


kind = sys.argv[1]
port = int(sys.argv[2])
if kind == "once":
    print(once(port, int(sys.argv[3])))
else:
    print(keepalive(port, int(sys.argv[3])))
PY

write_config() {
    cat > "$RUN/exy.toml" <<EOF
config_version = 1

$1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${RUN}/www"
index = "index.html"
EOF
}

start_server() {
    write_config "$1"
    "$BIN" validate --config "$RUN/exy.toml" >> "$RUN/validate.log" 2>&1 ||
        { echo "FAIL validate rejected the E2E config"; FAIL=$((FAIL + 1)); return 1; }
    "$BIN" serve -c "$RUN/exy.toml" >> "$RUN/server.log" 2>&1 &
    SERVER_PID=$!
    local deadline=$((SECONDS + 20))
    while ((SECONDS < deadline)); do
        if python3 "$RUN/client.py" once "$PORT" 0 >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    echo "FAIL server did not answer on ${PORT}"
    return 1
}

stop_server() {
    if [[ -n "$SERVER_PID" ]]; then
        kill "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null
        SERVER_PID=""
    fi
}

# 1. Default geometry: normal request served, oversized header block refused.
start_server "" || exit 1
check "default small header served" "200" "$(python3 "$RUN/client.py" once "$PORT" 0)"
oversized="$(python3 "$RUN/client.py" once "$PORT" 12000)"
if [[ "$oversized" == "200" ]]; then
    check "default rejects a 12 KiB header block" "not-200" "$oversized"
else
    check "default rejects a 12 KiB header block" "not-200" "not-200"
fi
stop_server

# 2. max_header_bytes = 16384: the same block is served, 20 KiB is not.
start_server "[tuning]
max_header_bytes = 16384" || exit 1
check "configured cap serves a 12 KiB header block" "200" "$(python3 "$RUN/client.py" once "$PORT" 12000)"
over="$(python3 "$RUN/client.py" once "$PORT" 20000)"
if [[ "$over" == "200" ]]; then
    check "configured cap still rejects 20 KiB" "not-200" "$over"
else
    check "configured cap still rejects 20 KiB" "not-200" "not-200"
fi
stop_server

# 3. keepalive_timeout_ms = 400: reuse inside the window, close outside it.
start_server "[tuning]
keepalive_timeout_ms = 400" || exit 1
check "keep-alive reuse inside the window" "200/200" "$(python3 "$RUN/client.py" keepalive "$PORT" 100)"
check "keep-alive idle past the window closes" "200/closed" "$(python3 "$RUN/client.py" keepalive "$PORT" 900)"
stop_server

# 4. Validation is fail-closed on both keys.
write_config "[tuning]
keepalive_timeout_ms = 400
max_header_bytes = 16384"
"$BIN" validate --config "$RUN/exy.toml" > "$RUN/validate-good.log" 2>&1
check "validate accepts the configured geometry" "0" "$?"

for bad in "keepalive_timeout_ms = 0" "keepalive_timeout_ms = 600001" "max_header_bytes = 4096" "max_header_bytes = 65535"; do
    write_config "[tuning]
${bad}"
    "$BIN" validate --config "$RUN/exy.toml" > "$RUN/validate-bad.log" 2>&1
    if [[ $? -eq 0 ]]; then
        check "validate rejects '${bad}'" "rejected" "accepted"
    else
        check "validate rejects '${bad}'" "rejected" "rejected"
    fi
done

echo "SUMMARY pass=$PASS fail=$FAIL run=$RUN"
if [[ "$FAIL" -eq 0 ]]; then
    echo "RESULT=PASS NG21_TUNING"
    exit 0
fi
echo "RESULT=FAIL NG21_TUNING"
exit 1
