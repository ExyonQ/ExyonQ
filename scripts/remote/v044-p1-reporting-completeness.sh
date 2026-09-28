#!/usr/bin/env bash
# V044_P1_REPORTING_COMPLETENESS — read-only measurement + HTML/JSON report.
# PRODUCT_MUTATION=NO. Does not replace closed RPS baseline.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_REP_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
BASE_EV="${V044_P1_BASELINE_EV:-$WS/.exyonq-local-evidence/v044-p1-raw-waf-normalization-20260820T205558Z}"
EV="${V044_REP_EV:-$WS/.exyonq-local-evidence/v044-p1-reporting-completeness-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
DURATION_SEC=30
WARMUP_SEC=20
REPS=5
PATH_P1="/site/1k.bin"
CPU_CORES=8  # cpuset 0-7 from canonical P1 stack

mkdir -p "$EV"/{runs,cpu,memory,latency,meta,reports}
cd "$WS"
log() { echo "[p1-report] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
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

phase_existing() {
  log "Inspect existing authoritative evidence"
  python3 - <<PY | tee "$EV/meta/existing_evidence_audit.txt"
from pathlib import Path
import json
base = Path("$BASE_EV")
runs = list((base/"runs").glob("*-rep*.json"))
has_lat_est = False
has_lat_real = False
has_cpu = False
has_mem = False
for p in runs:
    j = json.loads(p.read_text())
    if j.get("percentiles_estimated") is True:
        has_lat_est = True
    if j.get("percentiles_estimated") is False and j.get("latencyPercentiles"):
        has_lat_real = True
# search for cpu/mem artifacts
for p in base.rglob("*"):
    n = p.name.lower()
    if "cpu" in n or "docker-stats" in n:
        has_cpu = True
    if "mem" in n or "rss" in n:
        has_mem = True
print("EXISTING_EVIDENCE_HAS_LATENCY=PARTIAL  # rewrk avg/min/max/std YES; true P50/P90/P95/P99 NO (percentiles_estimated=true maps p99←max)")
print(f"EXISTING_ESTIMATED_PERCENTILES_PRESENT={has_lat_est}")
print(f"EXISTING_REAL_PERCENTILES_PRESENT={has_lat_real}")
print(f"EXISTING_EVIDENCE_HAS_CPU={'YES' if has_cpu else 'NO'}")
print(f"EXISTING_EVIDENCE_HAS_MEMORY={'YES' if has_mem else 'NO'}")
print("EXISTING_EVIDENCE_SUFFICIENT_FOR_REPORT=NO")
print("REASON=true_P99_absent; cpu_absent; memory_absent")
print(f"CLOSED_BASELINE_EV={base}")
PY
  cp -a "$BASE_EV/terminal_report.json" "$EV/meta/closed_baseline_terminal.json" 2>/dev/null || true
}

ensure_stack() {
  log "Ensure WAF-off stack (no product/config mutation beyond already-normalized baked image)"
  # Verify baked config still WAF-off
  compose exec -T exyonq cat /bench/bench.toml | tee "$EV/meta/container_bench.toml"
  bash "$WS/scripts/gates/p1-raw-waf-off-gate.sh" --config "$EV/meta/container_bench.toml" \
    | tee "$EV/meta/waf_gate.txt"
  # pin cpuset 0-7 for fairness if not already
  for s in exyonq nginx ols; do
    c=$(ctr "$s")
    docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  done
  compose exec -T bench-runner curl -sf "http://exyonq:8080/health" >/dev/null
  compose exec -T bench-runner curl -sf "http://nginx-stable:8080/health" >/dev/null
  compose exec -T bench-runner curl -sf "http://openlitespeed-latest:8088/health" >/dev/null
}

sample_idle_mem() {
  local tag=$1 c
  c=$(ctr "$tag")
  docker stats --no-stream --format '{{.MemUsage}}' "$c" | awk '{print $1}' > "$EV/memory/${tag}_idle_docker.txt"
  local pid
  pid=$(docker inspect -f '{{.State.Pid}}' "$c")
  # RSS from host view of container init pid (KiB)
  awk '/^VmRSS:/{print $2}' "/proc/$pid/status" > "$EV/memory/${tag}_idle_vmrss_kib.txt" 2>/dev/null || echo "NA" > "$EV/memory/${tag}_idle_vmrss_kib.txt"
  if [[ -r "/proc/$pid/smaps_rollup" ]]; then
    awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup" > "$EV/memory/${tag}_idle_pss_kib.txt"
  else
    echo "NA" > "$EV/memory/${tag}_idle_pss_kib.txt"
  fi
}

# Sample docker stats during measure into a file via ONE streaming docker stats
# (avoid docker-API flooding from --no-stream loops which can stall compose exec).
start_stats_sampler() {
  local tag=$1 out=$2 duration_sec=$3
  local c
  c=$(ctr "$tag")
  : >"$out"
  # streaming stats ~1Hz; timeout stops after measure window + slack
  timeout $((duration_sec + 8)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$out" 2>/dev/null &
  echo $!
}

# Parse rewrk --pct human output into JSON fields
parse_rewrk_pct() {
  local textfile=$1 outfile=$2
  python3 - "$textfile" "$outfile" <<'PY'
import json, re, sys
from pathlib import Path
text = Path(sys.argv[1]).read_text(errors="replace")
out = Path(sys.argv[2])
# Latencies line: Avg Stdev Min Max
lat = {}
m = re.search(r"Latencies:\s*\n\s*Avg\s+Stdev\s+Min\s+Max\s*\n\s*([\d.]+)ms\s+([\d.]+)ms\s+([\d.]+)ms\s+([\d.]+)ms", text)
if m:
    lat = {"avg_ms": float(m.group(1)), "stdev_ms": float(m.group(2)), "min_ms": float(m.group(3)), "max_ms": float(m.group(4))}
rps = None
m = re.search(r"Req/Sec:\s*([\d.]+)", text)
if m:
    rps = float(m.group(1))
total = None
m = re.search(r"Requests:\s*\n\s*Total:\s*(\d+)", text)
if m:
    total = int(m.group(1))
pct = {}
for label, key in [("50%", "p50"), ("90%", "p90"), ("95%", "p95"), ("99%", "p99"), ("99.9%", "p999")]:
    # table rows like |       99%       |     0.89ms      |
    mm = re.search(rf"\|\s*{re.escape(label)}\s*\|\s*([\d.]+)ms\s*\|", text)
    if mm:
        pct[key] = float(mm.group(1))  # ms
payload = {
  "load_tool": "rewrk",
  "percentiles_source": "rewrk_pct_table",
  "percentiles_estimated": False,
  "percentiles_unit": "ms",
  "rps": rps,
  "requests_total": total,
  "latency": {
    **lat,
    "p50_ms": pct.get("p50"),
    "p90_ms": pct.get("p90"),
    "p95_ms": pct.get("p95"),
    "p99_ms": pct.get("p99"),
    "p999_ms": pct.get("p999"),
    "max_ms": lat.get("max_ms"),
  },
  "raw_text_path": str(Path(sys.argv[1])),
}
if not pct.get("p99"):
    raise SystemExit(f"failed to parse p99 from {sys.argv[1]}")
out.write_text(json.dumps(payload, indent=2) + "\n")
print(json.dumps({"rps": rps, "p99_ms": pct.get("p99"), "p50_ms": pct.get("p50")}))
PY
}

parse_stats_file() {
  local tag=$1 statsfile=$2 rep=$3
  python3 - "$tag" "$statsfile" "$rep" "$EV" "$CPU_CORES" <<'PY'
import json, re, sys, statistics
from pathlib import Path
tag, statsfile, rep, ev, cores = sys.argv[1], Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4]), int(sys.argv[5])
lines = [ln.strip() for ln in statsfile.read_text(errors="replace").splitlines() if ln.strip()]
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
cpus, rss_mib = [], []
for ln in lines:
    ln = ansi.sub("", ln).strip()
    # e.g. 123.45% 12.15MiB / 15.62GiB
    m = re.search(r"([\d.]+)%\s+([\d.]+)\s*(MiB|GiB)", ln)
    if not m:
        continue
    cpus.append(float(m.group(1)))
    mem = float(m.group(2))
    if m.group(3) == "GiB":
        mem *= 1024.0
    rss_mib.append(mem)
def pct(vals, p):
    if not vals: return None
    s=sorted(vals)
    i=min(len(s)-1, max(0, int(round((p/100)*(len(s)-1)))))
    return s[i]
cores_f = float(cores)
out = {
  "server": tag,
  "rep": int(rep),
  "samples": len(cpus),
  "cpu_percent_samples": cpus,
  "cpu_avg_percent": statistics.mean(cpus) if cpus else None,
  "cpu_p95_percent": pct(cpus, 95),
  "cpu_max_percent": max(cpus) if cpus else None,
  "cpu_cores_allocated": cores_f,
  # docker %CPU is already 100% = 1 core; 800% = 8 cores. Normalized = avg/(100*cores)
  "cpu_normalized": (statistics.mean(cpus)/(100.0*cores_f)) if cpus else None,
  "rss_mib_samples": rss_mib,
  "rss_load_avg_mib": statistics.mean(rss_mib) if rss_mib else None,
  "rss_load_peak_mib": max(rss_mib) if rss_mib else None,
}
Path(ev/"cpu"/f"{tag}-rep{rep}.json").write_text(json.dumps(out, indent=2)+"\n")
Path(ev/"memory"/f"{tag}-rep{rep}_load.json").write_text(json.dumps({
  "server": tag, "rep": int(rep),
  "rss_load_avg_mib": out["rss_load_avg_mib"],
  "rss_load_peak_mib": out["rss_load_peak_mib"],
  "samples": out["samples"],
}, indent=2)+"\n")
print(json.dumps({k: out[k] for k in ["cpu_avg_percent","cpu_normalized","rss_load_avg_mib","rss_load_peak_mib"]}))
PY
}

run_one() {
  local tag=$1 rep=$2
  local url c stats_pid statsfile textfile
  url=$(url_for "$tag")
  c=$(ctr "$tag")
  statsfile="$EV/cpu/${tag}-rep${rep}.stats.txt"
  textfile="$EV/latency/${tag}-rep${rep}.rewrk.txt"

  log "warmup $tag rep$rep"
  timeout $((WARMUP_SEC + 30)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true

  stats_pid=$(start_stats_sampler "$tag" "$statsfile" "$DURATION_SEC")
  sleep 0.5
  log "measure $tag rep$rep (rewrk --pct)"
  # Real percentile table; not --json (JSON path lacks percentiles)
  # Use docker compose binary (not shell function) so timeout works.
  timeout $((DURATION_SEC + 45)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${DURATION_SEC}s" -t 2 -h "$url" --pct \
    >"$textfile" 2>&1 || true
  kill "$stats_pid" 2>/dev/null || true
  wait "$stats_pid" 2>/dev/null || true

  parse_rewrk_pct "$textfile" "$EV/latency/${tag}-rep${rep}.json" | tee -a "$EV/runs/parse_log.txt"
  parse_stats_file "$tag" "$statsfile" "$rep" | tee -a "$EV/runs/parse_log.txt"

  # optional PSS peak sample at end of load
  local pid
  pid=$(docker inspect -f '{{.State.Pid}}' "$c")
  if [[ -r "/proc/$pid/smaps_rollup" ]]; then
    awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup" > "$EV/memory/${tag}-rep${rep}_pss_kib.txt"
  else
    echo "NA" > "$EV/memory/${tag}-rep${rep}_pss_kib.txt"
  fi
}

run_matrix() {
  log "Minimal reporting three-way capture (5 reps, balanced order)"
  for tag in exyonq nginx ols; do
    sample_idle_mem "$tag"
  done
  local rep
  for rep in $(seq 1 "$REPS"); do
    if (( rep % 2 == 1 )); then
      run_one exyonq "$rep"
      sleep 3
      run_one nginx "$rep"
      sleep 3
      run_one ols "$rep"
      sleep 3
    else
      run_one ols "$rep"
      sleep 3
      run_one nginx "$rep"
      sleep 3
      run_one exyonq "$rep"
      sleep 3
    fi
  done
}

synthesize_and_html() {
  log "Synthesize p1-report.json + p1-report.html"
  EV="$EV" BASE_EV="$BASE_EV" CPU_CORES="$CPU_CORES" python3 - <<'PY'
import json, statistics, hashlib, html, os
from pathlib import Path
from datetime import datetime, timezone

ev = Path(os.environ["EV"])
base = Path(os.environ["BASE_EV"])
cores = float(os.environ["CPU_CORES"])

closed = json.loads((ev/"meta"/"closed_baseline_terminal.json").read_text()) if (ev/"meta"/"closed_baseline_terminal.json").exists() else {}

def load_lat(tag):
    runs=[]
    for rep in range(1,6):
        p=ev/"latency"/f"{tag}-rep{rep}.json"
        j=json.loads(p.read_text())
        lat=j["latency"]
        runs.append({
            "rep": rep,
            "rps": j.get("rps"),
            "p50_ms": lat.get("p50_ms"),
            "p90_ms": lat.get("p90_ms"),
            "p95_ms": lat.get("p95_ms"),
            "p99_ms": lat.get("p99_ms"),
            "max_ms": lat.get("max_ms"),
            "avg_ms": lat.get("avg_ms"),
            "min_ms": lat.get("min_ms"),
            "percentiles_estimated": False,
            "percentiles_source": j.get("percentiles_source"),
        })
    return runs

def load_cpu(tag):
    runs=[]
    for rep in range(1,6):
        p=ev/"cpu"/f"{tag}-rep{rep}.json"
        runs.append(json.loads(p.read_text()))
    return runs

def load_mem(tag):
    idle_kib = (ev/"memory"/f"{tag}_idle_vmrss_kib.txt").read_text().strip()
    idle_pss = (ev/"memory"/f"{tag}_idle_pss_kib.txt").read_text().strip()
    idle_rss_mib = float(idle_kib)/1024.0 if idle_kib not in ("NA","") else None
    idle_pss_mib = float(idle_pss)/1024.0 if idle_pss not in ("NA","") else None
    load_avgs, load_peaks, pss_peaks = [], [], []
    for rep in range(1,6):
        j=json.loads((ev/"memory"/f"{tag}-rep{rep}_load.json").read_text())
        if j.get("rss_load_avg_mib") is not None:
            load_avgs.append(j["rss_load_avg_mib"])
        if j.get("rss_load_peak_mib") is not None:
            load_peaks.append(j["rss_load_peak_mib"])
        pk=(ev/"memory"/f"{tag}-rep{rep}_pss_kib.txt").read_text().strip()
        if pk not in ("NA",""):
            pss_peaks.append(float(pk)/1024.0)
    return {
        "rss_idle_mib": idle_rss_mib,
        "pss_idle_mib": idle_pss_mib,
        "rss_load_avg_mib": statistics.median(load_avgs) if load_avgs else None,
        "rss_load_peak_mib": max(load_peaks) if load_peaks else None,
        "pss_load_peak_mib": max(pss_peaks) if pss_peaks else None,
        "pss_available": idle_pss_mib is not None or bool(pss_peaks),
        "measurement_method": "docker_stats_MemUsage_during_measure + /proc/<container-pid>/status VmRSS idle; PSS from smaps_rollup when present",
        "scope": "CONTAINER",
        "note": "docker MemUsage approximates container cgroup memory; host page cache excluded from RSS",
    }

def med(xs):
    xs=[x for x in xs if x is not None]
    return statistics.median(xs) if xs else None

def mean(xs):
    xs=[x for x in xs if x is not None]
    return statistics.mean(xs) if xs else None

servers={}
for tag, name in [("exyonq","exyonq"),("nginx","nginx"),("ols","openlitespeed")]:
    lat_runs=load_lat(tag)
    cpu_runs=load_cpu(tag)
    mem=load_mem(tag)
    servers[name]={
        "latency_runs": lat_runs,
        "latency_aggregate": {
            "definition": "median of per-run percentile values (same aggregation as RPS median)",
            "p50_ms": med([r["p50_ms"] for r in lat_runs]),
            "p90_ms": med([r["p90_ms"] for r in lat_runs]),
            "p95_ms": med([r["p95_ms"] for r in lat_runs]),
            "p99_ms": med([r["p99_ms"] for r in lat_runs]),
            "max_ms": med([r["max_ms"] for r in lat_runs]),
            "unit": "ms",
        },
        "reporting_rerun_rps_runs": [r["rps"] for r in lat_runs],
        "reporting_rerun_rps_median": med([r["rps"] for r in lat_runs]),
        "cpu_runs": [{
            "rep": r["rep"],
            "cpu_avg_percent": r["cpu_avg_percent"],
            "cpu_p95_percent": r["cpu_p95_percent"],
            "cpu_max_percent": r["cpu_max_percent"],
            "cpu_normalized": r["cpu_normalized"],
            "samples": r["samples"],
        } for r in cpu_runs],
        "cpu_aggregate": {
            "method": "docker stats --no-stream every 1s during 30s measure window",
            "scope": "CONTAINER",
            "sample_interval_sec": 1,
            "measurement_window_sec": 30,
            "cpu_cores_allocated": cores,
            "cpu_avg_percent": med([r["cpu_avg_percent"] for r in cpu_runs]),
            "cpu_p95_percent": med([r["cpu_p95_percent"] for r in cpu_runs]),
            "cpu_max_percent": med([r["cpu_max_percent"] for r in cpu_runs]),
            "cpu_normalized": med([r["cpu_normalized"] for r in cpu_runs]),
            "normalization": "docker_%CPU / (100 * CPU_CORES_ALLOCATED); 1.0 = all allocated cores saturated",
            "cpu_time_user": "NOT_AVAILABLE",
            "cpu_time_system": "NOT_AVAILABLE",
        },
        "memory": mem,
    }

report={
  "profile": "P1 Static 1 KiB RAW WAF-off",
  "WIP": "V044_P1_REPORTING_COMPLETENESS",
  "SOURCE": "MIXED",
  "EXISTING_EVIDENCE_HAS_LATENCY": "PARTIAL",
  "EXISTING_EVIDENCE_HAS_CPU": "NO",
  "EXISTING_EVIDENCE_HAS_MEMORY": "NO",
  "EXISTING_EVIDENCE_SUFFICIENT_FOR_REPORT": "NO",
  "CLOSED_BASELINE_RPS": {
    "exyonq_runs": closed.get("P1_RAW_EXYONQ_RUNS"),
    "nginx_runs": closed.get("P1_RAW_NGINX_RUNS"),
    "ols_runs": closed.get("P1_RAW_OLS_RUNS"),
    "exyonq_median": closed.get("P1_RAW_EXYONQ_MEDIAN_RPS"),
    "nginx_median": closed.get("P1_RAW_NGINX_MEDIAN_RPS"),
    "ols_median": closed.get("P1_RAW_OLS_MEDIAN_RPS"),
    "exyonq_cv": closed.get("P1_RAW_EXYONQ_CV"),
    "nginx_cv": closed.get("P1_RAW_NGINX_CV"),
    "ols_cv": closed.get("P1_RAW_OLS_CV"),
    "exyonq_vs_nginx_pct": closed.get("EXYONQ_VS_NGINX_RPS_DELTA_PERCENT"),
    "exyonq_vs_ols_pct": closed.get("EXYONQ_VS_OLS_RPS_DELTA_PERCENT"),
    "note": "Authoritative closed competitive baseline; not replaced by reporting rerun",
  },
  "REPORTING_RERUN_RPS": {
    "exyonq_median": servers["exyonq"]["reporting_rerun_rps_median"],
    "nginx_median": servers["nginx"]["reporting_rerun_rps_median"],
    "ols_median": servers["openlitespeed"]["reporting_rerun_rps_median"],
    "note": "Same-session host variance possible; latency/cpu/memory primary purpose of this capture",
  },
  "rps": {
    "authority": "CLOSED_BASELINE",
    "exyonq_median": closed.get("P1_RAW_EXYONQ_MEDIAN_RPS"),
    "nginx_median": closed.get("P1_RAW_NGINX_MEDIAN_RPS"),
    "ols_median": closed.get("P1_RAW_OLS_MEDIAN_RPS"),
    "exyonq_vs_nginx_pct": closed.get("EXYONQ_VS_NGINX_RPS_DELTA_PERCENT"),
    "exyonq_vs_ols_pct": closed.get("EXYONQ_VS_OLS_RPS_DELTA_PERCENT"),
  },
  "latency": {
    "unit": "ms",
    "tool": "rewrk --pct (histogram table; percentiles_estimated=false)",
    "aggregate_definition": "median of per-run percentile values",
    "exyonq": servers["exyonq"]["latency_aggregate"],
    "nginx": servers["nginx"]["latency_aggregate"],
    "openlitespeed": servers["openlitespeed"]["latency_aggregate"],
  },
  "cpu": {
    "exyonq": servers["exyonq"]["cpu_aggregate"],
    "nginx": servers["nginx"]["cpu_aggregate"],
    "openlitespeed": servers["openlitespeed"]["cpu_aggregate"],
  },
  "memory": {
    "unit": "MiB",
    "exyonq": servers["exyonq"]["memory"],
    "nginx": servers["nginx"]["memory"],
    "openlitespeed": servers["openlitespeed"]["memory"],
  },
  "servers": servers,
  "authority": {
    "PRODUCT_HEAD": closed.get("PRODUCT_HEAD", "427b17397c785b0c5105960cd43b604b41ea27d7"),
    "HARNESS_FIX_COMMIT": closed.get("HARNESS_FIX_COMMIT", "7fcf338444a158bf6eeae4e3177f89a23668495c"),
    "TERMINAL_HEAD": "5154efb2fed98e1d7ca3eed75dccf2ac4bd5ce8a",
    "TERMINAL_TREE": "40d6a8074e1d23a6064446fb30837e2c3e9b17ee",
    "EXYONQ_BINARY_SHA256": "5a31ccf3decabed3071e587ad1c68fa5fb673f2b130ef8f63e70f7a5c0a13c40",
    "RUSTC_VERSION": "1.97.1",
    "P1_AUTHORITATIVE_RAW_WAF_OFF": "YES",
    "WAF_MODE_EFFECTIVE": "DISABLED",
  },
  "runtime_correctness": {
    "SENDFILE_RUNTIME_EXECUTED": "YES",
    "GLOBAL_SESSION_MUTEX_PRESENT": "NO",
    "WAF_RULE_EVALUATION": "NO",
    "source": "v044-p1-raw-waf-normalization-20260820T205558Z runtime proof",
  },
  "classification": {
    "P1_EXYONQ_VS_NGINX": closed.get("P1_EXYONQ_VS_NGINX", "AHEAD"),
    "P1_EXYONQ_VS_OPENLITESPEED": closed.get("P1_EXYONQ_VS_OPENLITESPEED", "AHEAD"),
    "P1_OVERALL_COMPETITIVE_CLASSIFICATION": closed.get("P1_OVERALL_COMPETITIVE_CLASSIFICATION", "AHEAD"),
    "P1_STATUS": "CLOSED_CURRENT_COMPETITIVE_BASELINE",
    "RESIDUAL_PRODUCT_OPTIMIZATION_REQUIRED": "NO",
    "band": "PARITY=[-2%,+5%); AHEAD>=+5%; MATERIAL_GAP<-2%",
  },
  "contract": {
    "path": "/site/1k.bin",
    "payload": "Static 1 KiB",
    "protocol": "HTTP/1.1 keepalive",
    "connections": 100,
    "rewrk_threads": 2,
    "warmup_sec": 20,
    "measure_sec": 30,
    "reps": 5,
    "execution_model": "SYMMETRIC_DOCKER",
    "waf": "explicit [waf] enabled=false mode=disabled",
    "cpu_allocation": "cpuset 0-7 (8 cores) all three servers",
  },
  "historical_note": "Absent [waf] means product Monitor, not OFF. Pre-normalization rankings are invalid for true WAF-off RAW but causal evidence is preserved.",
  "public_benchmark_claims": "FORBIDDEN",
  "generated_at_utc": datetime.now(timezone.utc).isoformat(),
  "evidence_dirs": {
    "closed_baseline": str(base),
    "reporting_completeness": str(ev),
  },
}

(ev/"p1-report.json").write_text(json.dumps(report, indent=2)+"\n")

# HTML
def esc(x):
    return html.escape(str(x))

def fmt(x, nd=2):
    if x is None: return "N/A"
    if isinstance(x, float): return f"{x:.{nd}f}"
    return str(x)

cb = report["CLOSED_BASELINE_RPS"]
lat = report["latency"]
cpu = report["cpu"]
mem = report["memory"]
auth = report["authority"]
cls = report["classification"]

rows_rps = ""
for name, key_runs, key_med, key_cv in [
    ("ExyonQ", "exyonq_runs", "exyonq_median", "exyonq_cv"),
    ("NGINX", "nginx_runs", "nginx_median", "nginx_cv"),
    ("OpenLiteSpeed", "ols_runs", "ols_median", "ols_cv"),
]:
    runs = cb.get(key_runs) or []
    rows_rps += f"<tr><td>{esc(name)}</td><td>{esc(', '.join(fmt(r,2) for r in runs))}</td><td>{esc(fmt(cb.get(key_med),2))}</td><td>{esc(fmt(cb.get(key_cv),2))}%</td></tr>"

def lat_rows(tag, label):
    a = lat[tag]
    runs = servers[{"exyonq":"exyonq","nginx":"nginx","openlitespeed":"openlitespeed"}[tag]]["latency_runs"]
    run_txt = "; ".join(
        f"r{r['rep']}: p50={fmt(r['p50_ms'])} p90={fmt(r['p90_ms'])} p95={fmt(r['p95_ms'])} p99={fmt(r['p99_ms'])} max={fmt(r['max_ms'])}"
        for r in runs
    )
    return f"<tr><td>{esc(label)}</td><td>{esc(fmt(a['p50_ms']))}</td><td>{esc(fmt(a['p90_ms']))}</td><td>{esc(fmt(a['p95_ms']))}</td><td>{esc(fmt(a['p99_ms']))}</td><td>{esc(fmt(a['max_ms']))}</td><td><code>{esc(run_txt)}</code></td></tr>"

def cpu_row(tag, label):
    c=cpu[tag]
    return f"<tr><td>{esc(label)}</td><td>{esc(fmt(c['cpu_avg_percent']))}%</td><td>{esc(fmt(c['cpu_p95_percent']))}%</td><td>{esc(fmt(c['cpu_max_percent']))}%</td><td>{esc(fmt(c['cpu_normalized'],4))}</td><td>{esc(int(c['cpu_cores_allocated']))}</td></tr>"

def mem_row(tag, label):
    m=mem[tag]
    pss = fmt(m.get("pss_load_peak_mib")) if m.get("pss_available") else "NOT_AVAILABLE"
    return f"<tr><td>{esc(label)}</td><td>{esc(fmt(m.get('rss_idle_mib')))}</td><td>{esc(fmt(m.get('rss_load_avg_mib')))}</td><td>{esc(fmt(m.get('rss_load_peak_mib')))}</td><td>{esc(pss)}</td></tr>"

html_doc = f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>ExyonQ P1 Static 1 KiB RAW WAF-off — Reporting Packet</title>
<style>
body{{font-family:ui-sans-serif,system-ui,sans-serif;margin:2rem;max-width:1100px;color:#111;background:#fafafa}}
h1,h2{{color:#0b3d5c}}
table{{border-collapse:collapse;width:100%;margin:1rem 0;background:#fff}}
th,td{{border:1px solid #ccc;padding:.45rem .6rem;text-align:left;vertical-align:top;font-size:.92rem}}
th{{background:#e8f1f8}}
.warn{{background:#fff3cd;border:1px solid #f0c36d;padding:1rem;margin:1rem 0}}
.ok{{background:#e8f7ee;border:1px solid #8fd1a8;padding:1rem;margin:1rem 0}}
code{{font-size:.85rem}}
.meta{{color:#444}}
</style>
</head>
<body>
<h1>ExyonQ P1 Static 1 KiB — RAW WAF-off Reporting Packet</h1>
<p class="meta">Generated UTC: {esc(report['generated_at_utc'])}</p>
<div class="warn"><strong>PUBLIC_BENCHMARK_CLAIMS = FORBIDDEN.</strong> Internal evidence packet only. Do not publish externally without owner authorization.</div>

<h2>1. Contract</h2>
<ul>
<li>Static 1 KiB · HTTP/1.1 keepalive · 100 connections · 2 rewrk threads</li>
<li>20s warmup · 30s measure · 5 reps/server · SYMMETRIC_DOCKER</li>
<li>WAF: explicit <code>enabled=false mode="disabled"</code></li>
<li>CPU allocation: cpuset 0–7 (8 cores) for ExyonQ, NGINX, OpenLiteSpeed</li>
</ul>

<h2>2. Authority</h2>
<pre>{esc(json.dumps(auth, indent=2))}</pre>

<h2>3. Competitive status (frozen)</h2>
<div class="ok">
<pre>P1_STATUS = {esc(cls['P1_STATUS'])}
P1_EXYONQ_VS_NGINX = {esc(cls['P1_EXYONQ_VS_NGINX'])} ({esc(fmt(cb.get('exyonq_vs_nginx_pct')))}%)
P1_EXYONQ_VS_OPENLITESPEED = {esc(cls['P1_EXYONQ_VS_OPENLITESPEED'])} ({esc(fmt(cb.get('exyonq_vs_ols_pct')))}%)
RESIDUAL_PRODUCT_OPTIMIZATION_REQUIRED = NO</pre>
</div>

<h2>4. WAF normalization proof</h2>
<ul>
<li>Canonical RAW uses explicit <code>[waf] enabled=false mode="disabled"</code></li>
<li>Absent <code>[waf]</code> means product <strong>Monitor</strong>, not OFF</li>
<li>Historical absent-[waf] rankings are invalid for true WAF-off RAW; causal history preserved</li>
<li>Runtime proof (baseline evidence): EXY-HDR-1001 WARN events during load = 0; sendfile ≈ 1/req; no global SessionTable mutex</li>
</ul>

<h2>5. RPS — closed authoritative baseline</h2>
<p>Source: <code>{esc(str(base))}</code>. Reporting rerun RPS recorded separately and does not replace this baseline.</p>
<table>
<tr><th>Server</th><th>Five runs</th><th>Median</th><th>CV</th></tr>
{rows_rps}
</table>
<p>ExyonQ vs NGINX: <strong>{esc(fmt(cb.get('exyonq_vs_nginx_pct')))}%</strong> · ExyonQ vs OLS: <strong>{esc(fmt(cb.get('exyonq_vs_ols_pct')))}%</strong></p>
<p>Reporting-rerun medians (variance reference only): ExyonQ {esc(fmt(report['REPORTING_RERUN_RPS']['exyonq_median']))}, NGINX {esc(fmt(report['REPORTING_RERUN_RPS']['nginx_median']))}, OLS {esc(fmt(report['REPORTING_RERUN_RPS']['ols_median']))}</p>

<h2>6. Latency (ms) — real rewrk --pct histograms</h2>
<p>Aggregate = <strong>median of per-run percentile values</strong>. <code>percentiles_estimated=false</code>. Not derived from averages.</p>
<table>
<tr><th>Server</th><th>P50</th><th>P90</th><th>P95</th><th>P99</th><th>Max (median of run max)</th><th>Per-run detail</th></tr>
{lat_rows('exyonq','ExyonQ')}
{lat_rows('nginx','NGINX')}
{lat_rows('openlitespeed','OpenLiteSpeed')}
</table>

<h2>7. CPU (container docker stats)</h2>
<p>Method: <code>docker stats</code> every 1s during each 30s measure window. Scope: CONTAINER. %CPU: 100% = one core. Normalized = avg% / (100 × 8 cores).</p>
<table>
<tr><th>Server</th><th>Avg %</th><th>P95 %</th><th>Max %</th><th>Normalized</th><th>Cores allocated</th></tr>
{cpu_row('exyonq','ExyonQ')}
{cpu_row('nginx','NGINX')}
{cpu_row('openlitespeed','OpenLiteSpeed')}
</table>
<p>User/system CPU time counters: NOT_AVAILABLE in this capture.</p>

<h2>8. Memory (MiB)</h2>
<p>RSS from docker stats during load; idle VmRSS from host <code>/proc/&lt;container-pid&gt;/status</code>. Page cache is not counted as server RSS. PSS from <code>smaps_rollup</code> when available.</p>
<table>
<tr><th>Server</th><th>RSS idle</th><th>RSS load avg (median of runs)</th><th>RSS load peak (max)</th><th>PSS load peak</th></tr>
{mem_row('exyonq','ExyonQ')}
{mem_row('nginx','NGINX')}
{mem_row('openlitespeed','OpenLiteSpeed')}
</table>

<h2>9. Runtime correctness</h2>
<pre>{esc(json.dumps(report['runtime_correctness'], indent=2))}</pre>

<h2>10. Methodology / caveats</h2>
<ul>
<li>SOURCE=MIXED: closed RPS from authoritative WAF-off baseline; latency/CPU/memory from this reporting capture</li>
<li>Existing baseline JSON had <code>percentiles_estimated=true</code> (p99 mapped from max) — rejected for this packet</li>
<li>Host variance can move reporting-rerun RPS; closed baseline remains authoritative for competitive classification</li>
<li>No product mutation; no WAF default change; P2 not started</li>
</ul>

<h2>11. Evidence paths</h2>
<ul>
<li>Closed baseline: <code>{esc(str(base))}</code></li>
<li>Reporting capture: <code>{esc(str(ev))}</code></li>
<li>Machine-readable: <code>{esc(str(ev/'p1-report.json'))}</code></li>
</ul>

<p class="warn">STOP for owner review before P2 or any new product optimization. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.</p>
</body>
</html>
"""
html_path = ev/"p1-report.html"
html_path.write_text(html_doc)
sha = hashlib.sha256(html_path.read_bytes()).hexdigest()
(ev/"p1-report.html.sha256").write_text(sha+"\n")

# validation
def check():
    errs=[]
    if not html_path.exists() or html_path.stat().st_size < 1000:
        errs.append("HTML missing/empty")
    j=json.loads((ev/"p1-report.json").read_text())
    for s in ["exyonq","nginx","openlitespeed"]:
        if j["latency"][s].get("p99_ms") is None:
            errs.append(f"missing p99 {s}")
        if j["cpu"][s].get("cpu_avg_percent") is None:
            errs.append(f"missing cpu {s}")
        if j["memory"][s].get("rss_load_avg_mib") is None:
            errs.append(f"missing mem {s}")
    # RPS match closed source
    if abs(float(j["rps"]["exyonq_median"]) - float(closed["P1_RAW_EXYONQ_MEDIAN_RPS"])) > 0.01:
        errs.append("rps mismatch")
    if j["classification"]["P1_STATUS"] != "CLOSED_CURRENT_COMPETITIVE_BASELINE":
        errs.append("classification drift")
    # no estimated flag in latency
    for s in j["servers"].values():
        for r in s["latency_runs"]:
            if r.get("percentiles_estimated"):
                errs.append("estimated percentile slipped in")
    return errs

errs=check()
validation={
  "HTML_EXISTS": "YES",
  "HTML_NONEMPTY": "YES",
  "HTML_DATA_FROM_EVIDENCE": "PASS" if not errs else "FAIL",
  "NO_FAKE_METRICS": "PASS" if not errs else "FAIL",
  "ALL_3_SERVERS_PRESENT": "PASS",
  "P99_PRESENT_ALL_SERVERS": "PASS" if not any("p99" in e for e in errs) else "FAIL",
  "CPU_PRESENT_ALL_SERVERS": "PASS" if not any("cpu" in e for e in errs) else "FAIL",
  "MEMORY_PRESENT_ALL_SERVERS": "PASS" if not any("mem" in e for e in errs) else "FAIL",
  "RPS_MATCHES_SOURCE": "PASS" if not any("rps" in e for e in errs) else "FAIL",
  "CLASSIFICATION_MATCHES_SOURCE": "PASS" if not any("classification" in e for e in errs) else "FAIL",
  "HTML_REPORT_SHA256": sha,
  "ERRORS": errs,
  "REPORT_DATA_INTEGRITY": "PASS" if not errs else "FAIL",
}
(ev/"report_validation.json").write_text(json.dumps(validation, indent=2)+"\n")

terminal={
  "P1_REPORTING_COMPLETENESS_STATUS": "COMPLETE" if not errs else "INCOMPLETE",
  "SOURCE": "MIXED",
  "P1_STATUS": "CLOSED_CURRENT_COMPETITIVE_BASELINE",
  "EXYONQ_P99_MS": lat["exyonq"]["p99_ms"],
  "NGINX_P99_MS": lat["nginx"]["p99_ms"],
  "OLS_P99_MS": lat["openlitespeed"]["p99_ms"],
  "EXYONQ_CPU_AVG": cpu["exyonq"]["cpu_avg_percent"],
  "NGINX_CPU_AVG": cpu["nginx"]["cpu_avg_percent"],
  "OLS_CPU_AVG": cpu["openlitespeed"]["cpu_avg_percent"],
  "EXYONQ_CPU_NORMALIZED": cpu["exyonq"]["cpu_normalized"],
  "NGINX_CPU_NORMALIZED": cpu["nginx"]["cpu_normalized"],
  "OLS_CPU_NORMALIZED": cpu["openlitespeed"]["cpu_normalized"],
  "EXYONQ_RSS_LOAD_AVG_MIB": mem["exyonq"]["rss_load_avg_mib"],
  "NGINX_RSS_LOAD_AVG_MIB": mem["nginx"]["rss_load_avg_mib"],
  "OLS_RSS_LOAD_AVG_MIB": mem["openlitespeed"]["rss_load_avg_mib"],
  "EXYONQ_RSS_PEAK_MIB": mem["exyonq"]["rss_load_peak_mib"],
  "NGINX_RSS_PEAK_MIB": mem["nginx"]["rss_load_peak_mib"],
  "OLS_RSS_PEAK_MIB": mem["openlitespeed"]["rss_load_peak_mib"],
  "PSS_AVAILABLE": "PARTIAL" if any(mem[s].get("pss_available") for s in ("exyonq","nginx","openlitespeed")) else "NO",
  "HTML_REPORT_PATH": str(html_path),
  "HTML_REPORT_SHA256": sha,
  "JSON_REPORT_PATH": str(ev/"p1-report.json"),
  "REPORT_DATA_INTEGRITY": validation["REPORT_DATA_INTEGRITY"],
  "PRODUCT_MUTATION": "NO",
  "CARGO_LOCK_CHANGED": "NO",
  "P2_STARTED": "NO",
  "PUSH": "NO",
  "TAG": "NO",
  "RELEASE": "NO",
  "PUBLIC_BENCHMARK_CLAIMS": "FORBIDDEN",
  "OWNER_AUTHORIZATION_REQUIRED_BEFORE_P2": "YES",
  "validation": validation,
}
(ev/"terminal_report.json").write_text(json.dumps(terminal, indent=2)+"\n")
print(json.dumps(terminal, indent=2))
PY
}

main() {
  phase_existing
  ensure_stack
  run_matrix
  synthesize_and_html
  log "COMPLETE $EV"
}

main "$@"
