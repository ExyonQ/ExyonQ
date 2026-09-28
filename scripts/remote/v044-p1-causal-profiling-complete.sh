#!/usr/bin/env bash
# V044 P1 causal profiling — complete missing phases (worker PIDs, network, aggregate).
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
EV="${P1_CAUSAL_EVIDENCE:-$WS/.exyonq-local-evidence/v044-p1-causal-profiling}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
RECORD_SEC="${P1_RECORD_SEC:-20}"
PATH_P1="/site/1k.bin"

mkdir -p "$EV"/{perf-stat,perf-record,syscalls,scheduler,network,reports}
cd "$WS"
log() { echo "[p1-causal-complete] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
EXY_C="${PROJECT}-exyonq-1"
NGX_C="${PROJECT}-nginx-stable-1"
OLS_C="${PROJECT}-openlitespeed-latest-1"

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

live_pids() {
  local pids=("$@")
  local out=()
  for p in "${pids[@]}"; do
    [[ -n "$p" && -d "/proc/$p" ]] && out+=("$p")
  done
  (IFS=,; echo "${out[*]}")
}

# Resolve worker PIDs on host (live only).
resolve_nginx_workers() {
  local main=$1
  local kids
  kids=$(pgrep -P "$main" 2>/dev/null || true)
  live_pids $main $kids
}

resolve_ols_workers() {
  local main=$1
  # OLS: skip entrypoint shell; prefer litespeed/httpd children.
  mapfile -t kids < <(pgrep -P "$main" 2>/dev/null || true)
  local workers=()
  for k in "${kids[@]}"; do
    local cmd
    cmd=$(ps -p "$k" -o comm= 2>/dev/null || true)
    if [[ "$cmd" == *litespeed* || "$cmd" == *openlitespeed* || "$cmd" == *httpd* ]]; then
      workers+=("$k")
      mapfile -t gc < <(pgrep -P "$k" 2>/dev/null || true)
      workers+=("${gc[@]}")
    fi
  done
  if ((${#workers[@]} == 0)); then
    # fallback: all live descendants
    mapfile -t workers < <(pstree -p "$main" 2>/dev/null | grep -oP '\(\K[0-9]+' || pgrep -P "$main" 2>/dev/null || true)
  fi
  live_pids "${workers[@]}"
}

resolve_exyonq_pids() {
  local main=$1
  mapfile -t tids < <(ps -T -p "$main" -o tid= 2>/dev/null || true)
  live_pids "$main" "${tids[@]}"
}

EXY_MAIN=$(main_pid "$EXY_C")
NGX_MAIN=$(main_pid "$NGX_C")
OLS_MAIN=$(main_pid "$OLS_C")
EXY_PIDS=$(resolve_exyonq_pids "$EXY_MAIN")
NGX_PIDS=$(resolve_nginx_workers "$NGX_MAIN")
OLS_PIDS=$(resolve_ols_workers "$OLS_MAIN")

# First worker for attach-style tools
first_worker() {
  echo "${1##*,}" | awk -F, '{print $NF}'
}

NGX_WORKER=$(first_worker "$NGX_PIDS")
OLS_WORKER=$(first_worker "$OLS_PIDS")
EXY_WORKER=$(first_worker "$EXY_PIDS")

{
  echo "EXY_PIDS=$EXY_PIDS"
  echo "NGX_PIDS=$NGX_PIDS"
  echo "OLS_PIDS=$OLS_PIDS"
  echo "NGX_WORKER=$NGX_WORKER OLS_WORKER=$OLS_WORKER EXY_WORKER=$EXY_WORKER"
} | tee "$EV/container_pids_v2.txt"

EXY_URL="http://exyonq:8080${PATH_P1}"
NGX_URL="http://nginx-stable:8080${PATH_P1}"
OLS_URL="http://openlitespeed-latest:8088${PATH_P1}"

run_load() {
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "$DURATION" -h "$1" --json -t 2 >/dev/null &
  echo $!
}
run_warmup() {
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "${WARMUP}s" -h "$1" --json -t 2 >/dev/null || true
}

PERF_EVENTS="task-clock,cpu-clock,cycles,instructions,branches,branch-misses,context-switches,cpu-migrations,page-faults,cache-references,cache-misses"

parse_perf_stat() {
  python3 - "$1" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
vals = {}
for line in text.splitlines():
    if '<not counted>' in line: continue
    m = re.match(r'\s*([\d,]+(?:\.\d+)?)\s+(\S+)', line)
    if m:
        v = m.group(1).replace(',','')
        k = m.group(2)
        if k in ('msec', 'GHz', 'CPUs', 'sec', 'K/sec', 'M/sec', '/sec', 'refs', 'utilized', 'cycle', 'branches', 'insn'): continue
        if k == 'task-clock': vals['task_clock_ms'] = float(v)
        elif k == 'cpu-clock': vals['cpu_clock_ms'] = float(v)
        elif k == 'cycles': vals['cycles'] = int(float(v))
        elif k == 'instructions': vals['instructions'] = int(float(v))
        elif k == 'branches': vals['branches'] = int(float(v))
        elif k == 'branch-misses': vals['branch_misses'] = int(float(v))
        elif k == 'context-switches': vals['context_switches'] = int(float(v))
        elif k == 'cpu-migrations': vals['cpu_migrations'] = int(float(v))
        elif k == 'cache-misses': vals['cache_misses'] = int(float(v))
    m2 = re.search(r'(\d+\.\d+) seconds time elapsed', line)
    if m2: vals['elapsed'] = float(m2.group(1))
print(vals)
PY
}

get_rps() {
  local url=$1
  local tmp; tmp=$(mktemp)
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" exec -T bench-runner \
    rewrk -c 100 -d "$DURATION" -h "$url" --json -t 2 >"$tmp" || true
  python3 -c "import json; d=json.load(open('$tmp')); print(d.get('requests_avg',0))" 2>/dev/null || echo 0
}

stat_one() {
  local tag=$1 url=$2 pids=$3 rep=$4
  local out="$EV/perf-stat/${tag}-rep${rep}-v2.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  # shellcheck disable=SC2086
  perf stat -e "$PERF_EVENTS" -p "$pids" -- sleep 30 >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  local rps; rps=$(get_rps "$url")
  local parsed; parsed=$(parse_perf_stat "$out")
  echo "${tag},${rep},${rps},${parsed}" >> "$EV/perf-stat/raw_rows_v2.txt"
}

log "Phase 2b: OLS + confirm nginx perf stat (v2)"
echo "tag,rep,rps,parsed_dict" > "$EV/perf-stat/raw_rows_v2.txt"
for rep in 1 2 3; do
  stat_one ols "$OLS_URL" "$OLS_PIDS" "$rep"; sleep 5
done

strace_sample() {
  local tag=$1 url=$2 worker=$3
  local out="$EV/syscalls/${tag}-strace-tx-v2.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  timeout 8 strace -f -p "$worker" -e trace=sendfile,splice,write,writev,read,recvfrom,sendto,send,recv,epoll_wait,epoll_pwait,epoll_ctl,accept,accept4,openat,close,futex -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}

log "Phase 3-4b: strace on worker PIDs"
strace_sample nginx "$NGX_URL" "$NGX_WORKER"
sleep 5
strace_sample ols "$OLS_URL" "$OLS_WORKER"
sleep 5

sched_workers() {
  local tag=$1 url=$2 pids=$3
  local out="$EV/scheduler/${tag}-pidstat-v2.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  # shellcheck disable=SC2086
  pidstat -t -p "$pids" 1 15 > "$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
}

log "Phase 5b: scheduler on workers"
sched_workers nginx "$NGX_URL" "$NGX_PIDS"
sleep 5
sched_workers ols "$OLS_URL" "$OLS_PIDS"

record_one() {
  local tag=$1 url=$2 worker=$3
  local data="$EV/perf-record/${tag}-v2.data"
  local report="$EV/perf-record/${tag}-report-v2.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  perf record -F 997 -g -p "$worker" -o "$data" -- sleep "$RECORD_SEC" 2>"$EV/perf-record/${tag}-record-v2.log" || true
  wait "$lp" 2>/dev/null || true
  perf report -i "$data" --stdio --sort dso,symbol --percent-limit 0.5 2>/dev/null | head -200 > "$report" || true
}

log "Phase 6b: perf record on workers"
record_one nginx "$NGX_URL" "$NGX_WORKER"
sleep 5
record_one ols "$OLS_URL" "$OLS_WORKER"

log "Phase 8: network snmp deltas"
snap() { grep -E '^Tcp:|^ [0-9]' /proc/net/snmp; echo "TS $(date +%s)"; }
for spec in "exyonq $EXY_URL" "nginx $NGX_URL" "ols $OLS_URL"; do
  tag=${spec%% *}
  url=${spec#* }
  snap > "$EV/network/snmp_before_${tag}.txt"
  run_warmup "$url"
  lp=$(run_load "$url")
  wait "$lp" 2>/dev/null || true
  snap > "$EV/network/snmp_after_${tag}.txt"
done

log "Phase 9: aggregate"
python3 <<PY > "$EV/reports/causal_summary.json"
import ast, json, re, statistics
from pathlib import Path
EV = Path("$EV")

def load_rows(name):
    p = EV / "perf-stat" / name
    if not p.exists(): return []
    rows = []
    for line in p.read_text().strip().split("\n")[1:]:
        tag, rep, rps, parsed = line.split(",", 3)
        d = ast.literal_eval(parsed)
        d["tag"] = tag; d["rep"] = int(rep); d["rps"] = float(rps)
        rows.append(d)
    return rows

rows = load_rows("raw_rows.txt") + load_rows("raw_rows_v2.txt")

def med(tag, key):
    vals = [r[key] for r in rows if r["tag"]==tag and key in r and r[key] is not None]
    return statistics.median(vals) if vals else None

def med_rps(tag):
    vals = [r["rps"] for r in rows if r["tag"]==tag and r["rps"]>0]
    return statistics.median(vals) if vals else None

BASELINE = {"exyonq":106199.35,"nginx":159901.99,"ols":131386.24}
out = {"per_request": {}, "median_rps_window": {}, "baseline_rps": BASELINE}

for tag in ["exyonq","nginx","ols"]:
    rps = med_rps(tag) or BASELINE[tag]
    reqs = rps * 30
    out["median_rps_window"][tag] = rps
    tc = med(tag,"task_clock_ms")
    out["per_request"][tag] = {
        "cycles": med(tag,"cycles")/reqs if med(tag,"cycles") else None,
        "instructions": med(tag,"instructions")/reqs if med(tag,"instructions") else None,
        "branches": med(tag,"branches")/reqs if med(tag,"branches") else None,
        "context_switches": med(tag,"context_switches")/reqs if med(tag,"context_switches") else None,
        "cpu_migrations": med(tag,"cpu_migrations")/reqs if med(tag,"cpu_migrations") else None,
        "cache_misses": med(tag,"cache_misses")/reqs if med(tag,"cache_misses") else None,
        "cpu_time_ns_per_req": (tc/reqs*1e6) if tc else None,
        "ipc": (med(tag,"instructions")/med(tag,"cycles")) if med(tag,"instructions") and med(tag,"cycles") else None,
    }

def parse_strace(path):
    tx = {}
    if not path.exists(): return tx
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[-1].isidentifier() if False else True:
            try:
                if parts[-1] in ('syscall','total'): continue
                tx[parts[-1]] = int(parts[-2])
            except: pass
    return tx

for tag, suffix in [("exyonq","tx.txt"),("nginx","tx-v2.txt"),("ols","tx-v2.txt")]:
    for name in [f"{tag}-strace-{suffix}", f"{tag}-strace-tx.txt", f"{tag}-strace-tx-v2.txt"]:
        p = EV / "syscalls" / name
        if p.exists():
            out.setdefault("strace", {})[tag] = parse_strace(p)
            break

# network Tcp OutSegs / RetransSegs
def tcp_delta(tag):
    b = EV / f"network/snmp_before_{tag}.txt"
    a = EV / f"network/snmp_after_{tag}.txt"
    if not b.exists() or not a.exists(): return {}
    def parse(p):
        for line in p.read_text().splitlines():
            if line.startswith('Tcp:'):
                hdr = line.split()
            elif line.startswith(' ') and 'hdr' in dir():
                vals = line.split()
                return dict(zip(hdr[1:], vals[1:]))
        return {}
    bb, aa = parse(b), parse(a)
    outd = {}
    for k in bb:
        if k in aa:
            try: outd[k] = int(aa[k]) - int(bb[k])
            except: pass
    return outd

out["network_delta"] = {t: tcp_delta(t) for t in ["exyonq","nginx","ols"]}
print(json.dumps(out, indent=2))
PY

log "DONE complete"
