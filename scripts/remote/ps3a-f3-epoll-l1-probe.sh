#!/usr/bin/env bash
# PS3A-F3 EPOLL_NETCUP_L1 — short P1 soak for epoll_listen after I6 (not a PS2 claim).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PS2_DIR="$ROOT/docs/benchmarks/platform-split"
RUN_ID="${PS3A_F3_EPOLL_L1_RUN_ID:-epoll-l1-$(date -u +%Y%m%dT%H%M%SZ)}"
PROBE_HOST="${PS2_PROBE_HOST:-ps3a-f3-epoll-l1}"
RESULTS="$PS2_DIR/ps3a-f3-i5-linux/${PROBE_HOST}/$RUN_ID"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
PROBE_TIMEOUT="${PS3A_F3_EPOLL_PROBE_TIMEOUT_SEC:-120}"
COMPOSE_PROJECT="${PS3A_F3_EPOLL_COMPOSE_PROJECT:-exyonq-ps3a-epoll-l1}"

mkdir -p "$RESULTS"/{env,logs,runs,traces}

exec > >(tee -a "$RESULTS/logs/epoll-l1-probe-${STAMP}.log") 2>&1
echo "=== PS3A-F3 EPOLL_NETCUP_L1 probe $STAMP run=$RUN_ID ==="
cd "$ROOT"

export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export COMPOSE_PROJECT_NAME="$COMPOSE_PROJECT"
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/ps3a-f3-l1}"

capture_env() {
  local out="$RESULTS/env/environment-${STAMP}.txt"
  {
    echo "=== date -u ==="; date -u +%Y-%m-%dT%H:%M:%SZ
    echo "=== uname -a ==="; uname -a
    echo "=== mode ==="; echo "EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
  } | tee "$out"
}

health_wait() {
  local url="$1" label="$2"
  local i code
  for i in $(seq 1 30); do
    code="$(curl --connect-timeout 3 --max-time 5 -s -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || echo 000)"
    if [[ "$code" == "200" ]]; then
      echo "HEALTH_OK $label code=$code attempt=$i"
      return 0
    fi
    sleep 2
  done
  echo "HEALTH_FAIL $label last_code=$code"
  return 1
}

bootstrap_epoll_listen() {
  echo "=== bootstrap epoll_listen stack ==="
  unset EXYONQ_IO_URING EXYONQ_SYNC_ACCEPT
  export EXYONQ_EPOLL_STATIC=1
  export EXYONQ_EPOLL_LISTEN=1
  export EXYONQ_IO_URING=
  export EXYONQ_SYNC_ACCEPT=
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build mock-upstream exyonq bench-runner 2>&1 | tail -8
  if [[ "${PS2_FORCE_EXYONQ_REBUILD:-0}" == "1" ]]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tail -8
  fi
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d mock-upstream exyonq bench-runner
  for i in $(seq 1 60); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-mock-upstream-1" 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
    echo "health_poll=$i mock=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && return 0
    sleep 2
  done
  echo "STACK_BOOTSTRAP_FAIL mock=$mu exyonq=$ex"
  docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tail -40
  return 1
}

snapshot_container() {
  local tag="$1"
  local dir="$RESULTS/runs/$tag"
  mkdir -p "$dir"
  local c="${COMPOSE_PROJECT}-exyonq-1"
  docker inspect "$c" >"$dir/exyonq-inspect.json" 2>/dev/null || true
  docker logs --timestamps "$c" >"$dir/exyonq.log" 2>&1 || true
  ss -lntp 2>/dev/null | tee "$dir/ss-lntp.txt" || true
  local pid
  pid="$(docker inspect -f '{{.State.Pid}}' "$c" 2>/dev/null || echo 0)"
  if [[ "$pid" != "0" && -r "/proc/$pid/status" ]]; then
    cp "/proc/$pid/status" "$dir/exyonq-proc-status.txt"
    cp "/proc/$pid/limits" "$dir/exyonq-proc-limits.txt"
    ls "/proc/$pid/fd" 2>/dev/null | wc -l | tee "$dir/exyonq-fd-count.txt"
  fi
}

run_short_p1() {
  local rep="$1"
  local out="$RESULTS/runs/rep${rep}-${STAMP}"
  mkdir -p "$out"
  echo "=== short P1 epoll_listen rep=$rep duration=10s ==="
  set +e
  BENCH_SCENARIOS_FILTER=p1 BENCH_WARMUP_SEC=5 BENCH_DURATION=10s \
    BENCH_RESULTS_DIR="$out" \
    timeout "${PROBE_TIMEOUT}s" cargo run -p xtask -- bench perf --duration 10s --in-docker \
    2>&1 | tee "$out/bench-perf.log"
  local rc=$?
  set -e
  echo "bench_perf_rc=$rc" | tee "$out/run-meta.txt"
  snapshot_container "post-rep${rep}"
  # Fatal worker markers for epoll (keep loose — panic/abort/worker stopped)
  grep -Ei "epoll.*(worker|loop).*(stopped|fatal|panic)|panicked at|accept worker" \
    "$RESULTS/runs/post-rep${rep}/exyonq.log" 2>/dev/null | tee "$out/worker-errors.txt" || true
  docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" | tee "$out/exyonq-health.txt"
  return "$rc"
}

capture_env

if ! bootstrap_epoll_listen; then
  echo "EPOLL_L1_ABORT=bootstrap_fail"
  exit 2
fi

# Confirm listen log mode
docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/logs/exyonq-boot.log" | tail -20
if ! grep -q 'epoll_static=true' "$RESULTS/logs/exyonq-boot.log" 2>/dev/null \
  && ! grep -q 'epoll_static.*true' "$RESULTS/logs/exyonq-boot.log" 2>/dev/null; then
  # structured log format uses epoll_static=...
  if ! grep -E 'epoll_static.{0,3}true|epoll_static=true' "$RESULTS/logs/exyonq-boot.log" >/dev/null 2>&1; then
    echo "WARN: could not confirm epoll_static=true in boot log (continuing)"
  fi
fi

snapshot_container "pre-p1"

REPS="${PS3A_F3_EPOLL_L1_REPS:-5}"
fail_workers=0
for rep in $(seq 1 "$REPS"); do
  if ! run_short_p1 "$rep"; then
    echo "EPOLL_L1_REP_FAIL rep=$rep"
  fi
  if [[ -s "$RESULTS/runs/rep${rep}-${STAMP}/worker-errors.txt" ]]; then
    fail_workers=$((fail_workers + 1))
    echo "WORKER_FATAL_DETECTED rep=$rep"
  fi
  health_wait "http://127.0.0.1:8080/health" "host-exyonq-post-rep$rep" || true
done

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

cat >"$RESULTS/epoll-l1-verdicts.txt" <<EOF
EPOLL_L1_RUN_ID=$RUN_ID
EPOLL_L1_REPS=$REPS
MODE=epoll_listen
WORKER_FATAL_EVENTS=$fail_workers
EOF

echo "PS3A_F3_EPOLL_L1_COMPLETE run=$RUN_ID worker_fatal_events=$fail_workers"
