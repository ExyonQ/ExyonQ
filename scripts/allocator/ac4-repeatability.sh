#!/usr/bin/env bash
# AC4.6 repeatability — 3× at frozen safe point (system allocator only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Reuse ladder helpers by sourcing the run_one logic via calling ladder's patterns
COMPOSE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_DIR="$ROOT/benchmarks/docker"
RESULTS="${BENCH_RESULTS_DIR:-$ROOT/benchmarks/results/allocator-ac4-headroom}"
mkdir -p "$RESULTS"
SAMPLE="$ROOT/benchmarks/scripts/sample_resources.sh"
REWRK_THREADS="${BENCH_REWRK_THREADS:-2}"
WARMUP_SEC="${BENCH_WARMUP_SEC:-10}"
MEASURE_SEC="${BENCH_MEASURE_SEC:-20}"
HOST_CPUS="${BENCH_HOST_CPUS:-$(nproc)}"
CONNS="${BENCH_SAFE_CONNS:-100}"
SAT_STOP="${BENCH_LOADGEN_STOP_PCT:-85}"

export BENCH_COMPOSE_FILE="$COMPOSE"
CSV="$RESULTS/repeat-summary.tsv"
echo -e "scenario\tconns\trep\trps\tsuccess\tp50_ms\tp95_ms\tp99_ms\tserver_cpu_peak\tserver_mem_peak\tloadgen_cpu_peak\tloadgen_cpu_avg\tstop_reason" >"$CSV"

stop_rivals() {
  docker compose -f "$COMPOSE" stop \
    nginx-stable nginx-mainline haproxy envoy traefik caddy apache \
    openlitespeed-stable openlitespeed-latest >/dev/null 2>&1 || true
}

recreate_server() {
  local cfg="$1"
  local mode="${2:-http}" # http | https
  cd "$COMPOSE_DIR"
  docker tag exyonq-alloc-system:local docker-exyonq:latest
  EXYONQ_CONFIG="$cfg" docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq
  docker compose -f docker-compose.bench.yml up -d --no-deps mock-upstream
  docker compose -f docker-compose.bench.yml --profile bench up -d --no-deps bench-runner
  stop_rivals
  sleep 8
  for _ in $(seq 1 40); do
    if [[ "$mode" == "https" ]]; then
      if curl -skf --max-time 2 "https://127.0.0.1:8443/site/1k.bin" -o /dev/null; then
        return 0
      fi
    else
      if curl -sf --max-time 2 http://127.0.0.1:8080/health >/dev/null; then
        return 0
      fi
    fi
    sleep 1
  done
  echo "recreate_server: health timeout mode=$mode cfg=$cfg" >&2
  return 1
}

run_one() {
  local scenario="$1" path="$2" conns="$3" rep="$4" scheme="$5" http2_flag="$6" port="$7"
  local tag="rep-${scenario}-c${conns}-r${rep}"
  local out_json="$RESULTS/${tag}.json"
  local lg_json="$RESULTS/loadgen-${tag}.json"
  local srv_json="$RESULTS/resources-${tag}.json"
  local raw="$RESULTS/raw-${tag}.json"
  local url
  if [[ "$scheme" == "https" ]]; then
    url="https://exyonq:${port}${path}"
  else
    url="http://exyonq:8080${path}"
  fi

  sleep 3
  local cid_lg cid_srv
  cid_lg="$(docker compose -f "$COMPOSE" ps -q bench-runner)"
  cid_srv="$(docker compose -f "$COMPOSE" ps -q exyonq)"
  local total_sec=$((WARMUP_SEC + MEASURE_SEC + 2))
  local lg_csv srv_csv
  lg_csv="$(mktemp)"; srv_csv="$(mktemp)"
  bash "$SAMPLE" sample "$cid_lg" "$total_sec" "$lg_csv" &
  local lg_pid=$!
  bash "$SAMPLE" sample "$cid_srv" "$total_sec" "$srv_csv" &
  local srv_pid=$!

  local http2_args=()
  [[ -n "$http2_flag" ]] && http2_args=(--http2)

  docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$conns" -d "${WARMUP_SEC}s" -h "$url" --json -t "$REWRK_THREADS" \
    "${http2_args[@]}" >/dev/null 2>&1 || true

  if ! docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$conns" -d "${MEASURE_SEC}s" -h "$url" --json -t "$REWRK_THREADS" \
    "${http2_args[@]}" >"$raw" 2>/dev/null; then
    echo '{}' >"$raw"
  fi

  wait "$lg_pid" || true
  wait "$srv_pid" || true
  bash "$SAMPLE" summarize "$lg_csv" "$lg_json" "bench-runner" "$tag"
  bash "$SAMPLE" summarize "$srv_csv" "$srv_json" "exyonq" "$tag"
  rm -f "$lg_csv" "$srv_csv"

  local converted="$RESULTS/converted-${tag}.json"
  python3 "$ROOT/benchmarks/scenarios/perf/rewrk-report-to-json.py" "$raw" -o "$converted"

  python3 - "$converted" "$out_json" "$lg_json" "$srv_json" "$HOST_CPUS" "$SAT_STOP" "$scenario" "$conns" "$rep" "$CSV" <<'PY'
import json, sys
from pathlib import Path
converted, out, lg, srv, host, stop, scenario, conns, rep, csv_path = sys.argv[1:11]
host = int(host); stop = float(stop)
data = json.loads(Path(converted).read_text() or "{}")
summary = data.get("summary") or {}
lat = data.get("latencyPercentiles") or {}
rps = summary.get("requestsPerSec")
success = float(summary.get("successRate") or 0.0)
p50 = round(float(lat["p50"]) * 1000.0, 3) if lat.get("p50") is not None else None
p95 = round(float(lat["p95"]) * 1000.0, 3) if lat.get("p95") is not None else None
p99 = round(float(lat["p99"]) * 1000.0, 3) if lat.get("p99") is not None else None
lgd = json.loads(Path(lg).read_text())
srvd = json.loads(Path(srv).read_text())

def norm(peak):
    if peak is None: return None
    peak = float(peak)
    if peak > 100.0:
        cap = max(100.0, host * 100.0)
        if peak > cap: return None
        peak = peak / host
    return round(peak, 2)

lg_peak = norm(lgd.get("cpu_pct_peak"))
lg_avg = norm(lgd.get("cpu_pct_avg") or lgd.get("cpu_avg_pct"))
srv_peak = norm(srvd.get("cpu_pct_peak"))
srv_mem = srvd.get("mem_mib_peak")
result = {
    "scenario": scenario, "connections": int(conns), "rep": int(rep),
    "rps": rps, "success_rate": success,
    "p50_ms": p50, "p95_ms": p95, "p99_ms": p99,
    "percentiles_estimated": data.get("percentiles_estimated", True),
    "server_cpu_peak": srv_peak, "server_mem_peak_mib": srv_mem,
    "loadgen_cpu_peak": lg_peak, "loadgen_cpu_avg": lg_avg,
}
Path(out).write_text(json.dumps(result, indent=2) + "\n")
stop_reason = ""
if lg_peak is not None and lg_peak >= stop:
    stop_reason = "loadgen_cpu_peak>=stop"
if success < 1.0:
    stop_reason = (stop_reason + "+success<1").strip("+") if stop_reason else "success<1"
line = f"{scenario}\t{conns}\t{rep}\t{rps}\t{success}\t{p50}\t{p95}\t{p99}\t{srv_peak}\t{srv_mem}\t{lg_peak}\t{lg_avg}\t{stop_reason}\n"
open(csv_path, "a", encoding="utf-8").write(line)
print(line.strip())
PY
}

SCENARIOS="${BENCH_REPEAT_SCENARIOS:-p1 p4 p11}"
echo "AC4 repeatability CONNS=$CONNS MEASURE=${MEASURE_SEC}s SCENARIOS=$SCENARIOS RESULTS=$RESULTS"
for scenario in $SCENARIOS; do
  for rep in 1 2 3; do
    echo "=== REP $rep $scenario ==="
    case "$scenario" in
      p1)
        recreate_server /bench/bench.toml http
        run_one p1 "/site/1k.bin" "$CONNS" "$rep" http "" 8080
        ;;
      p4)
        recreate_server /bench/bench.toml http
        run_one p4 "/api/" "$CONNS" "$rep" http "" 8080
        ;;
      p11)
        recreate_server /bench/bench-tls.toml https
        run_one p11 "/site/1k.bin" "$CONNS" "$rep" https "--http2" 8443
        ;;
      *)
        echo "unknown scenario $scenario" >&2
        exit 1
        ;;
    esac
  done
done

# Stats
python3 - "$CSV" "$RESULTS/repeat-stats.json" <<'PY'
import csv, json, statistics, sys
from pathlib import Path
rows = list(csv.DictReader(open(sys.argv[1]), delimiter="\t"))
out = {}
for scenario in ("p1", "p4", "p11"):
    rs = [r for r in rows if r["scenario"] == scenario]
    rps = [float(r["rps"]) for r in rs]
    p99 = [float(r["p99_ms"]) for r in rs if r["p99_ms"] not in ("", "None")]
    lg = [float(r["loadgen_cpu_peak"]) for r in rs]
    srv = [float(r["server_cpu_peak"]) for r in rs]
    mean = statistics.mean(rps)
    med = statistics.median(rps)
    sd = statistics.pstdev(rps) if len(rps) > 1 else 0.0
    cv = (sd / mean * 100.0) if mean else None
    out[scenario] = {
        "n": len(rps),
        "rps_mean": round(mean, 2),
        "rps_median": round(med, 2),
        "rps_cv_pct": round(cv, 3) if cv is not None else None,
        "p99_mean_ms": round(statistics.mean(p99), 3) if p99 else None,
        "loadgen_cpu_peak_max": max(lg) if lg else None,
        "server_cpu_peak_max": max(srv) if srv else None,
        "success_all_1": all(float(r["success"]) == 1.0 for r in rs),
        "rps_cv_pass": cv is not None and cv <= 5.0,
        "loadgen_pass": max(lg) < 85.0 if lg else False,
    }
Path(sys.argv[2]).write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out, indent=2))
PY

echo REPEAT_DONE
cat "$CSV"
