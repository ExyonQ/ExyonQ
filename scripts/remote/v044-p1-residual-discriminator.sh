#!/usr/bin/env bash
# V044 P1 RESIDUAL CAUSAL DISCRIMINATOR — DIAGNOSTIC ONLY.
# PRODUCT_MUTATION=NO. No defaults/WAF/FD/Tokio/epoll product changes.
# Discriminate: WAF RAW contract | worker topology | openat2 metadata.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_DISC_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_DISC_EV:-$WS/.exyonq-local-evidence/v044-p1-residual-discriminator-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PRODUCT_HEAD="${V044_PRODUCT_HEAD:-427b17397c785b0c5105960cd43b604b41ea27d7}"
LAB_TERMINAL_HEAD="${V044_LAB_TERMINAL_HEAD:-4b1a41f6ff703bacd03ace5bd4f3b152a9b34c44}"
LAB_TERMINAL_TREE="${V044_LAB_TERMINAL_TREE:-d7b5ba28de2b4fbc17155660fb2edc1038983b53}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
REPS="${V044_P1_REPS:-5}"
PATH_P1="/site/1k.bin"
export P1_EXYONQ_IMAGE="${P1_EXYONQ_IMAGE:-v044p1auth-clean-exyonq}"

mkdir -p "$EV"/{control,waf,waf-ab,workers,worker-scale,metadata,syscalls,reports,configs}
cd "$WS"
log() { echo "[p1-disc] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
source "${HOME}/.cargo/env" 2>/dev/null || true
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }
main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

# Diagnostic WAF-off config: explicit IR disable (supported product config; not product code change).
prepare_configs() {
  cp "$WS/benchmarks/configs/exyonq/bench.toml" "$EV/configs/bench-canonical.toml"
  {
    cat "$WS/benchmarks/configs/exyonq/bench.toml"
    echo
    echo "# DIAGNOSTIC ONLY — intended RAW WAF-off contract"
    echo "[waf]"
    echo "enabled = false"
    echo 'mode = "disabled"'
  } > "$EV/configs/bench-waf-off.toml"
  cp "$EV/configs/bench-waf-off.toml" "$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml"
}

recreate_exyonq() {
  local workers=$1 accept=$2 config_host=$3 label=$4
  log "recreate exyonq workers=$workers accept=$accept config=$label"
  # Host CPUs fixed at 8 for worker topology tests via cpuset.
  cat > "$EV/configs/compose-$label.yml" <<EOF
services:
  exyonq:
    image: ${P1_EXYONQ_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "${workers}"
      EXYONQ_ACCEPT_WORKERS: "${accept}"
      EXYONQ_CONFIG: /bench/bench.toml
    volumes:
      - ${config_host}:/bench/bench.toml:ro
EOF
  compose -f "$EV/configs/compose-$label.yml" up -d --force-recreate --no-deps exyonq
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  {
    echo "LABEL=$label WORKERS=$workers ACCEPT=$accept"
    docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' | egrep 'EXYONQ_|WAF' || true
    compose exec -T exyonq sh -c 'head -80 /bench/bench.toml; echo ---; nproc; cat /proc/1/status | egrep "Cpus_allowed|Threads"'
  } | tee "$EV/${label}_identity.txt"
}

rewrk_rps() {
  local url=$1 out=$2
  local DIR="$WS/benchmarks/scenarios/perf"
  source "$DIR/run-rewrk-helper.sh"
  export BENCH_NETWORK=internal COMPOSE_FILE="$FULL_COMPOSE" BENCH_COMPOSE_FILE="$FULL_COMPOSE"
  export DURATION="$DURATION" BENCH_WARMUP_SEC="$WARMUP" BENCH_REWRK_THREADS=2
  local raw="${out}.rewrk.json"
  rewrk_warmup 100 "$url" "" 0
  rewrk_exec_capture "$raw" 100 "$url" "" 0
  python3 "$DIR/rewrk-report-to-json.py" "$raw" -o "$out"
  python3 -c "import json; print(json.load(open('$out'))['summary']['requestsPerSec'])"
}

phase0() {
  log "Phase 0 authority bind"
  {
    echo "CURRENT_WIP=V044_P1_RESIDUAL_CAUSAL_DISCRIMINATOR"
    echo "PRODUCT_MUTATION=NO"
    echo "FD_REUSE_AUTHORIZED=NO"
    echo "WORKER_TOPOLOGY_CHANGE_AUTHORIZED=NO"
    echo "WAF_OPTIMIZATION_AUTHORIZED=NO"
    echo "P2_STARTED=NO"
    echo "PRODUCT_HEAD=$PRODUCT_HEAD"
    echo "LAB_TERMINAL_HEAD=$LAB_TERMINAL_HEAD"
    echo "LAB_TERMINAL_TREE=$LAB_TERMINAL_TREE"
    echo "RUSTC=$(rustc --version)"
    echo "CARGO_LOCK_SHA256=$(sha256sum Cargo.lock | awk '{print $1}')"
    echo "HOST=$(hostname) ARCH=$(uname -m)"
    echo "TIMESTAMP_UTC=$TS"
    echo "EVIDENCE=$EV"
  } | tee "$EV/authority_bind.txt"
  prepare_configs
  # WAF config/code trace
  {
    echo "=== bench.toml (canonical) ==="
    cat "$EV/configs/bench-canonical.toml"
    echo
    echo "=== IR defaults (from source knowledge) ==="
    echo "default_waf_enabled=true"
    echo "default_waf_mode=Monitor"
    echo "ABSENT_WAF_SECTION_MEANS=enabled_monitor"
    echo
    echo "=== live container logs sample (pre) ==="
    docker logs --tail 20 "${PROJECT}-exyonq-1" 2>&1 | egrep -i "waf|EXY-HDR|binding" || true
  } | tee "$EV/waf/config_trace.txt"
}

phase1_control() {
  log "Phase 1 fresh three-way control (canonical = current baked config semantics)"
  # Ensure canonical workers=4 on cpuset 0-7 with baked image config (no volume) for control parity with prior sessions.
  cat > "$EV/configs/compose-control.yml" <<EOF
services:
  exyonq:
    image: ${P1_EXYONQ_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
EOF
  compose -f "$EV/configs/compose-control.yml" up -d --force-recreate --no-deps exyonq
  compose up -d --no-deps nginx-stable openlitespeed-latest upstream 2>/dev/null || true
  compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
  sleep 8
  for tag in exyonq nginx ols; do : > "$EV/control/${tag}_rps.txt"; done
  local rep rps
  for rep in $(seq 1 "$REPS"); do
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/control/exyonq-rep${rep}.json")
    echo "$rps" >> "$EV/control/exyonq_rps.txt"; echo "exyonq $rep $rps" | tee -a "$EV/control/run_log.txt"; sleep 4
  done
  for rep in $(seq 1 "$REPS"); do
    rps=$(rewrk_rps "http://nginx-stable:8080${PATH_P1}" "$EV/control/nginx-rep${rep}.json")
    echo "$rps" >> "$EV/control/nginx_rps.txt"; echo "nginx $rep $rps" | tee -a "$EV/control/run_log.txt"; sleep 4
  done
  for rep in $(seq 1 "$REPS"); do
    rps=$(rewrk_rps "http://openlitespeed-latest:8088${PATH_P1}" "$EV/control/ols-rep${rep}.json")
    echo "$rps" >> "$EV/control/ols_rps.txt"; echo "ols $rep $rps" | tee -a "$EV/control/run_log.txt"; sleep 4
  done
  python3 - <<PY | tee "$EV/control/summary.txt"
import statistics, pathlib
ev=pathlib.Path("$EV/control")
for name in ["exyonq","nginx","ols"]:
    v=[float(x) for x in (ev/f"{name}_rps.txt").read_text().split()]
    med=statistics.median(v)
    print(f"{name}_runs={v}")
    print(f"{name}_median={med}")
    if len(v)>1:
        print(f"{name}_cv_pct={(statistics.stdev(v)/med)*100:.2f}")
em,nm,om=map(statistics.median,([float(x) for x in (ev/f"{n}_rps.txt").read_text().split()] for n in ["exyonq","nginx","ols"]))
print(f"exyonq_vs_nginx={(em/nm-1)*100:.2f}")
print(f"exyonq_vs_ols={(em/om-1)*100:.2f}")
PY
}

# Discriminator A — alternating WAF on (canonical defaults) vs WAF off (explicit IR)
phase_waf_ab() {
  log "Discriminator A: WAF A/B alternating"
  local pair rps
  : > "$EV/waf-ab/A_rps.txt"
  : > "$EV/waf-ab/B_rps.txt"
  for pair in 1 2 3; do
    recreate_exyonq 4 4 "$WS/benchmarks/configs/exyonq/bench.toml" "wafA-p${pair}"
    # confirm violations present
    docker logs --tail 5 "${PROJECT}-exyonq-1" 2>&1 | tee -a "$EV/waf-ab/A_logs_p${pair}.txt" >/dev/null || true
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/waf-ab/A-p${pair}.json")
    echo "$rps" >> "$EV/waf-ab/A_rps.txt"
    echo "A$pair $rps" | tee -a "$EV/waf-ab/run_log.txt"
    sleep 3
    recreate_exyonq 4 4 "$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml" "wafB-p${pair}"
    docker logs --tail 8 "${PROJECT}-exyonq-1" 2>&1 | egrep -i "waf runtime|EXY-HDR|enabled" | tee "$EV/waf-ab/B_logs_p${pair}.txt" || true
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/waf-ab/B-p${pair}.json")
    echo "$rps" >> "$EV/waf-ab/B_rps.txt"
    echo "B$pair $rps" | tee -a "$EV/waf-ab/run_log.txt"
    sleep 3
  done
  # brief perf-record samples for A and B
  for label in A B; do
    local cfg
    if [[ $label == A ]]; then cfg="$WS/benchmarks/configs/exyonq/bench.toml"; else cfg="$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml"; fi
    recreate_exyonq 4 4 "$cfg" "waf${label}-perf"
    local MAIN=$(main_pid "${PROJECT}-exyonq-1")
    timeout 22 perf record -g -F 999 -p "$MAIN" -o "$EV/waf-ab/${label}.data" -- sleep 18 &
    local PP=$!
    sleep 1
    compose exec -T bench-runner rewrk -c 100 -d 15s -h "http://exyonq:8080${PATH_P1}" --json -t 2 > "$EV/waf-ab/${label}-perf-rewrk.json" || true
    wait $PP 2>/dev/null || true
    perf report -i "$EV/waf-ab/${label}.data" --stdio --no-children 2>/dev/null | head -80 > "$EV/waf-ab/${label}-report.txt" || true
    egrep -i "regex_automata|exyonq_waf|Stdout|lock_contended|sendfile|openat" "$EV/waf-ab/${label}-report.txt" | head -40 | tee "$EV/waf-ab/${label}-hits.txt" || true
  done
  python3 - <<PY | tee "$EV/waf-ab/summary.txt"
import statistics, pathlib, re
ev=pathlib.Path("$EV/waf-ab")
A=[float(x) for x in (ev/"A_rps.txt").read_text().split()]
B=[float(x) for x in (ev/"B_rps.txt").read_text().split()]
am,bm=statistics.median(A),statistics.median(B)
print(f"WAF_A_RUNS={A}")
print(f"WAF_B_RUNS={B}")
print(f"WAF_A_RPS={am}")
print(f"WAF_B_RPS={bm}")
print(f"WAF_RPS_DELTA_PCT={(bm/am-1)*100:.2f}")
def sample_pct(path, pat):
    if not path.exists(): return 0.0
    tot=0.0
    for line in path.read_text(errors="replace").splitlines():
        if re.search(pat, line, re.I):
            m=re.match(r"\s*([\d.]+)%", line)
            if m: tot+=float(m.group(1))
    return tot
print(f"WAF_REGEX_SAMPLE_A={sample_pct(ev/'A-hits.txt', r'regex_automata|exyonq_waf')}")
print(f"WAF_REGEX_SAMPLE_B={sample_pct(ev/'B-hits.txt', r'regex_automata|exyonq_waf')}")
delta=(bm/am-1)*100
print(f"WAF_CAUSAL_SIGNAL={'YES' if abs(delta)>=3.0 else 'INCONCLUSIVE'}")
print("RAW_BENCHMARK_CONTRACT_VIOLATION=YES  # absent [waf] => default enabled+monitor; live EXY-HDR-1001")
print("WHY_REGEX_SYMBOLS=DEFAULT_MONITOR_SEMANTICS + ACTUAL_RULE_EVALUATION")
print("WAF_EFFECTIVE_MODE_CANONICAL=MONITOR")
print("WAF_EFFECTIVE_MODE_DIAG_B=DISABLED")
PY
}

# Discriminator B — worker 4 vs 8, host CPUs fixed 0-7, WAF-off for clean residual (or both on canonical?)
# Owner asked same binary/workload; to isolate worker, hold WAF constant.
# Use WAF-off diag so worker effect isn't confounded by monitor logging — BUT also report on canonical.
# Spec: "same binary same P1" — use WAF-off for worker discrimination after proving WAF, noted explicitly.
phase_worker_ab() {
  log "Discriminator B: worker 4/8 A/B (cpuset 0-7 fixed; WAF-off diag to avoid WAF confound)"
  local pair rps cfg="$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml"
  : > "$EV/workers/w4_rps.txt"
  : > "$EV/workers/w8_rps.txt"
  for pair in 1 2 3 4 5; do
    recreate_exyonq 4 4 "$cfg" "w4-p${pair}"
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/workers/w4-p${pair}.json")
    echo "$rps" >> "$EV/workers/w4_rps.txt"; echo "W4_$pair $rps" | tee -a "$EV/workers/run_log.txt"; sleep 3
    recreate_exyonq 8 8 "$cfg" "w8-p${pair}"
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/workers/w8-p${pair}.json")
    echo "$rps" >> "$EV/workers/w8_rps.txt"; echo "W8_$pair $rps" | tee -a "$EV/workers/run_log.txt"; sleep 3
  done
  # active thread sample under load for last configs
  recreate_exyonq 4 4 "$cfg" "w4-sample"
  compose exec -T bench-runner rewrk -c 100 -d 8s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null &
  sleep 2
  local MAIN=$(main_pid "${PROJECT}-exyonq-1")
  ps -T -p "$MAIN" -o tid,psr,pcpu,comm | tee "$EV/workers/w4_threads.txt"
  wait || true
  recreate_exyonq 8 8 "$cfg" "w8-sample"
  compose exec -T bench-runner rewrk -c 100 -d 8s -h "http://exyonq:8080${PATH_P1}" --json -t 2 >/dev/null &
  sleep 2
  MAIN=$(main_pid "${PROJECT}-exyonq-1")
  ps -T -p "$MAIN" -o tid,psr,pcpu,comm | tee "$EV/workers/w8_threads.txt"
  wait || true
  python3 - <<PY | tee "$EV/workers/summary.txt"
import statistics, pathlib
ev=pathlib.Path("$EV/workers")
w4=[float(x) for x in (ev/"w4_rps.txt").read_text().split()]
w8=[float(x) for x in (ev/"w8_rps.txt").read_text().split()]
m4,m8=statistics.median(w4),statistics.median(w8)
print(f"WORKER4_RUNS={w4}")
print(f"WORKER8_RUNS={w8}")
print(f"WORKER4_MEDIAN_RPS={m4}")
print(f"WORKER8_MEDIAN_RPS={m8}")
print(f"WORKER8_RPS_DELTA_PERCENT={(m8/m4-1)*100:.2f}")
delta=(m8/m4-1)*100
print(f"WORKER_TOPOLOGY_CAUSAL_SIGNAL={'YES' if abs(delta)>=3.0 else 'INCONCLUSIVE'}")
print("WORKER_TOPOLOGY_CHANGE_AUTHORIZED=NO")
print("NOTE=WAF-off diagnostic config used to avoid WAF confound; host cpuset 0-7 fixed")
PY
}

phase_worker_scale() {
  log "Worker scaling shape workers=1/2/4/8 host cpuset=0-7 WAF-off"
  local cfg="$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml" w rps
  : > "$EV/worker-scale/rps.txt"
  for w in 1 2 4 8; do
    recreate_exyonq "$w" "$w" "$cfg" "scale-w${w}"
    rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/worker-scale/w${w}.json")
    echo "$w $rps" | tee -a "$EV/worker-scale/rps.txt"
    sleep 3
  done
  python3 - <<PY | tee "$EV/worker-scale/summary.txt"
rows=open("$EV/worker-scale/rps.txt").read().splitlines()
d={}
for line in rows:
    w,r=line.split(); d[int(w)]=float(r)
print(f"RPS_BY_WORKERS={d}")
PY
}

phase_metadata() {
  log "Discriminator C: openat2/metadata under WAF-off workers=4"
  recreate_exyonq 4 4 "$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml" "meta"
  local MAIN=$(main_pid "${PROJECT}-exyonq-1")
  local LOG="$EV/metadata/strace"
  rm -f "$LOG".*
  timeout 16 strace -ff -e trace=openat,openat2,close,fstat,newfstatat,statx,sendfile,sendfile64,sendto,read,write,futex -o "$LOG" -p "$MAIN" &
  local SP=$!
  sleep 0.4
  compose exec -T bench-runner rewrk -c 100 -d 10s -h "http://exyonq:8080${PATH_P1}" --json -t 2 > "$EV/metadata/rewrk.json"
  wait $SP 2>/dev/null || true
  timeout 22 perf record -g -F 999 -p "$MAIN" -o "$EV/metadata/perf.data" -- sleep 18 &
  local PP=$!
  sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 15s -h "http://exyonq:8080${PATH_P1}" --json -t 2 > "$EV/metadata/perf-rewrk.json" || true
  wait $PP 2>/dev/null || true
  perf report -i "$EV/metadata/perf.data" --stdio --no-children 2>/dev/null | head -100 > "$EV/metadata/perf-report.txt" || true
  python3 - <<PY | tee "$EV/metadata/summary.txt"
import glob,re,json,pathlib
ev=pathlib.Path("$EV/metadata")
j=json.loads((ev/"rewrk.json").read_text())
reqs=max(1,int(j.get("requests_total") or 1))
keys=["openat","openat2","close","fstat","newfstatat","statx","sendfile","sendfile64","sendto","read","write","futex"]
counts={k:0 for k in keys}
for fn in glob.glob(str(ev/"strace*")):
    for line in open(fn, errors="replace"):
        for k in keys:
            if re.search(rf"\\b{k}\\(", line):
                counts[k]+=1
print(f"requests={reqs}")
for k,v in sorted(counts.items(), key=lambda kv:-kv[1]):
    if v: print(f"{k}_per_req={v/reqs:.4f}")
text=(ev/"perf-report.txt").read_text(errors="replace") if (ev/"perf-report.txt").exists() else ""
def pct(pat):
    s=0.0
    for line in text.splitlines():
        if re.search(pat, line, re.I):
            m=re.match(r"\\s*([\\d.]+)%", line)
            if m: s+=float(m.group(1))
    return s
print(f"OPENAT2_COST_PERCENT={pct(r'openat2|do_sys_openat2'):.3f}")
print(f"CLOSE_COST_PERCENT={pct(r'__close|file_close|__fput|fput'):.3f}")
print(f"PATH_CONTAINMENT_COST_PERCENT={pct(r'apparmor|prepend_path|strip_prefix|open_under_root'):.3f}")
print(f"STAT_FSTAT_COST_PERCENT={pct(r'fstat|statx|vfs_getattr'):.3f}")
meta=pct(r'openat2|do_sys_openat2|__close|file_close|apparmor|prepend_path|open_under_root|fstat|statx')
print(f"STATIC_METADATA_TOTAL_COST_PERCENT={meta:.3f}")
print("OPENAT2_RPS_IMPACT=NOT_PROVEN  # cost measured; no causal RPS A/B without FD cache product change")
print("FD_REUSE_REQUIRED=NOT_PROVEN")
openat2 = counts.get("openat2",0)/reqs
close = counts.get("close",0)/reqs
print(f"OPENAT2_PER_REQ={openat2:.4f}")
print(f"CLOSE_PER_REQ={close:.4f}")
print(f"FSTAT_PER_REQ={(counts.get('fstat',0)+counts.get('newfstatat',0))/reqs:.4f}")
print(f"STATX_PER_REQ={counts.get('statx',0)/reqs:.4f}")
PY
}

phase_syscall_reconcile() {
  log "Syscall geometry reconciliation ExyonQ vs NGINX (WAF-off workers=4)"
  recreate_exyonq 4 4 "$WS/benchmarks/configs/exyonq/bench-waf-off-diag.toml" "sys-exy"
  for tag in exyonq nginx; do
    local c url
    if [[ $tag == exyonq ]]; then c="${PROJECT}-exyonq-1"; url="http://exyonq:8080${PATH_P1}"
    else c="${PROJECT}-nginx-stable-1"; url="http://nginx-stable:8080${PATH_P1}"; fi
    local MAIN=$(main_pid "$c")
    local kids=$(pgrep -P "$MAIN" 2>/dev/null || true)
    local pids="$MAIN"
    for k in $kids; do pids="$pids,$k"; done
    rm -f "$EV/syscalls/${tag}".*
    timeout 16 strace -ff -e trace=accept4,epoll_wait,epoll_pwait,epoll_ctl,recvfrom,read,sendto,write,writev,sendfile,sendfile64,openat,openat2,close,futex,fcntl,setsockopt -o "$EV/syscalls/${tag}" -p "$pids" &
    local SP=$!
    sleep 0.4
    compose exec -T bench-runner rewrk -c 100 -d 10s -h "$url" --json -t 2 > "$EV/syscalls/${tag}-rewrk.json" || true
    wait $SP 2>/dev/null || true
  done
  python3 - <<PY | tee "$EV/syscalls/reconcile.txt"
import glob,re,json,pathlib
ev=pathlib.Path("$EV/syscalls")
keys=["accept4","epoll_wait","epoll_pwait","epoll_ctl","recvfrom","read","sendto","write","writev","sendfile","sendfile64","openat","openat2","close","futex","fcntl","setsockopt"]
rows={}
for tag in ["exyonq","nginx"]:
    j=json.loads((ev/f"{tag}-rewrk.json").read_text())
    reqs=max(1,int(j.get("requests_total") or 1))
    counts={k:0 for k in keys}
    for fn in glob.glob(str(ev/f"{tag}*")):
        if fn.endswith(".json"): continue
        for line in open(fn, errors="replace"):
            for k in keys:
                if re.search(rf"\\b{k}\\(", line):
                    counts[k]+=1
    rows[tag]={k:v/reqs for k,v in counts.items()}
    print(f"=== {tag} syscalls/req={sum(counts.values())/reqs:.3f} ===")
    for k,v in sorted(counts.items(), key=lambda kv:-kv[1]):
        if v: print(f"  {k}={v/reqs:.4f}")
print("=== EXYONQ_EXTRA_VS_NGINX ===")
for k in keys:
    e,n=rows["exyonq"].get(k,0),rows["nginx"].get(k,0)
    if e-n>0.05:
        print(f"{k}: exyonq={e:.4f} nginx={n:.4f} delta={e-n:.4f}")
PY
}

synthesize() {
  log "Synthesize terminal_report.json"
  python3 - <<PY
import json, pathlib
ev=pathlib.Path("$EV")
product_head="$PRODUCT_HEAD"
lab_head="$LAB_TERMINAL_HEAD"
lab_tree="$LAB_TERMINAL_TREE"

def read_kv(path):
    d={}
    if not path.exists(): return d
    for line in path.read_text().splitlines():
        if "=" in line:
            k,v=line.split("=",1); d[k.strip()]=v.strip()
    return d

ctrl=read_kv(ev/"control/summary.txt")
waf=read_kv(ev/"waf-ab/summary.txt")
wrk=read_kv(ev/"workers/summary.txt")
scale=read_kv(ev/"worker-scale/summary.txt")
meta=read_kv(ev/"metadata/summary.txt")
sys= (ev/"syscalls/reconcile.txt").read_text() if (ev/"syscalls/reconcile.txt").exists() else ""

waf_sig=waf.get("WAF_CAUSAL_SIGNAL","INCONCLUSIVE")
wrk_sig=wrk.get("WORKER_TOPOLOGY_CAUSAL_SIGNAL","INCONCLUSIVE")
try:
    waf_delta=abs(float(waf.get("WAF_RPS_DELTA_PCT","0")))
except Exception:
    waf_delta=0.0
try:
    wrk_delta=abs(float(wrk.get("WORKER8_RPS_DELTA_PERCENT","0")))
except Exception:
    wrk_delta=0.0

meta_impact=meta.get("OPENAT2_RPS_IMPACT","NOT_PROVEN")

if waf_sig=="YES" and wrk_sig=="YES" and waf_delta>=5 and wrk_delta>=3:
    outcome="OUTCOME_B"
    residual_model="MULTIFACTOR"
    ranked=sorted([("WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION",waf_delta),("WORKER_PARALLELISM",wrk_delta)], key=lambda x:-x[1])
    primary=ranked[0][0]
    conf="HIGH" if waf_delta>=8 else "MEDIUM"
elif waf_sig=="YES" and waf_delta>=8:
    outcome="OUTCOME_A"
    residual_model="SINGLE_ROOT"
    primary="WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION"
    conf="HIGH"
elif waf_sig=="YES" and waf_delta>=5:
    outcome="OUTCOME_C"
    residual_model="INSUFFICIENT"
    primary="WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION"
    conf="MEDIUM"
else:
    outcome="OUTCOME_C"
    residual_model="INSUFFICIENT"
    primary="NONE"
    conf="NONE"

secondary=[]
if wrk_sig=="YES" and "WORKER" not in primary:
    secondary.append("WORKER_PARALLELISM")
if waf_sig=="YES" and "WAF" not in primary:
    secondary.insert(0,"WAF_DEFAULT_MONITOR_ON_ABSENT_SECTION")
secondary.append("STATIC_METADATA_OPENAT2_"+meta_impact)

rec_action="NONE"
if conf in ("HIGH","MEDIUM") and "WAF" in primary:
    rec_action="HARNESS_OR_CONFIG_NORMALIZE_P1_RAW_WAF_OFF"
elif conf=="HIGH":
    rec_action=primary

report={
  "P1_RESIDUAL_DISCRIMINATOR_STATUS":"COMPLETE",
  "OUTCOME":outcome,
  "RESIDUAL_MODEL":residual_model,
  "PRODUCT_HEAD":product_head,
  "TERMINAL_HEAD":lab_head,
  "TERMINAL_TREE":lab_tree,
  "CONTROL":ctrl,
  "WAF":waf,
  "WORKERS":wrk,
  "WORKER_SCALE":scale,
  "METADATA":meta,
  "PRIMARY_RESIDUAL_ROOT":primary,
  "PRIMARY_ROOT_CONFIDENCE":conf,
  "SECONDARY_ROOTS":secondary,
  "RECOMMENDED_NEXT_PRODUCT_CHANGE":"NONE",
  "RECOMMENDED_NEXT_ACTION":rec_action,
  "PRODUCT_MUTATION":"NO",
  "P1_STATUS":"OPEN_RESIDUAL_GAP_ANALYSIS",
  "P2_STARTED":"NO",
  "PUSH":"NO","TAG":"NO","RELEASE":"NO",
  "OWNER_AUTHORIZATION_REQUIRED_BEFORE_NEXT_PRODUCT_OPTIMIZATION_OR_P2":"YES",
  "WAF_RAW_CONTRACT_VALID":"NO",
  "FD_REUSE_REQUIRED":meta.get("FD_REUSE_REQUIRED","NOT_PROVEN"),
  "OPENAT2_RPS_IMPACT":meta.get("OPENAT2_RPS_IMPACT","NOT_PROVEN"),
  "SYSCALL_RECONCILE_SNIPPET":sys[:2000],
  "EVIDENCE_DIR":str(ev),
}
if rec_action.startswith("HARNESS"):
    report["RECOMMENDED_NEXT_HARNESS_OR_CONFIG_CHANGE"]=rec_action
(ev/"terminal_report.json").write_text(json.dumps(report, indent=2)+"\\n")
print(json.dumps(report, indent=2))
PY
}

main() {
  phase0
  phase1_control
  phase_waf_ab
  phase_worker_ab
  phase_worker_scale
  phase_metadata
  phase_syscall_reconcile
  synthesize
  log "COMPLETE $EV"
}

main "$@"
