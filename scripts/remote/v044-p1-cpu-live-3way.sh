#!/usr/bin/env bash
# V044_P1_CPU_EFFICIENCY_LIVE_THREE_WAY — ExyonQ (Root1+Root3) vs NGINX vs OLS.
# P2=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN. PRODUCT_MUTATION=NO.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_3WAY_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_3WAY_EV:-$WS/.exyonq-local-evidence/v044-p1-cpu-live-3way-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P1="/site/1k.bin"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5
CPU_CORES=8

# Sealed Root3 product binary expected on ExyonQ (updated after rebuild).
EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-}"

mkdir -p "$EV"/{runs,stats,pct,meta,reports,fd}
cd "$WS"
log() { echo "[3way] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
  esac
}
url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P1}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P1}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P1}" ;;
  esac
}

prepare() {
  for tag in exyonq nginx ols; do
    c=$(ctr "$tag")
    docker update --cpuset-cpus "0-7" "$c" >/dev/null
    docker inspect -f 'Cpuset={{.HostConfig.CpusetCpus}} CpuQuota={{.HostConfig.CpuQuota}}' "$c" \
      | tee -a "$EV/meta/cpuset.txt"
  done
  EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
  if [[ -n "$EXPECTED_EXYONQ_SHA256" && "$EXY_SHA" != "$EXPECTED_EXYONQ_SHA256" ]]; then
    log "WARN: ExyonQ SHA $EXY_SHA != expected $EXPECTED_EXYONQ_SHA256"
  fi
  for tag in exyonq nginx ols; do
    u=$(url_for "$tag")
    out=$(compose exec -T bench-runner curl -sS -m 5 -o "/tmp/${tag}.bin" -w "%{http_code} %{size_download}" "$u")
    echo "${tag}_probe=$out" | tee -a "$EV/meta/http_probe.txt"
  done
}

run_rep() {
  local tag=$1 rep=$2
  local c url statsjson
  c=$(ctr "$tag")
  url=$(url_for "$tag")
  statsjson="$EV/stats/${tag}-rep${rep}.cpu.txt"
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  : >"$statsjson"
  timeout $((MEASURE_SEC + 15)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
  local sp=$!
  sleep 0.4
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/runs/${tag}-rep${rep}.json"
  wait "$sp" 2>/dev/null || true
  # percentile probe (human table) — one per server after last rep only handled outside
}

pct_probe() {
  local tag=$1
  local url
  url=$(url_for "$tag")
  compose exec -T bench-runner bash -lc \
    "rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" \
    | tee "$EV/pct/${tag}.txt"
}

log "START TS=$TS EV=$EV"
prepare
for rep in $(seq 1 "$REPS"); do
  log "rep $rep/$REPS exyonq"
  run_rep exyonq "$rep"
  log "rep $rep/$REPS nginx"
  run_rep nginx "$rep"
  log "rep $rep/$REPS ols"
  run_rep ols "$rep"
done
log "percentile probes"
for tag in exyonq nginx ols; do
  pct_probe "$tag" || true
done

python3 - "$EV" <<'PY' | tee "$EV/reports/threeway_terminal.txt"
import json, re, statistics, math, sys
from pathlib import Path

ev = Path(sys.argv[1])
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
tags = ("exyonq", "nginx", "ols")

def median(xs):
    return statistics.median(xs) if xs else None

def cv(xs):
    if not xs or len(xs) < 2:
        return None
    m = statistics.mean(xs)
    return (statistics.stdev(xs) / m) if m else None

def parse_cpu_mem(path):
    cpus, rss = [], []
    if not path.exists():
        return cpus, rss
    for ln in path.read_text(errors="replace").splitlines():
        ln = ansi.sub("", ln).strip()
        m2 = re.search(r"([\d.]+)%\s+(\d+(?:\.\d+)?)(MiB|GiB|KiB)", ln)
        if not m2:
            continue
        cpus.append(float(m2.group(1)))
        val = float(m2.group(2))
        unit = m2.group(3)
        mul = {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024}[unit]
        rss.append(val * mul)
    return cpus, rss

def parse_pct(path):
    text = path.read_text(errors="replace") if path.exists() else ""
    out = {}
    for pct in ("50", "90", "95", "99"):
        m = re.search(rf"\|\s*{pct}%\s*\|\s*([\d.]+)ms", text)
        if m:
            out[f"p{pct}"] = float(m.group(1))
    lines = text.splitlines()
    for i, ln in enumerate(lines):
        if "Max" in ln and i + 1 < len(lines) and "ms" in lines[i + 1]:
            nums = re.findall(r"([\d.]+)ms", lines[i + 1])
            if len(nums) >= 4:
                out["max"] = float(nums[3])
                break
    return out

calc = {
    "RPS": "rewrk --json requests_avg; median of 5 reps",
    "CPU_avg": "mean of docker stats CPUPerc samples during each measure window; median across reps",
    "req_per_core": "RPS / (CPU_avg_percent/100)",
    "core_ms_per_req": "1000 / req_per_core (= CPU_COST_PER_SUCCESSFUL_REQUEST)",
    "P99": "rewrk --pct after series",
    "RSS": "docker stats MemUsage first field converted to MiB",
}
rows = {}
for tag in tags:
    rps_list, cpu_avgs, cpu_p95s, cpu_maxs, rss_avgs, rss_peaks = [], [], [], [], [], []
    for rep in range(1, 6):
        j = json.loads((ev / "runs" / f"{tag}-rep{rep}.json").read_text())
        rps = float(j.get("requests_avg") or 0)
        rps_list.append(rps)
        cpus, rss = parse_cpu_mem(ev / "stats" / f"{tag}-rep{rep}.cpu.txt")
        if cpus:
            cpu_avgs.append(statistics.mean(cpus))
            cpu_p95s.append(sorted(cpus)[max(0, int(math.ceil(0.95 * len(cpus)) - 1))])
            cpu_maxs.append(max(cpus))
        if rss:
            rss_avgs.append(statistics.mean(rss))
            rss_peaks.append(max(rss))
    cpu = median(cpu_avgs)
    rps = median(rps_list)
    cores = (cpu / 100.0) if cpu else None
    rpc = (rps / cores) if cores and cores > 0 else None
    cms = (1000.0 / rpc) if rpc else None
    pct = parse_pct(ev / "pct" / f"{tag}.txt")
    rows[tag] = {
        "rps_runs": rps_list,
        "rps_median": rps,
        "rps_cv": cv(rps_list),
        "cpu_avg": cpu,
        "cpu_p95": median(cpu_p95s),
        "cpu_max": median(cpu_maxs),
        "req_per_core": rpc,
        "core_ms_per_req": cms,
        "rss_avg_mib": median(rss_avgs),
        "rss_peak_mib": median(rss_peaks),
        **pct,
    }
    print(f"=== {tag} ===")
    for k, v in rows[tag].items():
        print(f"{k}={v}")

ex, ng, ol = rows["exyonq"], rows["nginx"], rows["ols"]

def pct_delta(a, b):
    return None if a is None or b is None or b == 0 else 100.0 * (a - b) / b

vs_ng = pct_delta(ex["core_ms_per_req"], ng["core_ms_per_req"])
vs_ol = pct_delta(ex["core_ms_per_req"], ol["core_ms_per_req"])
print(f"EXYONQ_CPU_COST_VS_NGINX_PERCENT={vs_ng}")
print(f"EXYONQ_CPU_COST_VS_OLS_PERCENT={vs_ol}")
(ev / "reports" / "threeway_summary.json").write_text(
    json.dumps({"rows": rows, "vs_nginx_pct": vs_ng, "vs_ols_pct": vs_ol, "calc": calc}, indent=2)
)
print(f"EV={ev}")
PY

log "DONE EV=$EV"
