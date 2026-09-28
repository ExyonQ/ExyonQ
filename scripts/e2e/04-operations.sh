#!/usr/bin/env bash
# AUTHORITATIVE operations E2E for Core Regression Gate.
# Start + readiness HTTP + ctl reload + clean stop; fail on panic/crash.
# USES_REAL_DATA=YES | NOT a smoke gate.
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

[[ -x "${EXYONQCTL_BIN:-}" ]] || {
  echo "RESULT=BLOCKED missing=exyonqctl"
  exit 3
}

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d /tmp/exyonq-cg-ops.XXXXXX)"
DOCROOT="$TMP/public"
mkdir -p "$DOCROOT"
printf 'ops-ready\n' >"$DOCROOT/index.html"

CFG="$TMP/exyonq.toml"
# Short AF_UNIX path (deep evidence roots exceed sockaddr limits).
CTRL_SOCK="/tmp/exyonq-cg-ops-$$.sock"
rm -f "$CTRL_SOCK"

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF

SRV_LOG="$TMP/exyonq.log"
PID=""
cleanup() {
  if [[ -n "${PID:-}" ]] && kill -0 "$PID" 2>/dev/null; then
    kill -TERM "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -f "$CTRL_SOCK"
  rm -rf "$TMP"
}
trap cleanup EXIT

EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
  "$EXYONQ_BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
PID=$!

READY=0
for _ in $(seq 1 80); do
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "FAIL: server exited before readiness"
    cat "$SRV_LOG" || true
    exit 1
  fi
  if curl -sf -o /dev/null "http://127.0.0.1:${PORT}/"; then
    READY=1
    break
  fi
  sleep 0.25
done
[[ "$READY" -eq 1 ]] || {
  echo "FAIL: readiness timeout"
  cat "$SRV_LOG" || true
  exit 1
}
e2e_record_pass "start+readiness GET /"

BODY="$(curl -sS "http://127.0.0.1:${PORT}/")"
[[ "$BODY" == "ops-ready" ]] || {
  echo "FAIL: unexpected body: $BODY"
  exit 1
}
e2e_record_pass "body bytes match"

printf 'ops-after-reload\n' >"$DOCROOT/index.html"
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >/dev/null
AFTER="$(curl -sS "http://127.0.0.1:${PORT}/")"
[[ "$AFTER" == "ops-after-reload" ]] || {
  echo "FAIL: reload did not publish new body: $AFTER"
  exit 1
}
e2e_record_pass "ctl reload succeeded; new body served"

# Real process/runtime crash signatures only (avoid matching prose that says "panic").
if grep -Eiq "thread '.*' panicked|panicked at|fatal runtime error|SIGSEGV|Aborted \\(core dumped\\)" "$SRV_LOG"; then
  echo "FAIL: process crash marker in server log"
  cat "$SRV_LOG" || true
  exit 1
fi
e2e_record_pass "no process crash markers during reload"

kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
sleep 0.5
if curl -sf -o /dev/null "http://127.0.0.1:${PORT}/" 2>/dev/null; then
  echo "FAIL: listener still accepting after stop"
  exit 1
fi
e2e_record_pass "clean stop; listener closed"

e2e_finish "04-operations"
