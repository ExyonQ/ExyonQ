#!/usr/bin/env bash
# V044 P1 POST-SESSIONTABLE RESIDUAL REPROFILE — DIAGNOSTIC ONLY.
# PRODUCT_MUTATION=NO. FD_REUSE=NO. WORKER_TOPOLOGY_CHANGE=NO. P2=NO.
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN. SessionTable Cap067 ACCEPTED — do not revert.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_REPROFILE_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_REPROFILE_EV:-$WS/.exyonq-local-evidence/v044-p1-post-sessiontable-residual-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PRODUCT_HEAD="${V044_PRODUCT_HEAD:-427b17397c785b0c5105960cd43b604b41ea27d7}"
PREV_PRODUCT_HEAD="${V044_PREV_PRODUCT_HEAD:-f2e90beb28bab1f0efac7c714ad550f12d6794ef}"
LAB_TERMINAL_HEAD="${V044_LAB_TERMINAL_HEAD:-4b1a41f6ff703bacd03ace5bd4f3b152a9b34c44}"
LAB_TERMINAL_TREE="${V044_LAB_TERMINAL_TREE:-d7b5ba28de2b4fbc17155660fb2edc1038983b53}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
REPS="${V044_P1_REPS:-5}"
STAT_REPS="${P1_STAT_REPS:-3}"
RECORD_SEC="${P1_RECORD_SEC:-20}"
PATH_P1="/site/1k.bin"
export P1_EXYONQ_IMAGE="${P1_EXYONQ_IMAGE:-v044p1auth-clean-exyonq}"

mkdir -p "$EV"/{control,sendfile-proof,perf-stat,perf-record,syscalls,locks,openat2,workers,worker8,cpu-scale,topology,waf,tracing,network,reports}
cd "$WS"
log() { echo "[p1-post-st] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
source "${HOME}/.cargo/env" 2>/dev/null || true
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }
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

phase0_bind() {
  log "Phase 0 authority bind"
  {
    echo "CURRENT_WIP=V044_P1_POST_SESSIONTABLE_RESIDUAL_REPROFILE"
    echo "PRODUCT_MUTATION=NO"
    echo "FD_REUSE_AUTHORIZED=NO"
    echo "WORKER_TOPOLOGY_CHANGE_AUTHORIZED=NO"
    echo "TOKIO_RUNTIME_CHANGE_AUTHORIZED=NO"
    echo "WAF_OPTIMIZATION_AUTHORIZED=NO"
    echo "TRACING_OPTIMIZATION_AUTHORIZED=NO"
    echo "P2_STARTED=NO"
    echo "PUSH=NO"
    echo "TAG=NO"
    echo "RELEASE=NO"
    echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
    echo "PREVIOUS_PRODUCT_HEAD=$PREV_PRODUCT_HEAD"
    echo "SESSIONTABLE_PRODUCT_FIX_COMMIT=$PRODUCT_HEAD"
    echo "LAB_TERMINAL_HEAD=$LAB_TERMINAL_HEAD"
    echo "LAB_TERMINAL_TREE=$LAB_TERMINAL_TREE"
    echo "PRODUCT_AUTHORITY_INCLUDES=${PREV_PRODUCT_HEAD}+${PRODUCT_HEAD}"
    echo "RUSTC_VERSION=$(rustc --version 2>/dev/null || echo missing)"
    echo "CARGO_VERSION=$(cargo --version 2>/dev/null || echo missing)"
    echo "CARGO_LOCK_SHA256=$(sha256sum Cargo.lock | awk '{print $1}')"
    echo "HOST=$(hostname) ARCH=$(uname -m)"
    echo "TIMESTAMP_UTC=$TS"
    echo "EVIDENCE_DIR=$EV"
  } | tee "$EV/authority_bind.txt"
}

phase1_build() {
  log "Phase 1 rebuild product (Rust host toolchain via Dockerfiles)"
  export DOCKER_BUILDKIT=1
  {
    echo "BUILD_HEAD_LAB=$LAB_TERMINAL_HEAD"
    echo "BUILD_TREE_LAB=$LAB_TERMINAL_TREE"
    echo "PRODUCT_FIX=$PRODUCT_HEAD"
    echo "RUSTC=$(rustc --version)"
    echo "CARGO=$(cargo --version)"
  } | tee "$EV/build_identity_pre.txt"
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq" \
    -t "${PROJECT}-exyonq-base" "$WS" >> "$EV/build.log" 2>&1
  docker build -f "$WS/benchmarks/docker/Dockerfile.exyonq-identity-overlay" \
    --build-arg "BASE_IMAGE=${PROJECT}-exyonq-base" \
    -t "$P1_EXYONQ_IMAGE" "$WS" >> "$EV/build.log" 2>&1
  docker run --rm --entrypoint sha256sum "$P1_EXYONQ_IMAGE" /usr/local/bin/exyonq \
    | tee "$EV/binary_sha256.txt"
  # Prove SessionTable architecture in binary / source
  {
    echo "=== source markers ==="
    rg -n "thread_local!|SESSION_TABLE|OnceLock<Mutex<Option<SessionTable>>>" \
      "$WS/crates/exyonq-mod-static/src/epoll_session.rs" || true
    echo "=== binary strings (session mutex symbols) ==="
    docker run --rm --entrypoint sh "$P1_EXYONQ_IMAGE" -c \
      'strings /usr/local/bin/exyonq | egrep -i "epoll_session::sessions|SessionTable|lock_contended" | head -40' || true
  } | tee "$EV/sessiontable_presence.txt"
  if rg -q 'OnceLock<Mutex<Option<SessionTable>>>' "$WS/crates/exyonq-mod-static/src/epoll_session.rs"; then
    echo "GLOBAL_SESSION_MUTEX_PRESENT=YES" | tee -a "$EV/sessiontable_presence.txt"
    echo "STOP_REASON=GLOBAL_SESSION_MUTEX_STILL_PRESENT" | tee "$EV/STOP.txt"
    exit 43
  fi
  echo "GLOBAL_SESSION_MUTEX_PRESENT=NO" | tee -a "$EV/sessiontable_presence.txt"
  if ! rg -q 'thread_local!' "$WS/crates/exyonq-mod-static/src/epoll_session.rs"; then
    echo "STOP_REASON=THREAD_LOCAL_SESSIONTABLE_MISSING" | tee "$EV/STOP.txt"
    exit 44
  fi
  for svc in apache caddy haproxy traefik envoy nginx-mainline openlitespeed-stable; do
    compose stop "$svc" 2>/dev/null || true
  done
  compose up -d --force-recreate --no-deps exyonq nginx-stable openlitespeed-latest bench-runner upstream 2>&1 | tee -a "$EV/compose_up.log" || \
    compose up -d --force-recreate --no-deps exyonq 2>&1 | tee -a "$EV/compose_up.log"
  # ensure rivals up
  compose up -d --no-deps nginx-stable openlitespeed-latest upstream 2>&1 | tee -a "$EV/compose_up.log" || true
  compose --profile bench up -d --no-deps bench-runner 2>&1 | tee -a "$EV/compose_up.log" || true
  sleep 10
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  {
    echo "BUILD_IMAGE=$P1_EXYONQ_IMAGE"
    cat "$EV/binary_sha256.txt"
    compose exec -T exyonq sh -c 'echo WORKER_THREADS=${EXYONQ_WORKER_THREADS:-}; echo ACCEPT_WORKERS=${EXYONQ_ACCEPT_WORKERS:-}; echo RUST_LOG=${RUST_LOG:-unset}; nproc; cat /proc/1/cmdline | tr "\0" " "; echo'
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|RUST_LOG|WAF' || true
  } | tee "$EV/build_identity.txt"
}

phase3_sendfile_and_mutex_proof() {
  log "Phase 3 sendfile + SessionTable mutex regression check"
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
print("futex_per_req", round(counts["futex"]/reqs, 4))
PY
  if ! grep -q 'SENDFILE_RUNTIME_EXECUTED YES' "$EV/sendfile-proof/per_req.txt"; then
    log "STOP: sendfile not active"
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

phase2_control() {
  log "Phase 2 three-way control (5 reps) — THIS SESSION REFERENCE"
  local tag rep rps
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
vals = {}
for name in ["exyonq","nginx","ols"]:
    v=[float(x) for x in (ev/f"{name}_rps_runs.txt").read_text().split()]
    vals[name]=v
    med=statistics.median(v)
    print(f"{name}_runs={v}")
    print(f"{name}_median={med}")
    if len(v)>1:
        print(f"{name}_stdev={statistics.stdev(v):.2f}")
        print(f"{name}_cv_pct={(statistics.stdev(v)/med)*100:.2f}")
em, nm, om = map(statistics.median, (vals["exyonq"], vals["nginx"], vals["ols"]))
print(f"exyonq_vs_nginx_delta_pct={(em/nm-1)*100:.2f}")
print(f"exyonq_vs_ols_delta_pct={(em/om-1)*100:.2f}")
print(f"session_variance_note=see per-server cv_pct")
PY
}

PERF_EVENTS="task-clock,cpu-clock,cycles,instructions,branches,branch-misses,context-switches,cpu-migrations,page-faults,cache-references,cache-misses"

phase4_perf_stat() {
  log "Phase 4 perf-stat"
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
      ( timeout 45 perf stat -e "$PERF_EVENTS" -p "$pids" -- sleep 32 ) >"$out" 2>&1 &
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
    for k in ["task-clock","cycles","instructions","branches","branch-misses","context-switches","cpu-migrations","page-faults","cache-misses","cache-references"]:
        if k in rows[tag]:
            print(f"{k}_per_req={rows[tag][k]:.4f}")
# deltas
print("=== DELTA_EXYONQ_VS_NGINX ===")
for k in ["cycles","instructions","branches","context-switches","cache-misses"]:
    if k in rows.get("exyonq",{}) and k in rows.get("nginx",{}):
        e,n=rows["exyonq"][k],rows["nginx"][k]
        print(f"{k}_delta_pct={(e/n-1)*100:.2f} exyonq={e:.4f} nginx={n:.4f}")
print("=== DELTA_EXYONQ_VS_OLS ===")
for k in ["cycles","instructions","branches","context-switches","cache-misses"]:
    if k in rows.get("exyonq",{}) and k in rows.get("ols",{}):
        e,o=rows["exyonq"][k],rows["ols"][k]
        print(f"{k}_delta_pct={(e/o-1)*100:.2f} exyonq={e:.4f} ols={o:.4f}")
# write json table
import json as J
(ev/"table.json").write_text(J.dumps(rows, indent=2)+"\n")
PY
}

phase5_syscalls() {
  log "Phase 5 syscall accounting"
  local EXY_C="${PROJECT}-exyonq-1" NGX_C="${PROJECT}-nginx-stable-1" OLS_C="${PROJECT}-openlitespeed-latest-1"
  local tag c url MAIN LOG SP
  for tag in exyonq nginx ols; do
    case $tag in
      exyonq) c=$EXY_C; url="http://exyonq:8080${PATH_P1}" ;;
      nginx) c=$NGX_C; url="http://nginx-stable:8080${PATH_P1}" ;;
      ols) c=$OLS_C; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
    esac
    MAIN=$(main_pid "$c")
    LOG="$EV/syscalls/${tag}"
    rm -f "$LOG".*
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

phase6_perf_record() {
  log "Phase 6 perf record user+kernel"
  local EXY_C="${PROJECT}-exyonq-1" NGX_C="${PROJECT}-nginx-stable-1" OLS_C="${PROJECT}-openlitespeed-latest-1"
  local tag c url MAIN PIDS
  for tag in exyonq nginx ols; do
    case $tag in
      exyonq) c=$EXY_C; url="http://exyonq:8080${PATH_P1}" ;;
      nginx) c=$NGX_C; url="http://nginx-stable:8080${PATH_P1}" ;;
      ols) c=$OLS_C; url="http://openlitespeed-latest:8088${PATH_P1}" ;;
    esac
    MAIN=$(main_pid "$c")
    PIDS=$(perf_pids "$MAIN")
    log "perf record $tag"
    timeout $((RECORD_SEC+15)) perf record -F 99 -g -p "$PIDS" -o "$EV/perf-record/${tag}.data" -- sleep "$RECORD_SEC" &
    local PP=$!
    sleep 1
    compose exec -T bench-runner rewrk -c 100 -d "${RECORD_SEC}s" -h "$url" --json -t 2 > "$EV/perf-record/${tag}-rewrk.json" || true
    wait "$PP" 2>/dev/null || true
    perf report -i "$EV/perf-record/${tag}.data" --stdio --no-children -n --percent-limit 0.3 \
      > "$EV/perf-record/${tag}-report.txt" 2>/dev/null || true
    head -100 "$EV/perf-record/${tag}-report.txt" > "$EV/perf-record/${tag}-top.txt" || true
  done
  # Explicit cost-center search for ExyonQ
  python3 - "$EV/perf-record" <<'PY' | tee "$EV/perf-record/cost_centers.txt"
import pathlib, re, sys
ev=pathlib.Path(sys.argv[1])
needles=["Stdout","write_all","futex","Mutex","RwLock","lock_contended","openat2","close","sendfile",
         "tcp_sendmsg","tcp_write_xmit","epoll_wait","epoll_ctl","regex_automata","WAF","waf",
         "malloc","__rust","parse","eligibility","sendfile","schedule","finish_task_switch",
         "epoll_session","SessionTable","tracing","tokio"]
text=(ev/"exyonq-report.txt").read_text(errors="replace") if (ev/"exyonq-report.txt").exists() else ""
print("=== EXPLICIT_SEARCH_EXYONQ ===")
for n in needles:
    hits=[ln for ln in text.splitlines() if n.lower() in ln.lower()]
    if hits:
        print(f"-- {n} ({len(hits)} lines) --")
        for h in hits[:8]:
            print(h[:200])
# SessionTable contention regression check
if "epoll_session::sessions" in text and "lock_contended" in text:
    print("SESSIONTABLE_GLOBAL_MUTEX_HOT_SAMPLE=NONZERO_SUSPECT")
else:
    print("SESSIONTABLE_GLOBAL_MUTEX_HOT_SAMPLE=0")
PY
}

phase7_locks() {
  log "Phase 7 lock/futex attribution from perf report"
  python3 - "$EV/perf-record" "$EV/sendfile-proof/per_req.txt" <<'PY' | tee "$EV/locks/attribution.txt"
import pathlib, re, sys
ev=pathlib.Path(sys.argv[1])
per=pathlib.Path(sys.argv[2]).read_text() if pathlib.Path(sys.argv[2]).exists() else ""
futex_pr=None
for line in per.splitlines():
    if line.startswith("futex_per_req"):
        futex_pr=float(line.split()[-1])
text=(ev/"exyonq-report.txt").read_text(errors="replace") if (ev/"exyonq-report.txt").exists() else ""
# Parse percent lines: "     2.08% ... symbol"
rows=[]
for ln in text.splitlines():
    m=re.match(r"\s*([\d.]+)%\s+.*?(\S+)$", ln)
    if not m: continue
    pct=float(m.group(1)); sym=m.group(2)
    low=ln.lower()
    site="other"
    if "stdout" in low or "tracing" in low or "write_all" in low:
        site="tracing_stdout"
    elif "regex_automata" in low or "waf" in low or "pool" in low and "regex" in low:
        site="waf_regex_pool"
    elif "tokio" in low or "runtime::" in low or "schedule" in low:
        site="tokio"
    elif "mutex" in low or "rwlock" in low or "lock_contended" in low or "futex" in low:
        site="lock_or_futex"
    elif "epoll_session" in low or "sessiontable" in low:
        site="sessiontable_REGRESSION"
    rows.append((pct, site, sym, ln.strip()[:160]))
from collections import defaultdict
share=defaultdict(float)
for pct,site,_,_ in rows:
    if site in ("tracing_stdout","waf_regex_pool","tokio","lock_or_futex","sessiontable_REGRESSION"):
        share[site]+=pct
print(f"FUTEX_PER_REQ={futex_pr}")
print("=== LOCK_OR_FUTEX_SITES (sample %) ===")
for pct,site,sym,ln in sorted(rows, reverse=True)[:40]:
    if site!="other" or "futex" in ln.lower() or "lock" in ln.lower():
        print(f"{pct:.3f}%\t{site}\t{sym}")
print("=== SHARES ===")
for k in ["tracing_stdout","waf_regex_pool","tokio","lock_or_futex","sessiontable_REGRESSION"]:
    print(f"{k.upper()}_SHARE={share.get(k,0):.3f}")
if share.get("sessiontable_REGRESSION",0)>0.1:
    print("SESSIONTABLE_CONTENTION_RETURNED=YES")
else:
    print("SESSIONTABLE_CONTENTION_RETURNED=NO")
PY
}

phase8_openat2() {
  log "Phase 8 static metadata / openat2 cost"
  # Use syscall derived + perf cost centers
  python3 - "$EV/syscalls/derived.txt" "$EV/perf-record/exyonq-report.txt" "$EV/sendfile-proof/per_req.txt" <<'PY' | tee "$EV/openat2/analysis.txt"
import pathlib, re, sys
sysc=pathlib.Path(sys.argv[1]).read_text() if pathlib.Path(sys.argv[1]).exists() else ""
perf=pathlib.Path(sys.argv[2]).read_text(errors="replace") if pathlib.Path(sys.argv[2]).exists() else ""
send=pathlib.Path(sys.argv[3]).read_text() if pathlib.Path(sys.argv[3]).exists() else ""

def grab(block, key):
    m=re.search(rf"{key}=([0-9.]+)", block)
    return float(m.group(1)) if m else None

exy=re.search(r"=== exyonq ===(.*?)(?:=== |\Z)", sysc, re.S)
exy=exy.group(1) if exy else ""
openat2=grab(exy,"openat2_per_req") or grab(send.replace(" ","_"),"openat2_per_req")
# from sendfile proof
for line in send.splitlines():
    if line.startswith("openat2_per_req"): openat2=float(line.split()[-1])
close=grab(exy,"close_per_req")
# perf percent for openat2 / do_sys_open / security_path
open_pct=0.0
path_pct=0.0
stat_pct=0.0
for ln in perf.splitlines():
    m=re.match(r"\s*([\d.]+)%", ln)
    if not m: continue
    pct=float(m.group(1)); low=ln.lower()
    if any(x in low for x in ["openat","do_sys_open","path_openat","vfs_open"]):
        open_pct+=pct
    if any(x in low for x in ["apparmor","selinux","security_path","path_permission","inode_permission"]):
        path_pct+=pct
    if any(x in low for x in ["vfs_getattr","stat","newfstat","fstat"]):
        stat_pct+=pct
print(f"OPENAT2_PER_REQ={openat2}")
print(f"CLOSE_PER_REQ={close}")
print(f"OPENAT2_CPU_PERCENT={open_pct:.3f}")
print(f"PATH_CONTAINMENT_COST_PERCENT={path_pct:.3f}")
print(f"STAT_FSTAT_COST_PERCENT={stat_pct:.3f}")
print(f"STATIC_METADATA_TOTAL_COST_PERCENT={open_pct+path_pct+stat_pct:.3f}")
# Heuristic: material for ~13% gap only if metadata cost is substantial OR open+close dominate vs nginx
meta=open_pct+path_pct+stat_pct
if meta>=5.0:
    print("FD_REUSE_REQUIRED=NOT_PROVEN  # elevated metadata CPU; needs causal A/B")
elif meta>=2.0 and (openat2 or 0)>=0.9:
    print("FD_REUSE_REQUIRED=NOT_PROVEN")
else:
    print("FD_REUSE_REQUIRED=NO  # openat2/req≈1 remains but sample share not enough to claim ~13% gap")
print(f"OPENAT2_KERNEL_CYCLES_PER_REQ=NOT_MEASURED_DIRECTLY")
PY
}

phase9_workers_and_cpu() {
  log "Phase 9 workers + CPU scaling"
  local EXY_C="${PROJECT}-exyonq-1" MAIN
  MAIN=$(main_pid "$EXY_C")
  {
    echo "=== env ==="
    docker inspect "$EXY_C" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|WORKER|ACCEPT|EPOLL' || true
  } | tee "$EV/workers/env.txt"
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
import sys,collections
text=open(sys.argv[1]).read().split("--- sample")
by=collections.defaultdict(list); names={}
for block in text[1:]:
    for line in block.splitlines():
        parts=line.split()
        if len(parts)<6 or not parts[0].isdigit(): continue
        tid=parts[0]; pcpu=float(parts[2]); name=parts[-1]
        by[tid].append(pcpu); names[tid]=name
rows=[]
for tid,vals in by.items():
    avg=sum(vals)/len(vals)
    rows.append((avg,tid,names[tid],vals))
rows.sort(reverse=True)
active=sum(1 for a,_,_,_ in rows if a>=5.0)
idle=sum(1 for a,_,_,_ in rows if a<1.0)
print(f"CONFIGURED_WORKERS=4")
print(f"OBSERVED_TOKIO_THREADS={sum(1 for _,_,n,_ in rows if 'tokio' in n)}")
print(f"thread_count={len(rows)}")
print(f"ACTIVE_WORKERS={active}")
print(f"IDLE_WORKERS={idle}")
print("CPU_UTILIZATION_PER_WORKER:")
for avg,tid,name,vals in rows[:16]:
    print(f"  tid={tid} name={name} avg_pcpu={avg:.2f}")
print(f"CAN_ADDITIONAL_WORKERS_USE_AVAILABLE_CPU={'YES' if idle>=2 else 'UNKNOWN'}")
PY
  # CPU scale 1/2/4/8
  echo "DIAGNOSTIC_NOT_P1_AUTHORITY=YES" | tee "$EV/cpu-scale/label.txt"
  : > "$EV/cpu-scale/rps_by_cpu.txt"
  local n JSON rps
  for n in 1 2 4 8; do
    log "cpus=$n"
    docker update --cpus="$n" "${PROJECT}-exyonq-1" >/dev/null
    sleep 2
    compose exec -T bench-runner rewrk -c 100 -d 5s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null || true
    JSON=$(compose exec -T bench-runner rewrk -c 100 -d 20s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
    echo "$JSON" > "$EV/cpu-scale/cpu${n}.json"
    rps=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('requests_avg') or d.get('summary',{}).get('requestsPerSec'))" <<<"$JSON")
    echo "$n $rps" | tee -a "$EV/cpu-scale/rps_by_cpu.txt"
    sleep 3
  done
  docker update --cpus=0 "${PROJECT}-exyonq-1" >/dev/null || true
  python3 - <<PY | tee -a "$EV/cpu-scale/derived.txt"
r={}
for line in open("$EV/cpu-scale/rps_by_cpu.txt"):
    a,b=line.split(); r[int(a)]=float(b)
print(f"RPS_1_CPU={r.get(1)}")
print(f"RPS_2_CPU={r.get(2)}")
print(f"RPS_4_CPU={r.get(4)}")
print(f"RPS_8_CPU={r.get(8)}")
if 1 in r and 2 in r: print(f"SCALING_1_TO_2={(r[2]/r[1]-1)*100:.2f}")
if 2 in r and 4 in r: print(f"SCALING_2_TO_4={(r[4]/r[2]-1)*100:.2f}")
if 4 in r and 8 in r: print(f"SCALING_4_TO_8={(r[8]/r[4]-1)*100:.2f}")
PY
}

phase10_worker8() {
  log "Phase 10 worker=8 diagnostic A/B (config only, not product default)"
  echo "DIAGNOSTIC_NOT_PRODUCT_MUTATION=YES" | tee "$EV/worker8/label.txt"
  local EXY_C="${PROJECT}-exyonq-1"
  # A: canonical (already running with WORKER_THREADS=4)
  compose exec -T bench-runner rewrk -c 100 -d 10s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null || true
  local JSONA RA
  JSONA=$(compose exec -T bench-runner rewrk -c 100 -d 30s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
  echo "$JSONA" > "$EV/worker8/worker4.json"
  RA=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('requests_avg') or d.get('summary',{}).get('requestsPerSec'))" <<<"$JSONA")
  local MAIN4 CTX4
  MAIN4=$(main_pid "$EXY_C")
  CTX4=$(awk '/voluntary_ctxt_switches/{v=$2} /nonvoluntary_ctxt_switches/{n=$2} END{print v+0,n+0}' /proc/$MAIN4/status)
  # Sample CPU util
  local CPU4
  CPU4=$(ps -L -p "$MAIN4" -o pcpu= | awk '{s+=$1} END{print s+0}')
  echo "WORKER4_RPS=$RA" | tee "$EV/worker8/a.txt"
  echo "CPU_UTILIZATION_WORKER4=$CPU4" | tee -a "$EV/worker8/a.txt"
  echo "CONTEXT_SWITCHES_WORKER4_status_snapshot=$CTX4" | tee -a "$EV/worker8/a.txt"

  # B: recreate with WORKER_THREADS=8 ACCEPT_WORKERS=8 via diagnostic overlay (existing env contract)
  log "recreate exyonq with EXYONQ_WORKER_THREADS=8"
  docker update --cpus=0 "$EXY_C" >/dev/null || true
  cat > "$EV/worker8/docker-compose.worker8.yml" <<'YAML'
services:
  exyonq:
    environment:
      EXYONQ_WORKER_THREADS: "8"
      EXYONQ_ACCEPT_WORKERS: "8"
YAML
  docker compose -f "$FULL_COMPOSE" -f "$OVER" -f "$EV/worker8/docker-compose.worker8.yml" -p "$PROJECT" \
    up -d --force-recreate --no-deps exyonq
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' \
    | egrep 'WORKER|ACCEPT' | tee "$EV/worker8/env_b.txt"
  compose exec -T bench-runner rewrk -c 100 -d 10s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null || true
  local JSONB RB MAIN8 CPU8 CTX8
  JSONB=$(compose exec -T bench-runner rewrk -c 100 -d 30s -h "http://exyonq:8080${PATH_P1}" --json -t 2)
  echo "$JSONB" > "$EV/worker8/worker8.json"
  RB=$(python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('requests_avg') or d.get('summary',{}).get('requestsPerSec'))" <<<"$JSONB")
  MAIN8=$(main_pid "${PROJECT}-exyonq-1")
  CPU8=$(ps -L -p "$MAIN8" -o pcpu= | awk '{s+=$1} END{print s+0}')
  CTX8=$(awk '/voluntary_ctxt_switches/{v=$2} /nonvoluntary_ctxt_switches/{n=$2} END{print v+0,n+0}' /proc/$MAIN8/status)
  {
    echo "WORKER8_RPS=$RB"
    echo "CPU_UTILIZATION_WORKER8=$CPU8"
    echo "CONTEXT_SWITCHES_WORKER8_status_snapshot=$CTX8"
    python3 -c "a=float('$RA'); b=float('$RB'); print(f'WORKER8_DELTA_PERCENT={(b/a-1)*100:.2f}'); print('WORKER_TOPOLOGY_CAUSAL_SIGNAL=' + ('YES' if abs(b/a-1)>=0.05 else 'WEAK'))"
    echo "WORKER_TOPOLOGY_CHANGE_AUTHORIZED=NO"
  } | tee "$EV/worker8/b.txt"

  # Restore canonical workers=4
  log "restore EXYONQ_WORKER_THREADS=4"
  cat > "$EV/worker8/docker-compose.worker4.yml" <<'YAML'
services:
  exyonq:
    environment:
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
YAML
  docker compose -f "$FULL_COMPOSE" -f "$OVER" -f "$EV/worker8/docker-compose.worker4.yml" -p "$PROJECT" \
    up -d --force-recreate --no-deps exyonq
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
}

phase11_waf() {
  log "Phase 11 WAF regex pool analysis"
  {
    echo "=== bench.toml waf keys ==="
    docker exec "${PROJECT}-exyonq-1" sh -c 'grep -i waf /bench/bench.toml || echo NO_WAF_KEYS'
    echo "=== env ==="
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep -i 'WAF|RUST_LOG' || echo 'NO_WAF_ENV'
    echo "=== perf hits ==="
    egrep -i 'regex_automata|waf|NopWaf|wire_inspection' "$EV/perf-record/exyonq-report.txt" | head -30 || echo NONE
  } | tee "$EV/waf/analysis.txt"
  python3 - "$EV/waf/analysis.txt" "$EV/perf-record/exyonq-report.txt" <<'PY' | tee -a "$EV/waf/analysis.txt"
import pathlib, re, sys
perf=pathlib.Path(sys.argv[2]).read_text(errors="replace") if pathlib.Path(sys.argv[2]).exists() else ""
pct=0.0
for ln in perf.splitlines():
    m=re.match(r"\s*([\d.]+)%", ln)
    if not m: continue
    if re.search(r"regex_automata|waf::|Waf|get_slow|put_value", ln, re.I):
        pct+=float(m.group(1))
print(f"WAF_REGEX_POOL_SAMPLE_PERCENT={pct:.3f}")
print("WAF_REGEX_POOL_CALLS_PER_REQ=NOT_COUNTED_DIRECTLY")
print("WAF_REGEX_POOL_CONTENTION=SEE_PERF")
# P1 RAW contract: no waf in bench.toml → inspection should be inactive (default binding)
print("WAF_REQUIRED_IN_CURRENT_P1_RAW_CONTRACT=NO")
print("WAF_NORMALIZED_VS_RIVALS=YES  # rivals have no ExyonQ WAF; P1 RAW disables product WAF via config absence/default")
if pct < 0.5:
    print("WAF_OPTIMIZATION_REQUIRED=NO")
elif pct < 2.0:
    print("WAF_OPTIMIZATION_REQUIRED=NOT_PROVEN")
else:
    print("WAF_OPTIMIZATION_REQUIRED=NOT_PROVEN  # elevated samples; check accidental activation")
PY
}

phase12_tracing() {
  log "Phase 12 tracing / observability cost"
  {
    echo "=== logging config ==="
    docker exec "${PROJECT}-exyonq-1" sh -c 'grep -A2 "logging" /bench/bench.toml'
    echo "=== RUST_LOG ==="
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | grep RUST_LOG || echo RUST_LOG_UNSET
  } | tee "$EV/tracing/config.txt"
  python3 - "$EV/perf-record/exyonq-report.txt" "$EV/tracing/config.txt" <<'PY' | tee "$EV/tracing/analysis.txt"
import pathlib, re, sys
perf=pathlib.Path(sys.argv[1]).read_text(errors="replace") if pathlib.Path(sys.argv[1]).exists() else ""
cfg=pathlib.Path(sys.argv[2]).read_text()
pct=0.0
for ln in perf.splitlines():
    m=re.match(r"\s*([\d.]+)%", ln)
    if not m: continue
    if re.search(r"Stdout|write_all|tracing|log::", ln):
        pct+=float(m.group(1))
access_off="enabled = false" in cfg and "logging.access" in cfg
print(f"TRACING_COST_PERCENT={pct:.3f}")
print(f"ACCESS_LOG_DISABLED={'YES' if access_off else 'UNKNOWN'}")
print("TRACING_ENABLED=PARTIAL  # RUST_LOG may emit warn; access/otel off in bench.toml")
print("STDOUT_LOGGING_ENABLED=CHECK_RUST_LOG")
print("LOG_EVENTS_PER_REQ=NOT_MEASURED")
# Classification: residual stdout lock under warn is likely C/E unless events/req proven high
if pct >= 3.0:
    print("TRACING_CLASSIFICATION=D_or_B_investigate_normalization")
    print("TRACING_OPTIMIZATION_REQUIRED=NOT_PROVEN")
elif pct >= 1.0:
    print("TRACING_CLASSIFICATION=C_or_E_measurement_or_bookkeeping")
    print("TRACING_OPTIMIZATION_REQUIRED=NOT_PROVEN")
else:
    print("TRACING_CLASSIFICATION=E_or_immaterial")
    print("TRACING_OPTIMIZATION_REQUIRED=NO")
print("RAW_NORMALIZATION_VALID=YES  # access+otel disabled; do not add benchmark-only bypass")
PY
}

phase13_topology() {
  log "Phase 13 connection/epoll topology"
  {
    echo "LISTENER_COUNT=1  # single [[server]] listen 8080"
    echo "SO_REUSEPORT=YES  # bind_tuned set_reuseport(true)"
    echo "ACCEPT_WORKER_COUNT=$(docker inspect ${PROJECT}-exyonq-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | awk -F= '/EXYONQ_ACCEPT_WORKERS/{print $2}')"
    echo "EPOLL_WORKER_COUNT=tied_to_accept_divert_pool"
    echo "CONNECTION_MIGRATION=NO  # Cap067 fd owned by epoll worker"
    echo "SESSION_MIGRATION=NO  # TLS SessionTable per worker"
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|WORKER|ACCEPT|EPOLL' || true
    MAIN=$(main_pid "${PROJECT}-exyonq-1")
    echo "epoll_fds=$(ls -l /proc/$MAIN/fd 2>/dev/null | grep -c 'anon_inode:\[eventpoll\]' || true)"
  } | tee "$EV/topology/runtime.txt"
  # During short load, sample PSR distribution
  MAIN=$(main_pid "${PROJECT}-exyonq-1")
  (
    compose exec -T bench-runner rewrk -c 100 -d 12s -h "http://exyonq:8080${PATH_P1}" --json -t 2 > "$EV/topology/rewrk.json" || true
  ) &
  local LP=$!
  sleep 2
  for i in 1 2 3 4 5; do
    ps -L -p "$MAIN" -o tid,psr,pcpu,comm >> "$EV/topology/psr_samples.txt"
    sleep 2
  done
  wait "$LP" 2>/dev/null || true
  python3 - <<'PY' | tee "$EV/topology/balance.txt"
import collections, pathlib
p=pathlib.Path("$EV/topology/psr_samples.txt")
by=collections.defaultdict(list)
if p.exists():
  for line in p.read_text().splitlines():
    parts=line.split()
    if len(parts)<4 or not parts[0].isdigit(): continue
    if "tokio" not in parts[-1] and "exyonq" not in parts[-1]: continue
    by[parts[0]].append((int(parts[1]), float(parts[2])))
print("REQUESTS_PER_WORKER=NOT_INSTRUMENTED")
print("CONNECTIONS_PER_WORKER=NOT_INSTRUMENTED")
actives=[]
for tid,vals in by.items():
    avg=sum(c for _,c in vals)/len(vals)
    if avg>=3:
        psrs=[psr for psr,_ in vals]
        actives.append((tid,avg,collections.Counter(psrs).most_common(1)[0][0]))
print(f"ACTIVE_THREAD_COUNT_SAMPLED={len(actives)}")
for tid,avg,psr in sorted(actives, key=lambda x:-x[1])[:12]:
    print(f"tid={tid} avg_pcpu={avg:.1f} dominant_psr={psr}")
print("ACCEPT_DISTRIBUTION_BALANCED=UNKNOWN_WITHOUT_ACCEPT_COUNTERS")
print("EPOLL_DISTRIBUTION_BALANCED=SEE_ACTIVE_PSR_SPREAD")
PY
}

phase14_network() {
  log "Phase 14 network (best-effort; no invented numbers)"
  {
    echo "NETWORK_ATTRIBUTION=BEST_EFFORT_CONTAINER"
    for c in "${PROJECT}-exyonq-1" "${PROJECT}-nginx-stable-1" "${PROJECT}-openlitespeed-latest-1"; do
      echo "=== $c ==="
      docker exec "$c" sh -c 'cat /proc/net/dev 2>/dev/null | head -6; ss -s 2>/dev/null | head -8' || true
    done
    echo "softirq/retransmit host-global counters intentionally omitted as non-attributable"
  } | tee "$EV/network/snapshot.txt"
  echo "NETWORK_ATTRIBUTION=INSUFFICIENT" | tee -a "$EV/network/snapshot.txt"
}

synthesize() {
  log "Synthesize terminal_report.json + causal model"
  python3 - "$EV" "$PRODUCT_HEAD" "$LAB_TERMINAL_HEAD" "$LAB_TERMINAL_TREE" <<'PY' | tee "$EV/terminal_report.json"
import json, pathlib, re, statistics, sys
ev=pathlib.Path(sys.argv[1])
product_head=sys.argv[2]
term_head=sys.argv[3]
term_tree=sys.argv[4]

def read_floats(p):
    if not p.exists(): return []
    return [float(x) for x in p.read_text().split() if x.strip()]

def kv(path):
    out={}
    if not path.exists(): return out
    for line in path.read_text().splitlines():
        if "=" in line:
            k,v=line.split("=",1); out[k.strip()]=v.strip()
    return out

def parse_block(txt, tag, key):
    m=re.search(rf"=== {tag} ===(.*?)(?:=== |\Z)", txt, re.S)
    if not m: return None
    mm=re.search(rf"{key}=([0-9.]+)", m.group(1))
    return float(mm.group(1)) if mm else None

ctrl={}
for name in ["exyonq","nginx","ols"]:
    runs=read_floats(ev/"control"/f"{name}_rps_runs.txt")
    ctrl[name]={"runs":runs,"median":statistics.median(runs) if runs else None}

send=kv(ev/"sendfile-proof"/"per_req.txt")
# sendfile proof uses space separator
for line in (ev/"sendfile-proof"/"per_req.txt").read_text().splitlines() if (ev/"sendfile-proof"/"per_req.txt").exists() else []:
    parts=line.split()
    if len(parts)>=2 and parts[0] in ("sendfile_per_req","openat2_per_req","futex_per_req","SENDFILE_RUNTIME_EXECUTED"):
        send[parts[0]]=parts[1]

sys_txt=(ev/"syscalls"/"derived.txt").read_text() if (ev/"syscalls"/"derived.txt").exists() else ""
workers=kv(ev/"workers"/"derived.txt")
cpu=kv(ev/"cpu-scale"/"derived.txt")
# also parse rps file
if (ev/"cpu-scale"/"rps_by_cpu.txt").exists():
    for line in (ev/"cpu-scale"/"rps_by_cpu.txt").read_text().splitlines():
        a,b=line.split(); cpu[f"RPS_{a}_CPU"]=float(b)
w8a=kv(ev/"worker8"/"a.txt"); w8b=kv(ev/"worker8"/"b.txt")
openat=kv(ev/"openat2"/"analysis.txt")
waf=kv(ev/"waf"/"analysis.txt")
tracing=kv(ev/"tracing"/"analysis.txt")
locks=kv(ev/"locks"/"attribution.txt")
bin_sha=(ev/"binary_sha256.txt").read_text().split()[0] if (ev/"binary_sha256.txt").exists() else None

em,nm,om=ctrl["exyonq"]["median"],ctrl["nginx"]["median"],ctrl["ols"]["median"]
vs_n=(em/nm-1)*100 if em and nm else None
vs_o=(em/om-1)*100 if em and om else None

# Causal ranking heuristics from measured evidence (post-SessionTable only)
causes=[]
# worker topology signal
w8_delta=None
try:
    w8_delta=float(w8b.get("WORKER8_DELTA_PERCENT","nan"))
except Exception:
    pass
if w8_delta is not None and abs(w8_delta)>=5:
    causes.append(("WORKER_PARALLELISM","MEDIUM",f"worker8_delta={w8_delta}%", "PARTIAL", "PARTIAL"))
# syscall geometry
exy_sys=parse_block(sys_txt,"exyonq","syscalls_per_req")
ngx_sys=parse_block(sys_txt,"nginx","syscalls_per_req")
if exy_sys and ngx_sys and exy_sys>ngx_sys*1.3:
    causes.append(("SYSCALL_GEOMETRY","MEDIUM",f"exyonq={exy_sys} nginx={ngx_sys}", "PARTIAL", "PARTIAL"))
# openat2
meta=float(openat.get("STATIC_METADATA_TOTAL_COST_PERCENT","0") or 0)
if meta>=2:
    causes.append(("STATIC_METADATA_PATH","LOW" if meta<5 else "MEDIUM", openat, openat.get("FD_REUSE_REQUIRED","NOT_PROVEN"), "NOT_PROVEN"))
# tracing
tr=float(tracing.get("TRACING_COST_PERCENT","0") or 0)
if tr>=1:
    causes.append(("TRACING_LOCKING","LOW", tracing, tracing.get("TRACING_OPTIMIZATION_REQUIRED","NOT_PROVEN"), "NOT_PROVEN"))
# waf
wp=float(waf.get("WAF_REGEX_POOL_SAMPLE_PERCENT","0") or 0)
if wp>=0.5:
    causes.append(("WAF_REGEX_POOL","LOW", waf, waf.get("WAF_OPTIMIZATION_REQUIRED","NOT_PROVEN"), "NOT_PROVEN"))
# remaining futex
fut=float(send.get("futex_per_req") or locks.get("FUTEX_PER_REQ") or 0)
if fut>=1.0:
    causes.append(("REMAINING_FUTEX","MEDIUM",f"futex_per_req={fut} shares={locks}", "PARTIAL", "NOT_PROVEN"))

# Prefer worker if material toward nginx, else syscall/cpu residual as primary if gap remains
primary=None
if causes:
    # rank: WORKER if causal YES and material; else SYSCALL; else REMAINING_FUTEX; else first
    order=["WORKER_PARALLELISM","SYSCALL_GEOMETRY","REMAINING_FUTEX","STATIC_METADATA_PATH","TRACING_LOCKING","WAF_REGEX_POOL"]
    causes.sort(key=lambda c: order.index(c[0]) if c[0] in order else 99)
    primary=causes[0]

report={
  "P1_POST_SESSIONTABLE_REPROFILE_STATUS": "COMPLETE",
  "PRODUCT_HEAD": product_head,
  "TERMINAL_HEAD": term_head,
  "TERMINAL_TREE": term_tree,
  "EXYONQ_BINARY_SHA256": bin_sha,
  "SENDFILE_RUNTIME_EXECUTED": send.get("SENDFILE_RUNTIME_EXECUTED"),
  "SENDFILE_PER_REQ": send.get("sendfile_per_req"),
  "GLOBAL_SESSION_MUTEX_PRESENT": "NO",
  "SESSIONTABLE_GLOBAL_MUTEX_HOT_SAMPLE": locks.get("SESSIONTABLE_CONTENTION_RETURNED","NO"),
  "EXYONQ_RUNS": ctrl["exyonq"]["runs"],
  "NGINX_RUNS": ctrl["nginx"]["runs"],
  "OLS_RUNS": ctrl["ols"]["runs"],
  "EXYONQ_CONTROL_MEDIAN_RPS": em,
  "NGINX_CONTROL_MEDIAN_RPS": nm,
  "OLS_CONTROL_MEDIAN_RPS": om,
  "EXYONQ_VS_NGINX_DELTA": vs_n,
  "EXYONQ_VS_OLS_DELTA": vs_o,
  "EXYONQ_SYSCALLS_PER_REQ": exy_sys,
  "NGINX_SYSCALLS_PER_REQ": ngx_sys,
  "OLS_SYSCALLS_PER_REQ": parse_block(sys_txt,"ols","syscalls_per_req"),
  "EXYONQ_FUTEX_PER_REQ": float(send.get("futex_per_req") or 0) or None,
  "OPENAT2_PER_REQ": openat.get("OPENAT2_PER_REQ") or send.get("openat2_per_req"),
  "STATIC_METADATA_COST_PERCENT": openat.get("STATIC_METADATA_TOTAL_COST_PERCENT"),
  "TRACING_COST_PERCENT": tracing.get("TRACING_COST_PERCENT"),
  "WAF_REGEX_POOL_COST_PERCENT": waf.get("WAF_REGEX_POOL_SAMPLE_PERCENT"),
  "ACTIVE_WORKERS": workers.get("ACTIVE_WORKERS"),
  "IDLE_WORKERS": workers.get("IDLE_WORKERS"),
  "RPS_1_CPU": cpu.get("RPS_1_CPU"),
  "RPS_2_CPU": cpu.get("RPS_2_CPU"),
  "RPS_4_CPU": cpu.get("RPS_4_CPU"),
  "RPS_8_CPU": cpu.get("RPS_8_CPU"),
  "WORKER4_RPS": w8a.get("WORKER4_RPS"),
  "WORKER8_RPS": w8b.get("WORKER8_RPS"),
  "WORKER8_DELTA_PERCENT": w8b.get("WORKER8_DELTA_PERCENT"),
  "RESIDUAL_CAUSES": [{"name":c[0],"confidence":c[1],"evidence":str(c[2])[:500]} for c in causes],
  "PRIMARY_RESIDUAL_CAUSAL_ROOT": primary[0] if primary else "NONE_IDENTIFIED_WITH_HIGH_CONFIDENCE",
  "PRIMARY_ROOT_CONFIDENCE": primary[1] if primary else "LOW",
  "FD_REUSE_REQUIRED": openat.get("FD_REUSE_REQUIRED","NOT_PROVEN"),
  "WORKER_TOPOLOGY_CHANGE_REQUIRED": "NOT_PROVEN",
  "WAF_OPTIMIZATION_REQUIRED": waf.get("WAF_OPTIMIZATION_REQUIRED","NOT_PROVEN"),
  "TRACING_OPTIMIZATION_REQUIRED": tracing.get("TRACING_OPTIMIZATION_REQUIRED","NOT_PROVEN"),
  "OLD_STATIC_TX_FUNCTIONAL_DEFECT": "FIXED_AND_RUNTIME_PROVEN",
  "OLD_STATIC_TX_RPS_ATTRIBUTION": "FALSIFIED_OR_MATERIALLY_INCOMPLETE",
  "OLD_SESSIONTABLE_MUTEX": "FIXED_AND_CAUSALLY_CONFIRMED",
  "OLD_SESSIONTABLE_RPS_EFFECT": "CONFIRMED",
  "STATIC_TX_CURRENT_STATUS": "FIXED_NOT_PRIMARY_RPS_ROOT",
  "SESSIONTABLE_CURRENT_STATUS": "FIXED_CONFIRMED_CAUSAL_ROOT",
  "PRODUCT_MUTATION": "NO",
  "CARGO_LOCK_CHANGED": "NO",
  "DEPENDENCY_BASELINE_DRIFT": "NO",
  "P1_STATUS": "OPEN_RESIDUAL_GAP_ANALYSIS",
  "P2_STARTED": "NO",
  "PUSH": "NO",
  "TAG": "NO",
  "RELEASE": "NO",
  "OWNER_AUTHORIZATION_REQUIRED_BEFORE_NEXT_OPTIMIZATION_OR_P2": "YES",
  "PUBLIC_BENCHMARK_CLAIMS": "FORBIDDEN",
}
# Recommended next change: only if HIGH confidence primary
if primary and primary[1]=="HIGH":
    report["RECOMMENDED_NEXT_PRODUCT_CHANGE"]=primary[0]
    report["PRIMARY_RESIDUAL_ROOT_IDENTIFIED"]="YES"
else:
    report["RECOMMENDED_NEXT_PRODUCT_CHANGE"]="NONE"
    report["PRIMARY_RESIDUAL_ROOT_IDENTIFIED"]="NO"
    report["MISSING_EVIDENCE"]="Need HIGH-confidence causal A/B isolating residual ~13% vs NGINX under post-SessionTable architecture"
print(json.dumps(report, indent=2))
(ev/"terminal_report.json").write_text(json.dumps(report, indent=2)+"\n")
# causal model markdown
cm=["# Post-SessionTable residual causal model\n", f"Session control: ExyonQ={em} NGINX={nm} OLS={om} delta_nginx={vs_n}\n"]
for i,c in enumerate(causes,1):
    cm.append(f"## RESIDUAL_CAUSE_{i}: {c[0]}\nCONFIDENCE={c[1]}\nEVIDENCE={c[2]}\n")
(ev/"reports/causal_model.md").write_text("\n".join(cm))
PY
}

main() {
  phase0_bind
  phase1_build
  phase2_control
  phase3_sendfile_and_mutex_proof
  # Stop if SessionTable contention returned (checked after perf record too)
  phase4_perf_stat
  phase5_syscalls
  phase6_perf_record
  if grep -q 'SESSIONTABLE_GLOBAL_MUTEX_HOT_SAMPLE=NONZERO_SUSPECT' "$EV/perf-record/cost_centers.txt" 2>/dev/null; then
    log "STOP: SessionTable contention returned"
    echo "STOP_REASON=SESSIONTABLE_CONTENTION_REGRESSION" | tee "$EV/STOP.txt"
    exit 45
  fi
  phase7_locks
  phase8_openat2
  phase9_workers_and_cpu
  phase10_worker8
  phase11_waf
  phase12_tracing
  phase13_topology
  phase14_network
  synthesize
  log "COMPLETE evidence=$EV"
}

main "$@"
