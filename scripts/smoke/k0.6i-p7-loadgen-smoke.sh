#!/usr/bin/env bash
# K0.6i — P7 loadgen concurrency smoke (haproxy = amd64 saturation witness).
set -euo pipefail
REPO="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
STAMP="${K0I_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT="${K0I_SMOKE_DIR:-$REPO/benchmarks/results-dev/k0.6i-smoke-${STAMP}}"
LOG="$OUT/smoke.log"
THRESHOLD="${BENCH_LOADGEN_SAT_THRESHOLD:-90}"
mkdir -p "$OUT"

cd "$REPO"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export RESULTS_DIR="$OUT"
export BENCH_RESULTS_DIR="$OUT"
export BENCH_DURATION="${K0I_DURATION:-30s}"
export BENCH_WARMUP_SEC="${K0I_WARMUP_SEC:-20}"
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export BENCH_ALL_SERVERS=0
export BENCH_EXYONQ_ONLY=0
export BENCH_EPOLL_STATIC=1
export EXYONQ_EPOLL_STATIC=1
export BENCH_P7_MAX_PARALLEL="${BENCH_P7_MAX_PARALLEL:-4}"
export BENCH_COMPOSE_FILE="${BENCH_COMPOSE_FILE:-$REPO/benchmarks/docker/docker-compose.bench.yml}"
export BENCH_COMPOSE_DIR="$(dirname "$BENCH_COMPOSE_FILE")"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-docker}"

PERF_DIR="$REPO/benchmarks/scenarios/perf"
FAIL=0

{
  echo "K0.6i P7 loadgen smoke stamp=$STAMP out=$OUT"
  echo "BENCH_P7_MAX_PARALLEL=$BENCH_P7_MAX_PARALLEL threshold=${THRESHOLD}%"
  uname -a
  date -u
} | tee "$LOG"

for proj in docker exyonq-k0h-smoke exyonq-k0-baseline exyonq-protector-diag; do
  docker compose -p "$proj" -f "$BENCH_COMPOSE_FILE" down -v 2>/dev/null || true
done

docker compose -f "$BENCH_COMPOSE_FILE" --profile bench up -d \
  mock-upstream exyonq haproxy bench-runner 2>&1 | tee -a "$LOG"

bash "$PERF_DIR/capture-environment.sh" "$OUT" 2>&1 | tee -a "$LOG"

source "$REPO/benchmarks/scenarios/functional/lib.sh"
DIR="$PERF_DIR"
source "$PERF_DIR/run-rewrk-helper.sh"
DURATION="$BENCH_DURATION"

key=p7
server=haproxy
conns=100
p7_out="$OUT/${key}-${server}.json"
p7_res="$OUT/resources-${key}-${server}.json"
p7_lgout="$OUT/loadgen-${key}-${server}.json"
window=$(( ${BENCH_DURATION%s} + ${BENCH_WARMUP_SEC:-20} ))
base="$(url_for "$server")"

P7_LOG="$OUT/p7-smoke.stderr"
: >"$P7_LOG"
set +e
{
  bash "$PERF_DIR/collect-resources.sh" "$server" "$key" "$window" "$p7_res" &
  cpid=$!
  bash "$PERF_DIR/collect-loadgen.sh" "$key" "$window" "$p7_lgout" &
  lgpid=$!
  rewrk_load_p7 "$base" "$p7_out" "$conns" "$key"
  wait "$cpid"
  wait "$lgpid"
} 2>"$P7_LOG" | tee -a "$LOG"
set -e

if grep -q "wait: pid .* is not a child of this shell" "$P7_LOG"; then
  echo "K0.6i-FAIL: orphan wait on P7" | tee "$OUT/gate.txt"
  FAIL=1
fi

peak=$(python3 - "$p7_lgout" "$THRESHOLD" <<'PY'
import json, sys
from pathlib import Path
path, threshold = Path(sys.argv[1]), float(sys.argv[2])
data = json.loads(path.read_text())
peak = data.get("loadgen_cpu_peak_pct") or data.get("cpu_pct_peak")
peak = float(peak or 0)
sat = bool(data.get("loadgen_saturated")) or peak >= threshold
print(f"peak={peak} saturated={sat}")
sys.exit(1 if sat else 0)
PY
) || {
  echo "K0.6i-FAIL: P7 haproxy loadgen $peak (threshold ${THRESHOLD}%)" | tee "$OUT/gate.txt"
  FAIL=1
}

BENCH_LOAD_MODE=ceiling BENCH_PERF_MODE=docker BENCH_DURATION="$BENCH_DURATION" \
  BENCH_ARCH="${BENCH_ARCH:-$(uname -m)}" \
  bash "$PERF_DIR/patch-run-meta.sh" "$OUT" 2>&1 | tee -a "$LOG"

if [[ "$FAIL" -eq 0 ]]; then
  echo "K0.6i-SMOKE-PASS: P7 haproxy loadgen $peak < ${THRESHOLD}%" | tee "$OUT/gate.txt"
  exit 0
fi
echo "K0.6i smoke FAIL: $OUT" >&2
exit 2
