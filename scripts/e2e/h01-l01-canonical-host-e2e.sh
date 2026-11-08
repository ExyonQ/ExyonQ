#!/usr/bin/env bash
# H-01/L-01 real E2E: canonical path before auth/route gates, and host-scoped
# listeners reject origin-form HTTP/1.1 without Host. Real auth and real origin
# peers on loopback; portable on macOS and Linux. Not a mock.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${H01L01_RUN_DIR:-$ROOT/.exyonq-local/evidence/h01-l01/run-$STAMP}"
mkdir -p "$RUN/www"

PORT_EXY=19180
PORT_AUTH=19181
PORT_ORIGIN=19182
EXY_PID=""
AUTH_PID=""
ORIGIN_PID=""
PASS=0
FAIL=0

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null
  [[ -n "$AUTH_PID" ]] && kill "$AUTH_PID" 2>/dev/null
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

status() { curl -sS -o /dev/null -w '%{http_code}' --max-time 5 "$@"; }
wait_port() {
  local port=$1
  for _ in $(seq 1 100); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

printf 'open-index' >"$RUN/www/index.html"
printf 'hosted-index' >"$RUN/www/hosted.html"

cat >"$RUN/auth.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def reply(self, status, body=b'', headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)
    def do_GET(self):
        if self.path == '/health':
            self.reply(200, b'ok'); return
        if self.path == '/verify':
            if self.headers.get('Authorization') == 'Custom good':
                self.reply(200, b'ok', [('X-Auth-Request-User', 'alice')]); return
            self.reply(401, b'auth required', [('WWW-Authenticate', 'Custom realm="h01"')]); return
        self.reply(404, b'not found')
    def log_message(self, *args):
        pass

ThreadingHTTPServer(('127.0.0.1', 19181), H).serve_forever()
PY

cat >"$RUN/origin.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def do_GET(self):
        body = ('origin:' + self.path).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass

ThreadingHTTPServer(('127.0.0.1', 19182), H).serve_forever()
PY

cat >"$RUN/main.toml" <<EOF
config_version = 1

[modules.auth.routes.admin]
auth_request = { endpoint = "http://127.0.0.1:${PORT_AUTH}/verify", timeout_ms = 500 }

[[server]]
listen = "127.0.0.1:${PORT_EXY}"
routes = ["admin", "open", "hosted"]

[[route]]
name = "admin"
match = { path = "/admin" }
upstream = "origin"

[[route]]
name = "open"
match = { path = "/open" }
root = "$RUN/www"
index = "index.html"

[[route]]
name = "hosted"
match = { path = "/hosted", host = "hosted.example" }
root = "$RUN/www"
index = "hosted.html"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:${PORT_ORIGIN}"
EOF

echo "== environment =="
echo "host=$(uname -s)/$(uname -m)"
echo "binary=$BIN"
echo "binary_sha256=$(hash_file "$BIN")"
"$BIN" --version || { echo "FAIL binary --version"; exit 1; }
"$BIN" validate "$RUN/main.toml" >"$RUN/validate.log" 2>&1
check "config validates" 0 "$?"

python3 "$RUN/auth.py" >"$RUN/auth.log" 2>&1 & AUTH_PID=$!
python3 "$RUN/origin.py" >"$RUN/origin.log" 2>&1 & ORIGIN_PID=$!
wait_port "$PORT_AUTH" || { echo "FAIL auth server did not start"; exit 1; }
wait_port "$PORT_ORIGIN" || { echo "FAIL origin server did not start"; exit 1; }
RUST_LOG=error "$BIN" serve -c "$RUN/main.toml" >"$RUN/exyonq.log" 2>&1 & EXY_PID=$!
for _ in $(seq 1 100); do
  [[ "$(status "http://127.0.0.1:${PORT_EXY}/admin")" == "401" ]] && break
  kill -0 "$EXY_PID" 2>/dev/null || { cat "$RUN/exyonq.log" >&2; exit 1; }
  sleep 0.05
done

base="http://127.0.0.1:${PORT_EXY}"
check "GET /admin no credentials" 401 "$(status "$base/admin")"
check "GET /./admin no credentials" 401 "$(status --path-as-is "$base/./admin")"
check "GET //admin no credentials" 401 "$(status --path-as-is "$base//admin")"
check "GET /%61dmin no credentials" 401 "$(status --path-as-is "$base/%61dmin")"
check "GET /./admin with token" 200 "$(status --path-as-is -H 'Authorization: Custom good' "$base/./admin")"
body="$(curl -sS --path-as-is -H 'Authorization: Custom good' "$base/./admin")"
check "canonical protected body reaches origin" "origin:/admin" "$body"
check "GET /open/index.html" 200 "$(status "$base/open/index.html")"
for traversal in "/../etc/passwd" "/%2e%2e/etc/passwd"; do
  code="$(status --path-as-is "$base$traversal")"
  if [[ "$code" == "400" || "$code" == "403" ]]; then
    echo "PASS traversal rejected ($traversal -> $code)"
    PASS=$((PASS + 1))
  else
    echo "FAIL traversal not rejected ($traversal -> $code)"
    FAIL=$((FAIL + 1))
  fi
done

# Real raw HTTP/1.1 request with no Host: must be a Hyper 400, not a static 200.
NO_HOST="$(python3 - "$PORT_EXY" <<'PY'
import socket, sys
port = int(sys.argv[1])
s = socket.create_connection(('127.0.0.1', port), timeout=5)
s.sendall(b'GET /hosted/index.html HTTP/1.1\r\nConnection: close\r\n\r\n')
data = b''
while True:
    chunk = s.recv(4096)
    if not chunk:
        break
    data += chunk
s.close()
line = data.split(b'\r\n', 1)[0].decode('ascii', 'replace')
parts = line.split()
print(parts[1] if len(parts) > 1 else 'NO_STATUS')
PY
)"
check "host-scoped route without Host" 400 "$NO_HOST"
check "host-scoped route with correct Host" 200 \
  "$(status -H 'Host: hosted.example' "$base/hosted/index.html")"

echo "SUMMARY pass=$PASS fail=$FAIL run=$RUN"
[[ "$FAIL" -eq 0 && "$PASS" -gt 0 ]] || exit 1
echo "RESULT=PASS H01 L01"
