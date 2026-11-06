#!/usr/bin/env bash
# OL-03 real E2E on the host: complete perClientConnLimit surface.
# Portable across macOS and Linux; the same journey is used on both.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
COMPAT="${EXYONQ_COMPAT_BIN:-$ROOT/target/debug/exyonq-compat}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${OL03_RUN_DIR:-$ROOT/.exyonq-local/evidence/ol03/full-$STAMP}"
mkdir -p "$RUN"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
PORT_EXY=19380
PORT_ORIGIN=19381
EXY_PID=""
ORIGIN_PID=""
PASS=0
FAIL=0

cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

check() {
  local name=$1 expected=$2 actual=$3
  if [[ "$expected" == "$actual" ]]; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name expected=[$expected] actual=[$actual]"
    FAIL=$((FAIL + 1))
  fi
}

status() { curl -sS -o /dev/null -w '%{http_code}' --max-time 15 "$@"; }

# OL-03: OLS refuses a client by closing the accepted socket without writing an
# HTTP response. Probe the raw connection so a 429/503 cannot be mistaken for
# the expected close.
closed_probe() {
  python3 - "$1" "$2" <<'PY'
import socket, sys
port, path = int(sys.argv[1]), sys.argv[2]
try:
    s = socket.create_connection(('127.0.0.1', port), timeout=3)
    s.settimeout(3)
    try:
        s.sendall(f'GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n'.encode())
    except OSError:
        print('closed')
        raise SystemExit
    try:
        data = s.recv(1)
    except ConnectionResetError:
        print('closed')
    except socket.timeout:
        print('timeout')
    else:
        print('closed' if not data else 'http')
finally:
    try:
        s.close()
    except Exception:
        pass
PY
}
wait_port() {
  local port=$1
  for _ in $(seq 1 120); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" 2>/dev/null; then return 0; fi
    sleep 0.05
  done
  return 1
}

cat >"$RUN/origin.py" <<'PY'
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def _send(self, body):
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path.startswith('/dyn/slow'):
            time.sleep(4)
            self._send(b'slow')
        elif self.path.startswith('/dyn/large'):
            self._send(b'x' * (1024 * 1024))
        else:
            self._send(b'origin')
    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        data = self.rfile.read(length)
        self._send(b'uploaded:' + str(len(data)).encode())
    def log_message(self, *args):
        pass

ThreadingHTTPServer(('127.0.0.1', 19381), H).serve_forever()
PY
python3 "$RUN/origin.py" >"$RUN/origin.log" 2>&1 & ORIGIN_PID=$!
wait_port 19381 || { echo 'FAIL origin did not start'; exit 1; }
mkdir -p "$RUN/www"
printf 'static-ok' >"$RUN/www/file.txt"

echo "== environment =="
echo "host=$(uname -s)/$(uname -m)"
echo "binary=$BIN"
echo "binary_sha256=$(hash_file "$BIN")"
"$BIN" --version
echo "compat=$COMPAT"
echo "compat_sha256=$(hash_file "$COMPAT")"

cat >"$RUN/ols.conf" <<'EOF'
# OpenLiteSpeed httpd_config.conf excerpt
perClientConnLimit {
    staticReqPerSec 2
    dynReqPerSec 1
    outBandwidth 131072
    inBandwidth 131072
    softLimit 2
    hardLimit 3
    gracePeriod 1
    banPeriod 2
    blockBadReq 1
}
listener Default {
    address *:8080
}
EOF

"$COMPAT" ols-throttle --input "$RUN/ols.conf" --out "$RUN/throttle.toml" --report "$RUN/translate.md" >"$RUN/translate.log" 2>&1
check "OLS translation exit" 0 "$?"
grep -q 'static_requests_per_second = 2' "$RUN/throttle.toml"
check "OLS translation static RPS" 0 "$?"
grep -q 'dynamic_requests_per_second = 1' "$RUN/throttle.toml"
check "OLS translation dynamic RPS" 0 "$?"
grep -q 'input_bandwidth_bytes_per_second = 131072' "$RUN/throttle.toml"
check "OLS translation inbound bandwidth" 0 "$?"
grep -q 'block_bad_req = true' "$RUN/throttle.toml"
check "OLS translation blockBadReq" 0 "$?"

cat >"$RUN/exyonq.toml" <<EOF
config_version = 1
$(cat "$RUN/throttle.toml")

[[server]]
listen = "127.0.0.1:19380"
routes = ["static", "dyn"]

[[route]]
name = "static"
match = { path = "/static/" }
root = "$RUN/www"

[[route]]
name = "dyn"
match = { path = "/dyn/" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19381"
EOF

"$BIN" validate "$RUN/exyonq.toml" >"$RUN/validate.log" 2>&1
check "validate translated OLS configuration" 0 "$?"

start_exy() {
  EXYONQ_CONTROL_SOCKET="$RUN/control.sock" "$BIN" serve -c "$RUN/exyonq.toml" >"$RUN/exy.log" 2>&1 & EXY_PID=$!
  wait_port 19380 || { echo 'FAIL exyonq did not start'; cat "$RUN/exy.log"; exit 1; }
}
stop_exy() {
  kill "$EXY_PID" 2>/dev/null || true
  wait "$EXY_PID" 2>/dev/null || true
  EXY_PID=""
}
start_exy

# The health probe above creates the per-client state. OLS resets request
# quotas from its timer/client creation, so wait one full window before the
# measured burst; otherwise the third request can inherit the probe's window
# boundary and complete in less than one second even though it was deferred.
sleep 1.05

# staticReqPerSec=2: first two immediate, third deferred to the next second.
T=$(python3 - "$PORT_EXY" <<'PY'
import sys, time, urllib.request
port = sys.argv[1]
start = time.monotonic()
codes = []
for _ in range(3):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/static/file.txt', timeout=10) as r:
        codes.append(r.status)
        r.read()
print(f'{time.monotonic() - start:.3f}', ','.join(map(str, codes)))
PY
)
read -r STATIC_TIME STATIC_CODES <<<"$T"
check "static RPS statuses" "200,200,200" "$STATIC_CODES"
python3 - "$STATIC_TIME" <<'PY'
import sys
if float(sys.argv[1]) < 0.9:
    raise SystemExit('static RPS was not deferred: ' + sys.argv[1])
PY
check "static RPS defers excess" 0 "$?"

# dynReqPerSec=1: second request is deferred to the next window.
T=$(python3 - "$PORT_EXY" <<'PY'
import sys, time, urllib.request
port = sys.argv[1]
start = time.monotonic()
codes = []
for _ in range(2):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/dyn/', timeout=10) as r:
        codes.append(r.status)
        r.read()
print(f'{time.monotonic() - start:.3f}', ','.join(map(str, codes)))
PY
)
read -r DYN_TIME DYN_CODES <<<"$T"
check "dynamic RPS statuses" "200,200" "$DYN_CODES"
python3 - "$DYN_TIME" <<'PY'
import sys
if float(sys.argv[1]) < 0.75:
    raise SystemExit('dynamic RPS was not deferred: ' + sys.argv[1])
PY
check "dynamic RPS defers excess" 0 "$?"

# inBandwidth=128 KiB/s: a 1 MiB upload cannot complete inside the first
# one-second token-bucket burst.
python3 -c 'open("'$RUN'/upload.bin","wb").write(b"u"*(1024*1024))'
T=$(curl -sS -o /dev/null -w '%{time_total}' --max-time 20 -X POST --data-binary @"$RUN/upload.bin" http://127.0.0.1:19380/dyn/upload)
python3 - "$T" <<'PY'
import sys
if float(sys.argv[1]) < 2.0:
    raise SystemExit('inBandwidth did not throttle upload: ' + sys.argv[1])
PY
check "inBandwidth throttles 1 MiB upload" 0 "$?"
echo "inbound sample: $T"

# outBandwidth=128 KiB/s: 1 MiB response takes more than 3 s.
T=$(curl -sS -o /dev/null -w '%{time_total} %{size_download}' --max-time 20 http://127.0.0.1:19380/dyn/large)
read -r OUT_TIME OUT_SIZE <<<"$T"
check "outbound size" 1048576 "$OUT_SIZE"
python3 - "$OUT_TIME" <<'PY'
import sys
if float(sys.argv[1]) < 3.0:
    raise SystemExit('outBandwidth did not throttle response: ' + sys.argv[1])
PY
check "outBandwidth throttles 1 MiB response" 0 "$?"
echo "outbound sample: $T"

# OLS starts sending the body immediately; the token bucket must begin full so
# the first frame is not held for a refill interval.
T=$(python3 - "$PORT_EXY" <<'PY'
import http.client, time
c = http.client.HTTPConnection('127.0.0.1', 19380, timeout=20)
start = time.monotonic()
c.request('GET', '/dyn/large')
r = c.getresponse()
first = r.read(1)
first_at = time.monotonic() - start
rest = r.read()
total = time.monotonic() - start
print(f'{first_at:.3f} {total:.3f} {len(first) + len(rest)}')
PY
)
read -r FIRST_AT OUT_TOTAL OUT_TOTAL_SIZE <<<"$T"
check "outbound body size (first-frame probe)" 1048576 "$OUT_TOTAL_SIZE"
python3 - "$FIRST_AT" "$OUT_TOTAL" <<'PY'
import sys
if float(sys.argv[1]) >= 0.75:
    raise SystemExit('first body byte was held: ' + sys.argv[1])
if float(sys.argv[2]) <= 3.0:
    raise SystemExit('outbound stream was not throttled: ' + sys.argv[2])
PY
check "outbound first frame immediate and stream throttled" 0 "$?"
echo "first-frame sample: first_byte=${FIRST_AT}s total=${OUT_TOTAL}s"

# blockBadReq: three real malformed HTTP/1.1 requests, then a valid request
# must see the ban; after banPeriod it must recover.
python3 - "$PORT_EXY" <<'PY'
import socket, sys
port = int(sys.argv[1])
for _ in range(3):
    s = socket.create_connection(('127.0.0.1', port), timeout=3)
    s.sendall(b'GARBAGE\r\n\r\n')
    try:
        s.recv(64)
    except OSError:
        pass
    s.close()
PY
sleep 0.2
check "blockBadReq closes the next connection" closed "$(closed_probe "$PORT_EXY" /dyn/)"
sleep 2.2
check "blockBadReq ban expires" 200 "$(status http://127.0.0.1:19380/dyn/)"

stop_exy

# hardLimit=1: a second concurrent TCP connection is closed without a response.
cat >"$RUN/exyonq-hard.toml" <<EOF
config_version = 1
[modules.ratelimit.client]
enabled = true
max_connections_per_ip = 1
ban_seconds = 0
grace_seconds = 0

[[server]]
listen = "127.0.0.1:19380"
routes = ["static", "dyn"]

[[route]]
name = "static"
match = { path = "/static/" }
root = "$RUN/www"

[[route]]
name = "dyn"
match = { path = "/dyn/" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19381"
EOF
cp "$RUN/exyonq-hard.toml" "$RUN/exyonq.toml"
start_exy
status http://127.0.0.1:19380/dyn/slow >"$RUN/hard-a" & HARD_PID=$!
sleep 0.3
check "hard limit closes second connection" closed "$(closed_probe "$PORT_EXY" /dyn/)"
wait "$HARD_PID"
check "hard-limit slow request completes" 200 "$(cat "$RUN/hard-a")"
check "hard limit recovers after close" 200 "$(status http://127.0.0.1:19380/dyn/)"
stop_exy

# softLimit=1, hardLimit=2, grace=1, ban=2: a second concurrent connection is
# admitted during grace, then the IP is banned (new connection closed), and the
# ban expires.
cat >"$RUN/exyonq-soft.toml" <<EOF
config_version = 1
[modules.ratelimit.client]
enabled = true
soft_connections_per_ip = 1
max_connections_per_ip = 2
grace_seconds = 1
ban_seconds = 2

[[server]]
listen = "127.0.0.1:19380"
routes = ["static", "dyn"]

[[route]]
name = "static"
match = { path = "/static/" }
root = "$RUN/www"

[[route]]
name = "dyn"
match = { path = "/dyn/" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19381"
EOF
cp "$RUN/exyonq-soft.toml" "$RUN/exyonq.toml"
start_exy
status http://127.0.0.1:19380/dyn/slow >"$RUN/soft-a" & SOFT_PID=$!
sleep 0.3
check "soft limit allows connection inside grace" 200 "$(status http://127.0.0.1:19380/dyn/)"
sleep 1.2
check "soft limit closes connection after grace" closed "$(closed_probe "$PORT_EXY" /dyn/)"
check "soft ban remains active" closed "$(closed_probe "$PORT_EXY" /dyn/)"
wait "$SOFT_PID"
check "slow request completes" 200 "$(cat "$RUN/soft-a")"
sleep 2.2
check "soft ban expires" 200 "$(status http://127.0.0.1:19380/dyn/)"
stop_exy

echo
echo "SUMMARY pass=$PASS fail=$FAIL run=$RUN"
[[ $FAIL -eq 0 ]] || exit 1
echo "RESULT=PASS OL-03 FULL"
