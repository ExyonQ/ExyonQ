#!/usr/bin/env bash
# V044_P3_CAP067_LARGE_BODY_CORK_OFF_AB — Netcup live-3way (env-gated).
# PRODUCT_MUTATION=YES (env kill-switch only; default remains cork ON).
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Hypothesis (productivity 20260917T191201Z): NGINX does 1×1MiB sendfile without
# tcp_nopush; ExyonQ KEEP does 8×128KiB under TCP_CORK. Uncapped+cork already lost
# to 128KiB+cork. Test large-body cork OFF (MSG_MORE per chunk) at fixed clamp.
#
# Arms (same image):
#   A KEEP:  EXYONQ_LARGE_BODY_TCP_CORK unset/1 + CHUNK=131072
#   B:       EXYONQ_LARGE_BODY_TCP_CORK=0 + CHUNK=131072
#   C diag:  EXYONQ_LARGE_BODY_TCP_CORK=0 + CHUNK=1048576 (not default KEEP)
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

BUILD="${CORK_REMOTE_BUILD:-/root/exyonq-forensic-p1p3}"
SEAL="${CORK_REMOTE_SEAL:-/root/exyonq-v044-p1-seal-06742e1b}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
TS="${CORK_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
ROOT_EV="$SEAL/.exyonq-local-evidence/v044-p1p3-cork-off-ab-$TS"
IMAGE="${CORK_IMAGE:-v044p1auth-exyonq-corkab}"
FULL="$SEAL/benchmarks/docker/docker-compose.bench.yml"
OVER="$SEAL/benchmarks/docker/docker-compose.p1-authoritative.yml"
CTR="${PROJECT}-exyonq-1"

mkdir -p "$ROOT_EV"
log(){ echo "[cork-ab $(date -u +%H:%M:%S)] $*" | tee -a "$ROOT_EV/orchestrator.log"; }

compose() {
  docker compose -f "$FULL" -f "$OVER" -p "$PROJECT" "$@"
}

log "docker build $IMAGE from $BUILD"
cd "$BUILD"
docker build -f benchmarks/docker/Dockerfile.exyonq -t "$IMAGE" . 2>&1 | tee "$ROOT_EV/docker-build.log"
SHA=$(docker run --rm --entrypoint sha256sum "$IMAGE" /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$ROOT_EV/exyonq_sha.txt"
echo "IMAGE=$IMAGE" | tee -a "$ROOT_EV/exyonq_sha.txt"

recreate_exyonq() {
  local cork=$1
  local chunk=$2
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS=8
  export EXYONQ_WORKER_THREADS=8
  export EXYONQ_EPOLL_POOL_THREADS=8
  export EXYONQ_EPOLL_LISTEN=1
  export EXYONQ_SENDFILE_CHUNK="$chunk"
  if [[ "$cork" == "0" ]]; then
    export EXYONQ_LARGE_BODY_TCP_CORK=0
  else
    unset EXYONQ_LARGE_BODY_TCP_CORK || true
    export EXYONQ_LARGE_BODY_TCP_CORK=1
  fi
  cd "$SEAL/benchmarks/docker"
  compose up -d --no-deps --force-recreate exyonq
  for i in $(seq 1 90); do
    if docker exec "$CTR" curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then
      log "exyonq healthy cork=$cork chunk=$chunk (try $i)"
      break
    fi
    sleep 2
  done
  docker exec "$CTR" sha256sum /usr/local/bin/exyonq | tee -a "$ROOT_EV/exyonq_sha.txt"
  docker exec "$CTR" env | grep -E 'EXYONQ_(ACCEPT|WORKER|EPOLL|SENDFILE|EDGE|LARGE_BODY)' | sort \
    | tee "$ROOT_EV/env-cork${cork}-chunk${chunk}.txt"
  local live
  live=$(docker exec "$CTR" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  [[ "$live" == "$SHA" ]] || { log "FAIL sha mismatch live=$live expected=$SHA"; exit 2; }
}

run_arm() {
  local arm=$1
  local cork=$2
  local chunk=$3
  local suite="$ROOT_EV/$arm"
  mkdir -p "$suite"
  {
    echo "ARM=$arm"
    echo "EXYONQ_LARGE_BODY_TCP_CORK=$cork"
    echo "EXYONQ_SENDFILE_CHUNK=$chunk"
    echo "GEOMETRY=8/8/8"
    echo "HYPOTHESIS=large-body cork OFF vs KEEP (MSG_MORE per 128KiB chunk)"
    echo "EXPECTED_EXYONQ_SHA256=$SHA"
    echo "IMAGE=$IMAGE"
    echo "PROTECTORS=P1/P2 vs arm A: RPS drop<=3% p99 worsen<=10%"
    echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
  } | tee "$suite/suite_meta.txt"

  log "ARM $arm recreate cork=$cork chunk=$chunk"
  recreate_exyonq "$cork" "$chunk"

  export COMPOSE_PROJECT_NAME="$PROJECT"
  export EXPECTED_EXYONQ_SHA256="$SHA"
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS=8
  export EXYONQ_WORKER_THREADS=8
  export EXYONQ_EPOLL_POOL_THREADS=8
  export EXYONQ_SENDFILE_CHUNK="$chunk"
  if [[ "$cork" == "0" ]]; then
    export EXYONQ_LARGE_BODY_TCP_CORK=0
  else
    export EXYONQ_LARGE_BODY_TCP_CORK=1
  fi
  export V044_P1_WS="$SEAL" V044_3WAY_TS="${TS}-${arm}" V044_3WAY_EV="$suite/p1"
  export V044_P2_WS="$SEAL" V044_P2_3WAY_TS="${TS}-${arm}" V044_P2_3WAY_EV="$suite/p2"
  export V044_P3_WS="$SEAL" V044_P3_3WAY_TS="${TS}-${arm}" V044_P3_3WAY_EV="$suite/p3"
  docker update --cpuset-cpus 0-7 \
    "${PROJECT}-exyonq-1" "${PROJECT}-nginx-stable-1" "${PROJECT}-openlitespeed-latest-1" >/dev/null

  run_one() {
    local n=$1; shift
    log "START $arm/$n"
    set +e
    "$@" >"$suite/${n}.stdout" 2>"$suite/${n}.stderr"
    ec=$?
    set -e
    echo "EXIT_${arm}_${n}=$ec" | tee -a "$ROOT_EV/orchestrator.log"
    [[ $ec -eq 0 ]] || return $ec
    log "OK $arm/$n"
  }

  run_one p1 bash "$SEAL/scripts/remote/v044-p1-cpu-live-3way.sh"
  run_one p2 bash "$SEAL/scripts/remote/v044-p2-static-64k-live-3way.sh"
  run_one p3 bash "$SEAL/scripts/remote/v044-p3-static-1mib-live-3way.sh"
  echo "ARM_${arm}_COMPLETE" | tee -a "$ROOT_EV/orchestrator.log"
}

{
  echo "SUITE_TS=$TS"
  echo "WIP=V044_P3_CAP067_LARGE_BODY_CORK_OFF_AB"
  echo "ARMS=A_cork_on_128k B_cork_off_128k C_cork_off_uncapped"
  echo "IMAGE=$IMAGE"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
} | tee "$ROOT_EV/suite_meta.txt"

run_arm A 1 131072
run_arm B 0 131072
run_arm C 0 1048576

log "SUITE_COMPLETE"
echo "SUITE_DIR=$ROOT_EV" >"$ROOT_EV/DONE"
echo DONE | tee -a "$ROOT_EV/orchestrator.log"
