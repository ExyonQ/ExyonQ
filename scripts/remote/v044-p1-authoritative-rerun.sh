#!/usr/bin/env bash
# V044 authoritative P1 Static 1 KiB — symmetric Docker internal, 3-way matrix.
# Harness-only: no product semantic changes. Requires Linux + Docker + rust 1.98.1.
set -euo pipefail

WS="${V044_P1_WS:-$(cd "$(dirname "$0")/../.." && pwd)}"
EV="${V044_P1_EVIDENCE:-$WS/.exyonq-local-evidence/v044-p1-authoritative}"
COMPOSE="$WS/benchmarks/docker/docker-compose.p1-minimal.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth}"
AUTH_HEAD="${V044_AUTH_HEAD:-353bf088309bf563dbf37e93b5e3fa85888b03f5}"
RUST_CHANNEL="${V044_RUST_CHANNEL:-1.98.1}"
REPS="${V044_P1_REPS:-5}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
DURATION="${BENCH_DURATION:-30s}"

mkdir -p "$EV/runs"
cd "$WS"

log() { echo "[v044-p1] $*" | tee -a "$EV/orchestrator.log"; }

require_rust() {
  source "${HOME}/.cargo/env" 2>/dev/null || true
  if ! rustup toolchain list | grep -q "$RUST_CHANNEL"; then
    log "installing rust $RUST_CHANNEL"
    rustup toolchain install "$RUST_CHANNEL" -c rustfmt -c clippy
  fi
  rustup default "$RUST_CHANNEL"
  {
    echo "RUSTC_VERSION=$(rustc --version)"
    echo "CARGO_VERSION=$(cargo --version)"
  } | tee "$EV/toolchain.txt"
}

isolate_host() {
  log "host isolation — stopping benchmark leftovers only"
  : > "$EV/isolation.txt"
  local pid
  for pid in $(pgrep -f '/exyonq-v044-p1/target/release/exyonq serve' 2>/dev/null || true); do
    echo "stop native indicative exyonq pid=$pid" | tee -a "$EV/isolation.txt"
    kill "$pid" 2>/dev/null || true
  done
  docker rm -f v044p1-nginx 2>/dev/null | tee -a "$EV/isolation.txt" || true
  docker compose -f "$COMPOSE" -p v044p1 down --remove-orphans 2>/dev/null | tee -a "$EV/isolation.txt" || true
  docker compose -f "$COMPOSE" -p "$PROJECT" down --remove-orphans 2>/dev/null | tee -a "$EV/isolation.txt" || true
  sleep 2
  pgrep -af 'exyonq serve' | tee -a "$EV/processes_after_isolation.txt" || true
}

build_exyonq_identity_image() {
  log "docker build exyonq base + identity overlay (uid 10001)"
  export DOCKER_BUILDKIT=1
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq" -t "${PROJECT}-exyonq-base" "$WS" \
    >> "$EV/build.log" 2>&1
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq-identity-overlay" \
    --build-arg "BASE_IMAGE=${PROJECT}-exyonq-base" \
    -t "${PROJECT}-exyonq" "$WS" >> "$EV/build.log" 2>&1
  docker run --rm --entrypoint sha256sum "${PROJECT}-exyonq" /usr/local/bin/exyonq | tee "$EV/binary.txt"
}

start_stack() {
  log "starting symmetric docker stack"
  export COMPOSE_PROJECT_NAME="$PROJECT"
  export P1_EXYONQ_IMAGE="${PROJECT}-exyonq"
  docker compose -f "$COMPOSE" build upstream bench-runner >> "$EV/build.log" 2>&1
  docker compose -f "$COMPOSE" up -d \
    upstream exyonq nginx-stable openlitespeed-latest bench-runner >> "$EV/build.log" 2>&1
  sleep 20
  docker compose -f "$COMPOSE" exec -T bench-runner curl -sf http://exyonq:8080/health
  docker compose -f "$COMPOSE" exec -T bench-runner curl -sf http://nginx-stable:8080/health
  docker compose -f "$COMPOSE" exec -T bench-runner curl -sf http://openlitespeed-latest:8088/health
}

capture_identity() {
  {
    echo "=== exyonq ==="
    docker compose -f "$COMPOSE" exec -T exyonq id
    docker compose -f "$COMPOSE" exec -T exyonq sh -c 'cat /proc/sys/fs/pipe-max-size; ulimit -n'
    echo "=== nginx-stable worker ==="
    docker compose -f "$COMPOSE" exec -T nginx-stable sh -c 'id nginx; ps aux | head -8'
    docker compose -f "$COMPOSE" exec -T nginx-stable sh -c 'cat /proc/sys/fs/pipe-max-size; ulimit -n'
    echo "=== openlitespeed-latest worker ==="
    docker compose -f "$COMPOSE" exec -T openlitespeed-latest sh -c 'id nobody; ps aux | head -8' || true
  } | tee "$EV/container_identity.txt"
}

run_p1_matrix() {
  local dir="$WS/benchmarks/scenarios/perf"
  DIR="$dir"
  export BENCH_NETWORK=internal
  export COMPOSE_FILE="$COMPOSE"
  export BENCH_COMPOSE_FILE="$COMPOSE"
  export COMPOSE_PROJECT_NAME="$PROJECT"
  export DURATION="$DURATION"
  export BENCH_WARMUP_SEC="$WARMUP"
  export BENCH_REWRK_THREADS=2
  source "$dir/run-rewrk-helper.sh"

  run_server_rep() {
    local server="$1" rep="$2" url="$3" tag="$4"
    local raw="$EV/runs/p1-${tag}-rep${rep}.rewrk.json"
    local out="$EV/runs/p1-${tag}-rep${rep}.json"
    rewrk_warmup 100 "$url" "" 0
    rewrk_exec_capture "$raw" 100 "$url" "" 0
    python3 "$dir/rewrk-report-to-json.py" "$raw" -o "$out"
    python3 -c "import json; d=json.load(open('$out')); print(json.dumps({'rps':d['summary']['requestsPerSec'],'p50':d['latencyPercentiles']['p50'],'p99':d['latencyPercentiles']['p99'],'total':d['summary'].get('total',0)}))"
  }

  : > "$EV/exyonq_rps_runs.txt"
  : > "$EV/nginx_rps_runs.txt"
  : > "$EV/ols_rps_runs.txt"
  local rep j rps
  for rep in $(seq 1 "$REPS"); do
    j=$(run_server_rep exyonq "$rep" "http://exyonq:8080/site/1k.bin" exyonq)
    rps=$(python3 -c "import json; print(json.loads('$j')['rps'])")
    echo "$rps" >> "$EV/exyonq_rps_runs.txt"
    echo "exyonq rep$rep $j" | tee -a "$EV/run_log.txt"
    sleep 5
  done
  for rep in $(seq 1 "$REPS"); do
    j=$(run_server_rep nginx "$rep" "http://nginx-stable:8080/site/1k.bin" nginx)
    rps=$(python3 -c "import json; print(json.loads('$j')['rps'])")
    echo "$rps" >> "$EV/nginx_rps_runs.txt"
    echo "nginx rep$rep $j" | tee -a "$EV/run_log.txt"
    sleep 5
  done
  for rep in $(seq 1 "$REPS"); do
    j=$(run_server_rep ols "$rep" "http://openlitespeed-latest:8088/site/1k.bin" ols)
    rps=$(python3 -c "import json; print(json.loads('$j')['rps'])")
    echo "$rps" >> "$EV/ols_rps_runs.txt"
    echo "ols rep$rep $j" | tee -a "$EV/run_log.txt"
    sleep 5
  done
}

perf_counters() {
  ulimit -n 65535 || true
  for pair in exyonq:8080 nginx:8081 ols:8087; do
    tag="${pair%%:*}"
    port="${pair##*:}"
    perf stat -e cycles,instructions,branches,branch-misses,task-clock,context-switches,cpu-migrations,page-faults \
      rewrk -c 100 -d 30s -h "http://127.0.0.1:${port}/site/1k.bin" --json -t 2 \
      > "$EV/perf_stat_${tag}.txt" 2>&1 || true
  done
}

main() {
  log "V044 P1 authoritative rerun evidence=$EV"
  echo "AUTHORITY_HEAD=$AUTH_HEAD" | tee "$EV/authority.txt"
  require_rust
  isolate_host
  df -h / | tee "$EV/disk_before.txt"
  build_exyonq_identity_image
  start_stack
  capture_identity
  ulimit -n 65535 || true
  run_p1_matrix
  perf_counters
  log "complete"
}

main "$@"
