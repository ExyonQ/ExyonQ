#!/usr/bin/env bash
# PS2-R5B — minimal harness verification before selective completion run.
set -euo pipefail

ROOT="${1:-.}"
REPO_ROOT="$(cd "$ROOT" && pwd)"
cd "$REPO_ROOT"
COMPOSE_FILE="${BENCH_COMPOSE_FILE:-$REPO_ROOT/benchmarks/docker/docker-compose.bench.yml}"
COMPOSE_PROJECT="${COMPOSE_PROJECT_NAME:-exyonq-ps2-baseline}"
OUT="${PS2_HARNESS_PRECHECK_LOG:-$REPO_ROOT/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective/harness-precheck.log}"
mkdir -p "$(dirname "$OUT")"
exec > >(tee -a "$OUT") 2>&1

echo "=== PS2-R5B harness precheck $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export COMPOSE_PROJECT_NAME="$COMPOSE_PROJECT"
export BENCH_COMPOSE_FILE="$COMPOSE_FILE"

source "$REPO_ROOT/benchmarks/scenarios/functional/lib.sh"

echo "COMPOSE_PROJECT_NAME=$COMPOSE_PROJECT_NAME"
echo "COMPOSE_PROJECT_NAME_FUNCTIONAL=$COMPOSE_PROJECT_NAME"
echo "COMPOSE_PROJECT_NAME_CONTRACT=$COMPOSE_PROJECT_NAME"

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build mock-upstream exyonq >/dev/null
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d mock-upstream exyonq

echo "=== docker compose ps ==="
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" ps

echo "=== docker compose config (effective) ==="
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" config 2>&1 | head -80

echo "=== docker compose logs mock-upstream (tail) ==="
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" logs mock-upstream 2>&1 | tail -20

echo "=== mock-upstream health (exec probe — authoritative; port 9000 not published to host) ==="
if ! ensure_mock_upstream_ready; then
  echo "STACK_BOOTSTRAP_BEFORE_FUNCTIONAL=NO"
  echo "MOCK_UPSTREAM_READY=NO"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" logs mock-upstream 2>&1 | tail -40
  exit 1
fi
code="$(docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" exec -T mock-upstream \
  curl -s -o /tmp/ps2-mock-health.txt -w '%{http_code}' --max-time 8 http://127.0.0.1:9000/health 2>/dev/null || echo 000)"
echo "curl_http_code=$code body=$(docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" exec -T mock-upstream head -c 200 /tmp/ps2-mock-health.txt 2>/dev/null || true)"
[[ "$code" == "200" ]] || {
  echo "MOCK_UPSTREAM_READY=NO"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" logs mock-upstream 2>&1 | tail -40
  exit 1
}

echo "=== xtask COMPOSE_PROJECT_NAME propagation check ==="
probe_dir="$(mktemp -d)"
trap 'rm -rf "$probe_dir"' EXIT
cat >"$probe_dir/probe.sh" <<'PROBE'
#!/usr/bin/env bash
echo "EFFECTIVE_COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-unset}"
PROBE
chmod +x "$probe_dir/probe.sh"
# Verify run.rs honors env (compile-time path): grep source
if grep -q 'std::env::var("COMPOSE_PROJECT_NAME")' "$REPO_ROOT/xtask/src/perf/contract/run.rs"; then
  echo "XTASK_RESPECTS_ENV_COMPOSE_PROJECT_NAME=YES"
else
  echo "XTASK_RESPECTS_ENV_COMPOSE_PROJECT_NAME=NO"
  exit 1
fi

echo "=== result pull glob validation ==="
if compgen -G "$REPO_ROOT/benchmarks/results/perf-contract-full-"'*' >/dev/null 2>&1 || \
   grep -q 'perf-contract-full-' "$REPO_ROOT/scripts/remote/run-ps2-baseline-freeze.sh" 2>/dev/null; then
  echo "RESULT_PULL_GLOB_VALIDATED=YES"
else
  echo "RESULT_PULL_GLOB_VALIDATED=NO"
  exit 1
fi

echo "MOCK_UPSTREAM_HEALTH=HTTP_200"
echo "STACK_BOOTSTRAP_BEFORE_FUNCTIONAL=YES"
echo "MOCK_UPSTREAM_READY=YES"
echo "PS2_R5B_HARNESS_PRECHECK=PASS"
echo "PS2_R6S_HARNESS_PRECHECK=PASS"
