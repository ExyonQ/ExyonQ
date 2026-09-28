#!/usr/bin/env bash
# Runs ON Netcup. Docker-build clamp binary (bookworm glibc), run three P1/P2/P3 arms.
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi
export PATH="${HOME}/.cargo/bin:${PATH}"

BUILD="${CLAMP_REMOTE_BUILD:-/root/exyonq-forensic-p1p3}"
SEAL="${CLAMP_REMOTE_SEAL:-/root/exyonq-v044-p1-seal-06742e1b}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
TS="${CLAMP_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
ROOT_EV="$SEAL/.exyonq-local-evidence/v044-p1p3-sendfile-chunk-ab-$TS"
IMAGE="${CLAMP_IMAGE:-v044p1auth-exyonq-chunkab}"
FULL="$SEAL/benchmarks/docker/docker-compose.bench.yml"
OVER="$SEAL/benchmarks/docker/docker-compose.p1-authoritative.yml"
CTR="${PROJECT}-exyonq-1"

mkdir -p "$ROOT_EV"
log(){ echo "[clamp-remote $(date -u +%H:%M:%S)] $*" | tee -a "$ROOT_EV/orchestrator.log"; }

compose() {
  docker compose -f "$FULL" -f "$OVER" -p "$PROJECT" "$@"
}

log "Ensure sendfile clamp sources in $BUILD (build context)"
grep -q LARGE_BODY_SENDFILE_COUNT_MAX "$BUILD/crates/exyonq-mod-static/src/sendfile.rs"
# Keep seal scripts/compose in sync for meter scripts
grep -q EXYONQ_SENDFILE_CHUNK "$SEAL/benchmarks/docker/docker-compose.bench.yml"

log "DOCKER BUILD $IMAGE from $BUILD (bookworm glibc)"
cd "$BUILD"
docker build -f benchmarks/docker/Dockerfile.exyonq -t "$IMAGE" . 2>&1 | tee "$ROOT_EV/docker-build.log"
SHA=$(docker run --rm --entrypoint sha256sum "$IMAGE" /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$ROOT_EV/exyonq_sha.txt"
echo "IMAGE=$IMAGE" | tee -a "$ROOT_EV/exyonq_sha.txt"

recreate_exyonq() {
  local chunk="${1:-}"
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS=8
  export EXYONQ_WORKER_THREADS=8
  export EXYONQ_EPOLL_POOL_THREADS=8
  export EXYONQ_EPOLL_LISTEN=1
  if [[ -n "$chunk" ]]; then
    export EXYONQ_SENDFILE_CHUNK="$chunk"
  else
    unset EXYONQ_SENDFILE_CHUNK || true
  fi
  cd "$SEAL/benchmarks/docker"
  compose up -d --no-deps --force-recreate exyonq
  for i in $(seq 1 90); do
    if docker exec "$CTR" curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then
      log "exyonq healthy (try $i)"
      break
    fi
    sleep 2
  done
  docker exec "$CTR" sha256sum /usr/local/bin/exyonq | tee -a "$ROOT_EV/exyonq_sha.txt"
  docker exec "$CTR" env | grep -E 'EXYONQ_(ACCEPT|WORKER|EPOLL|SENDFILE|EDGE)' | sort \
    | tee "$ROOT_EV/env-${chunk:-default256}.txt"
  local live
  live=$(docker exec "$CTR" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  [[ "$live" == "$SHA" ]] || { log "FAIL sha mismatch live=$live expected=$SHA"; exit 2; }
}

run_arm() {
  local arm=$1
  local chunk=${2:-}
  local suite="$ROOT_EV/$arm"
  mkdir -p "$suite"
  {
    echo "ARM=$arm"
    echo "EXYONQ_SENDFILE_CHUNK=${chunk:-<unset product 256KiB>}"
    echo "GEOMETRY=8/8/8"
    echo "HYPOTHESIS=large-body sendfile count clamp skb_split semantic"
    echo "EXPECTED_EXYONQ_SHA256=$SHA"
    echo "IMAGE=$IMAGE"
    echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
  } | tee "$suite/suite_meta.txt"

  log "ARM $arm recreate chunk='${chunk:-unset}'"
  recreate_exyonq "$chunk"

  export COMPOSE_PROJECT_NAME="$PROJECT"
  export EXPECTED_EXYONQ_SHA256="$SHA"
  export P1_EXYONQ_IMAGE="$IMAGE"
  export EXYONQ_ACCEPT_WORKERS=8 EXYONQ_WORKER_THREADS=8 EXYONQ_EPOLL_POOL_THREADS=8
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
  echo "WIP=V044_P3_CAP067_LARGE_BODY_SENDFILE_COUNT_CLAMP"
  echo "ARMS=A_uncapped1048576 B_product256KiB C_128KiB"
  echo "GEOMETRY=8/8/8"
  echo "BUILD=docker bookworm ($IMAGE)"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
} | tee "$ROOT_EV/suite_meta.txt"

run_arm A 1048576
run_arm B ""
run_arm C 131072

log "SUITE_COMPLETE"
echo "SUITE_DIR=$ROOT_EV" >"$ROOT_EV/DONE"
echo DONE | tee -a "$ROOT_EV/orchestrator.log"
