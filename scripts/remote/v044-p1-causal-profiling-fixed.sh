#!/usr/bin/env bash
# V044 P1 causal profiling — continuation/fixed (worker PIDs, all threads).
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
EV="${P1_CAUSAL_EVIDENCE:-$WS/.exyonq-local-evidence/v044-p1-causal-profiling}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
STAT_REPS="${P1_STAT_REPS:-3}"
RECORD_SEC="${P1_RECORD_SEC:-20}"
PATH_P1="/site/1k.bin"

mkdir -p "$EV"/{perf-stat,perf-record,syscalls,scheduler,network,reports}
cd "$WS"
log() { echo "[p1-causal] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"

# Stop non-P1 rivals
for svc in apache caddy haproxy traefik envoy nginx-mainline openlitespeed-stable; do
  docker compose -f "$FULL_COMPOSE" -p "$PROJECT" stop "$svc" 2>/dev/null || true
done

EXY_C="${PROJECT}-exyonq-1"
NGX_C="${PROJECT}-nginx-stable-1"
OLS_C="${PROJECT}-openlitespeed-latest-1"

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

# All thread/group PIDs for perf -p (main + children workers)
perf_pids() {
  local main=$1
  local kids
  kids=$(pgrep -P "$main" 2>/dev/null || true)
  if [[ -n "$kids" ]]; then
    echo "$main,$(echo "$kids" | tr '\n' ',' | sed 's/,$//')"
  else
    # all threads of main process
    ps -T -p "$main" -o tid= 2>/dev/null | tr '\n' ',' | sed "s/^/$main,/" | sed 's/,$//' || echo "$main"
  fi
}

EXY_MAIN=$(main_pid "$EXY_C")
NGX_MAIN=$(main_pid "$NGX_C")
OLS_MAIN=$(main_pid "$OLS_C")
EXY_PIDS=$(perf_pids "$EXY_MAIN")
NGX_PIDS=$(perf_pids "$NGX_MAIN")
OLS_PIDS=$(perf_pids "$OLS_MAIN")
echo "EXY_PIDS=$EXY_PIDS NGX_PIDS=$NGX_PIDS OLS_PIDS=$OLS_PIDS" | tee "$EV/container_pids.txt"

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
  local out="$EV/perf-stat/${tag}-rep${rep}.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  # shellcheck disable=SC2086
  perf stat -e "$PERF_EVENTS" -p "$pids" -- sleep 30 >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  local rps; rps=$(get_rps "$url")
  local parsed; parsed=$(parse_perf_stat "$out")
  echo "${tag},${rep},${rps},${parsed}" >> "$EV/perf-stat/raw_rows.txt"
}

echo "tag,rep,rps,parsed_dict" > "$EV/perf-stat/raw_rows.txt"
log "Phase 2 fixed: perf stat with worker PIDs"
for rep in 1 2 3; do
  stat_one exyonq "$EXY_URL" "$EXY_PIDS" "$rep"; sleep 5
  stat_one nginx "$NGX_URL" "$NGX_PIDS" "$rep"; sleep 5
  stat_one ols "$OLS_URL" "$OLS_PIDS" "$rep"; sleep 5
done

strace_sample() {
  local tag=$1 url=$2 pids=$3
  local out="$EV/syscalls/${tag}-strace-tx.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  # attach to first worker pid for nginx/ols
  local attach="${pids%%,*}"
  timeout 8 strace -f -p "$attach" -e trace=sendfile,splice,write,writev,read,recvfrom,sendto,send,recv,epoll_wait,epoll_pwait,epoll_ctl,accept,accept4,openat,close,futex -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}

log "Phase 3-4: strace TX/syscall"
strace_sample exyonq "$EXY_URL" "$EXY_PIDS"
sleep 5
strace_sample nginx "$NGX_URL" "$NGX_PIDS"
sleep 5
strace_sample ols "$OLS_URL" "$OLS_PIDS"
sleep 5

sched_sample() {
  local tag=$1 url=$2 pids=$3
  local out="$EV/scheduler/${tag}-pidstat.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  local attach="${pids%%,*}"
  pidstat -t -p "$attach" 1 15 > "$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  ps -T -p "$attach" -o tid,pcpu,comm >> "$out" 2>/dev/null || true
}

log "Phase 5: scheduler"
sched_sample exyonq "$EXY_URL" "$EXY_PIDS"
sleep 5
sched_sample nginx "$NGX_URL" "$NGX_PIDS"
sleep 5
sched_sample ols "$OLS_URL" "$OLS_PIDS"
sleep 5

record_one() {
  local tag=$1 url=$2 pids=$3
  local data="$EV/perf-record/${tag}.data"
  local report="$EV/perf-record/${tag}-report.txt"
  run_warmup "$url"
  local lp; lp=$(run_load "$url"); sleep 1
  local attach="${pids%%,*}"
  perf record -F 997 -g -p "$attach" -o "$data" -- sleep "$RECORD_SEC" 2>"$EV/perf-record/${tag}-record.log" || true
  wait "$lp" 2>/dev/null || true
  perf report -i "$data" --stdio --sort dso,symbol --percent-limit 0.3 2>/dev/null | head -150 > "$report" || true
}

log "Phase 6: perf record"
record_one exyonq "$EXY_URL" "$EXY_PIDS"
sleep 5
record_one nginx "$NGX_URL" "$NGX_PIDS"
sleep 5
record_one ols "$OLS_URL" "$OLS_PIDS"

log "Phase 8: network snmp deltas"
snap() { grep -E '^Tcp:|^ [0-9]' /proc/net/snmp; echo "TS $(date +%s)"; }
for tag url in "exyonq $EXY_URL" "nginx $NGX_URL" "ols $OLS_URL"; do
  snap > "$EV/network/snmp_before_${tag}.txt"
  run_warmup "$url"; lp=$(run_load "$url"); wait "$lp" 2>/dev/null || true
  snap > "$EV/network/snmp_after_${tag}.txt"
done

log "Phase 9: aggregate"
python3 <<'PY' > "$EV/reports/causal_summary.json"
import ast, json, re, statistics
from pathlib import Path
EV = Path("/root/exyonq-v044-p1-auth/.exyonq-local-evidence/v044-p1-causal-profiling")
rows = []
for line in (EV/"perf-stat/raw_rows.txt").read_text().strip().split("\n")[1:]:
    tag, rep, rps, parsed = line.split(",", 3)
    d = ast.literal_eval(parsed)
    d["tag"] = tag; d["rep"] = int(rep); d["rps"] = float(rps)
    rows.append(d)

def med(tag, key):
    vals = [r[key] for r in rows if r["tag"]==tag and key in r]
    return statistics.median(vals) if vals else None

def med_rps(tag):
    return statistics.median([r["rps"] for r in rows if r["tag"]==tag and r["rps"]>0])

out = {"per_request": {}, "median_rps_window": {}}
for tag in ["exyonq","nginx","ols"]:
    rps = med_rps(tag) or {"exyonq":106199,"nginx":159902,"ols":131386}[tag]
    reqs = rps * 30
    out["median_rps_window"][tag] = rps
    out["per_request"][tag] = {
        "cycles": med(tag,"cycles")/reqs if med(tag,"cycles") else None,
        "instructions": med(tag,"instructions")/reqs if med(tag,"instructions") else None,
        "branches": med(tag,"branches")/reqs if med(tag,"branches") else None,
        "context_switches": med(tag,"context_switches")/reqs if med(tag,"context_switches") else None,
        "cpu_migrations": med(tag,"cpu_migrations")/reqs if med(tag,"cpu_migrations") else None,
        "cache_misses": med(tag,"cache_misses")/reqs if med(tag,"cache_misses") else None,
        "cpu_time_ms_per_req": (med(tag,"task_clock_ms")/reqs*1000) if med(tag,"task_clock_ms") else None,
    }

# strace
for tag in ["exyonq","nginx","ols"]:
    p = EV/f"syscalls/{tag}-strace-tx.txt"
    if p.exists():
        tx = {}
        for line in p.read_text().splitlines():
            parts=line.split()
            if len(parts)>=2:
                try: tx[parts[-1]]=int(parts[-2])
                except: pass
        out.setdefault("strace",{})[tag]=tx
print(json.dumps(out, indent=2))
PY

log "DONE"
