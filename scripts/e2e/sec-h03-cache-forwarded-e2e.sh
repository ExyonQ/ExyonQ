#!/usr/bin/env bash
# H-03 real E2E: client forwarding metadata must not be forwarded on the
# cache GET path, and a cached response must not carry a poisoned host.
#
# The origin records the four client-controlled forwarding headers. A poisoned
# request warms the cache; a clean request must receive the same poison-free
# body with x-cache: hit, and the origin must be hit exactly once.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_ensure_bins

PORT="$(e2e_pick_port)"
ORIGIN_PORT="$(e2e_pick_port)"
[[ "$PORT" != "$ORIGIN_PORT" ]] || { echo "FAIL: duplicate port"; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-h03-cache.XXXXXX")"
EXY_PID=""
ORIGIN_PID=""
cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null || true
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null || true
  wait "$EXY_PID" "$ORIGIN_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

cat >"$WORK/origin.py" <<'PY'
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

hits = 0
hits_path = sys.argv[2]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        global hits
        hits += 1
        with open(hits_path, "w", encoding="ascii") as f:
            f.write(str(hits))
        xfh = self.headers.get("X-Forwarded-Host", "-")
        fwd = self.headers.get("Forwarded", "-")
        xri = self.headers.get("X-Real-IP", "-")
        xfp = self.headers.get("X-Forwarded-Port", "-")
        body = f"hit={hits} xfh={xfh} fwd={fwd} xri={xri} xfp={xfp}".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Cache-Control", "public, max-age=60")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY

python3 "$WORK/origin.py" "$ORIGIN_PORT" "$WORK/hits" >"$WORK/origin.log" 2>&1 &
ORIGIN_PID=$!

cat >"$WORK/exyonq.toml" <<EOF
config_version = 1

[waf]
enabled = false

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api" }
upstream = "origin"
cache = "default"

[[cache_policy]]
name = "default"
ttl_seconds = 60
max_object_bytes = 1048576

[[upstream]]
name = "origin"
target = "http://127.0.0.1:$ORIGIN_PORT"
timeout_ms = 5000
EOF

for _ in $(seq 1 100); do
  if curl -sS --max-time 1 -o /dev/null "http://127.0.0.1:$ORIGIN_PORT/" 2>/dev/null; then break; fi
  sleep 0.05
done

EXYONQ_CONTROL_SOCKET="$WORK/control.sock" "$EXYONQ_BIN" serve -c "$WORK/exyonq.toml" \
  >"$WORK/exyonq.log" 2>&1 &
EXY_PID=$!
ready=0
for _ in $(seq 1 200); do
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

URL="http://127.0.0.1:$PORT/api/poison?nonce=h03"
curl -sS --max-time 10 -D "$WORK/poison.hdr" -o "$WORK/poison.body" \
  -H 'X-Forwarded-Host: evil.example' \
  -H 'Forwarded: for=6.6.6.6;host=evil.example' \
  -H 'X-Real-IP: 6.6.6.6' \
  -H 'X-Forwarded-Port: 6666' \
  "$URL"
poison_hits="$(cat "$WORK/hits" 2>/dev/null || echo 0)"
curl -sS --max-time 10 -D "$WORK/clean1.hdr" -o "$WORK/clean1.body" "$URL"
clean1_hits="$(cat "$WORK/hits" 2>/dev/null || echo 0)"
curl -sS --max-time 10 -D "$WORK/clean2.hdr" -o "$WORK/clean2.body" "$URL"
clean2_hits="$(cat "$WORK/hits" 2>/dev/null || echo 0)"

poison_body="$(cat "$WORK/poison.body")"
clean1_body="$(cat "$WORK/clean1.body")"
clean2_body="$(cat "$WORK/clean2.body")"

echo "hits poison=$poison_hits clean1=$clean1_hits clean2=$clean2_hits"
echo "poison_body=$poison_body"
echo "clean1_body=$clean1_body"
echo "clean2_body=$clean2_body"

fail() { echo "FAIL: $*" >&2; exit 1; }

[[ "$clean2_hits" == "$clean1_hits" ]] || fail "second clean replay reached the origin ($clean1_hits -> $clean2_hits)"
[[ "$clean2_hits" -le 2 ]] || fail "poisoned/clean warmup hit origin too often ($clean2_hits)"
[[ "$poison_body" != *"evil.example"* ]] || fail "origin received X-Forwarded-Host/Forwarded"
[[ "$clean1_body" != *"evil.example"* && "$clean2_body" != *"evil.example"* ]] \
  || fail "cached body contains poisoned forwarding metadata"
[[ "$poison_body" == *"xfh=-"* && "$poison_body" == *"fwd=-"* && "$poison_body" == *"xri=-"* && "$poison_body" == *"xfp=-"* ]] \
  || fail "origin did not see all forwarding headers stripped"
[[ "$clean1_body" == "$clean2_body" ]] || fail "cached replay changed body"
[[ "$poison_body" == *"hit=1"* || "$poison_body" == *"hit=2"* ]] \
  || fail "unexpected origin hit marker in poison body"
[[ "$clean1_body" == *"xfh=-"* && "$clean1_body" == *"fwd=-"* && "$clean1_body" == *"xri=-"* && "$clean1_body" == *"xfp=-"* ]] \
  || fail "cached clean body shows forwarding metadata"

echo "PASS  X-Forwarded-Host/Forwarded/X-Real-IP/X-Forwarded-Port stripped before origin"
echo "PASS  second clean replay is served from cache with the poison-free body"
echo "RESULT=PASS H-03 cache forwarding metadata"
