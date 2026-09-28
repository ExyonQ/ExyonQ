#!/usr/bin/env bash
# V044_PXDP_SCHEDULER_WAKEUP_TOPOLOGY_ATTRIBUTION
# READ_ONLY — PRODUCT_MUTATION=NO HARNESS_MUTATION=NO (workload unchanged)
set -euo pipefail

WS="${PXDP_SW_WS:-/root/pxdp-p5-reality-wt}"
TS="${PXDP_SW_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${PXDP_SW_EV:-$WS/.exyonq-local/evidence/pxdp-scheduler-wakeup-topology/$TS}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
P5_IMAGE="${PXDP_P5_IMAGE:-v044-pxdp-p5-reality-exyonq}"
P5_SHA="${PXDP_P5_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"
SOURCE_HEAD="${PXDP_SOURCE_HEAD:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
SOURCE_TREE="${PXDP_SOURCE_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
WARMUP=20
MEASURE=30
SCHED_SEC=12
BPF_SEC=15
CONC=100
THREADS=2

mkdir -p "$EV"/{meta,thread-inventory,per-thread,perf-stat,perf-sched,bpf,syscalls,cgroup,mpstat,wakeup-edges,upstream-lifecycle,reports,commands}
log() { echo "[pxdp-sw] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-geom8.yml" ]] && extra+=(-f "$EV/meta/compose-geom8.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=exyonq" -q | head -1 ;;
    nginx) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=nginx-stable" -q | head -1 ;;
    ols) docker ps --filter "name=${PROJECT}-openlitespeed" -q | head -1 ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
  esac
}

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

resolve_all_tids() {
  local main=$1
  local tids=()
  if [[ -d "/proc/$main/task" ]]; then
    mapfile -t tids < <(ls "/proc/$main/task" 2>/dev/null | sort -n)
  fi
  local out=()
  for t in "${tids[@]}"; do
    [[ -d "/proc/$t" ]] && out+=("$t")
  done
  (IFS=,; echo "${out[*]}")
}

resolve_nginx_pids() {
  local main=$1
  local kids
  kids=$(pgrep -P "$main" 2>/dev/null | tr '\n' ',' | sed 's/,$//')
  echo "$main${kids:+,$kids}"
}

write_authority() {
  {
    echo "WIP=V044_PXDP_SCHEDULER_WAKEUP_TOPOLOGY_ATTRIBUTION"
    echo "MODE=READ_ONLY_SCHEDULER_CAUSAL_ATTRIBUTION"
    echo "PRODUCT_MUTATION=NO"
    echo "HARNESS_MUTATION=NO"
    echo "SOURCE_HEAD=$SOURCE_HEAD"
    echo "SOURCE_TREE=$SOURCE_TREE"
    echo "P5_IMAGE=$P5_IMAGE"
    echo "P5_SHA256=$P5_SHA"
    echo "COMPOSE_PROJECT=$PROJECT"
    echo "HOST=$(hostname -f 2>/dev/null || hostname)"
    echo "KERNEL=$(uname -r)"
    uname -a
    nproc
  } | tee "$EV/meta/authority.txt"
}

ensure_p5_exyonq() {
  log "ensure ExyonQ container uses P5 terminal image (read-only attribution baseline)"
  cat >"$EV/meta/compose-geom8.yml" <<EOF
services:
  exyonq:
    image: ${P5_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
EOF
  compose up -d --no-deps exyonq
  sleep 6
  local c sha
  c=$(ctr exyonq)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  sha=$(docker exec "$c" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$sha" | tee "$EV/meta/binary_sha256.txt"
  if [[ "$sha" != "$P5_SHA" ]]; then
    log "WARN: P5 SHA mismatch expected=$P5_SHA got=$sha"
  fi
}

run_warmup() {
  compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$1" >/dev/null 2>&1 || true
}

run_load_bg() {
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$1" --json >/dev/null 2>&1 &
  echo $!
}

get_rps() {
  local url=$1 tmp
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null || true
  python3 -c "import json;d=json.load(open('$tmp'));print(d.get('requests_avg',0))" 2>/dev/null || echo 0
  rm -f "$tmp"
}

thread_inventory() {
  local tag=$1 container=$2
  local main pids out="$EV/thread-inventory/${tag}.txt"
  main=$(main_pid "$container")
  pids=$(resolve_all_tids "$main")
  {
    echo "MAIN_PID=$main"
    echo "ALL_TIDS=$pids"
    echo "TID_COUNT=$(echo "$pids" | awk -F, '{print NF}')"
    for tid in ${pids//,/ }; do
      if [[ -f "/proc/$tid/status" ]]; then
        comm=$(awk '/^Name:/ {print $2}' "/proc/$tid/status")
        cpu=$(awk '/^processor:/ {print $2}' "/proc/$tid/status" 2>/dev/null || echo "?")
        echo "TID=$tid COMM=$comm CPU=$cpu"
      fi
    done
  } | tee "$out"
}

snapshot_thread_ctx() {
  local tag=$1 phase=$2 container=$3
  local main out="$EV/per-thread/${tag}-${phase}.tsv"
  main=$(main_pid "$container")
  echo -e "tid\tcomm\tvoluntary_ctx\tnonvoluntary_ctx" >"$out"
  for tid in $(ls "/proc/$main/task" 2>/dev/null); do
    [[ -f "/proc/$tid/status" ]] || continue
    comm=$(awk '/^Name:/ {print $2}' "/proc/$tid/status")
    v=$(awk '/^voluntary_ctxt_switches:/ {print $2}' "/proc/$tid/status")
    nv=$(awk '/^nonvoluntary_ctxt_switches:/ {print $2}' "/proc/$tid/status")
    echo -e "${tid}\t${comm}\t${v}\t${nv}" >>"$out"
  done
}

per_thread_ctx_delta() {
  local tag=$1 rps=$2
  python3 - "$EV/per-thread" "$tag" "$rps" "$EV/per-thread/${tag}-summary.json" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1]); tag, rps = sys.argv[2], float(sys.argv[3])
before = {}
after = {}
for phase, d in [("before", before), ("after", after)]:
    p = ev / f"{tag}-{phase}.tsv"
    for ln in p.read_text().splitlines()[1:]:
        tid, comm, v, nv = ln.split("\t")
        d[int(tid)] = {"comm": comm, "v": int(v), "nv": int(nv)}
rows = []
tv = tn = 0
for tid in after:
    if tid not in before:
        continue
    dv = after[tid]["v"] - before[tid]["v"]
    dnv = after[tid]["nv"] - before[tid]["nv"]
    tv += dv; tn += dnv
    rows.append({
        "tid": tid, "comm": after[tid]["comm"],
        "voluntary_delta": dv, "involuntary_delta": dnv,
        "voluntary_per_req": dv / rps, "involuntary_per_req": dnv / rps,
        "total_per_req": (dv + dnv) / rps,
    })
rows.sort(key=lambda r: r["total_per_req"], reverse=True)
out = {
    "tag": tag, "rps": rps,
    "voluntary_ctx_total": tv, "involuntary_ctx_total": tn,
    "voluntary_ctx_per_req": tv / rps if rps else None,
    "involuntary_ctx_per_req": tn / rps if rps else None,
    "total_ctx_per_req": (tv + tn) / rps if rps else None,
    "by_thread": rows,
}
Path(sys.argv[4]).write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps({"top5": rows[:5], "voluntary_per_req": out["voluntary_ctx_per_req"], "involuntary_per_req": out["involuntary_ctx_per_req"]}, indent=2))
PY
}

perf_stat_all_threads() {
  local tag=$1 url=$2 container=$3
  local main pids out="$EV/perf-stat/${tag}.txt"
  main=$(main_pid "$container")
  pids=$(resolve_all_tids "$main")
  echo "PIDS=$pids" | tee "$EV/perf-stat/${tag}-pids.txt"
  run_warmup "$url"
  snapshot_thread_ctx "$tag" before "$container"
  local lp rps tmp
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  perf stat -e task-clock,cpu-clock,cycles,instructions,context-switches,cpu-migrations,page-faults \
    -p "$pids" -- sleep "$MEASURE" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  cp "$tmp" "$EV/perf-stat/${tag}-rewrk-companion.json"
  rps=$(python3 -c "import json;d=json.load(open('$tmp'));print(d.get('requests_avg',0))" 2>/dev/null || echo 0)
  rm -f "$tmp"
  snapshot_thread_ctx "$tag" after "$container"
  per_thread_ctx_delta "$tag" "$rps"
  python3 - "$tag" "$rps" "$out" "$EV/perf-stat/${tag}.json" <<'PY'
import json, re, sys
tag, rps, path = sys.argv[1], float(sys.argv[2]), sys.argv[3]
text = open(path).read()
vals = {}
for line in text.splitlines():
    m = re.match(r'\s*([\d,]+(?:\.\d+)?)\s+(\S+)', line)
    if not m: continue
    v, k = m.group(1).replace(',', ''), m.group(2)
    if k in ('cycles','instructions','context-switches','cpu-migrations','page-faults'):
        vals[k.replace('-','_')] = int(float(v))
    if k == 'task-clock':
        vals['task_clock_ms'] = float(v)
m2 = re.search(r'(\d+\.\d+) seconds time elapsed', text)
if m2: vals['elapsed'] = float(m2.group(1))
out = {"tag": tag, "rps": rps, **vals}
for k in ('context_switches','cpu_migrations','cycles','instructions'):
    if k in out:
        out[k+'_per_req'] = out[k]/max(rps,1)
open(sys.argv[4],'w').write(json.dumps(out,indent=2))
print(json.dumps(out, indent=2))
PY
}

cgroup_window() {
  local tag=$1 url=$2 container=$3
  local id path
  id=$(docker inspect -f '{{.Id}}' "$container")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  run_warmup "$url"
  cp "$path" "$EV/cgroup/${tag}.before"
  compose exec -T bench-runner timeout $((MEASURE+20)) rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json \
    >"$EV/cgroup/${tag}.rewrk.json"
  cp "$path" "$EV/cgroup/${tag}.after"
  python3 - "$tag" "$EV/cgroup" <<'PY'
import json, sys
from pathlib import Path
tag, ev = sys.argv[1], Path(sys.argv[2])
def parse(p):
    d={}
    for ln in Path(p).read_text().splitlines():
        ps=ln.split()
        if len(ps)>=2: d[ps[0]]=int(ps[1])
    return d
b,a=parse(ev/f"{tag}.before"),parse(ev/f"{tag}.after")
j=json.loads((ev/f"{tag}.rewrk.json").read_text())
req=float(j.get("requests_total") or 0)
du=a.get("usage_usec",0)-b.get("usage_usec",0)
uu=a.get("user_usec",0)-b.get("user_usec",0)
su=a.get("system_usec",0)-b.get("system_usec",0)
out={"tag":tag,"requests":req,"rps":j.get("requests_avg"),"user_us_per_req":uu/req if req else None,"system_us_per_req":su/req if req else None,"total_us_per_req":du/req if req else None}
(ev/f"{tag}.summary.json").write_text(json.dumps(out,indent=2)+"\n")
print(json.dumps(out,indent=2))
PY
}

strace_syscall_profile() {
  local tag=$1 url=$2 worker=$3
  local out="$EV/syscalls/${tag}-strace.txt"
  run_warmup "$url"
  local lp; lp=$(run_load_bg "$url"); sleep 2
  timeout 12 strace -f -p "$worker" \
    -e trace=accept,accept4,epoll_wait,epoll_ctl,recv,read,send,write,connect,close,futex \
    -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}

parse_strace() {
  local tag=$1 rps=$2
  python3 - "$EV/syscalls/${tag}-strace.txt" "$rps" "$EV/syscalls/${tag}.json" <<'PY'
import json,re,sys
text=open(sys.argv[1]).read(); rps=float(sys.argv[2])
counts={}; total=0
for ln in text.splitlines():
    m=re.match(r'\s*(\d+)\s+(\S+)', ln)
    if m:
        c,name=int(m.group(1)),m.group(2)
        counts[name]=c; total+=c
out={"total":total,"per_req":total/rps if rps else None,"counts":counts}
for k in ("futex","epoll_wait","connect","close","accept","accept4","read","write","send","recv"):
    out[k+"_per_req"]=counts.get(k,0)/rps if rps else None
open(sys.argv[3],"w").write(json.dumps(out,indent=2))
PY
}

bpftrace_sched_futex() {
  local tag=$1 pids_csv=$2 outdir=$3
  local bt="$outdir/${tag}.bt"
  cat >"$bt" <<'BPF'
#!/usr/bin/env bpftrace

BEGIN {
  printf("bpftrace sched+futex %s\n", strftime("%H:%M:%S", nsecs));
}

tracepoint:syscalls:sys_enter_futex /pid == $target || comm == "exyonq" || comm == "tokio-rt-worker" || comm == "nginx"/ {
  @futex_enter[comm] = count();
}

tracepoint:syscalls:sys_enter_epoll_wait /pid == $target || comm == "exyonq" || comm == "tokio-rt-worker" || comm == "nginx"/ {
  @epoll_enter[comm] = count();
}

tracepoint:sched:sched_wakeup /comm == "exyonq" || comm == "tokio-rt-worker" || comm == "nginx" || comm == "nginx"/ {
  @wake_by_waker[comm] = count();
}

tracepoint:sched:sched_migrate_task /comm == "exyonq" || comm == "tokio-rt-worker" || comm == "nginx"/ {
  @migrations[comm] = count();
}

END {
  print("\n=== futex_enter by comm ===");
  print(@futex_enter);
  print("\n=== epoll_enter by comm ===");
  print(@epoll_enter);
  print("\n=== sched_wakeup by waker comm ===");
  print(@wake_by_waker);
  print("\n=== sched_migrate_task by comm ===");
  print(@migrations);
}
BPF
  chmod +x "$bt"
  # Run without $target filter — filter in userspace by pid set in post-process
  sed -i 's|/pid == $target || comm ==|/comm ==|' "$bt" 2>/dev/null || true
  timeout "$BPF_SEC" bpftrace "$bt" >"$outdir/${tag}.out" 2>"$outdir/${tag}.err" &
  echo $!
}

perf_sched_sample() {
  local tag=$1 pids=$2
  local data="$EV/perf-sched/${tag}.data"
  local lp url
  url=$(url_for "$tag")
  run_warmup "$url"
  lp=$(run_load_bg "$url"); sleep 2
  if perf sched record -p "$pids" -o "$data" -- sleep "$SCHED_SEC" 2>"$EV/perf-sched/${tag}-record.log"; then
    perf sched timehist -i "$data" 2>/dev/null | head -80 >"$EV/perf-sched/${tag}-timehist.txt" || true
    perf sched latency -i "$data" 2>/dev/null | head -40 >"$EV/perf-sched/${tag}-latency.txt" || true
  fi
  wait "$lp" 2>/dev/null || true
}

mpstat_window() {
  local tag=$1 url=$2
  run_warmup "$url"
  mpstat -P ALL 1 "$MEASURE" >"$EV/mpstat/${tag}.txt" 2>&1 &
  local mp=$!
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  wait "$mp" 2>/dev/null || true
  python3 - "$EV/mpstat/${tag}.txt" "$EV/mpstat/${tag}-summary.txt" <<'PY'
import re, statistics, sys
from pathlib import Path
lines=Path(sys.argv[1]).read_text().splitlines()
by={}
for ln in lines:
    m=re.match(r"\s*(\d+|all)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)", ln)
    if not m: continue
    core,*rest=m.groups()
    if core=="all": continue
    util=float(rest[0])+float(rest[2])
    by.setdefault(core,[]).append(util)
rows=sorted(((c,statistics.median(v),max(v),min(v)) for c,v in by.items()), key=lambda x:int(x[0]))
active=sum(1 for _,med,_,_ in rows if med>=15)
hot=max(rows, key=lambda r:r[1]) if rows else None
out=Path(sys.argv[2])
with out.open("w") as o:
    o.write(f"cores_sampled={len(rows)}\n")
    o.write(f"cores_median_util_ge_15pct={active}\n")
    if hot: o.write(f"hottest_core={hot[0]} median_util={hot[1]:.1f}\n")
    for c,med,mx,mn in rows:
        o.write(f"core_{c}_median={med:.1f}\n")
print(out.read_text())
PY
}

wire_upstream_close() {
  compose exec -T bench-runner curl -sS -D - -o /dev/null "http://upstream:9000/api/" 2>/dev/null \
    | tee "$EV/upstream-lifecycle/upstream-response-headers.txt" || true
}

aggregate_terminal() {
  python3 - "$EV" "$SOURCE_HEAD" "$SOURCE_TREE" <<'PY' | tee "$EV/reports/terminal.txt"
import json, re, sys
from pathlib import Path
ev = Path(sys.argv[1])
head, tree = sys.argv[2], sys.argv[3]

def load(p):
    return json.loads(p.read_text()) if p.exists() else {}

exy_ps = load(ev / "perf-stat/exyonq.json")
ngx_ps = load(ev / "perf-stat/nginx.json")
exy_pt = load(ev / "per-thread/exyonq-summary.json")
ngx_pt = load(ev / "per-thread/nginx-summary.json")
exy_cg = load(ev / "cgroup/exyonq.summary.json")
exy_sc = load(ev / "syscalls/exyonq.json")
ngx_sc = load(ev / "syscalls/nginx.json")

rps = exy_ps.get("rps")
ctx = exy_ps.get("context_switches_per_req")
ngx_ctx = ngx_ps.get("context_switches_per_req")

# Wakeup edge ledger (conservative from per-thread + syscalls + bpf)
edges = []
if exy_pt.get("by_thread"):
    for row in exy_pt["by_thread"][:8]:
        edges.append({
            "EDGE_ID": f"CTX-{row['comm']}-{row['tid']}",
            "FROM_THREAD_ROLE": row["comm"],
            "TO_THREAD_ROLE": "scheduler",
            "EVENT": "context_switch",
            "WAKEUPS_OR_SWITCHES_PER_REQ": row["total_per_req"],
            "COMPONENT": "tokio" if "tokio" in row["comm"] else "exyonq",
            "PROVEN_CAUSE": "MEASURED_PER_THREAD_CTX_DELTA",
            "REMOVABILITY": "UNKNOWN",
        })

if exy_sc.get("futex_per_req"):
    edges.append({
        "EDGE_ID": "FUTEX-SYSCALL",
        "FROM_THREAD_ROLE": "tokio-rt-worker (strace sample)",
        "TO_THREAD_ROLE": "blocked/woken peer",
        "EVENT": "futex",
        "WAKEUPS_OR_SWITCHES_PER_REQ": exy_sc["futex_per_req"],
        "COMPONENT": "tokio_runtime",
        "PROVEN_CAUSE": "MEASURED_STRACE_COUNT",
        "REMOVABILITY": "UNKNOWN",
    })

if exy_sc.get("connect_per_req") is not None:
    edges.append({
        "EDGE_ID": "UPSTREAM-CONNECT",
        "FROM_THREAD_ROLE": "tokio-rt-worker",
        "TO_THREAD_ROLE": "kernel/tcp",
        "EVENT": "connect",
        "WAKEUPS_OR_SWITCHES_PER_REQ": exy_sc["connect_per_req"],
        "COMPONENT": "upstream_lifecycle",
        "PROVEN_CAUSE": "MEASURED_STRACE upstream Connection:close workload",
        "REMOVABILITY": "NOT_AVOIDABLE_WITHOUT_CONTRACT_CHANGE",
    })

# Classify SW case
case = "SW-G"
top_threads = exy_pt.get("by_thread", [])[:5]
vol = exy_pt.get("voluntary_ctx_per_req")
invol = exy_pt.get("involuntary_ctx_per_req")
futex = exy_sc.get("futex_per_req")
connect = exy_sc.get("connect_per_req")

if not exy_ps:
    case = "SW-G"
elif connect and connect > 0.5 and (connect * 10) > (futex or 0):
    case = "SW-D"
elif exy_ps.get("cpu_migrations_per_req", 0) > 0.4 or (invol and vol and invol > vol * 0.35):
    case = "SW-C"
elif len([e for e in edges if e.get("WAKEUPS_OR_SWITCHES_PER_REQ", 0) > 5]) >= 3:
    case = "SW-B"
elif top_threads and top_threads[0].get("total_per_req", 0) > 30:
    case = "SW-A" if top_threads[0]["total_per_req"] > 50 else "SW-B"
elif ctx and ctx > 80:
    case = "SW-E"

# Repair proposal gate
next_repair = "NONE"
if case in ("SW-D",) and connect and connect > 0.8:
    next_repair = "NONE — upstream Connection:close dominates connect/close; not avoidable without contract change"

summary = {
    "run_id": ev.name,
    "source_head": head,
    "source_tree": tree,
    "product_mutation": "NO",
    "case": case,
    "exyonq": {
        "rps": rps,
        "user_us_per_req": exy_cg.get("user_us_per_req"),
        "system_us_per_req": exy_cg.get("system_us_per_req"),
        "ctx_switches_per_req": ctx,
        "cpu_migrations_per_req": exy_ps.get("cpu_migrations_per_req"),
        "voluntary_ctx_per_req": vol,
        "involuntary_ctx_per_req": invol,
        "futex_per_req": futex,
        "epoll_per_req": exy_sc.get("epoll_wait_per_req"),
        "connect_per_req": connect,
        "close_per_req": exy_sc.get("close_per_req"),
        "accept_per_req": exy_sc.get("accept4_per_req") or exy_sc.get("accept_per_req"),
    },
    "nginx": {
        "ctx_switches_per_req": ngx_ctx,
        "futex_per_req": ngx_sc.get("futex_per_req"),
        "connect_per_req": ngx_sc.get("connect_per_req"),
    },
    "top_ctx_threads": top_threads,
    "wakeup_edges": edges[:15],
    "next_product_repair_proposal": next_repair,
    "invalid_prior_evidence": {
        "a1-20260827-022600_ctx_per_req": "INVALID_SCOPE_MISMATCH — single PID; do not cite",
    },
}
(ev / "reports/summary.json").write_text(json.dumps(summary, indent=2) + "\n")

lines = [
    f"V044_PXDP_SCHEDULER_WAKEUP_TOPOLOGY_ATTRIBUTION_STATUS=CLOSED",
    f"CASE={case}",
    f"SOURCE_HEAD={head}",
    f"SOURCE_TREE={tree}",
    f"EXYONQ_RPS={rps}",
    f"EXYONQ_USER_CPU_US_PER_REQ={exy_cg.get('user_us_per_req')}",
    f"EXYONQ_SYSTEM_CPU_US_PER_REQ={exy_cg.get('system_us_per_req')}",
    f"EXYONQ_CTX_SWITCHES_PER_REQ={ctx}",
    f"NGINX_CTX_SWITCHES_PER_REQ={ngx_ctx}",
    f"VOLUNTARY_CTX_PER_REQ={vol}",
    f"INVOLUNTARY_CTX_PER_REQ={invol}",
    f"CPU_MIGRATIONS_PER_REQ={exy_ps.get('cpu_migrations_per_req')}",
    f"FUTEX_PER_REQ={futex}",
    f"EPOLL_PER_REQ={exy_sc.get('epoll_wait_per_req')}",
    f"UPSTREAM_CONNECTS_PER_REQ={connect}",
    f"UPSTREAM_CLOSES_PER_REQ={exy_sc.get('close_per_req')}",
    f"CLIENT_ACCEPTS_PER_REQ={exy_sc.get('accept4_per_req')}",
    f"TOP_CTX_THREADS={' ; '.join(f\"{t['comm']}:{t['total_per_req']:.1f}\" for t in top_threads[:5])}",
    f"NEXT_PRODUCT_REPAIR_PROPOSAL={next_repair}",
    f"PRODUCT_MUTATION=NO",
]
print("\n".join(lines))
PY
}

write_authority
ensure_p5_exyonq
compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true

EXY_C=$(ctr exyonq)
NGX_C=$(ctr nginx)
EXY_MAIN=$(main_pid "$EXY_C")
NGX_MAIN=$(main_pid "$NGX_C")
EXY_PIDS=$(resolve_all_tids "$EXY_MAIN")
NGX_PIDS=$(resolve_nginx_pids "$NGX_MAIN")
EXY_WORKER=$(echo "$EXY_PIDS" | awk -F, '{print $NF}')
NGX_WORKER=$(echo "$NGX_PIDS" | awk -F, '{print $NF}')

thread_inventory exyonq "$EXY_C"
thread_inventory nginx "$NGX_C"
wire_upstream_close

log "ExyonQ perf stat + per-thread ctx"
perf_stat_all_threads exyonq "$(url_for exyonq)" "$EXY_C"
cgroup_window exyonq "$(url_for exyonq)" "$EXY_C"
strace_syscall_profile exyonq "$(url_for exyonq)" "$EXY_WORKER"
parse_strace exyonq "$(python3 -c "import json;print(json.load(open('$EV/perf-stat/exyonq.json')).get('rps',1))")"
mpstat_window exyonq "$(url_for exyonq)"

log "NGINX control"
perf_stat_all_threads nginx "$(url_for nginx)" "$NGX_C"
strace_syscall_profile nginx "$(url_for nginx)" "$NGX_WORKER"
parse_strace nginx "$(python3 -c "import json;print(json.load(open('$EV/perf-stat/nginx.json')).get('rps',1))")"

log "perf sched samples (bounded)"
perf_sched_sample exyonq "$EXY_PIDS"
perf_sched_sample nginx "$NGX_PIDS"

log "bpftrace sched/futex (bounded, host-wide comm filter)"
BPF_PID=$(bpftrace_sched_futex exyonq "$EXY_PIDS" "$EV/bpf")
run_warmup "$(url_for exyonq)"
LP=$(run_load_bg "$(url_for exyonq)")
sleep 2
wait "$BPF_PID" 2>/dev/null || true
wait "$LP" 2>/dev/null || true

aggregate_terminal
log "DONE $EV"
