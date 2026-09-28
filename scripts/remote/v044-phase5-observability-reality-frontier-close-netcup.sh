#!/usr/bin/env bash
# V044 Phase-5 observability REALITY close — Netcup amd64 (harness-only).
# ZERO_FAKE / NO_SMOKE. Independent journalctl / file / OpenMetrics readback.
set -euo pipefail

ROOT="${ROOT:-/root/exyonq-cfd-phase2}"
RUN_ID="${RUN_ID:-p5rc-$(date -u +%Y%m%dT%H%M%SZ)}"
EVID="${EVID:-${ROOT}/.exyonq-local/evidence/phase5-observability-reality-frontier-close/${RUN_ID}}"
LISTEN_PORT="${LISTEN_PORT:-29181}"
METRICS_PORT="${METRICS_PORT:-29182}"
WORKDIR="${WORKDIR:-/tmp/p5rc-obs-${RUN_ID}}"
MARKER="P5RC-${RUN_ID}"

mkdir -p "$EVID"/{syslog,journald,file-rotation,console-json,openmetrics,fd-stability,rss-stability,thread-stability,queue-stress,performance} \
  "$WORKDIR"/{gen,logs}

cd "$ROOT"
{
  echo "RUN_ID=${RUN_ID}"
  echo "MARKER=${MARKER}"
  echo "HOST=$(hostname)"
  echo "ARCH=$(uname -m)"
  echo "ENTRY_HEAD=$(git rev-parse HEAD 2>/dev/null || echo UNKNOWN)"
  echo "UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$EVID/entry-authority.txt"

echo "[p5rc] cargo build -p exyonq-cfd-dataplane --release"
cargo build -p exyonq-cfd-dataplane --release --bin exyonq-dataplane 2>&1 | tee "$EVID/build.log" | tail -40
BIN="${ROOT}/target/release/exyonq-dataplane"
test -x "$BIN"
sha256sum "$BIN" | tee "$EVID/binary-identities.txt"

# Publish empty route generation via tiny helper crate (isolated [workspace]).
PUB="${ROOT}/target/p5rc_pub_${RUN_ID}"
rm -rf "$PUB"
mkdir -p "$PUB/src"
cat >"$PUB/Cargo.toml" <<EOF
[package]
name = "p5rc_pub"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
exyonq-cfd-gen = { path = "${ROOT}/crates/exyonq-cfd-gen" }
EOF
cat >"$PUB/src/main.rs" <<'RS'
fn main() {
    let dir = std::env::args().nth(1).expect("gen-dir");
    let gd = exyonq_cfd_gen::GenDir::new(&dir);
    gd.ensure().expect("ensure");
    let g = exyonq_cfd_gen::Generation::from_route_table(
        1,
        &exyonq_cfd_gen::RouteTable::default(),
    )
    .expect("gen");
    gd.publish(&g).expect("publish");
    println!("PUBLISHED {dir}");
}
RS
GEN_DIR="${WORKDIR}/gen"
rm -rf "$GEN_DIR"
mkdir -p "$GEN_DIR"
cargo run --release --manifest-path "$PUB/Cargo.toml" -- "$GEN_DIR" 2>&1 | tee "$EVID/gen_publish.log"

LOG_FILE="${WORKDIR}/logs/cfd-obs.jsonl"
export EXYONQ_CFD_OBS_CONSOLE_JSON=1
export EXYONQ_CFD_OBS_FILE="$LOG_FILE"
export EXYONQ_CFD_OBS_FILE_MAX_BYTES=2048
export EXYONQ_CFD_OBS_FILE_KEEP=4
export EXYONQ_CFD_OBS_SYSLOG=1
export EXYONQ_CFD_OBS_JOURNALD=1
export EXYONQ_CFD_OBS_METRICS_LISTEN="127.0.0.1:${METRICS_PORT}"

"$BIN" serve \
  --listen "127.0.0.1:${LISTEN_PORT}" \
  --gen-dir "$GEN_DIR" \
  --shards 1 \
  --schema-version 1 \
  >"${WORKDIR}/stdout.log" 2>"${WORKDIR}/stderr.log" &
DP_PID=$!
echo "DP_PID=${DP_PID}" | tee "$EVID/dataplane.pid"
cleanup() { kill "$DP_PID" 2>/dev/null || true; wait "$DP_PID" 2>/dev/null || true; }
trap cleanup EXIT

ready=0
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${METRICS_PORT}/metrics" >/dev/null 2>&1; then ready=1; break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "DATAPLANE_DIED" | tee "$EVID/fail.txt"
    cat "${WORKDIR}/stderr.log" | tee -a "$EVID/fail.txt"
    exit 3
  fi
  sleep 0.1
done
if [[ "$ready" != 1 ]]; then
  echo "METRICS_NOT_READY" | tee "$EVID/fail.txt"
  cat "${WORKDIR}/stderr.log" | tee -a "$EVID/fail.txt"
  exit 4
fi

FD_START=$(ls "/proc/${DP_PID}/fd" | wc -l | tr -d ' ')
RSS_START=$(awk '/VmRSS/{print $2}' "/proc/${DP_PID}/status")
THR_START=$(ls "/proc/${DP_PID}/task" | wc -l | tr -d ' ')
echo "FD_START=${FD_START} RSS_START_KB=${RSS_START} THREAD_START=${THR_START}" | tee "$EVID/fd-stability/start.txt"

for _ in $(seq 1 500); do
  curl -fsS "http://127.0.0.1:${LISTEN_PORT}/__exyonq_cfd/v1/foundation" >/dev/null || true
done
sleep 1

curl -fsS "http://127.0.0.1:${METRICS_PORT}/metrics" | tee "$EVID/openmetrics/metrics_after.txt" >/dev/null
python3 - <<PY
from pathlib import Path
t = Path("$EVID/openmetrics/metrics_after.txt").read_text()
need = ["exyonq_cfd_requests_total", "# EOF"]
missing = [n for n in need if n not in t]
Path("$EVID/openmetrics/parse.txt").write_text(
    "PASS\n" if not missing else "FAIL missing=" + ",".join(missing) + "\n"
)
print("OPENMETRICS", "PASS" if not missing else "FAIL", missing)
PY

ls -la "${WORKDIR}/logs" | tee "$EVID/file-rotation/ls.txt"
ROTATED=0
if [[ -f "$LOG_FILE" ]]; then
  head -5 "$LOG_FILE" | tee "$EVID/file-rotation/head.jsonl"
  python3 - <<PY
import json
from pathlib import Path
ok = 0
for line in Path("$LOG_FILE").read_text().splitlines():
    line = line.strip()
    if not line:
        continue
    json.loads(line)
    ok += 1
    if ok >= 5:
        break
Path("$EVID/file-rotation/json_parse.txt").write_text(f"PASS lines_parsed={ok}\n")
print("FILE_JSON", ok)
PY
  ROTATED=$(find "${WORKDIR}/logs" -name 'cfd-obs.jsonl.*' | wc -l | tr -d ' ')
  echo "ROTATED_ARTIFACT_COUNT=${ROTATED}" | tee "$EVID/file-rotation/rotated_count.txt"
  if [[ "$ROTATED" -ge 1 ]]; then
    echo "FILE_ROTATION_REAL=PASS" | tee "$EVID/file-rotation/verdict.txt"
  else
    echo "FILE_ROTATION_REAL=FAIL_NO_ROTATED_ARTIFACT" | tee "$EVID/file-rotation/verdict.txt"
  fi
else
  echo "FILE_ROTATION_REAL=FAIL_MISSING_ACTIVE" | tee "$EVID/file-rotation/verdict.txt"
fi

# journald independent readback
journalctl -t exyonq-cfd --since "3 min ago" --no-pager -n 80 2>&1 | tee "$EVID/journald/journalctl.txt" || true
if grep -qE 'exyonq-cfd|"event"|foundation|request' "$EVID/journald/journalctl.txt"; then
  echo "JOURNALD_REAL_E2E=PASS" | tee "$EVID/journald/verdict.txt"
else
  journalctl --since "3 min ago" --no-pager -n 300 2>&1 | grep -i exyonq | head -50 | tee "$EVID/journald/journalctl_grep.txt" || true
  if grep -qiE 'exyonq-cfd|cfd.obs|"event"' "$EVID/journald/journalctl_grep.txt"; then
    echo "JOURNALD_REAL_E2E=PASS" | tee "$EVID/journald/verdict.txt"
  else
    echo "JOURNALD_REAL_E2E=FAIL_OR_EMPTY" | tee "$EVID/journald/verdict.txt"
  fi
fi

# syslog: /dev/log typically lands in journal with SYSLOG_IDENTIFIER
journalctl --since "3 min ago" -o verbose --no-pager 2>/dev/null \
  | grep -E 'SYSLOG_IDENTIFIER=exyonq-cfd|_TRANSPORT=syslog' -A3 | head -80 \
  | tee "$EVID/syslog/journal_syslog_transport.txt" || true
if grep -q 'exyonq-cfd\|SYSLOG_IDENTIFIER' "$EVID/syslog/journal_syslog_transport.txt"; then
  echo "SYSLOG_REAL_E2E=PASS" | tee "$EVID/syslog/verdict.txt"
else
  ls -la /dev/log 2>&1 | tee "$EVID/syslog/socket.txt"
  echo "SYSLOG_REAL_E2E=FAIL_OR_EMPTY_READBACK" | tee "$EVID/syslog/verdict.txt"
fi

# console JSON on stdout
if grep -q '"event"' "${WORKDIR}/stdout.log"; then
  grep '"event"' "${WORKDIR}/stdout.log" | head -5 | tee "$EVID/console-json/sample.jsonl"
  python3 - <<PY
import json
from pathlib import Path
lines = [l for l in Path("$EVID/console-json/sample.jsonl").read_text().splitlines() if l.strip()]
for l in lines:
    json.loads(l)
Path("$EVID/console-json/verdict.txt").write_text("CONSOLE_JSON_REAL=PASS\n")
print("CONSOLE_JSON PASS", len(lines))
PY
else
  cp "${WORKDIR}/stdout.log" "$EVID/console-json/stdout.log"
  echo "CONSOLE_JSON_REAL=FAIL_OR_EMPTY" | tee "$EVID/console-json/verdict.txt"
fi

FD_END=$(ls "/proc/${DP_PID}/fd" | wc -l | tr -d ' ')
RSS_END=$(awk '/VmRSS/{print $2}' "/proc/${DP_PID}/status")
THR_END=$(ls "/proc/${DP_PID}/task" | wc -l | tr -d ' ')
{
  echo "FD_END=${FD_END} RSS_END_KB=${RSS_END} THREAD_END=${THR_END}"
  echo "FD_DELTA=$((FD_END - FD_START)) RSS_DELTA=$((RSS_END - RSS_START)) THR_DELTA=$((THR_END - THR_START))"
} | tee "$EVID/fd-stability/end.txt"
cp "$EVID/fd-stability/end.txt" "$EVID/rss-stability/end.txt"
cp "$EVID/fd-stability/end.txt" "$EVID/thread-stability/end.txt"

grep -E 'events_dropped|obs_events_dropped|requests_total' "$EVID/openmetrics/metrics_after.txt" \
  | tee "$EVID/queue-stress/metrics_snip.txt" || true

{
  echo "RUN_ID=${RUN_ID}"
  echo "MARKER=${MARKER}"
  cat "$EVID/syslog/verdict.txt" 2>/dev/null || echo "SYSLOG_REAL_E2E=UNKNOWN"
  cat "$EVID/journald/verdict.txt" 2>/dev/null || echo "JOURNALD_REAL_E2E=UNKNOWN"
  cat "$EVID/file-rotation/verdict.txt" 2>/dev/null || echo "FILE_ROTATION_REAL=UNKNOWN"
  cat "$EVID/console-json/verdict.txt" 2>/dev/null || echo "CONSOLE_JSON_REAL=UNKNOWN"
  cat "$EVID/openmetrics/parse.txt" 2>/dev/null || true
  echo "ROTATED_ARTIFACT_COUNT=${ROTATED}"
  echo "FD_START=${FD_START} FD_END=${FD_END}"
  echo "RSS_START_KB=${RSS_START} RSS_END_KB=${RSS_END}"
  echo "THREAD_START=${THR_START} THREAD_END=${THR_END}"
} | tee "$EVID/summary.txt"

echo "PHASE5_REALITY_CLOSE_NETCUP=DONE evidence=${EVID}"
