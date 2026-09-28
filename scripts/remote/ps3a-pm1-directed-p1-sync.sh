#!/usr/bin/env bash
# PS3A-PM1: sync_accept-only directed P1 × N using the same harness as F3 directed.
# Does NOT touch PS2_CANONICAL_BASELINE_INDEX.json.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling

RUN_ID="${PS2_DIRECTED_RUN_ID:-pm1-p1-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps2-directed/${RUN_ID}"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS2_DIRECTED_COMPOSE_PROJECT:-exyonq-ps3a-pm1-p1}"
REPS="${PS2_DIRECTED_REPS:-5}"
PROBE_TIMEOUT="${PS2_DIRECTED_TIMEOUT_SEC:-180}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RESULTS"/{env,logs,modes/sync_accept}

exec > >(tee -a "$RESULTS/logs/pm1-p1-${STAMP}.log") 2>&1
echo "=== PS3A-PM1 directed P1 sync_accept $STAMP run=$RUN_ID reps=$REPS ==="
echo "BASELINE_INDEX_TOUCHED=NO"

unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_IO_URING
export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_IO_URING=
export EXYONQ_SYNC_ACCEPT=1

health_wait() {
  local label="$1" i code
  for i in $(seq 1 30); do
    code="$(curl --connect-timeout 3 --max-time 5 -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/health 2>/dev/null || echo 000)"
    if [[ "$code" == "200" ]]; then
      echo "HEALTH_OK $label code=$code"
      return 0
    fi
    sleep 2
  done
  echo "HEALTH_FAIL $label last=$code"
  return 1
}

{
  echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "host=$(hostname)"
  echo "rustc=$(rustc --version)"
} | tee "$RESULTS/env/environment.txt"

cp "$ROOT/docs/benchmarks/platform-split/ps2-results/PS2_CANONICAL_BASELINE_INDEX.json" \
  "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json"
sha256sum "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json" | tee "$RESULTS/env/baseline-index.sha256"

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
# Rebuild exyonq image so PM1 platform sync_accept is in the binary.
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tee "$RESULTS/logs/build.log" | tail -40
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build upstream bench-runner 2>&1 | tee -a "$RESULTS/logs/build.log" | tail -10
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --force-recreate upstream exyonq bench-runner

for i in $(seq 1 90); do
  mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-upstream-1" 2>/dev/null || echo missing)
  ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
  echo "health_poll=$i upstream=$mu exyonq=$ex"
  [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
  sleep 2
done
docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/modes/sync_accept/boot.log" | tail -20
health_wait boot || true

HEALTH_OK=0
FATAL=0
for rep in $(seq 1 "$REPS"); do
  out="$RESULTS/modes/sync_accept/rep${rep}"
  mkdir -p "$out"
  echo "=== sync_accept rep=$rep ==="
  set +e
  COMPOSE_PROJECT_NAME="$COMPOSE_PROJECT" \
  BENCH_SCENARIOS_FILTER=p1 BENCH_WARMUP_SEC=5 BENCH_DURATION=10s \
    BENCH_RESULTS_DIR="$out" \
    timeout "${PROBE_TIMEOUT}s" cargo run -p xtask -- bench perf --duration 10s --in-docker \
    2>&1 | tee "$out/bench-perf.log"
  rc=$?
  set -e
  echo "bench_perf_rc=$rc" | tee "$out/run-meta.txt"
  docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" >"$out/exyonq.log" 2>&1 || true
  if health_wait "post-rep$rep"; then
    HEALTH_OK=$((HEALTH_OK + 1))
  fi
  if grep -q "sync accept worker stopped" "$out/exyonq.log" 2>/dev/null; then
    FATAL=$((FATAL + 1))
    echo "WORKER_FATAL_DETECTED rep=$rep"
  fi
done

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

HEALTH_5_OF_5=NO
[[ "$HEALTH_OK" -eq "$REPS" ]] && HEALTH_5_OF_5=YES
WORKER_FATAL_EVENTS=$FATAL
PS2_DIRECTED_COMPARISON=FAIL
if [[ "$HEALTH_5_OF_5" == "YES" && "$WORKER_FATAL_EVENTS" -eq 0 ]]; then
  PS2_DIRECTED_COMPARISON=PASS
fi

{
  echo "HEALTH_5_OF_5=$HEALTH_5_OF_5"
  echo "HEALTH_OK_COUNT=$HEALTH_OK/$REPS"
  echo "WORKER_FATAL_EVENTS=$WORKER_FATAL_EVENTS"
  echo "PS2_DIRECTED_COMPARISON=$PS2_DIRECTED_COMPARISON"
  echo "MODE=sync_accept"
  echo "REPS=$REPS"
  echo "BASELINE_INDEX_TOUCHED=NO"
  echo "DIRECTED_PASS=$([[ "$PS2_DIRECTED_COMPARISON" == "PASS" ]] && echo YES || echo NO)"
} | tee "$RESULTS/verdicts.txt"

[[ "$PS2_DIRECTED_COMPARISON" == "PASS" ]]
