#!/usr/bin/env bash
# PS3A-PM4: short functional smoke — one rep per mode (not a directed campaign).
# Modes: default_tokio, sync_accept, io_uring, epoll_listen, epoll_static
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

RUN_ID="${PS3A_PM4_SMOKE_RUN_ID:-pm4-smoke-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps3a-pm4/$RUN_ID"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS3A_PM4_COMPOSE_PROJECT:-exyonq-ps3a-pm4-smoke}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RESULTS"/{env,logs,modes}

exec > >(tee -a "$RESULTS/logs/pm4-smoke-${STAMP}.log") 2>&1
echo "=== PS3A-PM4 smoke $STAMP run=$RUN_ID ==="
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

request_ok() {
  local label="$1" code
  code="$(curl --connect-timeout 3 --max-time 5 -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/site/1k.bin 2>/dev/null || echo 000)"
  if [[ "$code" == "200" ]]; then
    echo "REQUEST_OK $label code=$code"
    return 0
  fi
  echo "REQUEST_FAIL $label code=$code"
  return 1
}

clear_mode_env() {
  unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT EXYONQ_IO_URING
  export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_SYNC_ACCEPT= EXYONQ_IO_URING=
}

apply_mode_env() {
  # Args are KEY=VALUE tokens (space-separated list also accepted in one arg).
  local chunk key val
  for chunk in "$@"; do
    [[ -z "$chunk" || "$chunk" == ":" || "$chunk" == "true" ]] && continue
    # shellcheck disable=SC2086
    for pair in $chunk; do
      [[ "$pair" != *=* ]] && continue
      key="${pair%%=*}"
      val="${pair#*=}"
      export "${key}=${val}"
    done
  done
}

bootstrap_mode() {
  local mode="$1"
  shift
  echo "=== mode=$mode env=$* ==="
  clear_mode_env
  apply_mode_env "$@"
  mkdir -p "$RESULTS/modes/$mode"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  # Cached layers OK for PM4 functional smoke (not an official compare).
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --build --force-recreate mock-upstream exyonq
  for i in $(seq 1 90); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-mock-upstream-1" 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
    echo "health_poll=$i mock=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
    sleep 2
  done
  docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/modes/$mode/boot.log" | tail -25
}

run_mode() {
  local mode="$1"
  shift
  bootstrap_mode "$mode" "$@"
  local health=FAIL req=FAIL fatal=0
  if health_wait "$mode"; then health=PASS; fi
  if request_ok "$mode"; then req=PASS; fi
  if grep -Eiq 'worker stopped|worker fatal' "$RESULTS/modes/$mode/boot.log" 2>/dev/null; then
    fatal=1
  fi
  health_wait "post-$mode" || true
  docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" >"$RESULTS/modes/$mode/exyonq.log" 2>&1 || true
  if grep -Eiq 'worker stopped|worker fatal' "$RESULTS/modes/$mode/exyonq.log" 2>/dev/null; then
    fatal=1
  fi
  echo "MODE=$mode HEALTH=$health REQUEST=$req FATAL=$fatal" | tee "$RESULTS/modes/$mode/summary.txt"
}

{
  echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "host=$(hostname)"
} | tee "$RESULTS/env/environment.txt"

run_mode default_tokio ":"
run_mode sync_accept "EXYONQ_SYNC_ACCEPT=1"
run_mode io_uring "EXYONQ_IO_URING=1"
run_mode epoll_listen "EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
run_mode epoll_static "EXYONQ_EPOLL_STATIC=1"

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

echo "=== summaries ==="
cat "$RESULTS"/modes/*/summary.txt
FAILS=0
while IFS= read -r line; do
  if ! printf '%s\n' "$line" | grep -q 'HEALTH=PASS' || ! printf '%s\n' "$line" | grep -q 'REQUEST=PASS' || ! printf '%s\n' "$line" | grep -q 'FATAL=0'; then
    FAILS=$((FAILS + 1))
  fi
done < <(cat "$RESULTS"/modes/*/summary.txt)
echo "SMOKE_FAILS=$FAILS"
echo "RESULTS=$RESULTS"
[[ "$FAILS" -eq 0 ]]
