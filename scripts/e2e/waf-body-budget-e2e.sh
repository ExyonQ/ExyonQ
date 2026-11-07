#!/usr/bin/env bash
# Real E2E: WAF body inspection budget and access-log outcome.
#
# Contract:
# - A body over `max_inspection_body_bytes` under `on_inspection_limit =
#   "log_and_allow"` is allowed (documented policy) but its access-log outcome
#   is `waf_body_limit`, never `ok`.
# - Chunked (no Content-Length) and Content-Length bodies follow identical
#   semantics.
# - A real 404 is logged as `not_found`, not `ok`.
set -euo pipefail
: "${EXYONQ_BIN:?set EXYONQ_BIN to the built exyonq binary}"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/exyonq-waf-budget.XXXXXX")
ORIGIN_PID=""
EXY_PID=""
cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null || true
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null || true
  wait "$EXY_PID" "$ORIGIN_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cat > "$TMP/origin.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class H(BaseHTTPRequestHandler):
    def _ok(self):
        body = b'origin'
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    do_GET = _ok
    do_POST = _ok
    def log_message(self, *args): pass
ThreadingHTTPServer(('127.0.0.1', 19082), H).serve_forever()
PY
python3 "$TMP/origin.py" & ORIGIN_PID=$!

cat > "$TMP/exyonq.toml" <<'TOML'
config_version = 1

[waf]
enabled = true
mode = "monitor"
max_inspection_body_bytes = 1024
on_inspection_limit = "log_and_allow"

[[server]]
listen = "127.0.0.1:19081"
routes = ["upload", "small"]

[[route]]
name = "upload"
match = { path = "/upload" }
upstream = "origin"

[[route]]
name = "small"
match = { path = "/small" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19082"
TOML

wait_port() {
  local port=$1
  for _ in $(seq 1 100); do
    if curl -sS --max-time 1 "http://127.0.0.1:$port/" >/dev/null 2>&1; then return 0; fi
    sleep 0.05
  done
  return 1
}
wait_port 19082

EXYONQ_ACCESS_LOG=1 EXYONQ_CONTROL_SOCKET="$TMP/control.sock" \
  "$EXYONQ_BIN" serve -c "$TMP/exyonq.toml" >"$TMP/exy.log" 2>&1 & EXY_PID=$!
for _ in $(seq 1 100); do
  if curl -sS --max-time 1 -o /dev/null "http://127.0.0.1:19081/health" 2>/dev/null; then break; fi
  sleep 0.05
done

python3 - <<'PY'
import http.client
BODY = b'A' * 4096
def req(method, path, body=None, chunked=False):
    c = http.client.HTTPConnection('127.0.0.1', 19081, timeout=10)
    if body is None:
        c.request(method, path)
    elif chunked:
        c.request(method, path, body=body, encode_chunked=True,
                  headers={'Transfer-Encoding': 'chunked', 'Content-Type': 'text/plain'})
    else:
        c.putrequest(method, path)
        c.putheader('Content-Length', str(len(body)))
        c.putheader('Content-Type', 'text/plain')
        c.endheaders()
        c.send(body)
    r = c.getresponse()
    r.read()
    status = r.status
    c.close()
    return status
print('CL', req('POST', '/upload', BODY))
print('CHUNKED', req('POST', '/upload', BODY, chunked=True))
print('SMALL', req('POST', '/small', b'B' * 64))
print('MISSING', req('GET', '/definitely-missing'))
PY

sleep 0.3
LOG="$TMP/exy.log"
echo '--- access lines ---'
grep 'event="access"' "$LOG" || true
echo '--- waf limit line ---'
grep 'EXY-LIMIT-BODY-001' "$LOG" || true

fail() { echo "FAIL: $*"; exit 1; }

grep -q 'EXY-LIMIT-BODY-001' "$LOG" || fail 'engine did not apply on_inspection_limit'
[[ $(grep 'event="access"' "$LOG" | grep 'method=POST' | grep 'status=200' | grep -c 'outcome=.*waf_body_limit') -eq 2 ]] \
  || fail 'CL and chunked allowed bodies must both log outcome=waf_body_limit'
grep 'event="access"' "$LOG" | grep 'status=200' | grep 'outcome=.*waf_body_limit' | grep -q 'path=/small' \
  && fail 'small body must not be marked limited'
grep 'event="access"' "$LOG" | grep 'status=404' | grep -q 'outcome=.*not_found' \
  || fail '404 must log outcome=not_found'
grep 'event="access"' "$LOG" | grep 'path=/small' | grep -q 'outcome=.*ok' \
  || fail 'small body must log outcome=ok'

echo 'WAF_BODY_BUDGET PASS'
echo 'RESULT=PASS WAF-BODY-BUDGET'
