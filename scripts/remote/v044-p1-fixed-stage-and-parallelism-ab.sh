#!/usr/bin/env bash
# V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B — READ-ONLY measurement.
# PRODUCT_MUTATION=NO. ARCHITECTURE_MUTATION=NO. P4=NO.
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# CRITICAL: Cap067 keepalive pool size is:
#   min(EXYONQ_EPOLL_POOL_THREADS.unwrap_or(4), EXYONQ_ACCEPT_WORKERS)
# Historical A/B that only raised WORKER_THREADS/ACCEPT_WORKERS without
# EXYONQ_EPOLL_POOL_THREADS left Cap067 pinned at 4 → INCONCLUSIVE.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P1_AB_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P1_AB_EV:-$WS/.exyonq-local-evidence/v044-p1-fixed-stage-parallelism-ab-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P1="/site/1k.bin"
PATH_P3="/site/1m.bin"
WARMUP_SEC="${BENCH_WARMUP_SEC:-20}"
MEASURE_SEC="${BENCH_MEASURE_SEC:-30}"
REPS="${V044_P1_AB_REPS:-5}"
EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f}"
P1_EXYONQ_IMAGE="${P1_EXYONQ_IMAGE:-v044p1auth-clean-exyonq}"

mkdir -p "$EV"/{meta,arms,ka,stages,p3,reports,restore}
cd "$WS"
log() { echo "[p1-ab] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" --profile bench "$@"; }
ctr() { echo "${PROJECT}-exyonq-1"; }

# --- recreate with explicit worker geometry (runtime env only) ---
recreate_arm() {
  local label=$1 wt=$2 aw=$3 pool=$4
  log "recreate arm=$label WT=$wt AW=$aw EPOLL_POOL=$pool"
  cat > "$EV/meta/compose-${label}.yml" <<EOF
services:
  exyonq:
    image: ${P1_EXYONQ_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "${wt}"
      EXYONQ_ACCEPT_WORKERS: "${aw}"
      EXYONQ_EPOLL_POOL_THREADS: "${pool}"
      EXYONQ_CONFIG: /bench/bench.toml
EOF
  compose -f "$EV/meta/compose-${label}.yml" up -d --force-recreate --no-deps exyonq
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  local c sha
  c=$(ctr)
  sha=$(docker exec "$c" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  {
    echo "LABEL=$label WT=$wt AW=$aw EPOLL_POOL=$pool"
    echo "EXYONQ_BINARY_SHA256=$sha"
    docker inspect -f 'Cpuset={{.HostConfig.CpusetCpus}} CpuQuota={{.HostConfig.CpuQuota}}' "$c"
    docker exec "$c" sh -c 'env | egrep "EXYONQ_(WORKER|ACCEPT|EPOLL)" | sort'
    docker exec "$c" grep -A3 '^\[waf\]' /bench/bench.toml || true
  } | tee "$EV/arms/${label}_identity.txt"
  if [[ "$sha" != "$EXPECTED_EXYONQ_SHA256" ]]; then
    log "FAIL binary sha $sha != $EXPECTED_EXYONQ_SHA256"
    exit 2
  fi
}

run_rewrk_json() {
  local url=$1 out=$2
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json >"$out"
}

run_rewrk_pct() {
  local url=$1 out=$2
  compose exec -T bench-runner bash -lc \
    "rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" | tee "$out"
}

# docker-stats CPU% + Mem during measure
run_arm_reps() {
  local label=$1
  local url="http://exyonq:8080${PATH_P1}"
  local c rep statsjson
  c=$(ctr)
  mkdir -p "$EV/arms/$label"
  : >"$EV/arms/${label}_rps.txt"
  for rep in $(seq 1 "$REPS"); do
    log "arm=$label rep=$rep/$REPS"
    statsjson="$EV/arms/$label/cpu-rep${rep}.txt"
    : >"$statsjson"
    timeout $((MEASURE_SEC + WARMUP_SEC + 20)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
    local sp=$!
    compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
    compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
      >"$EV/arms/$label/rep${rep}.json"
    wait "$sp" 2>/dev/null || true
    python3 -c "import json; j=json.load(open('$EV/arms/$label/rep${rep}.json')); print(j.get('requests_avg') or j.get('summary',{}).get('requestsPerSec'))" \
      | tee -a "$EV/arms/${label}_rps.txt"
    sleep 3
  done
  run_rewrk_pct "$url" "$EV/arms/$label/pct.txt" || true
}

# Under load: thread/CPU distribution + perf-stat counters
sample_under_load() {
  local label=$1
  local url="http://exyonq:8080${PATH_P1}"
  local c MAIN
  c=$(ctr)
  MAIN=$(docker inspect -f '{{.State.Pid}}' "$c")
  mkdir -p "$EV/arms/$label/sample"
  compose exec -T bench-runner rewrk -c 100 -d 18s -t 2 -h "$url" >/dev/null 2>&1 &
  local lp=$!
  sleep 2
  # Host view of container threads
  ps -T -p "$MAIN" -o tid,psr,pcpu,cputime,stat,comm 2>/dev/null \
    | tee "$EV/arms/$label/sample/threads.txt" || true
  # Per-CPU from /proc/<pid>/task
  python3 - "$MAIN" "$EV/arms/$label/sample/per_cpu.txt" <<'PY' || true
import os, sys, collections
pid = int(sys.argv[1])
out = sys.argv[2]
task = f"/proc/{pid}/task"
rows = []
by_cpu = collections.Counter()
for tid in os.listdir(task):
    try:
        with open(f"{task}/{tid}/stat") as f:
            parts = f.read().split()
        # field 39 = processor (0-based) in linux
        comm = parts[1].strip("()")
        processor = int(parts[38]) if len(parts) > 38 else -1
        utime = int(parts[13]); stime = int(parts[14])
        rows.append((tid, comm, processor, utime+stime))
        by_cpu[processor] += utime + stime
    except Exception:
        pass
rows.sort(key=lambda r: -r[3])
with open(out, "w") as f:
    f.write("tid comm processor jiffies\n")
    for r in rows[:80]:
        f.write(f"{r[0]} {r[1]} {r[2]} {r[3]}\n")
    f.write("\n# jiffies_by_cpu\n")
    for cpu, j in sorted(by_cpu.items()):
        f.write(f"cpu{cpu} {j}\n")
PY
  # perf stat (process-wide) during remaining load
  timeout 14 perf stat -p "$MAIN" -e cycles,instructions,context-switches,cpu-migrations,task-clock,cycles:u,cycles:k \
    -o "$EV/arms/$label/sample/perf-stat.txt" -- sleep 12 || true
  # futex/epoll sample via perf record (short)
  timeout 12 perf record -F 997 -g -p "$MAIN" -o "$EV/arms/$label/sample/perf.data" -- sleep 8 || true
  wait "$lp" 2>/dev/null || true
  if [[ -f "$EV/arms/$label/sample/perf.data" ]]; then
    perf report -i "$EV/arms/$label/sample/perf.data" --stdio --no-children 2>/dev/null \
      | head -120 > "$EV/arms/$label/sample/perf-report.txt" || true
    egrep -i 'futex|rwlock|mutex|SendfileHandle|epoll_wait|sendfile|parking_lot|std::sys|tokio|exyonq' \
      "$EV/arms/$label/sample/perf-report.txt" \
      | head -60 | tee "$EV/arms/$label/sample/perf-hits.txt" || true
  fi
  # cgroup cpu.stat snapshot
  CG=$(docker inspect -f '{{.HostConfig.CgroupParent}}' "$c" 2>/dev/null || true)
  docker exec "$c" sh -c 'cat /sys/fs/cgroup/cpu.stat 2>/dev/null || cat /sys/fs/cgroup/cpu/cpu.stat 2>/dev/null' \
    | tee "$EV/arms/$label/sample/cgroup-cpu.stat" || true
}

summarize_arm() {
  local label=$1
  python3 - "$EV" "$label" <<'PY' | tee "$EV/arms/${2:-$1}_summary.txt"
import json, re, statistics, sys
from pathlib import Path
ev = Path(sys.argv[1]); label = sys.argv[2]
arm = ev / "arms" / label
rps = [float(x) for x in (ev / f"arms/{label}_rps.txt").read_text().split()]
med = statistics.median(rps)
cv = (statistics.stdev(rps)/med) if len(rps)>1 and med else None
# cpu from docker stats
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
cpus, rss = [], []
for p in sorted(arm.glob("cpu-rep*.txt")):
    for ln in p.read_text(errors="replace").splitlines():
        ln = ansi.sub("", ln).strip()
        m = re.search(r"([\d.]+)%\s+(\d+(?:\.\d+)?)(MiB|GiB|KiB)", ln)
        if not m: continue
        cpus.append(float(m.group(1)))
        val=float(m.group(2)); unit=m.group(3)
        mul={"KiB":1/1024,"MiB":1,"GiB":1024}[unit]
        rss.append(val*mul)
cpu_med = statistics.median(cpus) if cpus else None
rss_med = statistics.median(rss) if rss else None
# core-ms/req ≈ (cpu%/100 * cores * 1000) / rps   with cores=8
core_ms = None
if cpu_med is not None and med:
    core_ms = (cpu_med/100.0 * 8.0 * 1000.0) / med
pct = {}
ptxt = (arm/"pct.txt").read_text(errors="replace") if (arm/"pct.txt").exists() else ""
for p in ("50","95","99"):
    m = re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", ptxt)
    if m: pct[f"p{p}"] = float(m.group(1))
print(f"LABEL={label}")
print(f"RPS_RUNS={rps}")
print(f"RPS_MEDIAN={med}")
print(f"RPS_CV={cv}")
print(f"CPU_PERCENT_MEDIAN={cpu_med}")
print(f"RSS_MIB_MEDIAN={rss_med}")
print(f"CPU_CORE_MS_PER_REQ={core_ms}")
for k,v in pct.items():
    print(f"{k.upper()}={v}")
# write machine json
out = {
  "label": label, "rps_runs": rps, "rps_median": med, "rps_cv": cv,
  "cpu_percent_median": cpu_med, "rss_mib_median": rss_med,
  "cpu_core_ms_per_req": core_ms, **pct
}
(arm/"summary.json").write_text(json.dumps(out, indent=2)+"\n")
PY
}

# --- A/B-B: first-request vs keepalive (same arm A0) ---
# close: 100 serial connects × N requests with Connection: close
# ka: 100 connections, many requests reused
phase_first_vs_ka() {
  local label=${1:-A0}
  log "A/B-B first vs keepalive on arm=$label"
  mkdir -p "$EV/ka"
  local url="http://exyonq:8080${PATH_P1}"
  # Keepalive bulk (rewrk default)
  compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" --json \
    >"$EV/ka/keepalive-rewrk.json"
  # Connection: close via Python in bench-runner (real sockets)
  compose exec -T bench-runner python3 - <<'PY' | tee "$EV/ka/close-vs-ka.txt"
import json, socket, time, statistics, urllib.parse
host = "exyonq"
port = 8080
path = "/site/1k.bin"
req_close = (
    f"GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
).encode()
req_ka = (
    f"GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n"
).encode()

def one_shot(n=400):
    lat = []
    for _ in range(n):
        t0 = time.perf_counter()
        s = socket.create_connection((host, port), timeout=5)
        s.sendall(req_close)
        buf = b""
        while b"\r\n\r\n" not in buf or (b"Content-Length:" in buf and len(buf) < 1200):
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
            if len(buf) > 8192 and b"\r\n\r\n" in buf:
                # enough for 1k body
                cl = 1024
                for ln in buf.split(b"\r\n"):
                    if ln.lower().startswith(b"content-length:"):
                        cl = int(ln.split(b":",1)[1].strip())
                hdr = buf.find(b"\r\n\r\n")+4
                while len(buf) < hdr+cl:
                    chunk = s.recv(65536)
                    if not chunk: break
                    buf += chunk
                break
        s.close()
        lat.append((time.perf_counter()-t0)*1e6)
    return lat

def keepalive_session(conns=20, reqs_per=40):
    first = []
    subsequent = []
    for _ in range(conns):
        s = socket.create_connection((host, port), timeout=5)
        for i in range(reqs_per):
            t0 = time.perf_counter()
            s.sendall(req_ka)
            buf = b""
            while True:
                chunk = s.recv(65536)
                if not chunk:
                    break
                buf += chunk
                if b"\r\n\r\n" not in buf:
                    continue
                cl = 1024
                for ln in buf.split(b"\r\n"):
                    if ln.lower().startswith(b"content-length:"):
                        cl = int(ln.split(b":",1)[1].strip())
                hdr = buf.find(b"\r\n\r\n")+4
                if len(buf) >= hdr+cl:
                    break
            dt = (time.perf_counter()-t0)*1e6
            if i == 0:
                first.append(dt)
            else:
                subsequent.append(dt)
            # leftover buffering not handled; reopen if short
            if len(buf) < 100:
                break
        s.close()
    return first, subsequent

close_lat = one_shot(500)
first, sub = keepalive_session(25, 50)

def pct(xs, p):
    xs = sorted(xs)
    if not xs: return None
    k = int(round((p/100)*(len(xs)-1)))
    return xs[k]

print(f"CLOSE_N={len(close_lat)}")
print(f"CLOSE_P50_US={pct(close_lat,50)}")
print(f"CLOSE_P95_US={pct(close_lat,95)}")
print(f"CLOSE_P99_US={pct(close_lat,99)}")
print(f"CLOSE_MEAN_US={statistics.mean(close_lat)}")
print(f"KA_FIRST_N={len(first)}")
print(f"KA_FIRST_P50_US={pct(first,50)}")
print(f"KA_FIRST_P95_US={pct(first,95)}")
print(f"KA_FIRST_MEAN_US={statistics.mean(first) if first else None}")
print(f"KA_SUB_N={len(sub)}")
print(f"KA_SUB_P50_US={pct(sub,50)}")
print(f"KA_SUB_P95_US={pct(sub,95)}")
print(f"KA_SUB_MEAN_US={statistics.mean(sub) if sub else None}")
if first and sub:
    print(f"HANDOFF_PROXY_DELTA_MEAN_US={statistics.mean(first)-statistics.mean(sub)}")
    print(f"HANDOFF_PROXY_DELTA_P50_US={pct(first,50)-pct(sub,50)}")
PY
}

# --- A/B-C: stage attribution via perf (A0 under load) ---
phase_stages() {
  local label=${1:-A0}
  log "A/B-C stage/perf attribution on arm=$label"
  mkdir -p "$EV/stages"
  sample_under_load "$label"
  # Copy sample into stages/
  cp -a "$EV/arms/$label/sample/." "$EV/stages/" 2>/dev/null || true
  python3 - "$EV/stages" <<'PY' | tee "$EV/stages/attribution.txt"
from pathlib import Path
import re, sys
ev = Path(sys.argv[1])
report = (ev/"perf-report.txt").read_text(errors="replace") if (ev/"perf-report.txt").exists() else ""
stat = (ev/"perf-stat.txt").read_text(errors="replace") if (ev/"perf-stat.txt").exists() else ""

def pct_for(pat):
    tot = 0.0
    for ln in report.splitlines():
        if re.search(pat, ln, re.I):
            m = re.match(r"\s*([\d.]+)%", ln)
            if m:
                tot += float(m.group(1))
    return tot

keys = {
  "FUTEX": r"futex|__futex",
  "RWLOCK": r"rwlock|RwLock|pthread_rwlock",
  "MUTEX": r"mutex|Mutex|parking_lot",
  "SENDFILE_REGISTRY": r"SendfileHandle|sendfile_fd_cache|HandleRegistry",
  "EPOLL_WAIT": r"epoll_wait",
  "SENDFILE": r"\bsendfile\b",
  "TOKIO": r"tokio::",
  "WAF": r"waf|NopWaf|evaluate_wire",
}
print("PERF_SYMBOL_PERCENT_SUMS (self, may double-count)")
for k,pat in keys.items():
    print(f"{k}_PCT={pct_for(pat):.4f}")
print("--- perf-stat ---")
print(stat)
print("--- classification (measurement) ---")
print("ROOTS_RWLOCK_COST_US=BELOW_MEASUREMENT_RESOLUTION_OR_NOT_VISIBLE_IN_PERF")
print("HANDLE_REGISTRY_MUTEX_COST_US=SEE_PERF_AND_HISTORICAL_ROOT2")
print("WAF_OFF_GATE_COST_US=BELOW_MEASUREMENT_RESOLUTION_IF_ABSENT")
print("GENERATION_GATE_COST_US=BELOW_MEASUREMENT_RESOLUTION_IF_ABSENT")
print("ACCESS_HOOK_COST_US=BELOW_MEASUREMENT_RESOLUTION_IF_ABSENT")
print("NOTE=No product instrumentation; attribution is perf-sample bound only")
PY
}

# short P3 sanity (one warmup+one measure) on an arm
phase_p3_sanity() {
  local label=$1
  log "P3 sanity on arm=$label"
  mkdir -p "$EV/p3/$label"
  local url="http://exyonq:8080${PATH_P3}"
  local c
  c=$(ctr)
  # probe
  out=$(compose exec -T bench-runner curl -sS -m 15 -o /tmp/p3.bin -w "%{http_code} %{size_download}" "$url")
  echo "probe=$out" | tee "$EV/p3/$label/probe.txt"
  compose exec -T bench-runner rewrk -c 100 -d 10s -t 2 -h "$url" >/dev/null 2>&1 || true
  timeout 50 docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$EV/p3/$label/cpu.txt" 2>/dev/null &
  local sp=$!
  compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" --json >"$EV/p3/$label/rewrk.json"
  wait "$sp" 2>/dev/null || true
  compose exec -T bench-runner bash -lc "rewrk -c 100 -d 20s -t 2 -h $url --pct 2>&1" \
    | tee "$EV/p3/$label/pct.txt" || true
  python3 - "$EV/p3/$label" <<'PY' | tee "$EV/p3/$label/summary.txt"
import json,re,statistics,sys
from pathlib import Path
d=Path(sys.argv[1])
j=json.loads((d/"rewrk.json").read_text())
rps=j.get("requests_avg") or j.get("summary",{}).get("requestsPerSec")
ansi=re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
cpus=[]
for ln in (d/"cpu.txt").read_text(errors="replace").splitlines():
    ln=ansi.sub("",ln).strip()
    m=re.search(r"([\d.]+)%", ln)
    if m: cpus.append(float(m.group(1)))
cpu=statistics.median(cpus) if cpus else None
core_ms=(cpu/100*8*1000/rps) if cpu and rps else None
ptxt=(d/"pct.txt").read_text(errors="replace") if (d/"pct.txt").exists() else ""
pct={}
for p in ("50","95","99"):
    m=re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", ptxt)
    if m: pct[f"p{p}"]=float(m.group(1))
print(f"P3_RPS={rps}")
print(f"P3_CPU_PERCENT={cpu}")
print(f"P3_CORE_MS_PER_REQ={core_ms}")
for k,v in pct.items(): print(f"P3_{k.upper()}={v}")
PY
}

restore_canonical() {
  log "restore canonical A0 geometry WT=4 AW=4 EPOLL_POOL=4"
  recreate_arm A0_restore 4 4 4
  echo "restored $(date -u +%Y%m%dT%H%M%SZ)" | tee "$EV/restore/done.txt"
}

# ---------------- main ----------------
{
  echo "WIP=V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B"
  echo "PRODUCT_MUTATION=NO"
  echo "ARCHITECTURE_MUTATION=NO"
  echo "P4_STARTED=NO"
  echo "PRODUCT_AUTHORITY_HEAD=06742e1bdc67b5ed86ef95750399e9edad16d646"
  echo "PRODUCT_AUTHORITY_TREE=5818095793373bd322ac13a2b3ebcf32661941e1"
  echo "EXPECTED_EXYONQ_SHA256=$EXPECTED_EXYONQ_SHA256"
  echo "HOST=$(hostname) ARCH=$(uname -m)"
  echo "TS=$TS EV=$EV"
  echo "NOTE=Cap067 pool = min(EPOLL_POOL_THREADS.default4, ACCEPT_WORKERS)"
} | tee "$EV/meta/authority.txt"

compose up -d --no-deps nginx-stable openlitespeed-latest upstream 2>/dev/null || true
compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true

# A0 canonical
recreate_arm A0 4 4 4
run_arm_reps A0
sample_under_load A0
summarize_arm A0

# A1 full scale Cap067+accept+tokio to 8
recreate_arm A1 8 8 8
run_arm_reps A1
sample_under_load A1
summarize_arm A1

# A2 intermediate
recreate_arm A2 6 6 6
run_arm_reps A2
summarize_arm A2

# A_TOKIO: raise Tokio only; Cap067 stays 4
recreate_arm A_TOKIO 8 4 4
run_arm_reps A_TOKIO
summarize_arm A_TOKIO

# A_CAP067: raise Cap067+accept; Tokio stays 4
recreate_arm A_CAP067 4 8 8
run_arm_reps A_CAP067
sample_under_load A_CAP067
summarize_arm A_CAP067

# First vs KA on A0
recreate_arm A0 4 4 4
phase_first_vs_ka A0
phase_stages A0

# P3 sanity on A0 and best of A1/A_CAP067 (always both for safety)
phase_p3_sanity A0
recreate_arm A1 8 8 8
phase_p3_sanity A1

restore_canonical

# Cross-arm terminal
python3 - "$EV" <<'PY' | tee "$EV/reports/ab_terminal.txt"
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
arms = ["A0","A1","A2","A_TOKIO","A_CAP067"]
rows = {}
for a in arms:
    p = ev/"arms"/a/"summary.json"
    if p.exists():
        rows[a] = json.loads(p.read_text())

def delta(a,b,key):
    if a not in rows or b not in rows: return None
    va, vb = rows[a].get(key), rows[b].get(key)
    if va is None or vb is None or va==0: return None
    return (vb/va - 1)*100

print("=== ARM TABLE ===")
for a,r in rows.items():
    print(f"{a}: RPS={r.get('rps_median')} CV={r.get('rps_cv')} CPU%={r.get('cpu_percent_median')} "
          f"core_ms={r.get('cpu_core_ms_per_req')} P50={r.get('p50')} P95={r.get('p95')} P99={r.get('p99')}")

base="A0"
print("\n=== DELTAS vs A0 ===")
for a in arms:
    if a==base: continue
    print(f"{a}_RPS_DELTA_PCT={delta(base,a,'rps_median')}")
    print(f"{a}_CPU_CORE_MS_DELTA_PCT={delta(base,a,'cpu_core_ms_per_req')}")
    print(f"{a}_P95_DELTA_PCT={delta(base,a,'p95')}")
    print(f"{a}_P99_DELTA_PCT={delta(base,a,'p99')}")

a1 = delta(base,"A1","rps_median")
cap = delta(base,"A_CAP067","rps_median")
tok = delta(base,"A_TOKIO","rps_median")
print("\n=== PARALLELISM CLASSIFICATION ===")
print(f"A1_FULL8_RPS_DELTA_PCT={a1}")
print(f"A_CAP067_RPS_DELTA_PCT={cap}")
print(f"A_TOKIO_RPS_DELTA_PCT={tok}")

def material(d, thr=3.0):
    return d is not None and abs(d) >= thr

if material(a1) and a1 > 0:
    print("IS_WORKER_COUNT_LIMITING_RPS=YES")
    print("DO_MORE_WORKERS_IMPROVE_RPS=YES")
elif material(cap) and cap > 0 and (tok is None or tok < 3):
    print("IS_WORKER_COUNT_LIMITING_RPS=YES  # Cap067/accept pool")
    print("DO_MORE_WORKERS_IMPROVE_RPS=YES")
elif a1 is not None and abs(a1) < 3:
    print("IS_WORKER_COUNT_LIMITING_RPS=NOT_PROVEN")
    print("DO_MORE_WORKERS_IMPROVE_RPS=NO")
else:
    print("IS_WORKER_COUNT_LIMITING_RPS=NOT_PROVEN")
    print("DO_MORE_WORKERS_IMPROVE_RPS=UNKNOWN")

for lab in ["A0","A1"]:
    sp = ev/"p3"/lab/"summary.txt"
    if sp.exists():
        print(f"\n=== P3 {lab} ===")
        print(sp.read_text())
PY

log "DONE EV=$EV"
echo "$EV"
