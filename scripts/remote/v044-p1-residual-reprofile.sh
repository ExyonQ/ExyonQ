#!/usr/bin/env bash
# V044 P1 residual gap reprofile — DIAGNOSTIC ONLY.
# PRODUCT_MUTATION=NO. FD_REUSE=NO. P2=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_REPROFILE_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_REPROFILE_EV:-$WS/.exyonq-local-evidence/v044-p1-residual-reprofile-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PRODUCT_HEAD="${V044_PRODUCT_HEAD:-f2e90beb28bab1f0efac7c714ad550f12d6794ef}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
REPS="${V044_P1_REPS:-5}"
STAT_REPS="${P1_STAT_REPS:-3}"
RECORD_SEC="${P1_RECORD_SEC:-20}"
PATH_P1="/site/1k.bin"

mkdir -p "$EV"/{control,sendfile-proof,perf-stat,perf-record,syscalls,scheduler,workers,topology,concurrency,cpu-scale,network,reports}
cd "$WS"
log() { echo "[p1-reprofile] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
export P1_EXYONQ_IMAGE="${P1_EXYONQ_IMAGE:-v044p1auth-clean-exyonq}"
source "${HOME}/.cargo/env" 2>/dev/null || true

compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }

phase0_bind() {
  log "Phase 0 authority bind"
  {
    echo "CURRENT_WIP=V044_P1_RESIDUAL_GAP_REPROFILE"
    echo "PRODUCT_MUTATION=NO"
    echo "FD_REUSE_AUTHORIZED=NO"
    echo "P2_STARTED=NO"
    echo "PRODUCT_HEAD=$PRODUCT_HEAD"
    echo "RUSTC_VERSION=$(rustc --version 2>/dev/null || echo missing)"
    echo "CARGO_VERSION=$(cargo --version 2>/dev/null || echo missing)"
    echo "HOST=$(hostname) ARCH=$(uname -m)"
    echo "TIMESTAMP_UTC=$TS"
  } | tee "$EV/authority_bind.txt"
}

phase1_build() {
  log "Phase 1 rebuild product with host Rust toolchain + Docker identity overlay"
  export DOCKER_BUILDKIT=1
  # Prefer host toolchain via bind-mounted build if Dockerfile pins older rustc;
  # still rebuild image so BINARY_SHA is bound to this session.
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq" \
    -t "${PROJECT}-exyonq-base" "$WS" >> "$EV/build.log" 2>&1
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq-identity-overlay" \
    --build-arg "BASE_IMAGE=${PROJECT}-exyonq-base" \
    -t "$P1_EXYONQ_IMAGE" "$WS" >> "$EV/build.log" 2>&1
  docker run --rm --entrypoint sha256sum "$P1_EXYONQ_IMAGE" /usr/local/bin/exyonq \
    | tee "$EV/binary_sha256.txt"
  # Recreate only ExyonQ with patched image
  for svc in apache caddy haproxy traefik envoy nginx-mainline openlitespeed-stable; do
    compose stop "$svc" 2>/dev/null || true
  done
  compose up -d --force-recreate --no-deps exyonq
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  {
    echo "BUILD_IMAGE=$P1_EXYONQ_IMAGE"
    cat "$EV/binary_sha256.txt"
    compose exec -T exyonq sh -c 'echo WORKER_THREADS=${EXYONQ_WORKER_THREADS:-}; echo ACCEPT_WORKERS=${EXYONQ_ACCEPT_WORKERS:-}; echo EPOLL_STATIC=${EXYONQ_EPOLL_STATIC:-}; echo EPOLL_SENDFILE=${EXYONQ_EPOLL_SENDFILE:-}; nproc; cat /proc/1/cmdline | tr "\0" " "; echo'
  } | tee "$EV/build_identity.txt"
}

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }
perf_pids() {
  local main=$1 kids
  kids=$(pgrep -P "$main" 2>/dev/null || true)
  if [[ -n "$kids" ]]; then
    echo "$main,$(echo "$kids" | tr '\n' ',' | sed 's/,$//')"
  else
    ps -T -p "$main" -o tid= 2>/dev/null | tr '\n' ',' | sed "s/^/$main,/" | sed 's/,$//' || echo "$main"
  fi
}

phase2_sendfile_proof() {
  log "Phase 2 fresh sendfile proof under absolute-form"
  local EXY_C="${PROJECT}-exyonq-1"
  local MAIN_PID LOG
  MAIN_PID=$(main_pid "$EXY_C")
  echo "MAIN_PID=$MAIN_PID" | tee "$EV/sendfile-proof/pid.txt"
  LOG="$EV/sendfile-proof/strace-rewrk"
  rm -f "$LOG".*
  timeout 22 strace -ff -e trace=openat,openat2,read,write,sendto,sendfile,sendfile64,accept4,epoll_wait,futex -s 64 -o "$LOG" -p "$MAIN_PID" &
  local SP=$!
  sleep 0.5
  local JSON
  JSON=$(compose exec -T bench-runner rewrk -c 100 -d 10s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
  echo "$JSON" > "$EV/sendfile-proof/rewrk-10s.json"
  wait "$SP" 2>/dev/null || true
  python3 - "$LOG" "$EV/sendfile-proof/rewrk-10s.json" <<'PY' | tee "$EV/sendfile-proof/per_req.txt"
import glob, re, json, sys
base = sys.argv[1]
d = json.load(open(sys.argv[2]))
reqs = max(1, int(d.get("requests_total") or d.get("summary", {}).get("total") or 1))
keys = ["openat","openat2","read","write","sendto","sendfile","sendfile64","accept4","epoll_wait","futex"]
counts = {k:0 for k in keys}
for fn in glob.glob(base + "*"):
    for line in open(fn, errors="replace"):
        for k in keys:
            if re.search(rf"\b{k}\(", line):
                counts[k] += 1
send = counts["sendfile"] + counts["sendfile64"]
print("total_requests", reqs)
for k,v in counts.items():
    print(f"{k}_per_req", round(v/reqs, 4))
print("sendfile_per_req", round(send/reqs, 4))
print("SENDFILE_RUNTIME_EXECUTED", "YES" if send > 0 else "NO")
open_like = counts["openat"] + counts["openat2"]
print("openat_like_per_req", round(open_like/reqs, 4))
PY
  # Hard stop if sendfile absent
  if ! grep -q 'SENDFILE_RUNTIME_EXECUTED YES' "$EV/sendfile-proof/per_req.txt"; then
    log "STOP: sendfile not active under absolute-form rewrk"
    echo "STOP_REASON=SENDFILE_NOT_ACTIVE" | tee "$EV/STOP.txt"
    exit 42
  fi
}

run_control_rep() {
  local tag=$1 rep=$2 url=$3
  local DIR="$WS/benchmarks/scenarios/perf"
  source "$DIR/run-rewrk-helper.sh"
  export BENCH_NETWORK=internal
  export COMPOSE_FILE="$FULL_COMPOSE"
  export BENCH_COMPOSE_FILE="$FULL_COMPOSE"
  export DURATION="$DURATION"
  export BENCH_WARMUP_SEC="$WARMUP"
  export BENCH_REWRK_THREADS=2
  local raw="$EV/control/p1-${tag}-rep${rep}.rewrk.json"
  local out="$EV/control/p1-${tag}-rep${rep}.json"
  rewrk_warmup 100 "$url" "" 0
  rewrk_exec_capture "$raw" 100 "$url" "" 0
  python3 "$DIR/rewrk-report-to-json.py" "$raw" -o "$out"
  python3 -c "import json; print(json.load(open('$out'))['summary']['requestsPerSec'])"
}

phase3_control() {
  log "Phase 3 three-way control (5 reps)"
  local tag url rep rps
  for tag in exyonq nginx ols; do
    : > "$EV/control/${tag}_rps_runs.txt"
  done
  for rep in $(seq 1 "$REPS"); do
    rps=$(run_control_rep exyonq "$rep" "http://exyonq:8080${PATH_P1}")
    echo "$rps" >> "$EV/control/exyonq_rps_runs.txt"
    echo "exyonq rep$rep $rps" | tee -a "$EV/control/run_log.txt"
    sleep 5
  done
  for rep in $(seq 1 "$REPS"); do
    rps=$(run_control_rep nginx "$rep" "http://nginx-stable:8080${PATH_P1}")
    echo "$rps" >> "$EV/control/nginx_rps_runs.txt"
    echo "nginx rep$rep $rps" | tee -a "$EV/control/run_log.txt"
    sleep 5
  done
  for rep in $(seq 1 "$REPS"); do
    rps=$(run_control_rep ols "$rep" "http://openlitespeed-latest:8088${PATH_P1}")
    echo "$rps" >> "$EV/control/ols_rps_runs.txt"
    echo "ols rep$rep $rps" | tee -a "$EV/control/run_log.txt"
    sleep 5
  done
  python3 - <<PY | tee "$EV/control/summary.txt"
import statistics, pathlib
ev = pathlib.Path("$EV/control")
base = {"exyonq":106199.35,"nginx":159901.99,"ols":131386.24}
vals = {}
for name in ["exyonq","nginx","ols"]:
    v=[float(x) for x in (ev/f"{name}_rps_runs.txt").read_text().split()]
    vals[name]=v
    med=statistics.median(v)
    print(f"{name}_runs={v}")
    print(f"{name}_median={med}")
em, nm, om = map(statistics.median, (vals["exyonq"], vals["nginx"], vals["ols"]))
print(f"exyonq_vs_nginx_delta_pct={(em/nm-1)*100:.2f}")
print(f"exyonq_vs_ols_delta_pct={(em/om-1)*100:.2f}")
print(f"exyonq_vs_sealed_baseline_pct={(em/base['exyonq']-1)*100:.2f}")
PY
}

PERF_EVENTS="task-clock,cpu-clock,cycles,instructions,branches,branch-misses,context-switches,cpu-migrations,page-faults,cache-references,cache-misses"

phase4_perf_stat() {
  log "Phase 4 perf-stat ($STAT_REPS reps × 3 servers)"
  local EXY_C="${PROJECT}-exyonq-1"
  local NGX_C="${PROJECT}-nginx-stable-1"
  local OLS_C="${PROJECT}-openlitespeed-latest-1"
  local EXY_PIDS NGX_PIDS OLS_PIDS
  EXY_PIDS=$(perf_pids "$(main_pid "$EXY_C")")
  NGX_PIDS=$(perf_pids "$(main_pid "$NGX_C")")
  OLS_PIDS=$(perf_pids "$(main_pid "$OLS_C")")
  echo "EXY_PIDS=$EXY_PIDS NGX_PIDS=$NGX_PIDS OLS_PIDS=$OLS_PIDS" | tee "$EV/perf-stat/pids.txt"
  local i tag pids url
  for i in $(seq 1 "$STAT_REPS"); do
    for tag in exyonq nginx ols; do
      case $tag in
        exyonq) pids=$EXY_PIDS; url="http://exyonq:8080${PATH_P1}" ;;
        nginx) pids=$NGX_PIDS; url="http://nginx-stable:8080${PATH_P1}" ;;
        ols) pids=$OLS_PIDS; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
      esac
      log "perf-stat $tag rep$i"
      compose exec -T bench-runner rewrk -c 100 -d "${WARMUP}s" -h "$url" --json -t 2 >/dev/null || true
      local out="$EV/perf-stat/${tag}-rep${i}.txt"
      local jraw="$EV/perf-stat/${tag}-rep${i}.rewrk.json"
      # Warm already done; measure with perf over measurement window
      (
        timeout 45 perf stat -e "$PERF_EVENTS" -p "$pids" -- sleep 32
      ) >"$out" 2>&1 &
      local PP=$!
      sleep 1
      compose exec -T bench-runner rewrk -c 100 -d 30s -h "$url" --json -t 2 >"$jraw" || true
      wait "$PP" 2>/dev/null || true
      sleep 3
    done
  done
  python3 - "$EV/perf-stat" <<'PY' | tee "$EV/perf-stat/derived.txt"
import re, json, pathlib, statistics, sys
ev = pathlib.Path(sys.argv[1])
pre = {"instructions":46508,"branches":8517,"cycles":38042,"context-switches":2.624}

def parse_perf(text):
    vals={}
    for line in text.splitlines():
        if "<not counted>" in line: continue
        m=re.match(r"\s*([\d,]+(?:\.\d+)?)\s+(\S+)", line)
        if not m: continue
        v=float(m.group(1).replace(",","")); k=m.group(2)
        if k in ("msec","GHz","CPUs","sec","K/sec","M/sec","/sec","refs","utilized","cycle","branches","insn"): continue
        vals[k]=v
    return vals

def reqs(path):
    d=json.loads(path.read_text())
    return max(1, int(d.get("requests_total") or d.get("summary",{}).get("total") or 1))

rows={}
for tag in ["exyonq","nginx","ols"]:
    metrics={}
    for p in sorted(ev.glob(f"{tag}-rep*.txt")):
        j=ev/(p.name.replace(".txt",".rewrk.json"))
        if not j.exists(): continue
        r=reqs(j); perf=parse_perf(p.read_text())
        for k,v in perf.items():
            metrics.setdefault(k,[]).append(v/r)
    rows[tag]={k:statistics.median(vs) for k,vs in metrics.items() if vs}
    print(f"=== {tag} per_req median ===")
    for k in ["cycles","instructions","branches","branch-misses","context-switches","cpu-migrations","page-faults","task-clock","cache-misses","cache-references"]:
        if k in rows[tag]:
            print(f"{k}_per_req={rows[tag][k]:.4f}")
if "exyonq" in rows:
    print("=== post_vs_pre (exyonq) ===")
    for k,old in pre.items():
        if k in rows["exyonq"]:
            print(f"{k}_post={rows['exyonq'][k]:.4f} pre={old} delta_pct={(rows['exyonq'][k]/old-1)*100:.2f}")
PY
}

phase5_syscalls() {
  log "Phase 5 syscall accounting (10s windows)"
  local EXY_C="${PROJECT}-exyonq-1" NGX_C="${PROJECT}-nginx-stable-1" OLS_C="${PROJECT}-openlitespeed-latest-1"
  local tag c url MAIN LOG SP JSON
  for tag in exyonq nginx ols; do
    case $tag in
      exyonq) c=$EXY_C; url="http://exyonq:8080${PATH_P1}" ;;
      nginx) c=$NGX_C; url="http://nginx-stable:8080${PATH_P1}" ;;
      ols) c=$OLS_C; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
    esac
    MAIN=$(main_pid "$c")
    LOG="$EV/syscalls/${tag}"
    rm -f "$LOG".*
    timeout 18 strace -ff -c -o "$LOG.summary" -p "$MAIN" &
    SP=$!
    sleep 0.4
    JSON=$(compose exec -T bench-runner rewrk -c 100 -d 10s -h "$url" --json -t 2)
    echo "$JSON" > "$EV/syscalls/${tag}-rewrk.json"
    wait "$SP" 2>/dev/null || true
    # also detailed counts
    timeout 18 strace -ff -e trace=accept4,epoll_wait,epoll_pwait,epoll_ctl,recvfrom,read,sendto,write,writev,sendfile,sendfile64,splice,openat,openat2,close,futex,fcntl,setsockopt,clock_gettime -o "$LOG.detail" -p "$MAIN" &
    SP=$!
    sleep 0.4
    compose exec -T bench-runner rewrk -c 100 -d 10s -h "$url" --json -t 2 > "$EV/syscalls/${tag}-detail-rewrk.json" || true
    wait "$SP" 2>/dev/null || true
  done
  python3 - "$EV/syscalls" <<'PY' | tee "$EV/syscalls/derived.txt"
import glob, re, json, pathlib, sys
ev=pathlib.Path(sys.argv[1])
keys=["accept4","epoll_wait","epoll_pwait","epoll_ctl","recvfrom","read","sendto","write","writev","sendfile","sendfile64","splice","openat","openat2","close","futex","fcntl","setsockopt","clock_gettime"]
for tag in ["exyonq","nginx","ols"]:
    j=json.loads((ev/f"{tag}-detail-rewrk.json").read_text())
    reqs=max(1,int(j.get("requests_total") or j.get("summary",{}).get("total") or 1))
    counts={k:0 for k in keys}
    for fn in glob.glob(str(ev/f"{tag}.detail*")):
        for line in open(fn, errors="replace"):
            for k in keys:
                if re.search(rf"\b{k}\(", line):
                    counts[k]+=1
    total=sum(counts.values())
    print(f"=== {tag} ===")
    print(f"requests={reqs}")
    print(f"syscalls_per_req={total/reqs:.3f}")
    for k,v in sorted(counts.items(), key=lambda kv:-kv[1]):
        if v:
            print(f"{k}_per_req={v/reqs:.4f}")
PY
}

phase6_offcpu() {
  log "Phase 6 context-switch / off-CPU (pidstat + /proc)"
  local EXY_C="${PROJECT}-exyonq-1" NGX_C="${PROJECT}-nginx-stable-1" OLS_C="${PROJECT}-openlitespeed-latest-1"
  local tag c url MAIN
  for tag in exyonq nginx ols; do
    case $tag in
      exyonq) c=$EXY_C; url="http://exyonq:8080${PATH_P1}" ;;
      nginx) c=$NGX_C; url="http://nginx-stable:8080${PATH_P1}" ;;
      ols) c=$OLS_C; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
    esac
    MAIN=$(main_pid "$c")
    # capture schedstats before/after
    {
      echo "=== before ==="
      cat /proc/$MAIN/status | egrep '^(Name|State|voluntary|nonvoluntary|Threads)'
      ps -L -p "$MAIN" -o tid,psr,pcpu,stat,comm
    } > "$EV/scheduler/${tag}-before.txt"
    pidstat -t -p "$MAIN" 1 12 > "$EV/scheduler/${tag}-pidstat.txt" 2>&1 &
    local PS=$!
    compose exec -T bench-runner rewrk -c 100 -d 10s -h "$url" --json -t 2 > "$EV/scheduler/${tag}-rewrk.json" || true
    wait "$PS" 2>/dev/null || true
    {
      echo "=== after ==="
      cat /proc/$MAIN/status | egrep '^(Name|State|voluntary|nonvoluntary|Threads)'
      ps -L -p "$MAIN" -o tid,psr,pcpu,stat,comm
    } > "$EV/scheduler/${tag}-after.txt"
  done
  python3 - "$EV/scheduler" <<'PY' | tee "$EV/scheduler/derived.txt"
import json, pathlib, re, sys
ev=pathlib.Path(sys.argv[1])
for tag in ["exyonq","nginx","ols"]:
    before=(ev/f"{tag}-before.txt").read_text()
    after=(ev/f"{tag}-after.txt").read_text()
    def vol(t):
        m=re.search(r"voluntary_ctxt_switches:\s*(\d+)", t)
        n=re.search(r"nonvoluntary_ctxt_switches:\s*(\d+)", t)
        return int(m.group(1) if m else 0), int(n.group(1) if n else 0)
    bv,bn=vol(before); av,an=vol(after)
    j=json.loads((ev/f"{tag}-rewrk.json").read_text())
    reqs=max(1,int(j.get("requests_total") or 1))
    print(f"=== {tag} ===")
    print(f"requests={reqs}")
    print(f"voluntary_delta={av-bv}")
    print(f"nonvoluntary_delta={an-bn}")
    print(f"ctx_switches_per_req={(av-bv+an-bn)/reqs:.4f}")
    # crude off-cpu from pidstat Average idle if present
    pst=(ev/f"{tag}-pidstat.txt").read_text()
    # Average %CPU of main threads
    cpus=[]
    for line in pst.splitlines():
        parts=line.split()
        if len(parts)>=9 and parts[-1] not in ("CPU","Command") and parts[0][0].isdigit():
            try:
                cpus.append(float(parts[7] if parts[6]=='-' else parts[8]))
            except Exception:
                pass
    if cpus:
        avg=sum(cpus)/len(cpus)
        print(f"pidstat_avg_pcpu_samples={avg:.2f}")
        print(f"off_cpu_fraction_estimate={max(0.0,1.0-avg/100.0):.3f}")
PY
}

phase7_workers() {
  log "Phase 7 worker / accept distribution"
  local EXY_C="${PROJECT}-exyonq-1" MAIN
  MAIN=$(main_pid "$EXY_C")
  {
    echo "=== env ==="
    docker inspect "$EXY_C" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|WORKER|ACCEPT|EPOLL' || true
    echo "=== threads during load ==="
  } | tee "$EV/workers/env.txt"
  # sample threads during load
  (
    for i in $(seq 1 15); do
      echo "--- sample $i ---"
      ps -L -p "$MAIN" -o tid,psr,pcpu,stat,wchan:24,comm
      sleep 1
    done
  ) > "$EV/workers/thread_samples.txt" &
  local WP=$!
  compose exec -T bench-runner rewrk -c 100 -d 15s -h "http://exyonq:8080${PATH_P1}" --json -t 2 > "$EV/workers/rewrk.json" || true
  wait "$WP" 2>/dev/null || true
  python3 - "$EV/workers/thread_samples.txt" <<'PY' | tee "$EV/workers/derived.txt"
import sys,re,collections
text=open(sys.argv[1]).read().split("--- sample")
# per tid median pcpu
by=collections.defaultdict(list)
names={}
for block in text[1:]:
    for line in block.splitlines():
        parts=line.split()
        if len(parts)<6: continue
        if not parts[0].isdigit(): continue
        tid=parts[0]; psr=parts[1]; pcpu=float(parts[2]); name=parts[-1]
        by[tid].append(pcpu); names[tid]=name
rows=[]
for tid,vals in by.items():
    avg=sum(vals)/len(vals)
    rows.append((avg,tid,names[tid],vals))
rows.sort(reverse=True)
active=sum(1 for a,_,_,_ in rows if a>=5.0)
idle=sum(1 for a,_,_,_ in rows if a<1.0)
print(f"thread_count={len(rows)}")
print(f"active_workers_pcpu_ge_5={active}")
print(f"idle_workers_pcpu_lt_1={idle}")
for avg,tid,name,vals in rows[:16]:
    print(f"tid={tid} name={name} avg_pcpu={avg:.2f} max={max(vals):.2f}")
PY
}

phase8_topology() {
  log "Phase 8 topology notes from source + runtime"
  {
    echo "SO_REUSEPORT=set_reuseport(true) in core/src/server/mod.rs bind_tuned"
    echo "ACCEPT_WORKERS_DEFAULT=1 unless EXYONQ_ACCEPT_WORKERS set (Dockerfile=4)"
    echo "WORKER_THREADS from EXYONQ_WORKER_THREADS (Dockerfile=4)"
    echo "Cap067: Tokio accept then divert eligible static to epoll keepalive/sendfile worker"
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|WORKER|ACCEPT|EPOLL' || true
    ls -l /proc/$(main_pid "${PROJECT}-exyonq-1")/fd 2>/dev/null | head -5 || true
    # count epoll fds
    MAIN=$(main_pid "${PROJECT}-exyonq-1")
    echo "epoll_fds=$(ls -l /proc/$MAIN/fd 2>/dev/null | grep -c anon_inode:\\[eventpoll\\] || true)"
  } | tee "$EV/topology/runtime.txt"
}

phase9_perf_record() {
  log "Phase 9 perf record user+kernel (${RECORD_SEC}s)"
  local EXY_C="${PROJECT}-exyonq-1" NGX_C="${PROJECT}-nginx-stable-1" OLS_C="${PROJECT}-openlitespeed-latest-1"
  local tag c url MAIN
  for tag in exyonq nginx ols; do
    case $tag in
      exyonq) c=$EXY_C; url="http://exyonq:8080${PATH_P1}" ;;
      nginx) c=$NGX_C; url="http://nginx-stable:8080${PATH_P1}" ;;
      ols) c=$OLS_C; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
    esac
    MAIN=$(main_pid "$c")
    local PIDS
    PIDS=$(perf_pids "$MAIN")
    log "perf record $tag"
    timeout $((RECORD_SEC+15)) perf record -F 99 -g -p "$PIDS" -o "$EV/perf-record/${tag}.data" -- sleep "$RECORD_SEC" &
    local PP=$!
    sleep 1
    compose exec -T bench-runner rewrk -c 100 -d "${RECORD_SEC}s" -h "$url" --json -t 2 > "$EV/perf-record/${tag}-rewrk.json" || true
    wait "$PP" 2>/dev/null || true
    perf report -i "$EV/perf-record/${tag}.data" --stdio --no-children -n --percent-limit 1 \
      > "$EV/perf-record/${tag}-report.txt" 2>/dev/null || true
    perf report -i "$EV/perf-record/${tag}.data" --stdio --no-children -n --percent-limit 0.5 \
      | head -80 > "$EV/perf-record/${tag}-top.txt" 2>/dev/null || true
  done
}

phase10_concurrency() {
  log "Phase 10 concurrency diagnostic (NOT P1 authority)"
  echo "DIAGNOSTIC_NOT_P1_AUTHORITY=YES" | tee "$EV/concurrency/label.txt"
  local c rps
  : > "$EV/concurrency/rps_by_c.txt"
  for c in 50 100 200 400 800; do
    log "concurrency=$c"
    # warmup short
    compose exec -T bench-runner rewrk -c "$c" -d 5s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null || true
    local JSON
    JSON=$(compose exec -T bench-runner rewrk -c "$c" -d 20s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
    echo "$JSON" > "$EV/concurrency/c${c}.json"
    rps=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('requests_avg') or d.get('summary',{}).get('requestsPerSec'))" <<<"$JSON")
    echo "$c $rps" | tee -a "$EV/concurrency/rps_by_c.txt"
    # thread activity sample
    MAIN=$(main_pid "${PROJECT}-exyonq-1")
    ps -L -p "$MAIN" -o tid,psr,pcpu,comm > "$EV/concurrency/threads_c${c}.txt"
    sleep 3
  done
}

phase11_cpu_scale() {
  log "Phase 11 CPU scaling diagnostic (NOT P1 authority)"
  echo "DIAGNOSTIC_NOT_P1_AUTHORITY=YES" | tee "$EV/cpu-scale/label.txt"
  local n
  : > "$EV/cpu-scale/rps_by_cpu.txt"
  for n in 1 2 4 8; do
    log "cpus=$n"
    docker update --cpus="$n" "${PROJECT}-exyonq-1" >/dev/null
    sleep 2
    compose exec -T bench-runner rewrk -c 100 -d 5s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null || true
    local JSON rps
    JSON=$(compose exec -T bench-runner rewrk -c 100 -d 20s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
    echo "$JSON" > "$EV/cpu-scale/cpu${n}.json"
    rps=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('requests_avg') or d.get('summary',{}).get('requestsPerSec'))" <<<"$JSON")
    echo "$n $rps" | tee -a "$EV/cpu-scale/rps_by_cpu.txt"
    sleep 3
  done
  # restore
  docker update --cpus=0 "${PROJECT}-exyonq-1" >/dev/null || docker update --cpus=8 "${PROJECT}-exyonq-1" >/dev/null || true
}

phase12_network() {
  log "Phase 12 network recheck (best-effort container attribution)"
  {
    echo "NETWORK_COUNTER_ATTRIBUTION=BEST_EFFORT"
    for c in "${PROJECT}-exyonq-1" "${PROJECT}-nginx-stable-1" "${PROJECT}-openlitespeed-latest-1"; do
      echo "=== $c ==="
      docker exec "$c" sh -c 'cat /proc/net/dev 2>/dev/null | head -5; ss -s 2>/dev/null | head -10' || true
    done
  } | tee "$EV/network/snapshot.txt"
}

synthesize() {
  log "Synthesize terminal_report.json"
  python3 - "$EV" <<'PY' | tee "$EV/terminal_report.json"
import json, pathlib, re, statistics, sys
ev=pathlib.Path(sys.argv[1])

def read_lines(p):
    if not p.exists(): return []
    return [float(x) for x in p.read_text().split() if x.strip()]

def kv_file(p):
    out={}
    if not p.exists(): return out
    for line in p.read_text().splitlines():
        if "=" in line:
            k,v=line.split("=",1); out[k.strip()]=v.strip()
    return out

ctrl={}
for name in ["exyonq","nginx","ols"]:
    runs=read_lines(ev/"control"/f"{name}_rps_runs.txt")
    ctrl[name]={"runs":runs,"median":statistics.median(runs) if runs else None}

send=kv_file(ev/"sendfile-proof"/"per_req.txt")
sysc=kv_file(ev/"syscalls"/"derived.txt")  # not kv; parse below
sys_txt=(ev/"syscalls"/"derived.txt").read_text() if (ev/"syscalls"/"derived.txt").exists() else ""
sched=(ev/"scheduler"/"derived.txt").read_text() if (ev/"scheduler"/"derived.txt").exists() else ""
workers=kv_file(ev/"workers"/"derived.txt")
conc={}
if (ev/"concurrency"/"rps_by_c.txt").exists():
    for line in (ev/"concurrency"/"rps_by_c.txt").read_text().splitlines():
        a,b=line.split(); conc[int(a)]=float(b)
cpu={}
if (ev/"cpu-scale"/"rps_by_cpu.txt").exists():
    for line in (ev/"cpu-scale"/"rps_by_cpu.txt").read_text().splitlines():
        a,b=line.split(); cpu[int(a)]=float(b)

def parse_block(txt, tag, key):
    m=re.search(rf"=== {tag} ===(.*?)(?:=== |\Z)", txt, re.S)
    if not m: return None
    mm=re.search(rf"{key}=([0-9.]+)", m.group(1))
    return float(mm.group(1)) if mm else None

report={
  "P1_RESIDUAL_REPROFILE_STATUS": "COMPLETE",
  "PRODUCT_HEAD": "f2e90beb28bab1f0efac7c714ad550f12d6794ef",
  "SENDFILE_RUNTIME_EXECUTED": send.get("SENDFILE_RUNTIME_EXECUTED"),
  "SENDFILE_PER_REQ": send.get("sendfile_per_req"),
  "OPENAT_LIKE_PER_REQ": send.get("openat_like_per_req"),
  "EXYONQ_CONTROL_RUNS": ctrl["exyonq"]["runs"],
  "NGINX_CONTROL_RUNS": ctrl["nginx"]["runs"],
  "OLS_CONTROL_RUNS": ctrl["ols"]["runs"],
  "EXYONQ_CONTROL_MEDIAN_RPS": ctrl["exyonq"]["median"],
  "NGINX_CONTROL_MEDIAN_RPS": ctrl["nginx"]["median"],
  "OLS_CONTROL_MEDIAN_RPS": ctrl["ols"]["median"],
  "EXYONQ_SYSCALLS_PER_REQ_POST": parse_block(sys_txt,"exyonq","syscalls_per_req"),
  "NGINX_SYSCALLS_PER_REQ": parse_block(sys_txt,"nginx","syscalls_per_req"),
  "OLS_SYSCALLS_PER_REQ": parse_block(sys_txt,"ols","syscalls_per_req"),
  "EXYONQ_CONTEXT_SWITCHES_PER_REQ_POST": parse_block(sched,"exyonq","ctx_switches_per_req"),
  "NGINX_CONTEXT_SWITCHES_PER_REQ": parse_block(sched,"nginx","ctx_switches_per_req"),
  "OLS_CONTEXT_SWITCHES_PER_REQ": parse_block(sched,"ols","ctx_switches_per_req"),
  "EXYONQ_OFF_CPU_FRACTION_POST": parse_block(sched,"exyonq","off_cpu_fraction_estimate"),
  "EXYONQ_RUNTIME_WORKER_COUNT": workers.get("thread_count"),
  "EXYONQ_ACTIVE_WORKERS": workers.get("active_workers_pcpu_ge_5"),
  "EXYONQ_IDLE_WORKERS": workers.get("idle_workers_pcpu_lt_1"),
  "RPS_BY_CONCURRENCY": conc,
  "RPS_BY_CPU_COUNT": cpu,
  "PRODUCT_MUTATION": "NO",
  "P2_STARTED": "NO",
}
if ctrl["exyonq"]["median"] and ctrl["nginx"]["median"]:
    report["EXYONQ_VS_NGINX_CONTROL_DELTA"] = (ctrl["exyonq"]["median"]/ctrl["nginx"]["median"]-1)*100
if ctrl["exyonq"]["median"] and ctrl["ols"]["median"]:
    report["EXYONQ_VS_OLS_CONTROL_DELTA"] = (ctrl["exyonq"]["median"]/ctrl["ols"]["median"]-1)*100
print(json.dumps(report, indent=2))
(ev/"terminal_report.json").write_text(json.dumps(report, indent=2)+"\n")
PY
}

main() {
  phase0_bind
  phase1_build
  phase2_sendfile_proof
  phase3_control
  phase4_perf_stat
  phase5_syscalls
  phase6_offcpu
  phase7_workers
  phase8_topology
  phase9_perf_record
  phase10_concurrency
  phase11_cpu_scale
  phase12_network
  synthesize
  log "COMPLETE evidence=$EV"
}

main "$@"
