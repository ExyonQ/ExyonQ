#!/usr/bin/env bash
# K1-3b — read-only profile execution on Linux (Netcup amd64).
# ExyonQ-only perf + strace + perf record. No code changes. Diagnostic tier.
set -euo pipefail

REPO="${1:?repo path}"
RUN_ID="${2:?run_id}"
DEST="$REPO/benchmarks/results-dev/k1-3b-profile-${RUN_ID}"
COMPOSE="$REPO/benchmarks/docker/docker-compose.bench.yml"
PERF_DIR="$REPO/benchmarks/scenarios/perf"
LOG="$DEST/k1-3b-remote.log"

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
export BENCH_P7_MAX_PARALLEL="${BENCH_P7_MAX_PARALLEL:-4}"
export BENCH_COMPOSE_FILE="$COMPOSE"
export BENCH_COMPOSE_DIR="$REPO/benchmarks/docker"
export BENCH_SERVERS_FILTER=exyonq
export COMPOSE_PROJECT_NAME=exyonq-k1-3b
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_SKIP_UP=1

exec > >(tee -a "$LOG") 2>&1

echo "=== K1-3b read-only profile ==="
echo "run_id=$RUN_ID dest=$DEST"
echo "commit=$(git rev-parse HEAD 2>/dev/null || echo unknown)" | tee "$DEST/.k1-3b-commit"
date -u

# Host idle snapshot
{
  echo "captured_at_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  uptime
  free -h
  vmstat 1 3 2>/dev/null || true
} > "$DEST/host-idle-snapshot.txt"

for proj in docker exyonq-k1-rr exyonq-k0h-smoke exyonq-k0-baseline exyonq-protector-diag exyonq-k1-3b; do
  docker compose -p "$proj" -f "$COMPOSE" down -v 2>/dev/null || true
done
# Free host ports if a stray default-project stack is still bound.
docker ps -q --filter publish=8080 | xargs -r docker stop 2>/dev/null || true
docker ps -q --filter publish=8443 | xargs -r docker stop 2>/dev/null || true

echo "[stack] minimal exyonq + mock-upstream + bench-runner (project=$COMPOSE_PROJECT_NAME)"
docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" up -d --build mock-upstream exyonq bench-runner

bash "$PERF_DIR/capture-environment.sh" "$DEST"

run_exyonq_perf() {
  local scen="$1"
  local roadmap="${2:-0}"
  local outdir="$DEST/perf-${scen}"
  mkdir -p "$outdir"
  export BENCH_RESULTS_DIR="$outdir"
  export BENCH_SCENARIOS_FILTER="$scen"
  export BENCH_INCLUDE_ROADMAP="$roadmap"
  echo "--- bench perf ExyonQ-only scenario=$scen roadmap=$roadmap -> $outdir ---"
  bash "$PERF_DIR/run-perf-docker.sh"
  cp -a "$outdir"/*.json "$DEST/" 2>/dev/null || true
}

run_strace_exyonq() {
  local scen="$1"
  local url="$2"
  local extra=("${@:3}")
  local cid pid
  cid="$(docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" ps -q exyonq)"
  [[ -n "$cid" ]] || { echo "no exyonq container"; return 1; }
  pid="$(docker inspect -f '{{.State.Pid}}' "$cid")"
  echo "--- strace exyonq scenario=$scen pid=$pid ---"
  docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" exec -T bench-runner \
    rewrk -h "$url" -c 100 -d 8s -t 4 "${extra[@]}" >/dev/null &
  local load_pid=$!
  sleep 0.5
  timeout 10 strace -f -e trace=write,send,sendmsg,read,recvfrom,recvmsg,epoll_wait,epoll_ctl \
    -c -p "$pid" -o "$DEST/strace-${scen}-exyonq.txt" 2>/dev/null || true
  wait "$load_pid" 2>/dev/null || true
}

run_perf_record() {
  local scen="$1"
  local url="$2"
  local extra=("${@:3}")
  local profdir="$DEST/profile-${scen}"
  mkdir -p "$profdir"
  if ! command -v perf >/dev/null 2>&1; then
    echo "perf not installed" > "$profdir/SKIP-no-perf.txt"
    return 0
  fi
  local cid pid
  cid="$(docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" ps -q exyonq)"
  pid="$(docker inspect -f '{{.State.Pid}}' "$cid")"
  echo "--- perf record scenario=$scen pid=$pid ---"
  docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" exec -T bench-runner \
    rewrk -h "$url" -c 100 -d 15s -t 4 "${extra[@]}" >/dev/null &
  local load_pid=$!
  sleep 1
  perf record -F 997 -p "$pid" -g -o "$profdir/perf.data" -- sleep 12 2>/dev/null || true
  wait "$load_pid" 2>/dev/null || true
  perf report -i "$profdir/perf.data" --stdio --sort comm,dso,symbol --no-children 2>/dev/null \
    | head -100 > "$profdir/perf-top.txt" || true
  if command -v stackcollapse-perf.pl >/dev/null 2>&1 && command -v flamegraph.pl >/dev/null 2>&1; then
    perf script -i "$profdir/perf.data" 2>/dev/null \
      | stackcollapse-perf.pl 2>/dev/null \
      | flamegraph.pl > "$profdir/flamegraph.svg" 2>/dev/null || true
  fi
}

# --- P1 ---
run_exyonq_perf p1 0
run_strace_exyonq p1 "http://exyonq:8080/site/1k.bin"
run_perf_record p1 "http://exyonq:8080/site/1k.bin"

# --- P7 ---
run_exyonq_perf p7 0
run_strace_exyonq p7 "http://exyonq:8080/site/routes/route050.bin"
run_perf_record p7 "http://exyonq:8080/site/routes/route050.bin"

# --- P11 (roadmap / TLS+h2) ---
run_exyonq_perf p11 1
run_strace_exyonq p11 "https://exyonq:8443/site/1k.bin" --http2
run_perf_record p11 "https://exyonq:8443/site/1k.bin" --http2

# Optional P1 repeat (variance check)
if [[ "${K1_3B_P1_REPEAT:-1}" == "1" ]]; then
  run_exyonq_perf p1 0
  cp "$DEST/perf-p1/p1-exyonq.json" "$DEST/p1-exyonq-repeat.json" 2>/dev/null || true
fi

export REPO="$REPO"
python3 - "$DEST" "$RUN_ID" "$REPO" <<'PY'
import json
import subprocess
import sys
from pathlib import Path

dest = Path(sys.argv[1])
run_id = sys.argv[2]
repo = Path(sys.argv[3])

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
elif (repo / ".k1-3b-commit").is_file():
    commit = (repo / ".k1-3b-commit").read_text().strip()

meta = {
    "k1_3b": True,
    "run_id": run_id,
    "tier": "diagnostic",
    "official": False,
    "official_publishable": False,
    "public_claims_forbidden": True,
    "host": "netcup-amd64",
    "arch": "x86_64",
    "commit": commit,
}
(dest / "run_meta.json").write_text(json.dumps(meta, indent=2) + "\n")

summary = {
    "p1": metrics_from_report(load_json("p1-exyonq.json")),
    "p7": metrics_from_report(load_json("p7-exyonq.json")),
    "p11": metrics_from_report(load_json("p11-exyonq.json")),
    "p1_repeat": metrics_from_report(load_json("p1-exyonq-repeat.json")),
}
(dest / "metrics-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print("Wrote metrics-summary.json")
PY

echo "K1-3b-REMOTE-DONE dest=$DEST"
docker compose -p "$COMPOSE_PROJECT_NAME" -f "$COMPOSE" down -v 2>/dev/null || true
