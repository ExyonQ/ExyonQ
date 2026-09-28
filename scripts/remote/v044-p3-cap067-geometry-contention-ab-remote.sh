#!/usr/bin/env bash
# V044_P3_CAP067_GEOMETRY_CONTENTION_AB — Netcup live-3way (env-only).
# PRODUCT_MUTATION=NO (geometry env A/B). PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Hypothesis (causal 20260917T143741Z): P3 residual is SYSTEM_DOMINATED with
# spinlock/memcg pressure under parallel large-body send. Reduce Cap067
# parallelism 8→4 and measure P3 RPS + core-ms; P1/P2 protectors.
#
# Arms (same image v044p1auth-exyonq-chunkab + SENDFILE_CHUNK=131072 KEEP):
#   A geom8: ACCEPT/WORKER/EPOLL_POOL=8
#   B geom4: ACCEPT/WORKER/EPOLL_POOL=4
set -euo pipefail

SEAL="${GEOM_REMOTE_SEAL:-/root/exyonq-v044-p1-seal-06742e1b}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
TS="${GEOM_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
ROOT_EV="$SEAL/.exyonq-local-evidence/v044-p1p3-geometry-contention-ab-$TS"
IMAGE="${GEOM_IMAGE:-v044p1auth-exyonq-chunkab}"
FULL="$SEAL/benchmarks/docker/docker-compose.bench.yml"
OVER="$SEAL/benchmarks/docker/docker-compose.p1-authoritative.yml"
CTR="${PROJECT}-exyonq-1"
CHUNK="${EXYONQ_SENDFILE_CHUNK:-131072}"

mkdir -p "$ROOT_EV"
log(){ echo "[geom-ab $(date -u +%H:%M:%S)] $*" | tee -a "$ROOT_EV/orchestrator.log"; }

compose() {
  docker compose -f "$FULL" -f "$OVER" -p "$PROJECT" "$@"
}

SHA=$(docker run --rm --entrypoint sha256sum "$IMAGE" /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$ROOT_EV/exyonq_sha.txt"
echo "IMAGE=$IMAGE" | tee -a "$ROOT_EV/exyonq_sha.txt"

recreate_exyonq() {
  local geom=$1
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS="$geom"
  export EXYONQ_WORKER_THREADS="$geom"
  export EXYONQ_EPOLL_POOL_THREADS="$geom"
  export EXYONQ_EPOLL_LISTEN=1
  export EXYONQ_SENDFILE_CHUNK="$CHUNK"
  cd "$SEAL/benchmarks/docker"
  compose up -d --no-deps --force-recreate exyonq
  for i in $(seq 1 90); do
    if docker exec "$CTR" curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then
      log "exyonq healthy geom=$geom (try $i)"
      break
    fi
    sleep 2
  done
  docker exec "$CTR" sha256sum /usr/local/bin/exyonq | tee -a "$ROOT_EV/exyonq_sha.txt"
  docker exec "$CTR" env | grep -E 'EXYONQ_(ACCEPT|WORKER|EPOLL|SENDFILE|EDGE)' | sort \
    | tee "$ROOT_EV/env-geom${geom}.txt"
  local live
  live=$(docker exec "$CTR" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  [[ "$live" == "$SHA" ]] || { log "FAIL sha mismatch live=$live expected=$SHA"; exit 2; }
}

run_arm() {
  local arm=$1
  local geom=$2
  local suite="$ROOT_EV/$arm"
  mkdir -p "$suite"
  {
    echo "ARM=$arm"
    echo "GEOMETRY=${geom}/${geom}/${geom}"
    echo "EXYONQ_SENDFILE_CHUNK=$CHUNK"
    echo "HYPOTHESIS=geometry contention under parallel large-body send (spinlock/memcg)"
    echo "EXPECTED_EXYONQ_SHA256=$SHA"
    echo "IMAGE=$IMAGE"
    echo "PROTECTORS=P1/P2 vs arm A: RPS drop<=3% p99 worsen<=10%"
    echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
  } | tee "$suite/suite_meta.txt"

  log "ARM $arm recreate geom=$geom"
  recreate_exyonq "$geom"

  export COMPOSE_PROJECT_NAME="$PROJECT"
  export EXPECTED_EXYONQ_SHA256="$SHA"
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS="$geom"
  export EXYONQ_WORKER_THREADS="$geom"
  export EXYONQ_EPOLL_POOL_THREADS="$geom"
  export EXYONQ_SENDFILE_CHUNK="$CHUNK"
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
  echo "WIP=V044_P3_CAP067_GEOMETRY_CONTENTION_AB"
  echo "ARMS=A_geom8 B_geom4"
  echo "KEEP_CLAMP=EXYONQ_SENDFILE_CHUNK=$CHUNK"
  echo "IMAGE=$IMAGE"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
} | tee "$ROOT_EV/suite_meta.txt"

run_arm A 8
run_arm B 4

log "SUITE_COMPLETE"
echo "SUITE_DIR=$ROOT_EV" >"$ROOT_EV/DONE"
echo DONE | tee -a "$ROOT_EV/orchestrator.log"
