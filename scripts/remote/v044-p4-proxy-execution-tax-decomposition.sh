#!/usr/bin/env bash
# V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION — read-only ROOT_1A/1F/1G decomposition.
# PRODUCT_MUTATION=NO. Uses canonical binary only (not Cap034 experiment).
set -euo pipefail

WS="${P4_TAX_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${P4_TAX_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${P4_TAX_EV:-$WS/.exyonq-local-evidence/v044-p4-proxy-execution-tax-decomposition-$TS}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export V044_P4_PROF_TS="$TS"
export V044_P4_PROF_EV="$EV"
export V044_P4_WS="$WS"
export P4_EXYONQ_IMAGE="${P4_EXYONQ_IMAGE:-v044p1-geom8-exyonq}"
export EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459}"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"

mkdir -p "$EV"/{concurrency,upstream-load,scheduler,reports,meta}
cd "$WS"
log() { echo "[p4-tax] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

log "phase 1: base P4P profiling (perf stat/record/strace/memory/wire)"
bash "$SCRIPT_DIR/v044-p4-proxy-path-causal-profiling.sh" 2>&1 | tee -a "$EV/orchestrator.log"

FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME}"
PATH_P4="/api/"
WARMUP_SEC=20
MEASURE_SEC=20

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-geom8.yml" ]] && extra+=(-f "$EV/meta/compose-geom8.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    upstream) echo "${PROJECT}-upstream-1" ;;
  esac
}

url_exyonq() { echo "http://exyonq:8080${PATH_P4}"; }

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

first_worker() {
  local main=$1
  ps -T -p "$main" -o tid= 2>/dev/null | head -1 | tr -d ' '
}

PERF_EVENTS="task-clock,cycles,instructions,context-switches,cpu-migrations,page-faults,major-faults,minor-faults"

concurrency_sweep() {
  local url; url=$(url_exyonq)
  local worker; worker=$(first_worker "$(main_pid "$(ctr exyonq)")")
  log "concurrency sweep C=1,8,32,100,256"
  for c in 1 8 32 100 256; do
    compose exec -T bench-runner rewrk -c "$c" -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
    local tmp; tmp=$(mktemp)
    compose exec -T bench-runner timeout $((MEASURE_SEC + 15)) rewrk -c "$c" -d "${MEASURE_SEC}s" -t 2 -h "$url" --json >"$tmp" 2>/dev/null || true
    local rps; rps=$(python3 -c "import json; print(json.load(open('$tmp')).get('requests_avg',0))" 2>/dev/null || echo 0)
    local pstat="$EV/concurrency/C${c}-perfstat.txt"
    perf stat -e "$PERF_EVENTS" -p "$worker" -- \
      bash -c "sleep 2; timeout $((MEASURE_SEC + 5)) docker exec ${PROJECT}-bench-runner-1 rewrk -c $c -d ${MEASURE_SEC}s -t 2 -h $url >/dev/null" \
      >"$pstat" 2>&1 || true
    python3 - "$c" "$rps" "$pstat" "$EV/concurrency/C${c}.json" <<'PY'
import json, re, sys
c, rps, pstat, out = sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[4]
vals = {}
for line in open(pstat):
    m = re.match(r'\s*([\d,]+)\s+(\S+)', line)
    if not m: continue
    k, v = m.group(2), int(m.group(1).replace(',',''))
    if k == 'cycles': vals['cycles'] = v
    elif k == 'instructions': vals['instructions'] = v
    elif k == 'context-switches': vals['context_switches'] = v
    elif k == 'cpu-migrations': vals['cpu_migrations'] = v
rps = max(rps, 1)
row = {"concurrency": int(c), "rps": rps, **{k+"_per_req": vals[k]/rps for k in vals}}
open(out,'w').write(json.dumps(row, indent=2))
print(f"C={c} rps={rps:.0f} ctx/req={row.get('context_switches_per_req','NA')}")
PY
    rm -f "$tmp"
    sleep 3
  done
}

upstream_pool_under_load() {
  local url; url=$(url_exyonq)
  local worker; worker=$(first_worker "$(main_pid "$(ctr exyonq)")")
  log "upstream pool observation under load"
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json >"$EV/upstream-load/rewrk.json" &
  local lp=$!
  sleep 2
  {
    echo "=== ss established exyonq->9000 (mid-load) ==="
    docker exec "$(ctr exyonq)" sh -c 'ss -tn state established 2>/dev/null | grep 9000 | wc -l; ss -tn state established 2>/dev/null | grep 9000 | head -20'
    echo "=== ss time-wait exyonq ==="
    docker exec "$(ctr exyonq)" sh -c 'ss -tn state time-wait 2>/dev/null | wc -l'
  } | tee "$EV/upstream-load/socket_midload.txt"
  timeout 15 strace -f -p "$worker" -e trace=connect,close -c 2>"$EV/upstream-load/exyonq-connect-close-strace.txt" || true
  wait "$lp" 2>/dev/null || true
  local rps; rps=$(python3 -c "import json; print(json.load(open('$EV/upstream-load/rewrk.json')).get('requests_avg',1))")
  python3 - "$EV/upstream-load/exyonq-connect-close-strace.txt" "$rps" "$EV/upstream-load/pool_metrics.json" <<'PY'
import json, re, sys
text = open(sys.argv[1]).read()
rps = float(sys.argv[2])
counts = {}
for line in text.splitlines():
    m = re.match(r'\s*(\d+)\s+(\S+)', line)
    if m: counts[m.group(2)] = int(m.group(1))
conn = counts.get('connect', 0)
close = counts.get('close', 0)
req_est = max(rps * 20, 1)  # MEASURE_SEC window approx
out = {
    "connect_syscalls": conn,
    "close_syscalls": close,
    "connects_per_request": conn / req_est,
    "closes_per_request": close / req_est,
    "rps_during_window": rps,
    "pool_hit_rate": "NOT_MEASURED_DIRECTLY",
    "upstream_requests_per_connection": "NOT_MEASURED_DIRECTLY",
}
open(sys.argv[3],'w').write(json.dumps(out, indent=2))
for k,v in out.items(): print(f"{k}={v}")
PY
}

worker_cpu_distribution() {
  local c; c=$(ctr exyonq)
  log "worker thread CPU snapshot under load"
  local url; url=$(url_exyonq)
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  compose exec -T bench-runner rewrk -c 100 -d 25 -t 2 -h "$url" >/dev/null 2>&1 &
  local lp=$!
  sleep 2
  docker exec "$c" sh -c 'ps -eLo pid,tid,pcpu,comm | grep -E "exyonq|tokio" | head -40' >"$EV/scheduler/worker_threads.txt" 2>&1 || true
  wait "$lp" 2>/dev/null || true
}

terminal_aggregate() {
  python3 - "$EV" <<'PY'
import json, re, sys
from pathlib import Path
ev = Path(sys.argv[1])
summary_p = ev / "reports" / "summary.json"
base = json.loads(summary_p.read_text()) if summary_p.exists() else {}
p4pb = {
    "cycles_per_req": 34617095,
    "instructions_per_req": 25460815,
    "ctx_per_req": 115.78,
    "ipc": 0.735,
}
exy = base.get("perf_stat_exyonq", {})
cur_c = exy.get("cycles_per_req")
cur_i = exy.get("instructions_per_req")
cur_ctx = exy.get("context_switches_per_req")
repro = "NO"
if cur_c and cur_i:
    ratio_c = cur_c / p4pb["cycles_per_req"]
    ratio_i = cur_i / p4pb["instructions_per_req"]
    if 0.7 <= ratio_c <= 1.4 and 0.7 <= ratio_i <= 1.4:
        repro = "YES"
    elif 0.5 <= ratio_c <= 1.6:
        repro = "PARTIAL"
out = {
    "wip": "V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION",
    "product_mutation": "NO",
    "p4pb_profile_reproduced": repro,
    "current_perf_stat_exyonq": exy,
    "p4pb_reference": p4pb,
    "syscalls_exyonq": base.get("syscalls_exyonq"),
    "top_symbols": base.get("top_exyonq_symbols"),
    "memory_exyonq": base.get("memory_exyonq"),
}
# concurrency
conc = []
for p in sorted((ev/"concurrency").glob("C*.json")):
    conc.append(json.loads(p.read_text()))
out["concurrency_sweep"] = conc
pool = ev/"upstream-load/pool_metrics.json"
if pool.exists():
    out["upstream_pool"] = json.loads(pool.read_text())
(ev/"reports"/"tax_terminal.json").write_text(json.dumps(out, indent=2))
print(f"P4P_B_PROFILE_REPRODUCED={repro}")
if cur_ctx and p4pb["ctx_per_req"]:
    print(f"CTX_RATIO_VS_P4PB={cur_ctx/p4pb['ctx_per_req']:.2f}")
PY
}

{
  echo "WIP=V044_P4_PROXY_EXECUTION_TAX_DECOMPOSITION"
  echo "MODE=READ_ONLY_CAUSAL_DECOMPOSITION"
  echo "CAP034_EXPERIMENT_STATUS=COMPLETE_AB_C34_B_NOT_AUTHORITY"
  echo "EXPECTED_BINARY=$EXPECTED_EXYONQ_SHA256"
} | tee "$EV/meta/tax_meta.txt"

concurrency_sweep
upstream_pool_under_load
worker_cpu_distribution
terminal_aggregate

echo "DONE $EV" | tee "$EV/reports/done.txt"
log "complete EV=$EV"
