#!/usr/bin/env bash
# PS3A-PM3-R2: epoll_listen + epoll_static (keepalive) directed P1 × 5.
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

RUN_ID="${PS3A_PM3_R2_RUN_ID:-pm3r2-p1-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps3a-pm3-r2/${RUN_ID}"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS3A_PM3_R2_COMPOSE_PROJECT:-exyonq-ps3a-pm3-r2}"
REPS="${PS2_DIRECTED_REPS:-5}"
PROBE_TIMEOUT="${PS2_DIRECTED_TIMEOUT_SEC:-180}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RESULTS"/{env,logs,modes/epoll_listen,modes/epoll_static,l1}

exec > >(tee -a "$RESULTS/logs/pm3r2-${STAMP}.log") 2>&1
echo "=== PS3A-PM3-R2 epoll directed+L1 $STAMP run=$RUN_ID reps=$REPS ==="
echo "BASELINE_INDEX_TOUCHED=NO"

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

clear_mode_env() {
  unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT EXYONQ_IO_URING
  export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_SYNC_ACCEPT= EXYONQ_IO_URING=
}

bootstrap_mode() {
  local mode="$1"
  shift
  echo "=== bootstrap mode=$mode env=$* ==="
  clear_mode_env
  # shellcheck disable=SC2086
  eval "export $*"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  if [[ ! -f "$RESULTS/env/exyonq-image-built.flag" ]]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tee "$RESULTS/logs/build.log" | tail -40
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build mock-upstream bench-runner 2>&1 | tee -a "$RESULTS/logs/build.log" | tail -10
    touch "$RESULTS/env/exyonq-image-built.flag"
  fi
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --force-recreate mock-upstream exyonq bench-runner
  for i in $(seq 1 90); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-mock-upstream-1" 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
    echo "health_poll=$i mock=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
    sleep 2
  done
  docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/modes/$mode/boot.log" | tail -30
  health_wait "boot-$mode" || true
}

run_reps() {
  local mode="$1"
  local HEALTH_OK=0
  local FATAL=0
  for rep in $(seq 1 "$REPS"); do
    out="$RESULTS/modes/$mode/rep${rep}"
    mkdir -p "$out"
    echo "=== $mode rep=$rep ==="
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
    if health_wait "post-$mode-rep$rep"; then
      HEALTH_OK=$((HEALTH_OK + 1))
    fi
    if grep -Eiq 'epoll static worker stopped|epoll keep-alive worker stopped|worker fatal' "$out/exyonq.log" 2>/dev/null; then
      FATAL=$((FATAL + 1))
      echo "WORKER_FATAL_DETECTED mode=$mode rep=$rep"
    fi
  done
  echo "MODE=$mode HEALTH_OK=$HEALTH_OK/$REPS FATAL=$FATAL" | tee "$RESULTS/modes/$mode/summary.txt"
}

{
  echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "host=$(hostname)"
  echo "rustc=$(rustc --version 2>/dev/null || true)"
} | tee "$RESULTS/env/environment.txt"

cp "$ROOT/docs/benchmarks/platform-split/ps2-results/PS2_CANONICAL_BASELINE_INDEX.json" \
  "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json"
sha256sum "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json" | tee "$RESULTS/env/baseline-index.sha256"

# L1 uses epoll_listen
bootstrap_mode epoll_listen "EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
run_reps epoll_listen
cp -a "$RESULTS/modes/epoll_listen/." "$RESULTS/l1/" || true

# Default keepalive path (Tokio accept + epoll keepalive pool)
bootstrap_mode epoll_static "EXYONQ_EPOLL_STATIC=1"
run_reps epoll_static

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

echo "=== PS3A-PM3-R2 directed complete ==="
echo "RESULTS=$RESULTS"
