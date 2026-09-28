#!/usr/bin/env bash
# K0.6h harness validity smoke: P7 wait + environment.txt (short, ExyonQ-only).
set -euo pipefail
REPO="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
STAMP="${K0H_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT="${K0H_SMOKE_DIR:-$REPO/benchmarks/results-dev/k0.6h-smoke-${STAMP}}"
LOG="$OUT/smoke.log"
mkdir -p "$OUT"

cd "$REPO"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export RESULTS_DIR="$OUT"
export BENCH_RESULTS_DIR="$OUT"
export BENCH_DURATION="${K0H_DURATION:-10s}"
export BENCH_WARMUP_SEC="${K0H_WARMUP_SEC:-5}"
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export BENCH_EXYONQ_ONLY=1
export BENCH_ALL_SERVERS=0
export BENCH_EPOLL_STATIC=1
export EXYONQ_EPOLL_STATIC=1
export BENCH_COMPOSE_FILE="${BENCH_COMPOSE_FILE:-$REPO/benchmarks/docker/docker-compose.bench.yml}"
export BENCH_COMPOSE_DIR="${BENCH_COMPOSE_DIR:-$(dirname "$BENCH_COMPOSE_FILE")}"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-exyonq-k0h-smoke}"

PERF_DIR="$REPO/benchmarks/scenarios/perf"
FAIL=0

{
  echo "K0.6h harness smoke stamp=$STAMP out=$OUT"
  uname -a
  date -u
} | tee "$LOG"

for proj in docker exyonq-k0h-smoke exyonq-k0-baseline exyonq-protector-diag; do
  docker compose -p "$proj" -f "$BENCH_COMPOSE_FILE" down -v 2>/dev/null || true
done
docker compose -f "$BENCH_COMPOSE_FILE" down -v 2>/dev/null || true

# bench-runner depends_on all rivals — use --no-deps for ExyonQ-only smoke.
docker compose -p exyonq-k0h-smoke -f "$BENCH_COMPOSE_FILE" up -d mock-upstream exyonq 2>&1 | tee -a "$LOG"
docker compose -p exyonq-k0h-smoke -f "$BENCH_COMPOSE_FILE" --profile bench up -d --no-deps bench-runner 2>&1 | tee -a "$LOG"

bash "$PERF_DIR/capture-environment.sh" "$OUT" 2>&1 | tee -a "$LOG"

# Minimal P7 iteration with collectors (same pattern as run-perf-inner.sh).
P7_LOG="$OUT/p7-smoke.stderr"
: >"$P7_LOG"
source "$REPO/benchmarks/scenarios/functional/lib.sh"
DIR="$PERF_DIR"
source "$PERF_DIR/run-rewrk-helper.sh"
DURATION="$BENCH_DURATION"
key=p7
server=exyonq
conns=100
p7_out="$OUT/${key}-${server}.json"
p7_res="$OUT/resources-${key}-${server}.json"
p7_lgout="$OUT/loadgen-${key}-${server}.json"
window=$(( ${BENCH_DURATION%s} + ${BENCH_WARMUP_SEC:-5} ))

set +e
{
  bash "$PERF_DIR/collect-resources.sh" "$server" "$key" "$window" "$p7_res" &
  cpid=$!
  bash "$PERF_DIR/collect-loadgen.sh" "$key" "$window" "$p7_lgout" &
  lgpid=$!
  base="$(url_for "$server")"
  rewrk_load_p7 "$base" "$p7_out" "$conns" "$key"
  wait "$cpid"
  cres=$?
  wait "$lgpid"
  lgres=$?
  if [[ "$cres" -ne 0 ]]; then echo "WARN: collect-resources failed for $server $key" >&2; fi
  if [[ "$lgres" -ne 0 ]]; then echo "WARN: collect-loadgen failed for $server $key" >&2; fi
} 2>"$P7_LOG" | tee -a "$LOG"
p7_rc=${PIPESTATUS[0]}
set -e
if [[ "$p7_rc" -ne 0 ]]; then
  echo "K0.6h-SMOKE-FAIL: P7 smoke iteration failed (rc=$p7_rc)" | tee "$OUT/gate.txt"
  FAIL=1
fi

if grep -q "WARN: collect-resources failed" "$P7_LOG" || grep -q "WARN: collect-loadgen failed" "$P7_LOG"; then
  echo "K0.6h-SMOKE-FAIL: false collect WARN on P7" | tee "$OUT/gate.txt"
  grep "WARN: collect" "$P7_LOG" | tee -a "$LOG" || true
  FAIL=1
elif grep -q "wait: pid .* is not a child of this shell" "$P7_LOG"; then
  echo "K0.6h-SMOKE-FAIL: orphan wait on P7" | tee "$OUT/gate.txt"
  FAIL=1
else
  echo "K0.6h-SMOKE-OK: no false collect WARN / orphan wait on P7" | tee -a "$LOG"
fi

if [[ ! -s "$OUT/environment.txt" ]]; then
  echo "K0.6h-SMOKE-FAIL: environment.txt missing or empty" | tee "$OUT/gate.txt"
  FAIL=1
else
  echo "K0.6h-SMOKE-OK: environment.txt present ($(wc -c <"$OUT/environment.txt") bytes)" | tee -a "$LOG"
fi

BENCH_ALL_SERVERS=0 BENCH_EXYONQ_ONLY=1 BENCH_LOAD_MODE=ceiling BENCH_PERF_MODE=docker \
  BENCH_DURATION="$BENCH_DURATION" BENCH_ARCH="${BENCH_ARCH:-$(uname -m)}" \
  bash "$PERF_DIR/patch-run-meta.sh" "$OUT" 2>&1 | tee -a "$LOG"

if ! python3 -c "import json,sys; m=json.load(open(sys.argv[1])); sys.exit(0 if m.get('environment_captured') else 1)" "$OUT/run_meta.json"; then
  echo "K0.6h-SMOKE-FAIL: environment_captured=false in run_meta" | tee "$OUT/gate.txt"
  FAIL=1
else
  echo "K0.6h-SMOKE-OK: environment_captured=true" | tee -a "$LOG"
fi

if [[ "$FAIL" -eq 0 ]]; then
  echo "K0.6h-SMOKE-PASS" | tee "$OUT/gate.txt"
  echo "K0.6h smoke PASS: $OUT"
  exit 0
fi

echo "K0.6h smoke FAIL: $OUT" >&2
exit 2
