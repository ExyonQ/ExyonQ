#!/usr/bin/env bash
# AC4 loadgen headroom ladder — ExyonQ system allocator only, P1/P4/P11.
# Usage: bash scripts/allocator/ac4-loadgen-ladder.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
COMPOSE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_DIR="$ROOT/benchmarks/docker"
RESULTS="${BENCH_RESULTS_DIR:-$ROOT/benchmarks/results/allocator-ac4-headroom}"
mkdir -p "$RESULTS"
SAMPLE="$ROOT/benchmarks/scripts/sample_resources.sh"
REWRK_THREADS="${BENCH_REWRK_THREADS:-2}"
WARMUP_SEC="${BENCH_WARMUP_SEC:-10}"
MEASURE_SEC="${BENCH_MEASURE_SEC:-20}"
SAT_STOP="${BENCH_LOADGEN_STOP_PCT:-85}"
HOST_CPUS="${BENCH_HOST_CPUS:-$(nproc)}"

export BENCH_COMPOSE_FILE="$COMPOSE"
export BENCH_COMPOSE_DIR="$COMPOSE_DIR"
export BENCH_NETWORK=internal
export BENCH_HOST_CPUS="$HOST_CPUS"

CSV_SUMMARY="$RESULTS/ladder-summary.tsv"
echo -e "scenario\tconns\trep\trps\tsuccess\tp50_ms\tp95_ms\tp99_ms\tserver_cpu_peak\tserver_mem_peak\tloadgen_cpu_peak\tloadgen_cpu_avg\tstop_reason" >"$CSV_SUMMARY"

normalize_cpu() {
  # sample_resources may report docker % over all CPUs; convert like collect-loadgen.sh
  python3 - "$1" "$HOST_CPUS" <<'PY'
import sys
peak = float(sys.argv[1])
host = int(sys.argv[2])
if peak > 100.0:
    cap = max(100.0, host * 100.0)
    if peak > cap:
        print("nan")
    else:
        print(round(peak / host, 2))
else:
    print(round(peak, 2))
PY
}

run_one() {
  local scenario="$1" path="$2" conns="$3" rep="$4" scheme="$5" http2="$6" port="$7"
  local tag="${scenario}-c${conns}-r${rep}"
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

  # Stabilize between runs
  sleep 2
  curl -skf --max-time 3 "http://127.0.0.1:8080/health" >/dev/null || true

  local cid_lg cid_srv
  cid_lg="$(docker compose -f "$COMPOSE" ps -q bench-runner)"
  cid_srv="$(docker compose -f "$COMPOSE" ps -q exyonq)"

  local total_sec=$((WARMUP_SEC + MEASURE_SEC + 2))
  local lg_csv srv_csv
  lg_csv="$(mktemp)"
  srv_csv="$(mktemp)"

  bash "$SAMPLE" sample "$cid_lg" "$total_sec" "$lg_csv" &
  local lg_pid=$!
  bash "$SAMPLE" sample "$cid_srv" "$total_sec" "$srv_csv" &
  local srv_pid=$!

  # Warmup
  docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$conns" -d "${WARMUP_SEC}s" -h "$url" --json -t "$REWRK_THREADS" \
    ${http2:+--http2} >/dev/null 2>&1 || true

  # Measure
  if ! docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$conns" -d "${MEASURE_SEC}s" -h "$url" --json -t "$REWRK_THREADS" \
    ${http2:+--http2} >"$raw" 2>/dev/null; then
    echo '{}' >"$raw"
  fi

  wait "$lg_pid" || true
  wait "$srv_pid" || true
  bash "$SAMPLE" summarize "$lg_csv" "$lg_json" "bench-runner" "$tag"
  bash "$SAMPLE" summarize "$srv_csv" "$srv_json" "exyonq" "$tag"
  rm -f "$lg_csv" "$srv_csv"

  local converted="$RESULTS/converted-${tag}.json"
  python3 "$ROOT/benchmarks/scenarios/perf/rewrk-report-to-json.py" "$raw" -o "$converted"

  python3 - "$converted" "$out_json" "$lg_json" "$srv_json" "$HOST_CPUS" "$SAT_STOP" "$scenario" "$conns" "$rep" "$CSV_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
converted, out, lg, srv, host, stop, scenario, conns, rep, csv_path = sys.argv[1:11]
host = int(host); stop = float(stop)
data = json.loads(Path(converted).read_text() or "{}")
summary = data.get("summary") or {}
lat = data.get("latencyPercentiles") or {}
rps = summary.get("requestsPerSec")
success = float(summary.get("successRate") or 0.0)
# converter stores seconds → ms for report
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
    "scenario": scenario,
    "connections": int(conns),
    "rep": int(rep),
    "rps": rps,
    "success_rate": success,
    "p50_ms": p50,
    "p95_ms": p95,
    "p99_ms": p99,
    "percentiles_estimated": data.get("percentiles_estimated", True),
    "server_cpu_peak": srv_peak,
    "server_mem_peak_mib": srv_mem,
    "loadgen_cpu_peak": lg_peak,
    "loadgen_cpu_avg": lg_avg,
    "loadgen_mem_peak_mib": lgd.get("mem_mib_peak"),
}
Path(out).write_text(json.dumps(result, indent=2) + "\n")

stop_reason = ""
if lg_peak is not None and lg_peak >= stop:
    stop_reason = "loadgen_cpu_peak>=stop"
if success < 1.0:
    stop_reason = (stop_reason + "+success<1").strip("+") if stop_reason else "success<1"

line = f"{scenario}\t{conns}\t{rep}\t{rps}\t{success}\t{p50}\t{p95}\t{p99}\t{srv_peak}\t{srv_mem}\t{lg_peak}\t{lg_avg}\t{stop_reason}\n"
with open(csv_path, "a", encoding="utf-8") as f:
    f.write(line)
print(line.strip())
if stop_reason:
    Path(out + ".STOP").write_text(stop_reason + "\n")
PY
}

ladder_scenario() {
  local scenario="$1" path="$2" scheme="$3" http2="$4" port="$5"
  local base_conns=100
  local stop=0
  for pct in 25 50 75 100; do
    local conns=$(( base_conns * pct / 100 ))
    [[ "$conns" -lt 1 ]] && conns=1
    echo "=== LADDER $scenario conns=$conns (${pct}%) ==="
    run_one "$scenario" "$path" "$conns" 1 "$scheme" "$http2" "$port"
    if [[ -f "$RESULTS/${scenario}-c${conns}-r1.json.STOP" ]]; then
      echo "STOP ladder $scenario at conns=$conns"
      stop=1
      break
    fi
  done
  echo "$stop" >"$RESULTS/ladder-stop-${scenario}.flag"
}

stop_rivals() {
  docker compose -f "$COMPOSE" stop \
    nginx-stable nginx-mainline haproxy envoy traefik caddy apache \
    openlitespeed-stable openlitespeed-latest >/dev/null 2>&1 || true
}

echo "AC4 ladder start RESULTS=$RESULTS"
# Ensure system image + minimal topology (no rivals)
docker tag exyonq-alloc-system:local docker-exyonq:latest
cd "$COMPOSE_DIR"
EXYONQ_CONFIG=/bench/bench.toml docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq
docker compose -f docker-compose.bench.yml up -d --no-deps mock-upstream
docker compose -f docker-compose.bench.yml --profile bench up -d --no-deps bench-runner
stop_rivals
sleep 8
curl -sf http://127.0.0.1:8080/health >/dev/null

ladder_scenario p1 "/site/1k.bin" http "" 8080
stop_rivals
# P4 needs mock upstream
ladder_scenario p4 "/api/" http "" 8080
stop_rivals
# P11 TLS — switch config
EXYONQ_CONFIG=/bench/bench-tls.toml docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq
stop_rivals
sleep 8
ladder_scenario p11 "/site/1k.bin" https "--http2" 8443
stop_rivals
# restore default
EXYONQ_CONFIG=/bench/bench.toml docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq >/dev/null
stop_rivals

echo "LADDER_DONE"
cat "$CSV_SUMMARY"
