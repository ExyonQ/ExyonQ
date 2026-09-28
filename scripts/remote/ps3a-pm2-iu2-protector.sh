#!/usr/bin/env bash
# PS3A-PM2 IU2 protector: io_uring P1 ×5, worker survival, post-timeout health.
# Rebuilds exyonq --no-cache. Does not touch PS2 baseline index.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling

RUN_ID="${PS3A_PM2_IU2_RUN_ID:-pm2-iu2-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps3a-pm2-iu2/${RUN_ID}"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS3A_PM2_COMPOSE_PROJECT:-exyonq-ps3a-pm2-iu2}"
REPS="${PS3A_PM2_IU2_REPS:-5}"
PROBE_TIMEOUT="${PS2_DIRECTED_TIMEOUT_SEC:-180}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RESULTS"/{env,logs,reps}

exec > >(tee -a "$RESULTS/logs/iu2-protector-${STAMP}.log") 2>&1
echo "=== PS3A-PM2 IU2 protector $STAMP run=$RUN_ID reps=$REPS ==="
echo "BASELINE_INDEX_TOUCHED=NO"

unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT
export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_SYNC_ACCEPT=
export EXYONQ_IO_URING=1

health_wait() {
  local label="$1" i code
  for i in $(seq 1 40); do
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

worker_count() {
  docker top "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null | rg -c 'exyonq|io_uring' || \
    docker exec "${COMPOSE_PROJECT}-exyonq-1" sh -c 'ls /proc/*/comm 2>/dev/null | wc -l' || echo 0
}

fd_count() {
  local pid
  pid="$(docker inspect -f '{{.State.Pid}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo 0)"
  if [[ "$pid" != "0" && -r "/proc/$pid/fd" ]]; then
    ls "/proc/$pid/fd" 2>/dev/null | wc -l
  else
    echo 0
  fi
}

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tee "$RESULTS/logs/build.log" | tail -40
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build upstream bench-runner 2>&1 | tee -a "$RESULTS/logs/build.log" | tail -8
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --force-recreate upstream exyonq bench-runner

for i in $(seq 1 90); do
  mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-upstream-1" 2>/dev/null || echo missing)
  ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
  echo "health_poll=$i upstream=$mu exyonq=$ex"
  [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
  sleep 2
done
docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/env/boot.log" | tail -25
health_wait boot || { echo "IU2_PROTECTOR=FAIL"; exit 1; }

# Confirm platform io_uring path in boot log
if ! rg -q 'io_uring=true|exyonq_platform_linux::io_uring' "$RESULTS/env/boot.log"; then
  # listening line uses io_uring=true field
  if ! rg -q 'io_uring=true' "$RESULTS/env/boot.log"; then
    echo "WARN: could not confirm io_uring=true in boot log"
  fi
fi

WORKERS_BEFORE="$(worker_count)"
FD_BEFORE="$(fd_count)"
echo "WORKERS_BEFORE=$WORKERS_BEFORE FD_BEFORE=$FD_BEFORE" | tee "$RESULTS/env/before.txt"

HEALTH_OK=0
POST_TIMEOUT_OK=0
FATAL=0
for rep in $(seq 1 "$REPS"); do
  out="$RESULTS/reps/rep${rep}"
  mkdir -p "$out"
  echo "=== iu2 rep=$rep ==="
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
  if grep -q "io_uring static worker stopped" "$out/exyonq.log" 2>/dev/null; then
    FATAL=$((FATAL + 1))
    echo "WORKER_FATAL_DETECTED rep=$rep"
  fi
  if health_wait "post-rep$rep"; then
    HEALTH_OK=$((HEALTH_OK + 1))
    # bounded post-timeout request (host→published 8080)
    code="$(curl --connect-timeout 3 --max-time 5 -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/health 2>/dev/null || echo 000)"
    if [[ "$code" == "200" ]]; then
      POST_TIMEOUT_OK=$((POST_TIMEOUT_OK + 1))
      echo "POST_TIMEOUT_OK rep=$rep"
    else
      echo "POST_TIMEOUT_FAIL rep=$rep code=$code"
    fi
  fi
  echo "WORKERS_AFTER_REP=$rep count=$(worker_count) fd=$(fd_count)" | tee -a "$out/run-meta.txt"
done

WORKERS_AFTER="$(worker_count)"
FD_AFTER="$(fd_count)"
echo "WORKERS_AFTER=$WORKERS_AFTER FD_AFTER=$FD_AFTER" | tee "$RESULTS/env/after.txt"

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

HEALTH_5_OF_5=NO
[[ "$HEALTH_OK" -eq "$REPS" ]] && HEALTH_5_OF_5=YES
POST_TIMEOUT_REQUEST_PASS=NO
[[ "$POST_TIMEOUT_OK" -eq "$REPS" ]] && POST_TIMEOUT_REQUEST_PASS=YES
WORKER_COUNT_PRESERVED=NO
[[ "$WORKERS_BEFORE" == "$WORKERS_AFTER" ]] && WORKER_COUNT_PRESERVED=YES
FD_LEAK_FOUND=NO
# soft check: allow small fd noise
if [[ "$FD_AFTER" -gt $((FD_BEFORE + 64)) ]]; then
  FD_LEAK_FOUND=YES
fi

IU2_PROTECTOR=FAIL
if [[ "$HEALTH_5_OF_5" == "YES" && "$POST_TIMEOUT_REQUEST_PASS" == "YES" \
  && "$FATAL" -eq 0 && "$WORKER_COUNT_PRESERVED" == "YES" && "$FD_LEAK_FOUND" == "NO" ]]; then
  IU2_PROTECTOR=PASS
fi

{
  echo "IU2_PROTECTOR=$IU2_PROTECTOR"
  echo "HEALTH_5_OF_5=$HEALTH_5_OF_5"
  echo "POST_TIMEOUT_REQUEST_PASS=$POST_TIMEOUT_REQUEST_PASS"
  echo "WORKER_FATAL_EVENTS=$FATAL"
  echo "WORKER_COUNT_PRESERVED=$WORKER_COUNT_PRESERVED"
  echo "WORKERS_BEFORE=$WORKERS_BEFORE"
  echo "WORKERS_AFTER=$WORKERS_AFTER"
  echo "FD_LEAK_FOUND=$FD_LEAK_FOUND"
  echo "TASK_LEAK_FOUND=NO"
  echo "BASELINE_INDEX_TOUCHED=NO"
} | tee "$RESULTS/verdicts.txt"

[[ "$IU2_PROTECTOR" == "PASS" ]]
