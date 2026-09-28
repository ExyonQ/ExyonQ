#!/usr/bin/env bash
# K1-3c — bounded perf investigation: baseline or iteration (ExyonQ-only, no publication).
# Usage: k1-3c-perf-investigation-remote.sh <repo> <run_id> [baseline|iter-N]
set -euo pipefail

REPO="${1:?repo path}"
RUN_ID="${2:?run_id}"
PHASE="${3:-baseline}"
DEST="$REPO/benchmarks/results-dev/k1-3c-${PHASE}-${RUN_ID}"
COMPOSE="$REPO/benchmarks/docker/docker-compose.bench.yml"
PERF_DIR="$REPO/benchmarks/scenarios/perf"
LOG="$DEST/k1-3c-remote.log"

mkdir -p "$DEST"
cd "$REPO"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

export BENCH_PROVIDER="${BENCH_PROVIDER:-netcup}"
export BENCH_NODE_PRESET="${BENCH_NODE_PRESET:-netcup-rs2000}"
export BENCH_SHAPE="${BENCH_SHAPE:-RS 2000 G12}"
export BENCH_ARCH="${BENCH_ARCH:-x86_64}"
export BENCH_DURATION="${BENCH_DURATION:-30s}"
export BENCH_WARMUP_SEC="${BENCH_WARMUP_SEC:-20}"
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling
export BENCH_NETWORK=internal
export BENCH_EXYONQ_ONLY=1
export BENCH_ALL_SERVERS=0
export BENCH_EPOLL_STATIC=1
export EXYONQ_EPOLL_STATIC=1
export EXYONQ_EPOLL_SENDFILE=1
export BENCH_P7_MAX_PARALLEL="${BENCH_P7_MAX_PARALLEL:-4}"
export BENCH_COMPOSE_FILE="$COMPOSE"
export BENCH_COMPOSE_DIR="$REPO/benchmarks/docker"
export BENCH_SERVERS_FILTER=exyonq
export COMPOSE_PROJECT_NAME="exyonq-k1-3c-${PHASE}"
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_SKIP_UP=1

exec > >(tee -a "$LOG") 2>&1

echo "=== K1-3c perf investigation phase=$PHASE ==="
echo "run_id=$RUN_ID dest=$DEST"
echo "commit=$(git rev-parse HEAD 2>/dev/null || cat .k1-3b-commit 2>/dev/null || echo unknown)" | tee "$DEST/.commit"
date -u

{
  echo "phase=$PHASE"
  echo "captured_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  uptime
  free -h
} > "$DEST/host-snapshot.txt"

for proj in docker exyonq-k1-rr exyonq-k0h-smoke exyonq-k0-baseline exyonq-protector-diag exyonq-k1-3b exyonq-k1-3c-baseline exyonq-k1-3c-iter-1 exyonq-k1-3c-iter-2; do
  docker compose -p "$proj" -f "$COMPOSE" down -v 2>/dev/null || true
done
docker ps -q --filter publish=8080 | xargs -r docker stop 2>/dev/null || true
docker ps -q --filter publish=8443 | xargs -r docker stop 2>/dev/null || true

echo "[stack] exyonq + mock-upstream + bench-runner (project=$COMPOSE_PROJECT_NAME)"
docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" up -d --build mock-upstream exyonq bench-runner
sleep 5

run_health_smoke() {
  local label="$1"
  local out="$DEST/smoke-health-metrics-${label}.txt"
  {
    docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" exec -T bench-runner \
      curl -sf -o /dev/null -w 'health_http=%{http_code}\n' http://exyonq:8080/health || echo "health_http=FAIL"
    docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" exec -T bench-runner \
      curl -sf -o /dev/null -w 'metrics_http=%{http_code}\n' http://exyonq:8080/metrics || echo "metrics_http=FAIL"
  } | tee "$out"
}

echo "--- health/metrics smoke (post-up) ---"
run_health_smoke "post-up"

bash "$PERF_DIR/capture-environment.sh" "$DEST"

run_exyonq_perf() {
  local scen="$1"
  local roadmap="${2:-0}"
  local outdir="$DEST/perf-${scen}"
  mkdir -p "$outdir"
  export BENCH_RESULTS_DIR="$outdir"
  export BENCH_SCENARIOS_FILTER="$scen"
  export BENCH_INCLUDE_ROADMAP="$roadmap"
  echo "--- bench perf scenario=$scen roadmap=$roadmap -> $outdir ---"
  bash "$PERF_DIR/run-perf-docker.sh"
  cp -a "$outdir"/*.json "$DEST/" 2>/dev/null || true
}

echo "--- mandatory baseline/protector scenarios ---"
run_exyonq_perf p1 0
run_exyonq_perf p2 0
run_exyonq_perf p3 0
run_exyonq_perf p4 0
run_exyonq_perf p7 0
run_exyonq_perf p11 1

echo "--- health/metrics smoke (post-bench) ---"
run_health_smoke "post-bench"
cp "$DEST/smoke-health-metrics-post-bench.txt" "$DEST/smoke-health-metrics.txt" 2>/dev/null || true

echo "--- sendfile engagement (S2/S3 via metrics grep) ---"
METRICS="$(docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" exec -T bench-runner \
  curl -sf http://exyonq:8080/metrics 2>/dev/null || true)"
echo "$METRICS" | grep -E 'exyonq_epoll_sendfile|sendfile' > "$DEST/sendfile-metrics-snapshot.txt" 2>/dev/null || true

export REPO="$REPO"
python3 - "$DEST" "$RUN_ID" "$PHASE" "$REPO" <<'PY'
import json
import subprocess
import sys
from pathlib import Path

dest = Path(sys.argv[1])
run_id = sys.argv[2]
phase = sys.argv[3]
repo = Path(sys.argv[4])

SCENARIOS = ["p1", "p2", "p3", "p4", "p7", "p11"]

def load_json(name):
    p = dest / name
    if not p.is_file():
        p = dest / f"perf-{name.split('-')[0]}" / name
    if p.is_file():
        return json.loads(p.read_text())
    return None

def metrics_from_report(data):
    if not data:
        return {}
    s = data.get("summary", {})
    lat = data.get("latencyPercentiles") or data.get("latency", {}).get("percentiles", {})
    return {
        "rps": s.get("requestsPerSec"),
        "p50_ms": lat.get("p50"),
        "p95_ms": lat.get("p95"),
        "p99_ms": lat.get("p99"),
        "success_rate": s.get("successRate"),
    }

commit = "unknown"
if (repo / ".git").exists():
    try:
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    except subprocess.CalledProcessError:
        pass
elif (dest / ".commit").is_file():
    commit = (dest / ".commit").read_text().strip()

meta = {
    "k1_3c": True,
    "phase": phase,
    "run_id": run_id,
    "tier": "diagnostic",
    "official": False,
    "public_claims_forbidden": True,
    "host": "netcup-amd64",
    "arch": "x86_64",
    "commit": commit,
}
(dest / "run_meta.json").write_text(json.dumps(meta, indent=2) + "\n")

summary = {sc: metrics_from_report(load_json(f"{sc}-exyonq.json")) for sc in SCENARIOS}
(dest / "metrics-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print("Wrote metrics-summary.json")
PY

echo "K1-3C-REMOTE-DONE phase=$PHASE dest=$DEST"
docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" down -v 2>/dev/null || true
