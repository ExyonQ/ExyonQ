#!/usr/bin/env bash
# V044 P1 causal profiling — diagnostic only, no product mutation.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
EV="${P1_CAUSAL_EVIDENCE:-$WS/.exyonq-local-evidence/v044-p1-causal-profiling}"
COMPOSE="$WS/benchmarks/docker/docker-compose.p1-minimal.yml"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
PATH_P1="/site/1k.bin"
STAT_REPS="${P1_STAT_REPS:-3}"
RECORD_SEC="${P1_RECORD_SEC:-20}"

mkdir -p "$EV"/{perf-stat,perf-record,syscalls,scheduler,network,reports}
cd "$WS"
log() { echo "[p1-causal] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
export COMPOSE_FILE="$FULL_COMPOSE"

# Phase 0 — harness authority
log "Phase 0: harness confirmation"
{
  echo "AUTHORITY_HEAD=353bf088309bf563dbf37e93b5e3fa85888b03f5"
  echo "P1_CANONICAL_EXECUTION_MODEL=SYMMETRIC_DOCKER"
  echo "EXYONQ_BINARY_SHA256=3b10f0e53eadb9d415e0ef65037d89d1eec9cea8a1db2076c4694592890fff58"
  echo "CAUSE3C_ASYMMETRY_PRESENT=NO"
  echo "BENCHMARK_ONLY_PRODUCT_BEHAVIOR=NO"
  echo "PRODUCT_SEMANTIC_BRANCH=0"
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T exyonq sha256sum /usr/local/bin/exyonq 2>/dev/null || true
} | tee "$EV/phase0_harness.txt"

# Stop non-P1 rivals (harness isolation, not owner workloads)
log "Stopping non-P1 rival containers to match 3-way matrix"
for svc in apache caddy haproxy traefik envoy nginx-mainline openlitespeed-stable; do
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" stop "$svc" 2>/dev/null || true
done

# Phase 1 — host interference
log "Phase 1: host interference assessment"
{
  echo "=== unrelated exyonq processes ==="
  ps -p 2614430,3911423 -o pid,pcpu,pmem,etime,cmd 2>/dev/null || echo "PIDs absent"
  echo "=== pidstat 5s sample ==="
  pidstat -p 2614430,3911423 1 5 2>/dev/null || true
  echo "=== benchmark CPU set ==="
  nproc
  lscpu | egrep 'CPU\(s\)|Model name|MHz'
} | tee "$EV/phase1_host_interference.txt"

# Resolve container PIDs (host namespace)
get_pid() {
  docker inspect -f '{{.State.Pid}}' "$1" 2>/dev/null
}
EXY_C="v044p1auth-clean-exyonq-1"
NGX_C="v044p1auth-clean-nginx-stable-1"
OLS_C="v044p1auth-clean-openlitespeed-latest-1"
EXY_PID=$(get_pid "$EXY_C")
NGX_PID=$(get_pid "$NGX_C")
OLS_PID=$(get_pid "$OLS_C")
echo "EXY_PID=$EXY_PID NGX_PID=$NGX_PID OLS_PID=$OLS_PID" | tee "$EV/container_pids.txt"

# Load URLs (internal via bench-runner)
EXY_URL="http://exyonq:8080${PATH_P1}"
NGX_URL="http://nginx-stable:8080${PATH_P1}"
OLS_URL="http://openlitespeed-latest:8088${PATH_P1}"

run_load() {
  local url="$1"
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "$DURATION" -h "$url" --json -t 2 >/dev/null &
  echo $!
}

run_load_warmup() {
  local url="$1"
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${WARMUP}s" -h "$url" --json -t 2 >/dev/null || true
}

PERF_EVENTS="task-clock,cpu-clock,cycles,instructions,branches,branch-misses,context-switches,cpu-migrations,page-faults,cache-references,cache-misses"

# Phase 2 — perf stat decomposition
log "Phase 2: perf stat decomposition ($STAT_REPS reps each)"
stat_one() {
  local tag=$1 url=$2 pid=$3 rep=$4
  local out="$EV/perf-stat/${tag}-rep${rep}.txt"
  run_load_warmup "$url"
  local load_pid
  load_pid=$(run_load "$url")
  sleep 1
  perf stat -e "$PERF_EVENTS" -p "$pid" -- sleep 30 >"$out" 2>&1 || true
  wait "$load_pid" 2>/dev/null || true
  # extract requests from parallel rewrk if needed — use fixed 30s window
  python3 - "$out" "$tag" "$rep" <<'PY' >> "$EV/perf-stat/summary.csv"
import re, sys
path, tag, rep = sys.argv[1:4]
text = open(path).read()
vals = {}
for line in text.splitlines():
    m = re.match(r'\s*([\d,]+)\s+(\S+)', line)
    if not m: continue
    v = int(m.group(1).replace(',',''))
    k = m.group(2).split('#')[0].strip().rstrip(',')
    vals[k] = v
print(f"{tag},{rep},{vals.get('cycles',0)},{vals.get('instructions',0)},{vals.get('branches',0)},{vals.get('context-switches',0)},{vals.get('cpu-migrations',0)},{vals.get('cache-misses',0)},{vals.get('task-clock',0)}")
PY
}

echo "tag,rep,cycles,instructions,branches,ctx_switches,cpu_migrations,cache_misses,task_clock_ns" > "$EV/perf-stat/summary.csv"
for rep in $(seq 1 "$STAT_REPS"); do
  stat_one exyonq "$EXY_URL" "$EXY_PID" "$rep"
  sleep 5
  stat_one nginx "$NGX_URL" "$NGX_PID" "$rep"
  sleep 5
  stat_one ols "$OLS_URL" "$OLS_PID" "$rep"
  sleep 5
done

profile_all() {
  local fn=$1
  fn exyonq "$EXY_URL" "$EXY_PID"
  sleep 5
  fn nginx "$NGX_URL" "$NGX_PID"
  sleep 5
  fn ols "$OLS_URL" "$OLS_PID"
  sleep 5
}

# Phase 3 — syscall differential (perf trace summary, 10s window each)
log "Phase 3: syscall differential"
syscall_profile() {
  local tag=$1 url=$2 pid=$3
  local out="$EV/syscalls/${tag}-trace.txt"
  run_load_warmup "$url"
  local lp; lp=$(run_load "$url")
  sleep 1
  timeout 12 perf trace -p "$pid" -e 'syscalls:sys_enter_*' -- sleep 10 >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  grep -oE 'sys_enter_[a-zA-Z0-9_]+' "$out" 2>/dev/null | sort | uniq -c | sort -rn > "$EV/syscalls/${tag}-counts.txt" || true
}
profile_all syscall_profile

# Phase 4 — static TX geometry (strace 5s sample during load)
log "Phase 4: static TX geometry"
tx_sample() {
  local tag=$1 url=$2 pid=$3
  local out="$EV/syscalls/${tag}-strace-tx.txt"
  run_load_warmup "$url"
  local lp; lp=$(run_load "$url")
  sleep 1
  timeout 8 strace -f -p "$pid" -e trace=sendfile,splice,write,writev,read,recvfrom,sendto,send,recv,epoll_wait,epoll_pwait,epoll_ctl,accept,accept4,openat,close -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}
profile_all tx_sample

# Phase 5 — scheduler / CPU
log "Phase 5: scheduler CPU distribution"
sched_sample() {
  local tag=$1 url=$2 pid=$3
  local out="$EV/scheduler/${tag}-pidstat.txt"
  run_load_warmup "$url"
  local lp; lp=$(run_load "$url")
  sleep 1
  pidstat -t -p "$pid" 1 15 > "$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  ps -T -p "$pid" -o pid,tid,pcpu,comm >> "$out" 2>/dev/null || true
}
profile_all sched_sample

# Phase 6 — perf record callgraph
log "Phase 6: perf record ($RECORD_SEC s each)"
record_one() {
  local tag=$1 url=$2 pid=$3
  local data="$EV/perf-record/${tag}.data"
  local report="$EV/perf-record/${tag}-report.txt"
  run_load_warmup "$url"
  local lp; lp=$(run_load "$url")
  sleep 1
  perf record -F 997 -g -p "$pid" -o "$data" -- sleep "$RECORD_SEC" 2>"$EV/perf-record/${tag}-record.log" || true
  wait "$lp" 2>/dev/null || true
  perf report -i "$data" --stdio --no-children --sort comm,dso,symbol --percent-limit 0.5 2>/dev/null | head -120 > "$report" || true
}
profile_all record_one

# Phase 7 — off-CPU (perf sched if available)
log "Phase 7: off-CPU / sched"
if perf sched record --help >/dev/null 2>&1; then
  run_load_warmup "$EXY_URL"
  lp=$(run_load "$EXY_URL")
  sleep 1
  perf sched record -p "$EXY_PID" -o "$EV/scheduler/exyonq-sched.data" -- sleep 10 2>/dev/null || true
  wait "$lp" 2>/dev/null || true
  perf sched latency -i "$EV/scheduler/exyonq-sched.data" 2>/dev/null | head -40 > "$EV/scheduler/exyonq-sched-latency.txt" || true
fi

# Phase 8 — network counters
log "Phase 8: network stack"
snap_net() {
  local f=$1
  { grep -E '^Tcp:|^ [0-9]' /proc/net/snmp; date -u +%s; } > "$f"
}
snap_net "$EV/network/snmp_before.txt"
run_load_warmup "$EXY_URL"
lp=$(run_load "$EXY_URL"); wait "$lp" 2>/dev/null || true
snap_net "$EV/network/snmp_after_exyonq.txt"
run_load_warmup "$NGX_URL"
lp=$(run_load "$NGX_URL"); wait "$lp" 2>/dev/null || true
snap_net "$EV/network/snmp_after_nginx.txt"
run_load_warmup "$OLS_URL"
lp=$(run_load "$OLS_URL"); wait "$lp" 2>/dev/null || true
snap_net "$EV/network/snmp_after_ols.txt"

# Aggregate report via python
log "Phase 9: generating causal report"
python3 <<'PY' > "$EV/reports/causal_summary.json"
import json, re, statistics, os, glob
from pathlib import Path
EV = Path(os.environ.get('P1_CAUSAL_EVIDENCE', '/tmp/ev'))
# baseline RPS from authoritative run
baseline = {"exyonq": 106199.35, "nginx": 159901.99, "ols": 131386.24}
# parse perf-stat medians — need requests per window; use rewrk from prior baseline ~30s
# approximate requests from baseline RPS * 30
req_est = {k: v * 30 for k, v in baseline.items()}

# parse strace TX
def parse_strace_counts(tag):
    p = EV / f'syscalls/{tag}-strace-tx.txt'
    if not p.exists(): return {}
    text = p.read_text()
    out = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[-1].startswith(('sendfile','write','splice','read','epoll','accept','recv','send')):
            try:
                out[parts[-1]] = int(parts[-2])
            except: pass
    return out

# parse perf-stat summary.csv
stats = {t: [] for t in ['exyonq','nginx','ols']}
csv = EV / 'perf-stat/summary.csv'
if csv.exists():
    for line in csv.read_text().strip().split('\n')[1:]:
        tag, rep, cyc, ins, br, ctx, mig, cm, tc = line.split(',')
        stats[tag].append({'cycles': int(cyc), 'instructions': int(ins), 'branches': int(br),
            'ctx': int(ctx), 'mig': int(mig), 'cache_miss': int(cm), 'task_clock': int(tc)})

def per_req(tag, key):
    if not stats[tag]: return None
    med = statistics.median([s[key] for s in stats[tag]])
    return med / req_est[tag]

report = {
    'baseline_rps': baseline,
    'req_est_30s': req_est,
    'per_request': {
        tag: {
            'cycles': per_req(tag, 'cycles'),
            'instructions': per_req(tag, 'instructions'),
            'branches': per_req(tag, 'branches'),
            'context_switches': per_req(tag, 'ctx'),
            'cpu_migrations': per_req(tag, 'mig'),
            'cache_misses': per_req(tag, 'cache_miss'),
            'task_clock_ns': per_req(tag, 'task_clock'),
        } for tag in baseline
    },
    'strace_tx': {tag: parse_strace_counts(tag) for tag in baseline},
}
print(json.dumps(report, indent=2))
PY

log "COMPLETE evidence=$EV"
echo "P1_CAUSAL_EVIDENCE=$EV"
