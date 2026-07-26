#!/usr/bin/env bash
# AC3 functional gate for one allocator variant (ExyonQ focus).
# Expects: stack already up; host release binary at CARGO_TARGET_DIR/release/exyonq
# Usage: ALLOCATOR_VARIANT=system bash scripts/allocator/functional-gate-one.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
source "${HOME}/.cargo/env" 2>/dev/null || true

VARIANT="${ALLOCATOR_VARIANT:?set ALLOCATOR_VARIANT=system|jemalloc}"
case "$VARIANT" in
  system|jemalloc) ;;
  mimalloc)
    echo "ERROR: mimalloc is not a product allocator" >&2
    exit 2
    ;;
  *)
    echo "ERROR: ALLOCATOR_VARIANT must be system|jemalloc" >&2
    exit 2
    ;;
esac
COMPOSE="${BENCH_COMPOSE_FILE:-$ROOT/benchmarks/docker/docker-compose.bench.yml}"
COMPOSE_DIR="${BENCH_COMPOSE_DIR:-$ROOT/benchmarks/docker}"
RESULTS="${BENCH_RESULTS_DIR:-$ROOT/benchmarks/results/allocator-functional-$VARIANT}"
BIN="$ROOT/target/allocator-${VARIANT}/release/exyonq"
CTL="$(command -v exyonqctl || true)"
# Prefer matching release ctl from default target if present
if [[ -x "$ROOT/target/release/exyonqctl" ]]; then
  CTL="$ROOT/target/release/exyonqctl"
fi

mkdir -p "$RESULTS"
export BENCH_RESULTS_DIR="$RESULTS"
export BENCH_COMPOSE_FILE="$COMPOSE"
export BENCH_COMPOSE_DIR="$COMPOSE_DIR"
export BENCH_NETWORK=host
export ROOT

if [[ ! -x "$BIN" ]]; then
  echo "missing binary: $BIN" >&2
  exit 2
fi

echo "=== AC3 functional variant=$VARIANT bin=$BIN ==="
sha256sum "$BIN" | tee "$RESULTS/binary.sha256"

# Inject binary into running container without image recreate.
CONTAINER="$(docker compose -f "$COMPOSE" ps -q exyonq)"
if [[ -z "$CONTAINER" ]]; then
  echo "exyonq container not running" >&2
  exit 2
fi
docker cp "$BIN" "$CONTAINER:/usr/local/bin/exyonq"
docker compose -f "$COMPOSE" restart exyonq
sleep 5
curl -sf --max-time 5 http://127.0.0.1:8080/health >/dev/null

pass() { echo "PASS $1"; echo "{\"scenario\":\"$1\",\"variant\":\"$VARIANT\",\"status\":\"PASS\"}" >>"$RESULTS/functional.jsonl"; }
fail() { echo "FAIL $1 — $2" >&2; echo "{\"scenario\":\"$1\",\"variant\":\"$VARIANT\",\"status\":\"FAIL\",\"detail\":\"$2\"}" >>"$RESULTS/functional.jsonl"; FAILURES=$((FAILURES+1)); }
FAILURES=0
: >"$RESULTS/functional.jsonl"

# Config validation (host binary — same allocator feature)
if "$BIN" validate -c benchmarks/scenarios/fixtures/tls-minimal.toml >/dev/null 2>&1; then
  pass F9_validate
else
  fail F9_validate "tls-minimal validate"
fi
if "$BIN" validate -c benchmarks/scenarios/fixtures/http2-http3.toml >/dev/null 2>&1; then
  pass F10_validate
else
  fail F10_validate "http2-http3 validate"
fi

# Core functional scripts (multi-server); ExyonQ must PASS.
for s in f1_health f2_static_index f3_static_missing f4_path_traversal \
  f5_proxy f6_proxy_headers f7_upstream_down f8_unknown_route f11_exyonqctl_reload; do
  if bash "benchmarks/scenarios/functional/${s}.sh"; then
    pass "$s"
  else
    fail "$s" "script exit non-zero"
  fi
done

# F13 smoke without image recreate: switch config via env on existing container filesystem.
# Restart with http3 config using docker update is awkward; use compose run override:
# Copy http3 config is already in image at /bench/bench-http3.toml
docker compose -f "$COMPOSE" exec -T -e EXYONQ_CONFIG=/bench/bench-http3.toml exyonq \
  true >/dev/null 2>&1 || true
# Force process restart picking env from compose file temporarily:
(
  cd "$COMPOSE_DIR"
  EXYONQ_CONFIG=/bench/bench-http3.toml docker compose -f "$COMPOSE" up -d --no-deps --no-recreate exyonq >/dev/null
)
# --no-recreate may ignore env change. Fallback: docker restart after writing a wrapper.
# Reliable path: stop, commit is heavy. Use entrypoint env file if present.
sleep 3
# Re-inject binary in case up touched anything, then set config via kill+re-exec is complex.
# Minimal smoke: check default health still OK; then listener check after recreate from
# current container filesystem by restarting with docker exec kill.
docker cp "$BIN" "$(docker compose -f "$COMPOSE" ps -q exyonq):/usr/local/bin/exyonq"
# Recreate container FROM CURRENT IMAGE but we need http3 — build-arg images preferred for F13.
# For AC3: run curl health as startup; mark F13 as docker-log smoke if listener appears after
# switching config through compose recreate WITH volume-mounted binary.

# Mount-free approach for F13: run host binary briefly is out of harness.
# Classify F13: attempt image-based smoke only if ALLOCATOR_FEATURE image exists.
if docker image inspect "exyonq-alloc-${VARIANT}:local" >/dev/null 2>&1; then
  (
    cd "$COMPOSE_DIR"
    docker compose -f "$COMPOSE" stop exyonq >/dev/null
    # Temporarily retag
    docker tag "exyonq-alloc-${VARIANT}:local" docker-exyonq:latest
    EXYONQ_CONFIG=/bench/bench-http3.toml docker compose -f "$COMPOSE" up -d --no-deps --force-recreate exyonq >/dev/null
  )
  sleep 5
  if docker compose -f "$COMPOSE" logs --no-color exyonq 2>&1 | grep -q "HTTP/3 listening"; then
    pass F13_http3_smoke
  else
    fail F13_http3_smoke "HTTP/3 listening not found"
  fi
  # restore default config
  (
    cd "$COMPOSE_DIR"
    EXYONQ_CONFIG=/bench/bench.toml docker compose -f "$COMPOSE" up -d --no-deps --force-recreate exyonq >/dev/null
    docker cp "$BIN" "$(docker compose -f "$COMPOSE" ps -q exyonq):/usr/local/bin/exyonq"
    docker compose -f "$COMPOSE" restart exyonq >/dev/null
  )
else
  echo "{\"scenario\":\"F13_http3_smoke\",\"variant\":\"$VARIANT\",\"status\":\"NOT_APPLICABLE\",\"detail\":\"exyonq-alloc-${VARIANT}:local image missing; host docker-cp path used for F1-F11\"}" >>"$RESULTS/functional.jsonl"
  echo "NOT_APPLICABLE F13 (no tagged allocator image yet)"
fi

echo "FAILURES=$FAILURES RESULTS=$RESULTS"
exit "$FAILURES"
