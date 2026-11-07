#!/usr/bin/env bash
# OL-05 / Q-10: real CRS rule hit in Prometheus metrics.
#
# Usage: EXYONQ_CRS_DIR=/path/to/crs-v4.25.0 \
#   bash scripts/e2e/ol05-waf-metrics-e2e.sh [block|detection-only]
#
# The request is the real OWASP corpus case CRS-942100-t1 (Simple SQL
# Injection). Block mode must return 403; detection-only must reach the real
# origin with 200 while still reporting rule 942100 in /metrics.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODE="${1:-block}"
case "$MODE" in
    block|detection-only) ;;
    *)
        echo "usage: $0 [block|detection-only]" >&2
        exit 2
        ;;
esac

EXY="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$EXY" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$EXY" | awk '{print $1}')"
fi
echo "binary=$EXY binary_sha256=$BIN_SHA256"
CRS_DIR="${EXYONQ_CRS_DIR:-$ROOT/.exyonq-local/refs/crs-v4.25.0}"
EXPECTED_CRS_COMMIT=aabf675fcfc5e489b424b844e6c9f1b39802df69
if [[ ! -x "$EXY" ]]; then
    echo "missing exyonq binary: $EXY" >&2
    exit 2
fi
if [[ ! -f "$CRS_DIR/crs-setup.conf.example" || ! -d "$CRS_DIR/rules" ]]; then
    echo "missing real CRS checkout: $CRS_DIR" >&2
    exit 2
fi
actual_commit="$(git -C "$CRS_DIR" rev-parse HEAD 2>/dev/null)" || {
    echo "CRS checkout is not a git repository: $CRS_DIR" >&2
    exit 2
}
if [[ "$actual_commit" != "$EXPECTED_CRS_COMMIT" ]]; then
    echo "CRS commit mismatch: expected $EXPECTED_CRS_COMMIT, got $actual_commit" >&2
    exit 2
fi

WORK="$(mktemp -d "$ROOT/target/ol05-waf-metrics.XXXXXX")"
# Unix-socket chmod is not supported on some container bind mounts (for
# example a macOS checkout mounted into Wolfi). Keep the control socket in a
# local writable directory; the HTTP fixture and config can stay under WORK.
SOCKET_DIR="${OL05_SOCKET_DIR:-${TMPDIR:-/tmp}}"
CONTROL_SOCKET="$SOCKET_DIR/exyonq-ol05-metrics.$$.sock"
rm -f "$CONTROL_SOCKET"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
ORIGIN_PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
EXY_PID=""
ORIGIN_PID=""
cleanup() {
    [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null || true
    [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null || true
    rm -f "$CONTROL_SOCKET"
    wait 2>/dev/null || true
}
trap cleanup EXIT

cat >"$WORK/origin.py" <<'PY'
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.respond()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)
        self.respond()

    def respond(self):
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *_args):
        pass


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY
python3 "$WORK/origin.py" "$ORIGIN_PORT" >"$WORK/origin.log" 2>&1 &
ORIGIN_PID=$!

cat >"$WORK/exy-main.conf" <<EOF
Include $CRS_DIR/crs-setup.conf.example
Include $CRS_DIR/rules/*.conf
EOF

detection_only=false
[[ "$MODE" == "detection-only" ]] && detection_only=true
cat >"$WORK/exy.toml" <<EOF
config_version = 1

[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 1048576

[[waf.ruleset]]
id = "crs"
rules_file = "$WORK/exy-main.conf"
inbound_threshold = 5
detection_only = $detection_only

[waf.ruleset.builtins]
sql_injection = "off"
xss = "off"
path_traversal = "off"
command_injection = "off"
header_anomaly = "off"
bot_ua = "off"
ip_filter = { enabled = false }

[modules.metrics]
enabled = true
path = "/metrics"
health_path = "/exyonq-metrics-health"

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["all"]

[[route]]
name = "all"
match = { path = "/" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:$ORIGIN_PORT"
EOF

"$EXY" validate -c "$WORK/exy.toml"
RUST_LOG="${RUST_LOG:-exyonq_waf=warn}" \
    EXYONQ_CONTROL_SOCKET="$CONTROL_SOCKET" \
    "$EXY" serve -c "$WORK/exy.toml" >"$WORK/exyonq.log" 2>&1 &
EXY_PID=$!

ready=0
for _ in $(seq 1 200); do
    if curl --max-time 1 -sS -o /dev/null "http://127.0.0.1:$PORT/health" 2>/dev/null; then
        ready=1
        break
    fi
    sleep 0.1
done
if [[ "$ready" -ne 1 ]]; then
    echo "server did not become ready" >&2
    cat "$WORK/exyonq.log" >&2
    exit 1
fi

status="$(curl --max-time 10 -sS -o /dev/null -w '%{http_code}' \
    -X POST -A 'OWASP CRS test agent' \
    --data 'var=1234 OR 1=1' \
    "http://127.0.0.1:$PORT/post")"
echo "mode=$MODE attack_status=$status"
if [[ "$MODE" == "block" ]]; then
    [[ "$status" == "403" ]] || { echo "expected 403 in block mode" >&2; exit 1; }
else
    [[ "$status" == "200" ]] || { echo "expected 200 in detection-only mode" >&2; exit 1; }
fi

curl --max-time 10 -sS "http://127.0.0.1:$PORT/metrics" >"$WORK/metrics.txt"
echo "--- selected WAF metrics ---"
grep -E '^exyonq_waf_(inspections_total\{phase="request_body_complete"\}|decisions_total|violations_total\{rule_id="(942100|crs-anomaly-crs|949110)"\}|inspection_seconds_count)' \
    "$WORK/metrics.txt" || true

require_metric() {
    local pattern="$1"
    local description="$2"
    if ! grep -Eq "$pattern" "$WORK/metrics.txt"; then
        echo "missing $description" >&2
        echo "--- all WAF metrics ---" >&2
        grep '^exyonq_waf_' "$WORK/metrics.txt" >&2 || true
        exit 1
    fi
}

require_metric '^exyonq_waf_inspections_total\{phase="request_body_complete"\} [1-9][0-9]*$' \
    "request_body_complete inspection"
require_metric '^exyonq_waf_violations_total\{rule_id="942100"\} [1-9][0-9]*$' \
    "per-rule 942100 violation"
require_metric '^exyonq_waf_inspection_seconds_count [1-9][0-9]*$' \
    "inspection latency count"
if [[ "$MODE" == "block" ]]; then
    require_metric '^exyonq_waf_decisions_total\{action="block"\} [1-9][0-9]*$' \
        "block decision"
else
    require_metric '^exyonq_waf_decisions_total\{action="log"\} [1-9][0-9]*$' \
        "detection-only log decision"
fi
grep -q 'rule_id=942100' "$WORK/exyonq.log" || {
    echo "missing 942100 WAF log evidence" >&2
    exit 1
}

echo "RESULT=PASS OL-05 WAF metrics mode=$MODE"
