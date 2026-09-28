#!/usr/bin/env bash
# AC3: build Linux variants, docker images, functional gate per allocator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
source "${HOME}/.cargo/env" 2>/dev/null || true
chmod +x scripts/allocator/build-variants.sh

COMPOSE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
REPORT_DIR="$ROOT/docs/benchmarks/allocators"
IDENTITY_LINUX="$REPORT_DIR/identities/linux"
mkdir -p "$IDENTITY_LINUX" "$REPORT_DIR"

echo "=== AC2 Linux host builds ==="
IDENTITY_DIR="$IDENTITY_LINUX" bash scripts/allocator/build-variants.sh all

echo "=== Ensure bench stack (rivals + upstream) ==="
cd "$ROOT/benchmarks/docker"
docker compose -f docker-compose.bench.yml --profile bench up -d upstream \
  nginx-stable nginx-mainline haproxy envoy traefik caddy apache \
  openlitespeed-stable openlitespeed-latest bench-runner >/dev/null || true

build_image() {
  local variant="$1"
  local feature="$2"
  local tag="exyonq-alloc-${variant}:local"
  echo "=== Docker build $tag feature='$feature' ==="
  docker build \
    -f "$ROOT/benchmarks/docker/Dockerfile.exyonq" \
    --build-arg "ALLOCATOR_FEATURE=${feature}" \
    -t "$tag" \
    "$ROOT"
  local iid
  iid="$(docker image inspect "$tag" --format '{{.Id}}')"
  # merge container id into identity file
  local idf="$IDENTITY_LINUX/identity-${variant}.txt"
  if [[ -f "$idf" ]]; then
    grep -v '^CONTAINER_IMAGE_ID=' "$idf" >"${idf}.tmp" || true
    echo "CONTAINER_IMAGE_ID=${iid}" >>"${idf}.tmp"
    mv "${idf}.tmp" "$idf"
  fi
  echo "$tag -> $iid"
}

# Product variants only (mimalloc removed from product support; AC1–AC5 evidence kept).
build_image system ""
build_image jemalloc "allocator-jemalloc"

run_functional() {
  local variant="$1"
  local tag="exyonq-alloc-${variant}:local"
  local results="$ROOT/benchmarks/results/allocator-functional-${variant}"
  mkdir -p "$results"
  rm -f "$results/functional.jsonl"
  export BENCH_RESULTS_DIR="$results"
  export BENCH_COMPOSE_FILE="$COMPOSE"
  export BENCH_COMPOSE_DIR="$ROOT/benchmarks/docker"
  export BENCH_NETWORK=host
  export ROOT

  echo "=== Functional $variant ==="
  cd "$ROOT/benchmarks/docker"
  docker tag "$tag" docker-exyonq:latest
  # Recreate exyonq from tagged image
  docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq
  sleep 8
  curl -sf http://127.0.0.1:8080/health >/dev/null

  # Host validate with matching host binary
  local bin="$ROOT/target/allocator-${variant}/release/exyonq"
  "$bin" validate -c "$ROOT/benchmarks/scenarios/fixtures/tls-minimal.toml"
  "$bin" validate -c "$ROOT/benchmarks/scenarios/fixtures/http2-http3.toml"

  cd "$ROOT"
  local fail=0
  for s in f1_health f2_static_index f3_static_missing f4_path_traversal \
    f5_proxy f6_proxy_headers f7_upstream_down f8_unknown_route \
    f11_exyonqctl_reload f13_http3_probe; do
    if bash "benchmarks/scenarios/functional/${s}.sh"; then
      echo "PASS $variant $s"
    else
      echo "FAIL $variant $s"
      fail=1
    fi
  done
  # Restore default config image path after F13
  cd "$ROOT/benchmarks/docker"
  EXYONQ_CONFIG=/bench/bench.toml docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq >/dev/null || true
  echo "$fail" >"$results/exit.code"
  return "$fail"
}

overall=0
for v in system jemalloc; do
  if ! run_functional "$v"; then
    overall=1
  fi
done

echo "AC3_OVERALL_EXIT=$overall"
exit "$overall"
