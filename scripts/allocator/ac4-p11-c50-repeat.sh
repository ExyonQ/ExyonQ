#!/usr/bin/env bash
# AC4 supplemental: P11 repeatability at 50 connections (ladder peak).
set -euo pipefail
WS=/root/exyonq-allocator-compare
COMPOSE=$WS/benchmarks/docker/docker-compose.bench.yml
RESULTS=$WS/benchmarks/results/allocator-ac4-headroom
SAMPLE=$WS/benchmarks/scripts/sample_resources.sh
CSV=$RESULTS/repeat-p11-c50-summary.tsv
HOST_CPUS=$(nproc)
CONNS=50

echo -e "scenario\tconns\trep\trps\tsuccess\tp50_ms\tp95_ms\tp99_ms\tserver_cpu_peak\tserver_mem_peak\tloadgen_cpu_peak\tloadgen_cpu_avg\tstop_reason" >"$CSV"

for rep in 1 2 3; do
  echo "=== P11 c50 rep $rep ==="
  cd "$WS/benchmarks/docker"
  docker tag exyonq-alloc-system:local docker-exyonq:latest
  EXYONQ_CONFIG=/bench/bench-tls.toml docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq
  docker compose -f docker-compose.bench.yml stop \
    nginx-stable nginx-mainline haproxy envoy traefik caddy apache \
    openlitespeed-stable openlitespeed-latest >/dev/null 2>&1 || true
  sleep 8
  ok=0
  for _ in $(seq 1 40); do
    if curl -skf --max-time 2 https://127.0.0.1:8443/site/1k.bin -o /dev/null; then ok=1; break; fi
    sleep 1
  done
  [[ $ok -eq 1 ]] || { echo "health fail"; exit 1; }
  echo ready

  tag="rep-p11-c${CONNS}-r${rep}"
  cid_lg=$(docker compose -f "$COMPOSE" ps -q bench-runner)
  cid_srv=$(docker compose -f "$COMPOSE" ps -q exyonq)
  lg_csv=$(mktemp); srv_csv=$(mktemp)
  bash "$SAMPLE" sample "$cid_lg" 47 "$lg_csv" & lg_pid=$!
  bash "$SAMPLE" sample "$cid_srv" 47 "$srv_csv" & srv_pid=$!

  docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$CONNS" -d 15s -h https://exyonq:8443/site/1k.bin --json -t 2 --http2 >/dev/null 2>&1 || true
  raw=$RESULTS/raw-$tag.json
  docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk -c "$CONNS" -d 30s -h https://exyonq:8443/site/1k.bin --json -t 2 --http2 >"$raw"

  wait "$lg_pid" || true
  wait "$srv_pid" || true
  bash "$SAMPLE" summarize "$lg_csv" "$RESULTS/loadgen-$tag.json" bench-runner "$tag"
  bash "$SAMPLE" summarize "$srv_csv" "$RESULTS/resources-$tag.json" exyonq "$tag"
  rm -f "$lg_csv" "$srv_csv"
  python3 "$WS/benchmarks/scenarios/perf/rewrk-report-to-json.py" "$raw" -o "$RESULTS/converted-$tag.json"

  python3 - "$RESULTS/converted-$tag.json" "$RESULTS/loadgen-$tag.json" "$RESULTS/resources-$tag.json" "$HOST_CPUS" "$rep" "$CSV" <<'PY'
import json, sys
from pathlib import Path
conv, lgp, srvp, host, rep, csv = sys.argv[1:7]
host = int(host)
data = json.loads(Path(conv).read_text())
lg = json.loads(Path(lgp).read_text())
srv = json.loads(Path(srvp).read_text())

def norm(p):
    if p is None:
        return None
    p = float(p)
    return round(p / host, 2) if p > 100 else round(p, 2)

s = data["summary"]
lat = data["latencyPercentiles"]
rps = s["requestsPerSec"]
success = float(s["successRate"])
p50 = round(lat["p50"] * 1000, 3)
p95 = round(lat["p95"] * 1000, 3)
p99 = round(lat["p99"] * 1000, 3)
line = (
    f"p11\t50\t{rep}\t{rps}\t{success}\t{p50}\t{p95}\t{p99}\t"
    f"{norm(srv.get('cpu_pct_peak'))}\t{srv.get('mem_mib_peak')}\t"
    f"{norm(lg.get('cpu_pct_peak'))}\t{norm(lg.get('cpu_pct_avg') or lg.get('cpu_avg_pct'))}\t\n"
)
open(csv, "a").write(line)
print(line.strip())
PY
done

python3 - "$CSV" "$RESULTS/repeat-p11-c50-stats.json" <<'PY'
import csv, json, statistics, sys
from pathlib import Path
rows = list(csv.DictReader(open(sys.argv[1]), delimiter="\t"))
rps = [float(r["rps"]) for r in rows]
lg = [float(r["loadgen_cpu_peak"]) for r in rows]
mean = statistics.mean(rps)
sd = statistics.pstdev(rps)
cv = sd / mean * 100
out = {
    "p11_c50": {
        "n": len(rps),
        "rps_mean": round(mean, 2),
        "rps_median": round(statistics.median(rps), 2),
        "rps_cv_pct": round(cv, 3),
        "loadgen_cpu_peak_max": max(lg),
        "server_cpu_peak_max": max(float(r["server_cpu_peak"]) for r in rows),
        "rps_cv_pass": cv <= 5.0,
        "loadgen_pass": max(lg) < 85.0,
        "success_all_1": all(float(r["success"]) == 1.0 for r in rows),
    }
}
Path(sys.argv[2]).write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out, indent=2))
PY
echo P11_C50_DONE
