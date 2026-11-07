#!/usr/bin/env bash
# Real Linux E2E for AP-04 external auth and OL-03 per-client throttling.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED"
  exit 2
fi
: "${EXYONQ_BIN:?set EXYONQ_BIN to the built exyonq binary}"

TMP=$(mktemp -d /tmp/exyonq-ap04-ol03.XXXXXX)
AUTH_PID=""
ORIGIN_PID=""
EXY_PID=""
cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null || true
  [[ -n "$AUTH_PID" ]] && kill "$AUTH_PID" 2>/dev/null || true
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null || true
  wait "$EXY_PID" "$AUTH_PID" "$ORIGIN_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cat > "$TMP/auth.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/verify':
            auth = self.headers.get('authorization', '')
            if auth == 'Bearer good':
                self.send_response(200); self.send_header('X-Auth-Request-User', 'alice'); self.end_headers(); self.wfile.write(b'ok'); return
            if auth == 'Bearer forbidden':
                self.send_response(403); self.end_headers(); return
            self.send_response(401); self.send_header('WWW-Authenticate', 'Bearer realm="api"'); self.end_headers(); return
        self.send_response(404); self.end_headers()
    def log_message(self, *args): pass
ThreadingHTTPServer(('127.0.0.1', 19001), H).serve_forever()
PY
cat > "$TMP/origin.py" <<'PY'
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/slow'):
            time.sleep(3)
            body = b'slow'
        elif self.path.startswith('/large'):
            body = b'x' * (1024 * 1024)
        else:
            body = b'origin'
        self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *args): pass
ThreadingHTTPServer(('127.0.0.1', 19000), H).serve_forever()
PY
python3 "$TMP/auth.py" & AUTH_PID=$!
python3 "$TMP/origin.py" & ORIGIN_PID=$!

wait_port() {
  local port=$1
  for _ in $(seq 1 100); do
    if curl -sS --max-time 1 "http://127.0.0.1:$port/health" >/dev/null 2>&1; then return 0; fi
    sleep 0.05
  done
  return 1
}
wait_port 19000
wait_port 19001

write_config() {
  local auth=$1
  local extra=$2
  cat > "$TMP/exyonq.toml" <<EOF
config_version = 1
$auth
$extra
[[server]]
listen = "127.0.0.1:19080"
routes = ["all"]

[[route]]
name = "all"
match = { path = "/" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19000"
EOF
}

start_exy() {
  EXYONQ_CONTROL_SOCKET="$TMP/control.sock" "$EXYONQ_BIN" serve -c "$TMP/exyonq.toml" >"$TMP/exy.log" 2>&1 & EXY_PID=$!
  wait_port 19080
}
stop_exy() {
  kill "$EXY_PID" 2>/dev/null || true
  wait "$EXY_PID" 2>/dev/null || true
  EXY_PID=""
}
status() { curl -sS -o /dev/null -w '%{http_code}' --max-time 5 "$@"; }
auth_header() { printf 'Authorization: %s %s' 'Bearer' "$1"; }

write_config '[modules.auth.routes.all]
auth_request = { endpoint = "http://127.0.0.1:19001/verify", timeout_ms = 500 }' ''
start_exy
[[ "$(status http://127.0.0.1:19080/)" == 401 ]] || { echo 'AP04 no-auth != 401'; exit 1; }
[[ "$(status -H "$(auth_header good)" http://127.0.0.1:19080/)" == 200 ]] || { echo 'AP04 good != 200'; exit 1; }
[[ "$(curl -sS -H "$(auth_header good)" http://127.0.0.1:19080/)" == origin ]] || { echo 'AP04 origin body mismatch'; exit 1; }
[[ "$(status -H "$(auth_header forbidden)" http://127.0.0.1:19080/)" == 403 ]] || { echo 'AP04 forbidden != 403'; exit 1; }
kill "$AUTH_PID"; wait "$AUTH_PID" 2>/dev/null || true; AUTH_PID=""
[[ "$(status -H "$(auth_header good)" http://127.0.0.1:19080/)" == 503 ]] || { echo 'AP04 auth-down != 503'; exit 1; }
echo 'AP04 PASS'
stop_exy

write_config '' '[modules.ratelimit.client]
enabled = true
soft_connections_per_ip = 1
max_connections_per_ip = 1
bandwidth_bytes_per_second = 131072
ban_seconds = 0
grace_seconds = 0'
start_exy
status http://127.0.0.1:19080/slow >"$TMP/a" & P1=$!
status http://127.0.0.1:19080/slow >"$TMP/b" & P2=$!
wait "$P1" "$P2"
A=$(cat "$TMP/a"); B=$(cat "$TMP/b")
[[ "$A$B" == *200* && "$A$B" == *429* ]] || { echo "OL03 concurrent unexpected: $A/$B"; exit 1; }
T=$(curl -sS -o /dev/null -w '%{time_total} %{size_download}' --max-time 10 http://127.0.0.1:19080/large)
TIME=${T%% *}; SIZE=${T##* }
python3 - "$T" <<'PY'
import sys
time_value, size_value = sys.argv[1].split()
if float(time_value) < 3.0 or int(size_value) != 1024 * 1024:
    raise SystemExit('OL03 bandwidth did not throttle: ' + sys.argv[1])
PY
stop_exy

# OpenLiteSpeed soft/hard limit: one slot over soft for the grace window bans
# the IP for banPeriod, and the ban expires afterwards.
write_config '' '[modules.ratelimit.client]
enabled = true
soft_connections_per_ip = 1
max_connections_per_ip = 2
grace_seconds = 1
ban_seconds = 2'
start_exy
status http://127.0.0.1:19080/slow >"$TMP/soft-a" & SP=$!
sleep 0.3
[[ "$(status http://127.0.0.1:19080/)" == 200 ]] || { echo 'OL03 grace request != 200'; exit 1; }
sleep 1.2
[[ "$(status http://127.0.0.1:19080/)" == 429 ]] || { echo 'OL03 soft grace ban != 429'; exit 1; }
[[ "$(status http://127.0.0.1:19080/)" == 429 ]] || { echo 'OL03 active ban != 429'; exit 1; }
wait "$SP"
sleep 2.2
[[ "$(status http://127.0.0.1:19080/)" == 200 ]] || { echo 'OL03 ban expiry != 200'; exit 1; }
echo "OL03 PASS concurrent=$A/$B large=$T ban=429/429->200"
stop_exy
echo 'RESULT=PASS AP-04 OL-03'
