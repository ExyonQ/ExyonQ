#!/usr/bin/env bash
# PS2-IU1 — minimal io_uring failure reproduction (short, no full functional matrix).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PS2_DIR="$ROOT/docs/benchmarks/platform-split"
RUN_ID="${PS2_IU1_RUN_ID:-iu1-$(date -u +%Y%m%dT%H%M%SZ)}"
PROBE_HOST="${PS2_PROBE_HOST:-netcup-amd64-iu1}"
RESULTS="$PS2_DIR/ps2-results/${PROBE_HOST}/$RUN_ID"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
PROBE_TIMEOUT="${PS2_IU1_PROBE_TIMEOUT_SEC:-120}"

mkdir -p "$RESULTS"/{env,logs,runs,traces}

exec > >(tee -a "$RESULTS/logs/iu1-probe-${STAMP}.log") 2>&1
echo "=== PS2-IU1 io_uring probe $STAMP run=$RUN_ID ==="
cd "$ROOT"

export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export COMPOSE_PROJECT_NAME=exyonq-ps2-iu1
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/ps2-iu1}"

capture_env() {
  local out="$RESULTS/env/environment-${STAMP}.txt"
  {
    echo "=== date -u ==="; date -u +%Y-%m-%dT%H:%M:%SZ
    echo "=== uname -a ==="; uname -a
    echo "=== ulimit ==="; ulimit -a
    echo "=== io_uring sysctls ==="
    sysctl kernel.io_uring_disabled 2>/dev/null || true
    cat /proc/sys/kernel/io_uring_group 2>/dev/null || true
    echo "=== memlock ==="; ulimit -l
    echo "=== cgroup (exyonq if running) ==="
    docker inspect exyonq-ps2-iu1-exyonq-1 --format '{{json .HostConfig}}' 2>/dev/null || true
    echo "=== ss -s ==="; ss -s 2>/dev/null || true
    echo "=== dmesg tail ==="; dmesg -T 2>/dev/null | tail -20 || true
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

bootstrap_io_uring() {
  echo "=== bootstrap io_uring stack ==="
  unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT
  export EXYONQ_IO_URING=1
  export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN=
  docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  docker compose -p exyonq-ps2-baseline -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" --profile bench build mock-upstream exyonq bench-runner 2>&1 | tail -5
  if [[ "${PS2_FORCE_EXYONQ_REBUILD:-0}" == "1" ]]; then
    docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tail -5
  fi
  docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" --profile bench up -d mock-upstream exyonq bench-runner
  for i in $(seq 1 60); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' exyonq-ps2-iu1-mock-upstream-1 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' exyonq-ps2-iu1-exyonq-1 2>/dev/null || echo missing)
    echo "health_poll=$i mock=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && return 0
    sleep 2
  done
  echo "STACK_BOOTSTRAP_FAIL mock=$mu exyonq=$ex"
  docker logs --timestamps exyonq-ps2-iu1-exyonq-1 2>&1 | tail -40
  return 1
}

snapshot_container() {
  local tag="$1"
  local dir="$RESULTS/runs/$tag"
  mkdir -p "$dir"
  docker inspect exyonq-ps2-iu1-exyonq-1 >"$dir/exyonq-inspect.json" 2>/dev/null || true
  docker logs --timestamps exyonq-ps2-iu1-exyonq-1 >"$dir/exyonq.log" 2>&1 || true
  ss -lntp 2>/dev/null | tee "$dir/ss-lntp.txt" || true
  local pid
  pid="$(docker inspect -f '{{.State.Pid}}' exyonq-ps2-iu1-exyonq-1 2>/dev/null || echo 0)"
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
  echo "=== short P1 rep=$rep duration=10s ==="
  set +e
  BENCH_SCENARIOS_FILTER=p1 BENCH_WARMUP_SEC=5 BENCH_DURATION=10s \
    BENCH_RESULTS_DIR="$out" \
    timeout "${PROBE_TIMEOUT}s" cargo run -p xtask -- bench perf --duration 10s --in-docker \
    2>&1 | tee "$out/bench-perf.log"
  local rc=$?
  set -e
  echo "bench_perf_rc=$rc" | tee "$out/run-meta.txt"
  snapshot_container "post-rep${rep}"
  grep -E "io_uring static worker stopped|EAGAIN|error 11" "$RESULTS/runs/post-rep${rep}/exyonq.log" 2>/dev/null | tee "$out/worker-errors.txt" || true
  docker inspect -f '{{.State.Health.Status}}' exyonq-ps2-iu1-exyonq-1 | tee "$out/exyonq-health.txt"
  return "$rc"
}

capture_env

echo "=== identity (bundle unchanged) ==="
bash "$ROOT/scripts/remote/ps2-tree-fingerprint.sh" "$ROOT" | tee "$RESULTS/env/bundle-fingerprint.txt"

if ! bootstrap_io_uring; then
  echo "IU1_ABORT=bootstrap_fail"
  exit 2
fi
snapshot_container "pre-p1"

REPS="${PS2_IU1_REPS:-3}"
fail_workers=0
for rep in $(seq 1 "$REPS"); do
  if ! run_short_p1 "$rep"; then
    echo "IU1_REP_FAIL rep=$rep"
  fi
  if grep -q "io_uring static worker stopped" "$RESULTS/runs/post-rep${rep}/exyonq.log" 2>/dev/null; then
    fail_workers=$((fail_workers + 1))
    echo "WORKER_FATAL_DETECTED rep=$rep"
  fi
  health_wait "http://127.0.0.1:8080/health" "host-exyonq-post-rep$rep" || true
done

docker compose -p exyonq-ps2-iu1 -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

cat >"$RESULTS/iu1-verdicts.txt" <<EOF
IU1_RUN_ID=$RUN_ID
IU1_REPS=$REPS
WORKER_FATAL_EVENTS=$fail_workers
EOF

echo "PS2_IU1_PROBE_COMPLETE run=$RUN_ID worker_fatal_events=$fail_workers"
