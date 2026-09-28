#!/usr/bin/env bash
# V044_P1_CPU_EFFICIENCY_REPAIR_ROOT1 — A/B CPU cost/req (ExyonQ AFTER only vs frozen BEFORE from reporting).
# PRODUCT_MUTATION already applied on binary under test. P2=NO.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_ROOT1_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_ROOT1_EV:-$WS/.exyonq-local-evidence/v044-p1-cpu-root1-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P1="/site/1k.bin"
URL="http://exyonq:8080${PATH_P1}"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5
CPU_CORES=8

mkdir -p "$EV"/{runs,perf,meta,reports}
cd "$WS"
log() { echo "[root1-ab] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }
# `timeout` cannot run shell functions — always use docker compose under timeout.
dcompose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }
CTR="${PROJECT}-exyonq-1"

{
  echo "CURRENT_WIP=V044_P1_CPU_EFFICIENCY_REPAIR_ROOT1"
  echo "PRODUCT_MUTATION=YES_WITHIN_ROOT1"
  echo "TIMESTAMP_UTC=$TS"
  echo "BINARY_SHA256=$(docker exec "$CTR" sha256sum /usr/local/bin/exyonq | awk '{print $1}')"
  echo "BEFORE_BINARY_SHA256=5a31ccf3decabed3071e587ad1c68fa5fb673f2b130ef8f63e70f7a5c0a13c40"
  echo "BEFORE_SOURCE=REPORTING_CAPTURE_20260820T223047Z"
} | tee "$EV/meta/authority_bind.txt"

compose exec -T exyonq cat /bench/bench.toml | tee "$EV/meta/bench.toml" >/dev/null
bash "$WS/scripts/gates/p1-raw-waf-off-gate.sh" --config "$EV/meta/bench.toml" | tee "$EV/meta/waf_gate.txt"
docker update --cpuset-cpus "0-7" "$CTR" >/dev/null

# Warmup once, then 5× measure with docker stats + rewrk --pct + perf stat
EX_PID=$(docker inspect -f '{{.State.Pid}}' "$CTR")

for i in $(seq 1 "$REPS"); do
  log "rep $i/$REPS"
  out="$EV/runs/rep${i}"
  : >"${out}.stats.txt"
  timeout $((MEASURE_SEC + 15)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$CTR" >"${out}.stats.txt" 2>/dev/null &
  SP=$!
  sleep 0.4
  timeout $((WARMUP_SEC + 25)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$URL" >/dev/null 2>&1 || true
  # JSON on stdout; human/--pct table on stderr
  timeout $((MEASURE_SEC + 45)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$URL" --pct --json \
    >"${out}.rewrk.json" 2>"${out}.rewrk.txt" || true
  wait $SP 2>/dev/null || true

  # perf stat during a companion measure window (same contract, separate load)
  (
    sleep 1
    timeout $((MEASURE_SEC + 40)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
      rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$URL" --json \
      >"${out}.perf.rewrk.json" 2>/dev/null || true
  ) &
  LP=$!
  timeout $((MEASURE_SEC + 10)) \
    perf stat -e cycles,instructions,task-clock,context-switches \
      -p "$EX_PID" -- sleep "$MEASURE_SEC" \
      >"${out}.perfstat.txt" 2>&1 || true
  wait $LP 2>/dev/null || true
done

EV="$EV" CPU_CORES="$CPU_CORES" python3 - <<'PY'
import json, re, statistics, os
from pathlib import Path

ev = Path(os.environ["EV"])
cores = float(os.environ.get("CPU_CORES", "8"))
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

def parse_pct_table(text):
    # rewrk --pct stderr/stdout table
    out = {}
    for line in text.splitlines():
        m = re.match(r"\s*(50|90|99|99\.9|99\.99)%\s+([\d.]+)", line)
        if m:
            out[f"p{m.group(1).replace('.','_')}"] = float(m.group(2))
    return out

def cpu_avg(path):
    cpus=[]
    for ln in path.read_text(errors="replace").splitlines():
        ln=ansi.sub("", ln).strip()
        m=re.search(r"([\d.]+)%", ln)
        if m: cpus.append(float(m.group(1)))
    return statistics.mean(cpus) if cpus else None, len(cpus)

def perf_parse(path):
    text=path.read_text(errors="replace") if path.exists() else ""
    d={}
    for line in text.splitlines():
        m=re.search(r"^\s*([\d,]+)\s+([a-z0-9\-]+)", line)
        if m:
            d[m.group(2)]=float(m.group(1).replace(",",""))
    m=re.search(r"([\d.]+)\s+msec\s+task-clock", text)
    if m: d["task_clock_msec"]=float(m.group(1))
    return d

reps=[]
for i in range(1,6):
    base=ev/"runs"/f"rep{i}"
    j=json.loads((base.with_suffix(".rewrk.json")).read_text()) if base.with_suffix(".rewrk.json").exists() else {}
    # rewrk may put json in .rewrk.json; pct in .rewrk.txt
    pct=parse_pct_table((base.with_suffix(".rewrk.txt")).read_text(errors="replace") if base.with_suffix(".rewrk.txt").exists() else "")
    if not pct:
        pct=parse_pct_table(json.dumps(j))
    cpu,n=cpu_avg(Path(str(base)+".stats.txt"))
    rps=float(j.get("requests_avg") or 0)
    reqs=int(j.get("requests_total") or 0)
    core_eq=(cpu or 0)/100.0
    req_per_core = rps/core_eq if core_eq>0 else None
    core_ms = (core_eq/rps*1000.0) if rps>0 and core_eq>0 else None
    pf=perf_parse(Path(str(base)+".perfstat.txt"))
    pj=json.loads(Path(str(base)+".perf.rewrk.json").read_text()) if Path(str(base)+".perf.rewrk.json").exists() else {}
    preqs=int(pj.get("requests_total") or 0) or None
    # perf window ≈ measure only (sleep MEASURE_SEC) overlapping load — normalize by companion reqs
    cycles_per= (pf["cycles"]/preqs) if preqs and "cycles" in pf else None
    instr_per= (pf["instructions"]/preqs) if preqs and "instructions" in pf else None
    ipc= (pf["instructions"]/pf["cycles"]) if pf.get("cycles") else None
    reps.append({
        "rps":rps,"reqs":reqs,"cpu_avg":cpu,"cpu_samples":n,
        "req_per_core":req_per_core,"core_ms_per_req":core_ms,
        "p99":pct.get("p99"),"p50":pct.get("p50"),
        "cycles_per_req":cycles_per,"instructions_per_req":instr_per,"IPC":ipc,
        "perf_reqs":preqs,
    })

def med(key):
    vals=[r[key] for r in reps if r.get(key) is not None]
    return statistics.median(vals) if vals else None

after={
    "rps_median":med("rps"),
    "cpu_avg_median":med("cpu_avg"),
    "req_per_core_median":med("req_per_core"),
    "core_ms_per_req_median":med("core_ms_per_req"),
    "p99_median":med("p99"),
    "p50_median":med("p50"),
    "cycles_per_req_median":med("cycles_per_req"),
    "instructions_per_req_median":med("instructions_per_req"),
    "IPC_median":med("IPC"),
    "reps":reps,
}

# BEFORE from reporting capture (authoritative efficiency) + causal cycles corrected
before={
    "rps":163858.57,
    "cpu_avg":413.08,
    "req_per_core":39667.51,
    "core_ms_per_req":0.02520954503630783,
    "p99":1.65,
    "cycles_per_req_corrected":76472,
    "instructions_per_req_corrected":84533,
    "IPC":1.105,
    "SOURCE":"REPORTING_CAPTURE+CAUSAL_CORRECTED_CYCLES",
}

def delta_pct(new, old):
    if new is None or old is None or old==0: return None
    return (new/old - 1.0)*100.0

cpu_cost_delta = delta_pct(after["core_ms_per_req_median"], before["core_ms_per_req"])
report={
  "P1_CPU_EFFICIENCY_REPAIR_ROOT1_STATUS":"COMPLETE",
  "AFTER":after,
  "BEFORE":before,
  "BEFORE_CPU_AVG":before["cpu_avg"],
  "AFTER_CPU_AVG":after["cpu_avg_median"],
  "BEFORE_REQ_PER_CORE":before["req_per_core"],
  "AFTER_REQ_PER_CORE":after["req_per_core_median"],
  "BEFORE_CORE_MS_PER_REQ":before["core_ms_per_req"],
  "AFTER_CORE_MS_PER_REQ":after["core_ms_per_req_median"],
  "CPU_COST_PER_REQ_DELTA_PERCENT":cpu_cost_delta,
  "BEFORE_RPS":before["rps"],
  "AFTER_RPS":after["rps_median"],
  "BEFORE_P99":before["p99"],
  "AFTER_P99":after["p99_median"],
  "CYCLES_PER_REQ_DELTA": None if after["cycles_per_req_median"] is None else after["cycles_per_req_median"]-before["cycles_per_req_corrected"],
  "INSTRUCTIONS_PER_REQ_DELTA": None if after["instructions_per_req_median"] is None else after["instructions_per_req_median"]-before["instructions_per_req_corrected"],
  "IPC_DELTA": None if after["IPC_median"] is None else after["IPC_median"]-before["IPC"],
  "ROOT1_CAUSAL_EFFECT": (
      "PROVEN" if cpu_cost_delta is not None and cpu_cost_delta < -2.0 else
      "PARTIAL" if cpu_cost_delta is not None and cpu_cost_delta < 0 else
      "FALSIFIED_OR_TOO_SMALL"
  ),
  "REAL_PRODUCT_EFFICIENCY_DEFECT":"YES",
  "OVERALL_CPU_DEFECT_STATUS":"OPEN_REMAINING_ROOTS",
  "P2_STARTED":"NO",
  "PUSH":"NO","TAG":"NO","RELEASE":"NO",
  "PUBLIC_BENCHMARK_CLAIMS":"FORBIDDEN",
  "EVIDENCE_DIR":str(ev),
}
(ev/"terminal_report.json").write_text(json.dumps(report, indent=2)+"\n")
print(json.dumps(report, indent=2))
PY

log "COMPLETE $EV"
