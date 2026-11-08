#!/usr/bin/env bash
# H-04 cost bound: a route-level request-body limit answers 413 before WAF
# inspection and before origin dispatch, while an unset route keeps the 64 MiB
# budget + Block contract. Real binary, real CRS, real origin.
#
# Measures the same body on both routes:
#   /upload  max_request_body_bytes = 2 MiB  -> 413, bounded time, origin 0
#   /post    unset (64 MiB contract)          -> 200, full inspection
# and proves the boundary still works on the limited route:
#   2 MiB benign -> 200; 2 MiB with a 942100 tail -> 403.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_ensure_bins

EXY="$EXYONQ_BIN"
if command -v sha256sum >/dev/null 2>&1; then
  BIN_SHA256="$(sha256sum "$EXY" | awk '{print $1}')"
else
  BIN_SHA256="$(shasum -a 256 "$EXY" | awk '{print $1}')"
fi
echo "binary=$EXY binary_sha256=$BIN_SHA256"

CRS_DIR="${EXYONQ_CRS_DIR:-$ROOT/.exyonq-local/refs/crs-v4.25.0}"
EXPECTED_CRS_COMMIT=aabf675fcfc5e489b424b844e6c9f1b39802df69
[[ -f "$CRS_DIR/crs-setup.conf.example" && -d "$CRS_DIR/rules" ]] \
  || { echo "FAIL: missing real CRS checkout: $CRS_DIR" >&2; exit 2; }
actual_commit="$(git -C "$CRS_DIR" rev-parse HEAD 2>/dev/null)" \
  || { echo "FAIL: CRS checkout is not a git repository: $CRS_DIR" >&2; exit 2; }
[[ "$actual_commit" == "$EXPECTED_CRS_COMMIT" ]] \
  || { echo "FAIL: CRS commit mismatch: expected $EXPECTED_CRS_COMMIT, got $actual_commit" >&2; exit 2; }

LIMIT_BYTES="${H04_ROUTE_LIMIT_BYTES:-2097152}"
BODY_BYTES="${H04_BODY_BYTES:-67108864}"
CLIENT_TIMEOUT="${H04_CLIENT_TIMEOUT:-300}"
[[ "$BODY_BYTES" -gt "$LIMIT_BYTES" ]] \
  || { echo "FAIL: H04_BODY_BYTES must exceed H04_ROUTE_LIMIT_BYTES" >&2; exit 1; }

PORT="$(e2e_pick_port)"
ORIGIN_PORT="$(e2e_pick_port)"
[[ "$PORT" != "$ORIGIN_PORT" ]] || { echo "FAIL: duplicate port"; exit 1; }
WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-h04-route.XXXXXX")"
EXY_PID=""
ORIGIN_PID=""
cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null || true
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null || true
  wait "$EXY_PID" "$ORIGIN_PID" 2>/dev/null || true
  if [[ "${KEEP_WORK:-0}" == "1" ]]; then
    echo "WORK=$WORK" >&2
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

cat >"$WORK/origin.py" <<'PY'
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

hits_path = sys.argv[2]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)
        with open(hits_path, "r+", encoding="ascii") as f:
            hits = int(f.read() or "0") + 1
            f.seek(0)
            f.truncate()
            f.write(str(hits))
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


open(hits_path, "w", encoding="ascii").write("0")
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY

python3 "$WORK/origin.py" "$ORIGIN_PORT" "$WORK/hits" >"$WORK/origin.log" 2>&1 &
ORIGIN_PID=$!

cat >"$WORK/exy-main.conf" <<EOF
Include $CRS_DIR/crs-setup.conf.example
Include $CRS_DIR/rules/*.conf
EOF

# /post keeps the product default 64 MiB + Block contract. /upload lowers the
# client-facing bound to 2 MiB; the 413 must happen before WAF and origin.
cat >"$WORK/exyonq.toml" <<EOF
config_version = 1

[[waf.ruleset]]
id = "crs"
rules_file = "$WORK/exy-main.conf"
inbound_threshold = 5
detection_only = false

[waf.ruleset.builtins]
sql_injection = "off"
xss = "off"
path_traversal = "off"
command_injection = "off"
header_anomaly = "off"
bot_ua = "off"
ip_filter = { enabled = false }

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["post", "upload"]

[[route]]
name = "post"
match = { path = "/post" }
upstream = "origin"

[[route]]
name = "upload"
match = { path = "/upload" }
upstream = "origin"
max_request_body_bytes = $LIMIT_BYTES

[[upstream]]
name = "origin"
target = "http://127.0.0.1:$ORIGIN_PORT"
timeout_ms = 5000
EOF

"$EXY" validate -c "$WORK/exyonq.toml" >"$WORK/validate.log" 2>&1 \
  || { echo "FAIL: config validation failed" >&2; cat "$WORK/validate.log" >&2; exit 1; }

for _ in $(seq 1 100); do
  if curl -sS --max-time 1 -o /dev/null "http://127.0.0.1:$ORIGIN_PORT/" 2>/dev/null; then break; fi
  sleep 0.05
done

RUST_LOG="${RUST_LOG:-exyonq_waf=warn}" \
  EXYONQ_CONTROL_SOCKET="$WORK/control.sock" \
  "$EXY" serve -c "$WORK/exyonq.toml" >"$WORK/exyonq.log" 2>&1 &
EXY_PID=$!
ready=0
for _ in $(seq 1 300); do
  if curl -sS --max-time 1 -o /dev/null "http://127.0.0.1:$PORT/health" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.05
done
if [[ "$ready" -ne 1 ]]; then
  echo "FAIL: ExyonQ did not become ready" >&2
  cat "$WORK/exyonq.log" >&2
  exit 1
fi

python3 - "$PORT" "$WORK/hits" "$LIMIT_BYTES" "$BODY_BYTES" "$CLIENT_TIMEOUT" <<'PY' | tee "$WORK/statuses"
import socket
import sys
import time

port = int(sys.argv[1])
hits_path = sys.argv[2]
limit = int(sys.argv[3])
body_bytes = int(sys.argv[4])
timeout = int(sys.argv[5])


def hits():
    with open(hits_path, encoding="ascii") as f:
        return int(f.read() or "0")


def post(path, body):
    """Raw HTTP/1.1 POST.

    A 413 can close the connection while the client is still writing, so send
    errors are expected; the response is read after the write attempt.
    """
    started = time.monotonic()
    sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    head = (
        f"POST {path} HTTP/1.1\r\n"
        "Host: 127.0.0.1\r\n"
        "Content-Type: application/x-www-form-urlencoded\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n\r\n"
    ).encode()
    send_failed = False
    try:
        sock.sendall(head)
        for offset in range(0, len(body), 1 << 20):
            sock.sendall(body[offset : offset + (1 << 20)])
    except (BrokenPipeError, ConnectionResetError):
        send_failed = True
    data = b""
    try:
        while True:
            part = sock.recv(65536)
            if not part:
                break
            data += part
    except (ConnectionResetError, socket.timeout):
        pass
    finally:
        sock.close()
    status = int(data.split(b" ", 2)[1]) if data.startswith(b"HTTP/") else 0
    return status, time.monotonic() - started, send_failed


attack = b"var=1234 OR 1=1"
big = b"A" * body_bytes

limited_status, limited_secs, limited_send_failed = post("/upload", big)
after_limited = hits()
print(f"LIMIT_BYTES={limit}")
print(f"LIMITED_STATUS={limited_status}")
print(f"LIMITED_SECS={limited_secs:.2f}")
print(f"LIMITED_SEND_FAILED={int(limited_send_failed)}")
print(f"ORIGIN_HITS_AFTER_LIMITED={after_limited}")

default_status, default_secs, _ = post("/post", big)
print(f"DEFAULT_STATUS={default_status}")
print(f"DEFAULT_SECS={default_secs:.2f}")
print(f"ORIGIN_HITS_AFTER_DEFAULT={hits()}")

boundary_status, boundary_secs, _ = post("/upload", b"A" * limit)
print(f"BOUNDARY_STATUS={boundary_status}")
print(f"BOUNDARY_SECS={boundary_secs:.2f}")

tail_status, tail_secs, _ = post("/upload", b"A" * (limit - len(attack)) + attack)
print(f"TAIL_STATUS={tail_status}")
print(f"TAIL_SECS={tail_secs:.2f}")
print(f"ORIGIN_HITS_FINAL={hits()}")
PY

get_status() { awk -F= -v k="$1" '$1 == k {print $2}' "$WORK/statuses"; }
get_secs() { awk -F= -v k="$1" '$1 == k {printf "%.2f", $2}' "$WORK/statuses"; }

limited_status="$(get_status LIMITED_STATUS)"
limited_secs="$(get_secs LIMITED_SECS)"
default_status="$(get_status DEFAULT_STATUS)"
default_secs="$(get_secs DEFAULT_SECS)"
boundary_status="$(get_status BOUNDARY_STATUS)"
tail_status="$(get_status TAIL_STATUS)"
hits_limited="$(get_status ORIGIN_HITS_AFTER_LIMITED)"
hits_default="$(get_status ORIGIN_HITS_AFTER_DEFAULT)"
hits_final="$(get_status ORIGIN_HITS_FINAL)"

echo "route_limit_bytes=$LIMIT_BYTES body_bytes=$BODY_BYTES"
echo "limited: status=$limited_status secs=$limited_secs origin_hits=$hits_limited"
echo "default: status=$default_status secs=$default_secs origin_hits=$hits_default"
echo "boundary: status=$boundary_status"
echo "tail_attack: status=$tail_status origin_hits=$hits_final"
echo "--- WAF evidence ---"
grep -E '942100' "$WORK/exyonq.log" | tail -5 || true

fail() { echo "FAIL: $*" >&2; exit 1; }

[[ "$limited_status" == "413" ]] \
  || fail "route limit did not answer 413 (got $limited_status)"
[[ "$hits_limited" == "0" ]] \
  || fail "route limit reached the origin after a 413 (hits=$hits_limited)"
[[ "$default_status" == "200" ]] \
  || fail "default route did not accept the same body (got $default_status)"
[[ "$hits_default" == "1" ]] \
  || fail "default route origin hits expected 1 (got $hits_default)"
[[ "$boundary_status" == "200" ]] \
  || fail "body equal to the route limit must be accepted (got $boundary_status)"
[[ "$tail_status" == "403" ]] \
  || fail "tail 942100 must be blocked on the limited route (got $tail_status)"
[[ "$hits_final" == "2" ]] \
  || fail "blocked tail reached the origin (hits=$hits_final)"
if grep -q 'EXY-LIMIT-BODY-001' "$WORK/exyonq.log"; then
  fail "inspection limit applied below the route body limit"
fi
# Bounded cost: the 64 MiB rejection must not pay the full inspection price.
awk -v secs="$limited_secs" 'BEGIN { exit !(secs < 5) }' \
  || fail "413 took ${limited_secs}s; route limit did not bound inspection cost"

echo "RESULT=PASS H-04 route body limit"
