#!/usr/bin/env bash
# K0.5 ExyonQ-only baseline — diagnostic, NOT publishable, no rivals.
#
# Usage (remote):
#   REPO=~/exyonq-dev-soak-src HOST_LABEL=netcup-amd64 ./scripts/smoke/k0-exyonq-baseline.sh
set -euo pipefail

REPO="${REPO:-$(cd "$(dirname "$0")/../.." && pwd)}"
HOST="${HOST_LABEL:-unknown}"
COMMIT="${K0_COMMIT:-$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)}"
HOST_PORT="${K0_PORT:-8090}"
TLS_HOST_PORT="${K0_TLS_PORT:-18443}"
HTTP3_HOST_PORT="${K0_HTTP3_PORT:-18444}"
WARMUP_SEC="${K0_WARMUP_SEC:-5}"
MEASURE_SEC="${K0_MEASURE_SEC:-30}"
EPOLL_STATIC="${EXYONQ_EPOLL_STATIC:-1}"
EPOLL_SENDFILE="${EXYONQ_EPOLL_SENDFILE:-1}"
COMPOSE="$REPO/benchmarks/docker/docker-compose.k0-baseline.yml"
COMPOSE_PROJECT="${K0_COMPOSE_PROJECT:-exyonq-k0-baseline}"
RUN_ROOT="${K0_RUN_ROOT:-$HOME/exyonq-dev-soak/k0.5-baseline}"
OUT_DIR="${K0_OUT_DIR:-$RUN_ROOT/$(date -u +%Y%m%dT%H%M%SZ)-$HOST}"
CONTAINER="${COMPOSE_PROJECT}-exyonq-1"
LOG="$OUT_DIR/k0-baseline.log"
LOADGEN_PY="$REPO/benchmarks/scenarios/perf/adr_025_protector_loadgen.py"

SCENARIOS=(p1 p2 p3 p4 health metrics)
GATE="K0.5-PASS"
NOTES=()

mkdir -p "$OUT_DIR"
dc() { docker compose -f "$COMPOSE" -p "$COMPOSE_PROJECT" "$@"; }
log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$LOG"; }

write_compose() {
  cat >"$COMPOSE" <<EOF
# K0.5 ExyonQ-only baseline — NOT publishable.
services:
  mock-upstream:
    build:
      context: ../..
      dockerfile: benchmarks/docker/Dockerfile.mock-upstream
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://127.0.0.1:9000/health"]
      interval: 5s
      timeout: 3s
      retries: 5

  exyonq:
    build:
      context: ../..
      dockerfile: benchmarks/docker/Dockerfile.exyonq
    ulimits:
      nofile:
        soft: 1048576
        hard: 1048576
    depends_on:
      mock-upstream:
        condition: service_healthy
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_CONTROL_SOCKET: /tmp/exyonq.sock
      EXYONQ_EPOLL_STATIC: \${EXYONQ_EPOLL_STATIC:-1}
      EXYONQ_EPOLL_SENDFILE: \${EXYONQ_EPOLL_SENDFILE:-1}
      EXYONQ_EPOLL_POOL_THREADS: \${EXYONQ_EPOLL_POOL_THREADS:-4}
    ports:
      - "${HOST_PORT}:8080"
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://127.0.0.1:8080/health"]
      interval: 5s
      timeout: 3s
      retries: 10
      start_period: 20s
EOF
}

metric_value() {
  local name="$1" text="${2:-}"
  [[ -n "$text" ]] || text=$(curl -sS --max-time 10 "http://127.0.0.1:${HOST_PORT}/metrics" 2>/dev/null || true)
  echo "$text" | awk -v n="$name" '$1 == n { print $2; exit }'
}

snapshot_metrics() {
  local dir="$1" label="$2"
  mkdir -p "$dir"
  curl -sS --max-time 15 "http://127.0.0.1:${HOST_PORT}/metrics" >"$dir/metrics-${label}.txt" 2>/dev/null || true
  {
    echo "# snapshot: $label"
    echo "# time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    for m in \
      exyonq_epoll_sendfile_complete_total \
      exyonq_epoll_sendfile_error_total \
      exyonq_epoll_sendfile_terminal_503_total \
      exyonq_epoll_sendfile_fallback_total \
      exyonq_epoll_sendfile_unknown_fd_event_total; do
      echo "$m=$(metric_value "$m" "$(cat "$dir/metrics-${label}.txt" 2>/dev/null)")"
    done
  } >"$dir/metrics-${label}.summary"
}

container_stats_line() {
  docker stats --no-stream --format 'cpu={{.CPUPerc}} mem={{.MemUsage}}' "$CONTAINER" 2>/dev/null || echo "unknown"
}

sample_resources_bg() {
  local csv="$1" dur="$2"
  local cid
  cid=$(docker inspect -f '{{.Id}}' "$CONTAINER" 2>/dev/null || echo "")
  [[ -n "$cid" ]] || return 0
  bash "$REPO/benchmarks/scripts/sample_resources.sh" sample "$cid" "$dur" "$csv" &
  echo $!
}

run_scenario() {
  local sid="$1"
  local sdir="$OUT_DIR/$sid"
  mkdir -p "$sdir"
  log "=== scenario $sid warmup=${WARMUP_SEC}s measure=${MEASURE_SEC}s ==="
  snapshot_metrics "$sdir" before
  echo "$(container_stats_line)" >"$sdir/stats-before.txt"
  local csv="$sdir/resources.csv"
  local spid
  spid=$(sample_resources_bg "$csv" "$((WARMUP_SEC + MEASURE_SEC + 5))")
  python3 "$LOADGEN_PY" "$sid" "$sdir/result.json" "$HOST_PORT" "$TLS_HOST_PORT" "$HTTP3_HOST_PORT" \
    "$WARMUP_SEC" "$MEASURE_SEC" | tee -a "$LOG"
  wait "$spid" 2>/dev/null || true
  snapshot_metrics "$sdir" after
  echo "$(container_stats_line)" >"$sdir/stats-after.txt"
  python3 "$REPO/scripts/smoke/k0_resource_summary.py" "$sdir" "$sid" | tee -a "$LOG"
  local fail sr
  fail=$(python3 -c "import json; print(json.load(open('$sdir/result.json'))['fail'])")
  sr=$(python3 -c "import json; print(json.load(open('$sdir/result.json'))['success_rate'])")
  if [[ "$fail" != "0" || "$sr" != "1.0" ]]; then
    GATE="K0.5-FAIL"
    NOTES+=("$sid: fail=$fail success_rate=$sr")
  fi
  local err fb t503 unk
  err=$(awk -F= '$1=="exyonq_epoll_sendfile_error_total"{print $2}' "$sdir/metrics-after.summary" 2>/dev/null || echo 0)
  fb=$(awk -F= '$1=="exyonq_epoll_sendfile_fallback_total"{print $2}' "$sdir/metrics-after.summary" 2>/dev/null || echo 0)
  t503=$(awk -F= '$1=="exyonq_epoll_sendfile_terminal_503_total"{print $2}' "$sdir/metrics-after.summary" 2>/dev/null || echo 0)
  unk=$(awk -F= '$1=="exyonq_epoll_sendfile_unknown_fd_event_total"{print $2}' "$sdir/metrics-after.summary" 2>/dev/null || echo 0)
  for pair in "error:$err" "fallback:$fb" "terminal_503:$t503" "unknown_fd:$unk"; do
    name="${pair%%:*}"; val="${pair##*:}"
    [[ "${val:-0}" == "0" ]] || { GATE="K0.5-FAIL"; NOTES+=("$sid: $name counter=$val"); }
  done
}

log "=== K0.5 ExyonQ-only baseline host=$HOST commit=$COMMIT ==="
{
  echo "k0.5_exyonq_only_baseline=true"
  echo "host=$HOST"
  echo "commit=$COMMIT"
  echo "warmup_s=$WARMUP_SEC"
  echo "measure_s=$MEASURE_SEC"
  echo "epoll_static=$EPOLL_STATIC"
  echo "epoll_sendfile=$EPOLL_SENDFILE"
  uname -a
} >"$OUT_DIR/host-env.txt"

cd "$REPO"
write_compose
export EXYONQ_EPOLL_STATIC="$EPOLL_STATIC"
export EXYONQ_EPOLL_SENDFILE="$EPOLL_SENDFILE"
docker compose -p exyonq-k0-baseline down -v 2>/dev/null || true
log "docker build exyonq..."
dc build exyonq 2>&1 | tail -8 | tee -a "$LOG"
dc up -d mock-upstream exyonq --force-recreate 2>&1 | tee -a "$LOG"
for i in $(seq 1 60); do
  curl -sf "http://127.0.0.1:${HOST_PORT}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "http://127.0.0.1:${HOST_PORT}/health" >/dev/null || { log "FAIL health timeout"; exit 1; }

for sid in "${SCENARIOS[@]}"; do
  run_scenario "$sid"
  sleep 2
done

snapshot_metrics "$OUT_DIR" final
python3 - "$OUT_DIR" "$HOST" "$COMMIT" "$GATE" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1])
host, commit, gate = sys.argv[2:5]
rows = []
for sid in ["p1", "p2", "p3", "p4", "health", "metrics"]:
    p = out / sid / "summary.json"
    if p.exists():
        rows.append(json.loads(p.read_text()))
meta = {
    "k0_5": True,
    "exyonq_only": True,
    "official": False,
    "publishable": False,
    "internal_baseline": True,
    "baseline_id": "k0.5-baseline-frozen",
    "host": host,
    "commit": commit,
    "gate": gate,
    "epoll_static": os.environ.get("EXYONQ_EPOLL_STATIC", "1"),
    "epoll_sendfile": os.environ.get("EXYONQ_EPOLL_SENDFILE", "1"),
    "load_tool": "protector_loadgen_asyncio",
    "warmup_s": int((out / "host-env.txt").read_text().split("warmup_s=")[1].split()[0]) if "warmup_s=" in (out / "host-env.txt").read_text() else None,
    "measure_s": int((out / "host-env.txt").read_text().split("measure_s=")[1].split()[0]) if "measure_s=" in (out / "host-env.txt").read_text() else None,
    "scenarios": rows,
}
(out / "run_meta.json").write_text(json.dumps(meta, indent=2) + "\n")
print(json.dumps(meta, indent=2))
PY

{
  echo "gate=$GATE"
  echo "notes=${NOTES[*]:-none}"
  echo "out_dir=$OUT_DIR"
} >"$OUT_DIR/gate.txt"

log "=== $GATE host=$HOST out=$OUT_DIR ==="
dc down -v 2>/dev/null || true
[[ "$GATE" == "K0.5-PASS" ]]
