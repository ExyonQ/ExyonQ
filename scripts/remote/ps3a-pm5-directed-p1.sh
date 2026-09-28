#!/usr/bin/env bash
# PS3A-PM5: directed P1 ×5 for all five worker modes vs frozen PS2 baseline.
# Does NOT touch PS2_CANONICAL_BASELINE_INDEX.json. Not a full R7C rebuild.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling

RUN_ID="${PS3A_PM5_DIRECTED_RUN_ID:-pm5-p1-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps3a-pm5/${RUN_ID}"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS3A_PM5_COMPOSE_PROJECT:-exyonq-ps3a-pm5}"
REPS="${PS2_DIRECTED_REPS:-5}"
PROBE_TIMEOUT="${PS2_DIRECTED_TIMEOUT_SEC:-180}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RESULTS"/{env,logs,modes,functional}

exec > >(tee -a "$RESULTS/logs/pm5-directed-${STAMP}.log") 2>&1
echo "=== PS3A-PM5 directed P1 $STAMP run=$RUN_ID reps=$REPS ==="
echo "BASELINE_INDEX_TOUCHED=NO"
echo "FULL_R7C=NO (human-authorized directed + prescribed contracts)"

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

request_ok() {
  local label="$1" code
  code="$(curl --connect-timeout 3 --max-time 5 -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/site/1k.bin 2>/dev/null || echo 000)"
  [[ "$code" == "200" ]] && echo "REQUEST_OK $label code=$code" && return 0
  echo "REQUEST_FAIL $label code=$code"
  return 1
}

keepalive_ok() {
  # Two sequential GETs on one curl connection (HTTP keep-alive).
  local label="$1" out
  out="$(curl --connect-timeout 3 --max-time 8 -s -o /dev/null -w '%{http_code} %{http_code}' \
    http://127.0.0.1:8080/site/1k.bin http://127.0.0.1:8080/site/1k.bin 2>/dev/null || echo '000 000')"
  if [[ "$out" == "200 200" ]]; then
    echo "KEEPALIVE_OK $label"
    return 0
  fi
  echo "KEEPALIVE_FAIL $label out=$out"
  return 1
}

clear_mode_env() {
  unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT EXYONQ_IO_URING
  export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_SYNC_ACCEPT= EXYONQ_IO_URING=
}

apply_mode_env() {
  local chunk pair key val
  for chunk in "$@"; do
    [[ -z "$chunk" || "$chunk" == ":" ]] && continue
    # shellcheck disable=SC2086
    for pair in $chunk; do
      [[ "$pair" != *=* ]] && continue
      key="${pair%%=*}"; val="${pair#*=}"
      export "${key}=${val}"
    done
  done
}

fd_count() {
  local pid
  pid="$(docker inspect -f '{{.State.Pid}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo 0)"
  if [[ "$pid" != "0" && -r "/proc/$pid/fd" ]]; then
    ls "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' '
  else
    echo 0
  fi
}

thread_count() {
  docker top "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null | wc -l | tr -d ' '
}

bootstrap_mode() {
  local mode="$1"
  shift
  echo "=== bootstrap mode=$mode env=$* ==="
  clear_mode_env
  apply_mode_env "$@"
  mkdir -p "$RESULTS/modes/$mode"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  if [[ ! -f "$RESULTS/env/exyonq-image-built.flag" ]]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build exyonq 2>&1 | tee "$RESULTS/logs/build.log" | tail -40
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build upstream bench-runner 2>&1 | tee -a "$RESULTS/logs/build.log" | tail -10
    touch "$RESULTS/env/exyonq-image-built.flag"
  fi
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --build --force-recreate upstream exyonq bench-runner
  for i in $(seq 1 90); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-upstream-1" 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
    echo "health_poll=$i upstream=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
    sleep 2
  done
  docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/modes/$mode/boot.log" | tail -30
  health_wait "boot-$mode" || true
  request_ok "boot-$mode" || true
  FD_BEFORE="$(fd_count)"; THR_BEFORE="$(thread_count)"
  echo "FD_BEFORE=$FD_BEFORE THREADS_BEFORE=$THR_BEFORE" | tee "$RESULTS/modes/$mode/leak-before.txt"
}

run_functional() {
  local mode="$1"
  local health=FAIL req=FAIL ka=SKIP fatal=0
  health_wait "func-$mode" && health=PASS
  request_ok "func-$mode" && req=PASS
  if [[ "$mode" == "epoll_static" || "$mode" == "default_tokio" ]]; then
    ka=FAIL
    keepalive_ok "func-$mode" && ka=PASS
  fi
  docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" >"$RESULTS/modes/$mode/func-exyonq.log" 2>&1 || true
  if grep -Eiq 'worker stopped|worker fatal' "$RESULTS/modes/$mode/func-exyonq.log" 2>/dev/null; then
    fatal=1
  fi
  echo "FUNC mode=$mode HEALTH=$health REQUEST=$req KEEPALIVE=$ka FATAL=$fatal" \
    | tee "$RESULTS/functional/$mode.txt"
}

run_reps() {
  local mode="$1"
  local HEALTH_OK=0 FATAL=0
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
    if grep -Eiq 'worker stopped|worker fatal|sync accept worker stopped|epoll static worker stopped|epoll keep-alive worker stopped' "$out/exyonq.log" 2>/dev/null; then
      FATAL=$((FATAL + 1))
      echo "WORKER_FATAL_DETECTED mode=$mode rep=$rep"
    fi
  done
  FD_AFTER="$(fd_count)"; THR_AFTER="$(thread_count)"
  echo "FD_AFTER=$FD_AFTER THREADS_AFTER=$THR_AFTER" | tee "$RESULTS/modes/$mode/leak-after.txt"
  echo "MODE=$mode HEALTH_OK=$HEALTH_OK/$REPS FATAL=$FATAL FD_BEFORE=$(cut -d= -f2 "$RESULTS/modes/$mode/leak-before.txt" | head -1) FD_AFTER=$FD_AFTER" \
    | tee "$RESULTS/modes/$mode/summary.txt"
}

run_contract_probe() {
  # Prescribed PS2 contract scenarios — one directed rep each (not full R7C).
  local mode="epoll_listen"
  bootstrap_mode "$mode" "EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
  local scen
  for scen in p3 p8 p11 p13; do
    out="$RESULTS/contracts/$scen"
    mkdir -p "$out"
    echo "=== contract probe $scen ==="
    set +e
    COMPOSE_PROJECT_NAME="$COMPOSE_PROJECT" \
      BENCH_SCENARIOS_FILTER="$scen" BENCH_WARMUP_SEC=3 BENCH_DURATION=8s \
      BENCH_RESULTS_DIR="$out" \
      timeout 240s cargo run -p xtask -- bench perf --duration 8s --in-docker \
      2>&1 | tee "$out/bench-perf.log"
    rc=$?
    set -e
    health=FAIL
    health_wait "post-$scen" && health=PASS
    fatal=0
    docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" >"$out/exyonq.log" 2>&1 || true
    grep -Eiq 'worker stopped|worker fatal' "$out/exyonq.log" 2>/dev/null && fatal=1
    echo "CONTRACT=$scen RC=$rc HEALTH=$health FATAL=$fatal" | tee "$out/summary.txt"
  done
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

# Functional + directed P1 for each mode
bootstrap_mode default_tokio ":"
run_functional default_tokio
run_reps default_tokio

bootstrap_mode sync_accept "EXYONQ_SYNC_ACCEPT=1"
run_functional sync_accept
run_reps sync_accept

bootstrap_mode io_uring "EXYONQ_IO_URING=1"
run_functional io_uring
run_reps io_uring

bootstrap_mode epoll_listen "EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
run_functional epoll_listen
run_reps epoll_listen

bootstrap_mode epoll_static "EXYONQ_EPOLL_STATIC=1"
run_functional epoll_static
run_reps epoll_static

run_contract_probe

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

python3 "$ROOT/scripts/remote/ps3a-pm5-summarize-directed.py" "$RESULTS" \
  "$ROOT/docs/benchmarks/platform-split/ps2-results/PS2_CANONICAL_BASELINE_INDEX.json" \
  | tee "$RESULTS/directed-summary.txt"

echo "=== PS3A-PM5 directed complete ==="
echo "RESULTS=$RESULTS"
