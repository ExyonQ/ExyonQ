#!/usr/bin/env bash
# PS2 directed comparison (development): 4 modes × P1 × 5 reps after F3 split.
# Does NOT touch PS2_CANONICAL_BASELINE_INDEX.json. Not an official Tier A run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/ps3a-f3-l1}"
export BENCH_EXYONQ_ONLY=1
export BENCH_SKIP_FUNCTIONAL=1
export BENCH_NETWORK=internal
export BENCH_PERF_MODE=docker
export BENCH_LOAD_MODE=ceiling

RUN_ID="${PS2_DIRECTED_RUN_ID:-ps2d-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS="$ROOT/docs/benchmarks/platform-split/ps2-directed/${RUN_ID}"
COMPOSE_FILE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_PROJECT="${PS2_DIRECTED_COMPOSE_PROJECT:-exyonq-ps2-directed}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
REPS="${PS2_DIRECTED_REPS:-5}"
PROBE_TIMEOUT="${PS2_DIRECTED_TIMEOUT_SEC:-120}"
FORCE_REBUILD="${PS2_FORCE_EXYONQ_REBUILD:-1}"

mkdir -p "$RESULTS"/{env,logs,modes}
exec > >(tee -a "$RESULTS/logs/directed-${STAMP}.log") 2>&1
echo "=== PS2 directed P1 compare $STAMP run=$RUN_ID reps=$REPS ==="
echo "BASELINE_INDEX_TOUCHED=NO"
echo "RUN_CLASS=DEVELOPMENT_DIRECTED_COMPARE"

# modes: name | env assignments (cleared others)
MODES=(
  "default_tokio|"
  "epoll_listen|EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_LISTEN=1"
  "sync_accept|EXYONQ_SYNC_ACCEPT=1"
  "io_uring|EXYONQ_IO_URING=1"
)

clear_mode_env() {
  unset EXYONQ_EPOLL_STATIC EXYONQ_EPOLL_LISTEN EXYONQ_SYNC_ACCEPT EXYONQ_IO_URING
  export EXYONQ_EPOLL_STATIC= EXYONQ_EPOLL_LISTEN= EXYONQ_SYNC_ACCEPT= EXYONQ_IO_URING=
}

apply_mode_env() {
  local spec="$1"
  clear_mode_env
  # shellcheck disable=SC2086
  eval "export $spec"
}

health_wait() {
  local label="$1"
  local i code
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

bootstrap_mode() {
  local mode="$1"
  echo "=== bootstrap mode=$mode ==="
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
  if [[ ! -f "$RESULTS/env/exyonq-image-built.flag" ]]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build upstream exyonq bench-runner 2>&1 | tail -6
    if [[ "$FORCE_REBUILD" == "1" ]]; then
      docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench build --no-cache exyonq 2>&1 | tail -6
    fi
    touch "$RESULTS/env/exyonq-image-built.flag"
  fi
  # Force recreate so mode env vars are applied to the exyonq container.
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" --profile bench up -d --force-recreate upstream exyonq bench-runner
  for i in $(seq 1 60); do
    mu=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-upstream-1" 2>/dev/null || echo missing)
    ex=$(docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" 2>/dev/null || echo missing)
    echo "health_poll=$i upstream=$mu exyonq=$ex"
    [[ "$mu" == "healthy" && "$ex" == "healthy" ]] && break
    sleep 2
  done
  docker logs "${COMPOSE_PROJECT}-exyonq-1" 2>&1 | tee "$RESULTS/modes/$mode/boot.log" | tail -15
}

run_rep() {
  local mode="$1" rep="$2"
  local out="$RESULTS/modes/$mode/rep${rep}"
  mkdir -p "$out"
  echo "=== mode=$mode rep=$rep ==="
  set +e
  COMPOSE_PROJECT_NAME="$COMPOSE_PROJECT" \
  BENCH_SCENARIOS_FILTER=p1 BENCH_WARMUP_SEC=5 BENCH_DURATION=10s \
    BENCH_RESULTS_DIR="$out" \
    timeout "${PROBE_TIMEOUT}s" cargo run -p xtask -- bench perf --duration 10s --in-docker \
    2>&1 | tee "$out/bench-perf.log"
  local rc=$?
  set -e
  echo "bench_perf_rc=$rc" | tee "$out/run-meta.txt"
  docker logs --timestamps "${COMPOSE_PROJECT}-exyonq-1" >"$out/exyonq.log" 2>&1 || true
  docker inspect -f '{{.State.Health.Status}}' "${COMPOSE_PROJECT}-exyonq-1" | tee "$out/exyonq-health.txt"
  health_wait "post-$mode-rep$rep" || true
  # iu2 marker
  if grep -q "io_uring static worker stopped" "$out/exyonq.log" 2>/dev/null; then
    echo "WORKER_FATAL_DETECTED mode=$mode rep=$rep"
  fi
  return "$rc"
}

{
  echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  echo "host=$(hostname)"
  echo "rustc=$(rustc --version)"
  echo "HEAD_NOTE=rsync-dirty-tree-f3"
} | tee "$RESULTS/env/environment.txt"

# freeze notice: do not write canonical index
cp "$ROOT/docs/benchmarks/platform-split/ps2-results/PS2_CANONICAL_BASELINE_INDEX.json" \
  "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json"
sha256sum "$RESULTS/env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json" | tee "$RESULTS/env/baseline-index.sha256"

for entry in "${MODES[@]}"; do
  mode="${entry%%|*}"
  spec="${entry#*|}"
  mkdir -p "$RESULTS/modes/$mode"
  apply_mode_env "$spec"
  echo "MODE_ENV mode=$mode EXYONQ_EPOLL_STATIC=${EXYONQ_EPOLL_STATIC:-} EXYONQ_EPOLL_LISTEN=${EXYONQ_EPOLL_LISTEN:-} EXYONQ_SYNC_ACCEPT=${EXYONQ_SYNC_ACCEPT:-} EXYONQ_IO_URING=${EXYONQ_IO_URING:-}"
  bootstrap_mode "$mode"
  for rep in $(seq 1 "$REPS"); do
    run_rep "$mode" "$rep" || echo "REP_FAIL mode=$mode rep=$rep"
  done
done

docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true

# score
python3 - <<'PY' "$RESULTS" "$REPS"
import json, sys, statistics, re
from pathlib import Path
results = Path(sys.argv[1])
reps = int(sys.argv[2])
modes = ["default_tokio", "epoll_listen", "sync_accept", "io_uring"]
baseline = json.loads((results/"env/PS2_CANONICAL_BASELINE_INDEX.readonly-copy.json").read_text())
base_med = {}
base_notes = {}
for s in baseline.get("scenarios", []):
    if s.get("scenario") == "p1":
        base_med[s["worker"]] = s.get("statistics", {}).get("rps", {}).get("median")
        base_notes[s["worker"]] = "index_p1_5rep_median"
# results = .../platform-split/ps2-directed/<run>
plat = results.parent.parent
for c in [
    plat / "ps2-results/netcup-amd64-r7c-post-iu2/performance/default_tokio/perf-contract-full-20260715T173108Z/p1/exyonq/summary.json",
    plat / "ps2-results/netcup-amd64-r7c-post-iu2/performance/default_tokio/perf-contract-full-20260715T150846Z/p1/exyonq/summary.json",
]:
    if c.exists():
        j = json.loads(c.read_text())
        rps = j.get("rps") or j.get("requests_per_sec")
        if rps is not None:
            base_med.setdefault("default_tokio", float(rps))
            base_notes["default_tokio"] = "contract_full_single_p1"
            break
base_notes.setdefault("sync_accept", "no_frozen_p1_baseline")

out_rows = []
fatal = 0
for mode in modes:
    rps_list = []
    health_ok = 0
    for rep in range(1, reps+1):
        d = results/"modes"/mode/f"rep{rep}"
        summ = d/"p1"/"exyonq"/"summary.json"
        p1 = d/"p1-exyonq.json"
        rps = None
        if summ.exists():
            j = json.loads(summ.read_text())
            rps = j.get("rps") or j.get("requests_per_sec")
        elif p1.exists():
            j = json.loads(p1.read_text())
            def dig(o):
                if isinstance(o, dict):
                    for k,v in o.items():
                        if k in ("rps","requests_per_sec") and isinstance(v,(int,float)): return v
                        r=dig(v)
                        if r is not None: return r
                return None
            rps = dig(j)
        if rps is not None:
            rps_list.append(float(rps))
        h = (d/"exyonq-health.txt").read_text().strip() if (d/"exyonq-health.txt").exists() else ""
        if h == "healthy":
            health_ok += 1
        log = d/"exyonq.log"
        if log.exists() and "io_uring static worker stopped" in log.read_text(errors="ignore"):
            fatal += 1
    med = statistics.median(rps_list) if rps_list else None
    mean = statistics.mean(rps_list) if rps_list else None
    cv = (statistics.pstdev(rps_list)/mean*100) if rps_list and mean else None
    b = base_med.get(mode)
    delta = ((med - b)/b*100) if (med is not None and b) else None
    out_rows.append({
        "mode": mode,
        "reps": len(rps_list),
        "rps_median": med,
        "rps_mean": mean,
        "rps_cv_pct": cv,
        "baseline_median": b,
        "delta_vs_baseline_pct": delta,
        "baseline_note": base_notes.get(mode, ""),
        "health_ok": health_ok,
        "rps_samples": rps_list,
    })

# sync_accept: if no frozen baseline, compare to directed default_tokio median
by = {r["mode"]: r for r in out_rows}
if by.get("sync_accept") and by["sync_accept"]["baseline_median"] is None and by.get("default_tokio", {}).get("rps_median"):
    sm = by["sync_accept"]["rps_median"]
    dm = by["default_tokio"]["rps_median"]
    if sm is not None and dm:
        by["sync_accept"]["baseline_median"] = dm
        by["sync_accept"]["delta_vs_baseline_pct"] = (sm - dm) / dm * 100
        by["sync_accept"]["baseline_note"] = "directed_default_tokio_median"

verdicts = results/"directed-verdicts.json"
verdicts.write_text(json.dumps({"rows": out_rows, "worker_fatal_events": fatal}, indent=2) + "\n")
print(json.dumps({"rows": out_rows, "worker_fatal_events": fatal}, indent=2))

# text verdicts
lines = ["PS2_DIRECTED_COMPARISON=COMPLETE", f"WORKER_FATAL_EVENTS={fatal}", "BASELINE_INDEX_TOUCHED=NO", "RUN_CLASS=DEVELOPMENT"]
block = False
for r in out_rows:
    d = r["delta_vs_baseline_pct"]
    lines.append(f"{r['mode']}: median={r['rps_median']} baseline={r['baseline_median']} delta_pct={d} cv={r['rps_cv_pct']} health={r['health_ok']}/{reps}")
    # soft gates: >-5% vs baseline OR missing
    if r["reps"] < reps or r["health_ok"] < reps:
        block = True
    if d is not None and d < -5.0:
        block = True
        lines.append(f"REGRESSION_FLAG mode={r['mode']} delta_pct={d}")
if fatal:
    block = True
lines.append("IU2_PROTECTOR=" + ("PASS" if fatal == 0 else "FAIL"))
lines.append("F3_CLOSURE_BLOCKED_BY_REGRESSION=" + ("YES" if block else "NO"))
lines.append("PS2_DIRECTED_COMPARISON_VERDICT=" + ("PASS" if not block else "FAIL"))
(results/"directed-verdicts.txt").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
PY

echo "PS2_DIRECTED_COMPLETE run=$RUN_ID dir=$RESULTS"
