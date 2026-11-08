#!/usr/bin/env bash
# H-04 real E2E: the default WAF configuration must inspect the whole proxy
# body, not just the first MiB. A real CRS 942100 payload at the tail of a
# 2 MiB body must be blocked; a benign 2 MiB body must reach the origin.
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

PORT="$(e2e_pick_port)"
ORIGIN_PORT="$(e2e_pick_port)"
[[ "$PORT" != "$ORIGIN_PORT" ]] || { echo "FAIL: duplicate port"; exit 1; }
WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-h04-waf.XXXXXX")"
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

# Deliberately omit [waf] mode/max_inspection_body_bytes/on_inspection_limit:
# this test exercises the product defaults (Block, 64 MiB, Block).
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
routes = ["post"]

[[route]]
name = "post"
match = { path = "/post" }
upstream = "origin"

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

python3 - "$PORT" <<'PY' | tee "$WORK/statuses"
import http.client
import os
import sys
import time

port = int(sys.argv[1])
size = int(os.environ.get("H04_BODY_BYTES", str(2 * 1024 * 1024)))
timeout = int(os.environ.get("H04_CLIENT_TIMEOUT", "120"))
attack = b"var=1234 OR 1=1"
benign = b"A" * size
poisoned = b"A" * (size - len(attack)) + attack


def post(path, body):
    started = time.monotonic()
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    c.request(
        "POST",
        path,
        body=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    r = c.getresponse()
    r.read()
    status = r.status
    c.close()
    return status, time.monotonic() - started


benign_status, benign_secs = post("/post", benign)
print(f"BENIGN={benign_status}")
print(f"BENIGN_SECS={benign_secs:.2f}")
attack_status, attack_secs = post("/post", poisoned)
print(f"TAIL_ATTACK={attack_status}")
print(f"TAIL_ATTACK_SECS={attack_secs:.2f}")
PY

benign="$(awk -F= '$1 == "BENIGN" {print $2}' "$WORK/statuses")"
attack="$(awk -F= '$1 == "TAIL_ATTACK" {print $2}' "$WORK/statuses")"
benign_secs="$(awk -F= '$1 == "BENIGN_SECS" {print $2}' "$WORK/statuses")"
attack_secs="$(awk -F= '$1 == "TAIL_ATTACK_SECS" {print $2}' "$WORK/statuses")"
size_label="$(awk -v b="${H04_BODY_BYTES:-2097152}" 'BEGIN { printf "%.2f MiB", b / 1048576 }')"
origin_hits="$(cat "$WORK/hits" 2>/dev/null || echo 0)"
echo "body_bytes=${H04_BODY_BYTES:-2097152} ($size_label)"
echo "benign_status=$benign tail_attack_status=$attack origin_hits=$origin_hits"
echo "benign_secs=$benign_secs tail_attack_secs=$attack_secs"
echo "--- WAF evidence ---"
grep -E 'rule_id=942100|942100' "$WORK/exyonq.log" | tail -5 || true

fail() { echo "FAIL: $*" >&2; exit 1; }

[[ "$benign" == "200" ]] || fail "benign $size_label body must reach the origin (got $benign)"
[[ "$attack" == "403" ]] || fail "tail SQLi payload must be blocked (got $attack)"
[[ "$origin_hits" == "1" ]] || fail "origin must see only the benign request; got $origin_hits"
grep -q '942100' "$WORK/exyonq.log" || fail "CRS rule 942100 was not recorded"
grep -q 'EXY-LIMIT-BODY-001' "$WORK/exyonq.log" \
  && fail "$size_label body must not hit the inspection limit"

echo "PASS  benign $size_label body reaches origin"
echo "PASS  SQLi payload at the tail of a $size_label body is blocked by CRS 942100"
echo "RESULT=PASS H-04 WAF body-tail inspection"
