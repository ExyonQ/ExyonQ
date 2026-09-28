#!/usr/bin/env bash
# V044_P1_CPU_EFFICIENCY_REPAIR_ROOT3 — A/B CPU cost/req vs sealed Root1 terminal binary.
# BEFORE = Root1 (14d22eae…); AFTER = Root1+Root3 binary under test.
# P2=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_ROOT3_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_ROOT3_EV:-$WS/.exyonq-local-evidence/v044-p1-cpu-root3-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P1="/site/1k.bin"
URL="http://exyonq:8080${PATH_P1}"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5
CPU_CORES=8

# Frozen Root1 terminal (CLOSED_PROVEN) — do not compare to pre-Root1.
BEFORE_BINARY_SHA256="${BEFORE_BINARY_SHA256:-14d22eae2c9fb100926e37eb334f9caac73d4c9c87904cc07d788dde3c57ce91}"
BEFORE_CORE_MS_PER_REQ="${BEFORE_CORE_MS_PER_REQ:-0.01976}"
BEFORE_REQ_PER_CORE="${BEFORE_REQ_PER_CORE:-50618.99}"
BEFORE_CPU_AVG="${BEFORE_CPU_AVG:-294.02}"
BEFORE_RPS="${BEFORE_RPS:-156952.95}"
BEFORE_P99_MS="${BEFORE_P99_MS:-1.64}"
ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ=0.02521
ORIGINAL_REQ_PER_CORE=39667.51

mkdir -p "$EV"/{runs,perf,meta,reports}
cd "$WS"
log() { echo "[root3-ab] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

CTR="${PROJECT}-exyonq-1"
AFTER_SHA=$(docker exec "$CTR" sha256sum /usr/local/bin/exyonq | awk '{print $1}')

{
  echo "CURRENT_WIP=V044_P1_CPU_EFFICIENCY_REPAIR_ROOT3"
  echo "PRODUCT_MUTATION=YES_WITHIN_ROOT3"
  echo "TIMESTAMP_UTC=$TS"
  echo "AFTER_BINARY_SHA256=$AFTER_SHA"
  echo "BEFORE_BINARY_SHA256=$BEFORE_BINARY_SHA256"
  echo "BEFORE_SOURCE=ROOT1_TERMINAL_CLOSED_PROVEN"
  echo "ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ=$ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ"
  echo "AFTER_ROOT1_CORE_MS_PER_REQ=$BEFORE_CORE_MS_PER_REQ"
} | tee "$EV/meta/authority_bind.txt"

if [[ "$AFTER_SHA" == "$BEFORE_BINARY_SHA256" ]]; then
  log "FAIL: AFTER binary SHA equals Root1 — rebuild Root3 before measuring"
  exit 2
fi

docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T exyonq cat /bench/bench.toml \
  | tee "$EV/meta/bench.toml" >/dev/null
bash "$WS/scripts/gates/p1-raw-waf-off-gate.sh" --config "$EV/meta/bench.toml" | tee "$EV/meta/waf_gate.txt"
docker update --cpuset-cpus "0-7" "$CTR" >/dev/null

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
  timeout $((MEASURE_SEC + 45)) docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$URL" --pct --json \
    >"${out}.rewrk.json" 2>"${out}.rewrk.txt" || true
  wait $SP 2>/dev/null || true

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

EV="$EV" CPU_CORES="$CPU_CORES" \
BEFORE_CORE_MS_PER_REQ="$BEFORE_CORE_MS_PER_REQ" \
BEFORE_REQ_PER_CORE="$BEFORE_REQ_PER_CORE" \
BEFORE_CPU_AVG="$BEFORE_CPU_AVG" \
BEFORE_RPS="$BEFORE_RPS" \
BEFORE_P99_MS="$BEFORE_P99_MS" \
ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ="$ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ" \
ORIGINAL_REQ_PER_CORE="$ORIGINAL_REQ_PER_CORE" \
python3 - <<'PY'
import json, re, statistics, os
from pathlib import Path

ev = Path(os.environ["EV"])
cores = float(os.environ.get("CPU_CORES", "8"))
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

def parse_pct_table(text):
    out = {}
    for line in text.splitlines():
        m = re.match(r"\s*(50|90|95|99|99\.9|99\.99)%\s+([\d.]+)", line)
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
    jpath=base.with_suffix(".rewrk.json")
    j=json.loads(jpath.read_text()) if jpath.exists() else {}
    pct=parse_pct_table((base.with_suffix(".rewrk.txt")).read_text(errors="replace") if base.with_suffix(".rewrk.txt").exists() else "")
    cpu,n=cpu_avg(Path(str(base)+".stats.txt"))
    rps=float(j.get("requests_avg") or 0)
    reqs=int(j.get("requests_total") or 0)
    perf=perf_parse(Path(str(base)+".perfstat.txt"))
    cores_used=(cpu/100.0) if cpu is not None else None
    req_per_core=(rps/cores_used) if cores_used and cores_used>0 else None
    core_ms=(1000.0/req_per_core) if req_per_core and req_per_core>0 else None
    cycles_per_req=(perf.get("cycles")/reqs) if perf.get("cycles") and reqs else None
    instr_per_req=(perf.get("instructions")/reqs) if perf.get("instructions") and reqs else None
    ipc=(perf["instructions"]/perf["cycles"]) if perf.get("cycles") and perf.get("instructions") else None
    reps.append(dict(rps=rps,cpu=cpu,req_per_core=req_per_core,core_ms=core_ms,pct=pct,
                     cycles_per_req=cycles_per_req,instr_per_req=instr_per_req,ipc=ipc,reqs=reqs))

def med(xs):
    xs=[x for x in xs if x is not None]
    return statistics.median(xs) if xs else None

after=dict(
    rps=med([r["rps"] for r in reps]),
    cpu=med([r["cpu"] for r in reps]),
    req_per_core=med([r["req_per_core"] for r in reps]),
    core_ms=med([r["core_ms"] for r in reps]),
    p99=med([r["pct"].get("p99") for r in reps]),
    p50=med([r["pct"].get("p50") for r in reps]),
    p90=med([r["pct"].get("p90") for r in reps]),
    p95=med([r["pct"].get("p95") for r in reps]),
    cycles_per_req=med([r["cycles_per_req"] for r in reps]),
    instr_per_req=med([r["instr_per_req"] for r in reps]),
    ipc=med([r["ipc"] for r in reps]),
)

before=dict(
    core_ms=float(os.environ["BEFORE_CORE_MS_PER_REQ"]),
    req_per_core=float(os.environ["BEFORE_REQ_PER_CORE"]),
    cpu=float(os.environ["BEFORE_CPU_AVG"]),
    rps=float(os.environ["BEFORE_RPS"]),
    p99=float(os.environ["BEFORE_P99_MS"]),
)
orig_core=float(os.environ["ORIGINAL_PRE_REPAIR_CORE_MS_PER_REQ"])
orig_rpc=float(os.environ["ORIGINAL_REQ_PER_CORE"])

def pct(a,b):
    if a is None or b is None or b==0: return None
    return 100.0*(a-b)/b

summary={
    "BEFORE_ROOT3": before,
    "AFTER_ROOT3": after,
    "ROOT3_CPU_COST_PER_REQ_DELTA_PERCENT": pct(after["core_ms"], before["core_ms"]),
    "CUMULATIVE_CPU_COST_PER_REQ_DELTA_FROM_ORIGINAL": pct(after["core_ms"], orig_core),
    "RPS_DELTA_PERCENT": pct(after["rps"], before["rps"]),
    "AFTER_ROOT3_REQ_PER_CORE": after["req_per_core"],
    "ORIGINAL_REQ_PER_CORE": orig_rpc,
    "reps": reps,
}
(ev/"reports"/"root3_summary.json").write_text(json.dumps(summary, indent=2))
lines=[
    f"BEFORE_ROOT3_CORE_MS_PER_REQ={before['core_ms']}",
    f"AFTER_ROOT3_CORE_MS_PER_REQ={after['core_ms']}",
    f"ROOT3_CPU_COST_PER_REQ_DELTA_PERCENT={summary['ROOT3_CPU_COST_PER_REQ_DELTA_PERCENT']}",
    f"CUMULATIVE_CPU_COST_PER_REQ_DELTA_FROM_ORIGINAL={summary['CUMULATIVE_CPU_COST_PER_REQ_DELTA_FROM_ORIGINAL']}",
    f"BEFORE_ROOT3_REQ_PER_CORE={before['req_per_core']}",
    f"AFTER_ROOT3_REQ_PER_CORE={after['req_per_core']}",
    f"BEFORE_ROOT3_CPU_AVG={before['cpu']}",
    f"AFTER_ROOT3_CPU_AVG={after['cpu']}",
    f"BEFORE_ROOT3_RPS={before['rps']}",
    f"AFTER_ROOT3_RPS={after['rps']}",
    f"RPS_DELTA_PERCENT={summary['RPS_DELTA_PERCENT']}",
    f"BEFORE_ROOT3_P99={before['p99']}",
    f"AFTER_ROOT3_P99={after['p99']}",
    f"AFTER_ROOT3_P50={after['p50']}",
    f"AFTER_ROOT3_P90={after['p90']}",
    f"AFTER_ROOT3_P95={after['p95']}",
    f"AFTER_CYCLES_PER_REQ={after['cycles_per_req']}",
    f"AFTER_INSTR_PER_REQ={after['instr_per_req']}",
    f"AFTER_IPC={after['ipc']}",
]
(ev/"reports"/"root3_terminal.txt").write_text("\n".join(lines)+"\n")
print("\n".join(lines))
PY

log "done EV=$EV"
