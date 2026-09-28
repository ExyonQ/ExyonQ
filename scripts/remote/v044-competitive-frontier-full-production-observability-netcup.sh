#!/usr/bin/env bash
# Phase-5 CFD observability real-sink check on Netcup (amd64).
# ZERO_FAKE / NO_SMOKE: real process, real TCP, real filesystem, real journal/syslog where present.
set -euo pipefail
ROOT="${1:-/tmp/exyonq-cfd-obs-p5}"
BIN="${ROOT}/exyonq-dataplane"
GEN="${ROOT}/gen"
LOGDIR="${ROOT}/logs"
METRICS_PORT="${METRICS_PORT:-18765}"
LISTEN_PORT="${LISTEN_PORT:-18766}"
EVIDENCE_OUT="${EVIDENCE_OUT:-/tmp/exyonq-cfd-obs-p5-evidence}"

rm -rf "$ROOT" "$EVIDENCE_OUT"
mkdir -p "$GEN" "$LOGDIR" "$EVIDENCE_OUT"

# Minimal generation fixture: expect caller already placed a valid gen tree,
# or we fail closed.
if [[ ! -f "$GEN/generation.json" && ! -f "$GEN/current" ]]; then
  echo "NEED_GENERATION_FIXTURE at $GEN" | tee "$EVIDENCE_OUT/blocker.txt"
  exit 2
fi

export EXYONQ_CFD_OBS_CONSOLE_JSON=1
export EXYONQ_CFD_OBS_FILE="$LOGDIR/cfd-obs.log"
export EXYONQ_CFD_OBS_FILE_MAX_BYTES=4096
export EXYONQ_CFD_OBS_FILE_KEEP=3
export EXYONQ_CFD_OBS_SYSLOG=1
export EXYONQ_CFD_OBS_JOURNALD=1
export EXYONQ_CFD_OBS_METRICS_LISTEN="127.0.0.1:${METRICS_PORT}"

"$BIN" serve --listen "127.0.0.1:${LISTEN_PORT}" --gen-dir "$GEN" --shards 1 --schema-version 1 \
  >"$EVIDENCE_OUT/dataplane.stdout" 2>"$EVIDENCE_OUT/dataplane.stderr" &
DP_PID=$!
cleanup() { kill "$DP_PID" 2>/dev/null || true; wait "$DP_PID" 2>/dev/null || true; }
trap cleanup EXIT

for i in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:${METRICS_PORT}/metrics" >/dev/null 2>&1; then break; fi
  sleep 0.1
done

# Real traffic
curl -fsS "http://127.0.0.1:${LISTEN_PORT}/__exyonq_cfd/v1/foundation" | tee "$EVIDENCE_OUT/foundation.body"
# Force rotation: write many events if foundation emits; else pad via repeated GETs
for i in $(seq 1 200); do
  curl -fsS "http://127.0.0.1:${LISTEN_PORT}/__exyonq_cfd/v1/foundation" >/dev/null || true
done

curl -fsS "http://127.0.0.1:${METRICS_PORT}/metrics" | tee "$EVIDENCE_OUT/metrics.txt"
python3 - <<'PY' "$EVIDENCE_OUT/metrics.txt"
import sys
p=sys.argv[1]
t=open(p).read()
assert "exyonq_cfd_requests_total" in t, "missing requests_total"
assert "# EOF" in t or t.strip().endswith("# EOF"), "missing EOF"
print("METRICS_PARSE=PASS")
PY

if [[ -f "$LOGDIR/cfd-obs.log" ]]; then
  head -5 "$LOGDIR/cfd-obs.log" | tee "$EVIDENCE_OUT/file_head.jsonl"
  python3 - <<'PY' "$EVIDENCE_OUT/file_head.jsonl"
import json,sys
for line in open(sys.argv[1]):
    line=line.strip()
    if not line: continue
    json.loads(line)
print("FILE_JSON_PARSE=PASS")
PY
  ls -la "$LOGDIR" | tee "$EVIDENCE_OUT/rotation_ls.txt"
else
  echo "FILE_MISSING" | tee "$EVIDENCE_OUT/file_missing.txt"
  exit 3
fi

# journald (best-effort real)
if command -v journalctl >/dev/null; then
  journalctl -t exyonq-cfd -n 20 --no-pager 2>/dev/null | tee "$EVIDENCE_OUT/journald.txt" || true
fi

# syslog (best-effort)
if [[ -S /dev/log ]]; then echo "SYSLOG_SOCKET=PRESENT" | tee "$EVIDENCE_OUT/syslog_socket.txt"; fi

echo "PHASE5_NETCUP_OBS_LOCAL_CHECK=DONE" | tee "$EVIDENCE_OUT/summary.txt"
