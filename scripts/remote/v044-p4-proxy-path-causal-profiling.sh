#!/usr/bin/env bash
# V044 P4 proxy path causal profiling — ROOT_1 decomposition.
# PRODUCT_MUTATION=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Contract: same as v044-p4-proxy-1k-3way-20260822T115113Z (P4_PROXY_1K).
set -euo pipefail

WS="${V044_P4_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P4_PROF_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P4_PROF_EV:-$WS/.exyonq-local-evidence/v044-p4-proxy-path-causal-profiling-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
IMAGE="${P4_EXYONQ_IMAGE:-v044p1-geom8-exyonq}"
PRODUCT_COMMIT="${V044_PRODUCT_COMMIT:-ade997a362e60d1dc6eafc984da454930600845a}"
PRODUCT_TREE="${V044_PRODUCT_TREE:-4ae6ffc7528d0881be23fcac246c345dcb9a1606}"
EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459}"
PATH_P4="/api/"
WARMUP_SEC=20
MEASURE_SEC=30
RECORD_SEC="${P4_RECORD_SEC:-25}"
CONCURRENCY=100
THREADS=2

mkdir -p "$EV"/{perf-stat,perf-record,syscalls,scheduler,memory,wire,upstream,reports,meta}
cd "$WS"
log() { echo "[p4-prof] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  if [[ -f "$EV/meta/compose-geom8.yml" ]]; then
    extra=(-f "$EV/meta/compose-geom8.yml")
  fi
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
    upstream) echo "${PROJECT}-upstream-1" ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
  esac
}

write_meta() {
  {
    echo "RUN_ID=v044-p4-proxy-path-causal-profiling-$TS"
    echo "PRODUCT_HEAD=$PRODUCT_COMMIT"
    echo "PRODUCT_TREE=$PRODUCT_TREE"
    echo "BINARY_SHA256=$EXPECTED_EXYONQ_SHA256"
    echo "BENCH_CONTRACT=P4_PROXY_1K same as v044-p4-proxy-1k-3way-20260822T115113Z"
    echo "CPUSET=0-7"
    echo "HOST=$(hostname -f 2>/dev/null || hostname)"
    echo "KERNEL=$(uname -r)"
    echo "RUST_VERSION=NOT_MEASURED_ON_HOST"
    echo "CLIENT_CONCURRENCY=$CONCURRENCY"
    echo "LOAD_THREADS=$THREADS"
    echo "WARMUP_SECONDS=$WARMUP_SEC"
    echo "MEASURE_SECONDS=$MEASURE_SEC"
    echo "PRODUCT_MUTATION=NO"
    uname -a
    nproc
  } | tee "$EV/meta/run_meta.txt"
}

ensure_stack() {
  unset EXYONQ_WORKER_THREADS EXYONQ_ACCEPT_WORKERS EXYONQ_EPOLL_POOL_THREADS || true
  export P1_EXYONQ_IMAGE="$IMAGE"
  if [[ ! -f "$EV/meta/compose-geom8.yml" ]]; then
    cat >"$EV/meta/compose-geom8.yml" <<EOF
services:
  exyonq:
    image: ${IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
EOF
  fi
  compose up -d --no-deps exyonq nginx-stable openlitespeed-latest upstream
  compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
  sleep 8
  for tag in exyonq nginx ols upstream; do
    docker update --cpuset-cpus "0-7" "$(ctr "$tag")" >/dev/null 2>&1 || true
  done
  EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/binary_sha256.txt"
  if [[ "$EXY_SHA" != "$EXPECTED_EXYONQ_SHA256" ]]; then
    log "WARN: binary SHA mismatch expected=$EXPECTED_EXYONQ_SHA256 got=$EXY_SHA"
  fi
  docker logs "$(ctr exyonq)" 2>&1 | egrep -i "workers|keepalive pool geometry" | tail -5 | tee "$EV/meta/startup_geometry.txt" || true
}

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

live_pids() {
  local pids=("$@")
  local out=()
  for p in "${pids[@]}"; do
    [[ -n "$p" && -d "/proc/$p" ]] && out+=("$p")
  done
  (IFS=,; echo "${out[*]}")
}

resolve_nginx_workers() {
  local main=$1
  local kids
  kids=$(pgrep -P "$main" 2>/dev/null || true)
  live_pids $main $kids
}

resolve_ols_workers() {
  local main=$1
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
    mapfile -t workers < <(pgrep -P "$main" 2>/dev/null || true)
  fi
  live_pids "${workers[@]}"
}

resolve_exyonq_pids() {
  local main=$1
  mapfile -t tids < <(ps -T -p "$main" -o tid= 2>/dev/null || true)
  live_pids "$main" "${tids[@]}"
}

first_pid() { echo "${1##*,}" | awk -F, '{print $NF}'; }

run_warmup() {
  compose exec -T bench-runner \
    rewrk -c "$CONCURRENCY" -d "${WARMUP_SEC}s" -h "$1" --json -t "$THREADS" >/dev/null 2>&1 || true
}

run_load_bg() {
  compose exec -T bench-runner \
    rewrk -c "$CONCURRENCY" -d "${MEASURE_SEC}s" -h "$1" --json -t "$THREADS" >/dev/null 2>&1 &
  echo $!
}

get_rps() {
  local url=$1 tmp
  tmp=$(mktemp)
  compose exec -T bench-runner \
    rewrk -c "$CONCURRENCY" -d "${MEASURE_SEC}s" -h "$url" --json -t "$THREADS" >"$tmp" 2>/dev/null || true
  python3 -c "import json; d=json.load(open('$tmp')); print(d.get('requests_avg',0))" 2>/dev/null || echo 0
  rm -f "$tmp"
}

PERF_EVENTS="task-clock,cpu-clock,cycles,instructions,branches,branch-misses,cache-references,cache-misses,context-switches,cpu-migrations,page-faults,minor-faults,major-faults"

parse_perf_stat() {
  python3 - "$1" <<'PY'
import re, sys, json
text = open(sys.argv[1]).read()
vals = {}
for line in text.splitlines():
    if '<not counted>' in line:
        continue
    m = re.match(r'\s*([\d,]+(?:\.\d+)?)\s+(\S+)', line)
    if m:
        v = m.group(1).replace(',', '')
        k = m.group(2)
        skip = {'msec', 'GHz', 'CPUs', 'sec', 'K/sec', 'M/sec', '/sec', 'refs', 'utilized', 'cycle', 'branches', 'insn'}
        if k in skip:
            continue
        if k == 'task-clock':
            vals['task_clock_ms'] = float(v)
        elif k == 'cycles':
            vals['cycles'] = int(float(v))
        elif k == 'instructions':
            vals['instructions'] = int(float(v))
        elif k == 'branches':
            vals['branches'] = int(float(v))
        elif k == 'branch-misses':
            vals['branch_misses'] = int(float(v))
        elif k == 'context-switches':
            vals['context_switches'] = int(float(v))
        elif k == 'cpu-migrations':
            vals['cpu_migrations'] = int(float(v))
        elif k == 'cache-misses':
            vals['cache_misses'] = int(float(v))
        elif k == 'page-faults':
            vals['page_faults'] = int(float(v))
    m2 = re.search(r'(\d+\.\d+) seconds time elapsed', line)
    if m2:
        vals['elapsed'] = float(m2.group(1))
print(json.dumps(vals))
PY
}

perf_stat_one() {
  local tag=$1 url=$2 pids=$3
  local out="$EV/perf-stat/${tag}.txt"
  run_warmup "$url"
  local lp; lp=$(run_load_bg "$url"); sleep 2
  # shellcheck disable=SC2086
  perf stat -e "$PERF_EVENTS" -p "$pids" -- sleep "$MEASURE_SEC" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  local rps; rps=$(get_rps "$url")
  local parsed; parsed=$(parse_perf_stat "$out")
  python3 - "$tag" "$rps" "$parsed" "$EV/perf-stat/${tag}.json" <<'PY'
import json, sys
tag, rps, parsed = sys.argv[1], float(sys.argv[2]), json.loads(sys.argv[3])
rps = max(rps, 1.0)
out = {"tag": tag, "rps": rps, **parsed}
for k in ("cycles", "instructions", "branches", "branch_misses", "context_switches", "cpu_migrations", "page_faults"):
    if k in out:
        out[k + "_per_req"] = out[k] / rps
if "instructions" in out and "cycles" in out and out["cycles"]:
    out["ipc"] = out["instructions"] / out["cycles"]
if "branch_misses" in out and "branches" in out and out["branches"]:
    out["branch_miss_rate"] = out["branch_misses"] / out["branches"]
open(sys.argv[4], "w").write(json.dumps(out, indent=2))
PY
}

strace_one() {
  local tag=$1 url=$2 worker=$3
  local out="$EV/syscalls/${tag}-strace.txt"
  run_warmup "$url"
  local lp; lp=$(run_load_bg "$url"); sleep 2
  timeout 12 strace -f -p "$worker" \
    -e trace=accept,accept4,epoll_wait,epoll_ctl,recv,recvfrom,recvmsg,read,readv,send,sendto,sendmsg,write,writev,connect,close,shutdown,futex,eventfd,clock_gettime,getsockopt,setsockopt \
    -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}

record_one() {
  local tag=$1 url=$2 worker=$3
  local data="$EV/perf-record/${tag}.data"
  local report="$EV/perf-record/${tag}-report.txt"
  local folded="$EV/perf-record/${tag}-folded.txt"
  run_warmup "$url"
  local lp; lp=$(run_load_bg "$url"); sleep 2
  perf record -F 997 -g -p "$worker" -o "$data" -- sleep "$RECORD_SEC" 2>"$EV/perf-record/${tag}-record.log" || true
  wait "$lp" 2>/dev/null || true
  perf report -i "$data" --stdio --sort symbol --percent-limit 0.3 --no-children 2>/dev/null | head -120 >"$report" || true
  perf report -i "$data" --stdio --sort dso,symbol --percent-limit 0.2 2>/dev/null | head -200 >"$EV/perf-record/${tag}-dso-report.txt" || true
}

smaps_one() {
  local tag=$1 container=$2
  local pid; pid=$(main_pid "$container")
  local out="$EV/memory/${tag}-smaps.txt"
  python3 - "$pid" "$out" <<'PY'
import sys
pid, out = int(sys.argv[1]), sys.argv[2]
fields = {"Rss":0,"Pss":0,"Private_Clean":0,"Private_Dirty":0,"Shared_Clean":0,"Shared_Dirty":0}
anon = file_backed = stack = heap = 0
with open(f"/proc/{pid}/smaps_rollup") as f:
    for line in f:
        for k in fields:
            if line.startswith(k+":"):
                fields[k] = int(line.split()[1]) * 1024
try:
    with open(f"/proc/{pid}/smaps") as f:
        name = ""
        for line in f:
            if line.endswith(":\n"):
                name = line.strip(":\n")
            elif line.startswith("Private_Dirty:"):
                pd = int(line.split()[1]) * 1024
                if name == "[heap]":
                    heap += pd
                if name.endswith("[stack]"):
                    stack += pd
                if "anon" in name.lower() or name.startswith("[anon"):
                    anon += pd
                elif name and not name.startswith("["):
                    file_backed += pd
except FileNotFoundError:
    pass
fields["heap_bytes"] = heap
fields["stack_bytes"] = stack
fields["anon_proxy_bytes"] = anon
with open(out, "w") as o:
    for k,v in fields.items():
        o.write(f"{k}={v}\n")
    o.write(f"pid={pid}\n")
PY
}

wire_capture() {
  local tag=$1 url=$2
  local hdr="$EV/wire/${tag}-response-headers.txt"
  compose exec -T bench-runner sh -c \
    "curl -sS -D - -o /dev/null -H 'Connection: keep-alive' '$url' 2>/dev/null | head -30" >"$hdr" || true
  for rival in nginx ols; do
    compose exec -T bench-runner sh -c \
      "curl -sS -D - -o /dev/null -H 'Connection: keep-alive' '$(url_for "$rival")' 2>/dev/null | head -30" \
      >"$EV/wire/${rival}-response-headers.txt" || true
  done
}

upstream_socket_obs() {
  local out="$EV/upstream/socket_obs.txt"
  {
    echo "=== exyonq -> upstream established ==="
    docker exec "$(ctr exyonq)" sh -c 'ss -tn state established 2>/dev/null | grep 9000 || netstat -tn 2>/dev/null | grep 9000 || true'
    echo "=== nginx -> upstream established ==="
    docker exec "$(ctr nginx)" sh -c 'ss -tn state established 2>/dev/null | grep 9000 || netstat -tn 2>/dev/null | grep 9000 || true'
    echo "=== ols -> upstream established ==="
    docker exec "$(ctr ols)" sh -c 'ss -tn state established 2>/dev/null | grep 9000 || netstat -tn 2>/dev/null | grep 9000 || true'
  } | tee "$out"
}

upstream_strace_connect() {
  local tag=$1 worker=$2 url=$3
  local out="$EV/upstream/${tag}-connect-close.txt"
  run_warmup "$url"
  local lp; lp=$(run_load_bg "$url"); sleep 2
  timeout 10 strace -f -p "$worker" -e trace=connect,close -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
}

aggregate() {
  python3 - "$EV" <<'PY'
import json, re, os, sys
from pathlib import Path
ev = Path(sys.argv[1])
summary = {"run_id": ev.name, "product_mutation": "NO"}

# perf-stat
for tag in ("exyonq", "nginx", "ols"):
    p = ev / "perf-stat" / f"{tag}.json"
    if p.exists():
        summary[f"perf_stat_{tag}"] = json.loads(p.read_text())

# strace
for tag in ("exyonq", "nginx", "ols"):
    p = ev / "syscalls" / f"{tag}-strace.txt"
    if not p.exists():
        continue
    counts = {}
    total = 0
    for line in p.read_text().splitlines():
        m = re.match(r'\s*(\d+)\s+(\S+)', line)
        if m:
            c, name = int(m.group(1)), m.group(2)
            counts[name] = c
            total += c
    rps = summary.get(f"perf_stat_{tag}", {}).get("rps", 1) or 1
    summary[f"syscalls_{tag}"] = {
        "total": total,
        "per_req": total / rps if rps else None,
        "connect": counts.get("connect", 0),
        "close": counts.get("close", 0),
        "futex": counts.get("futex", 0),
        "counts": counts,
    }

# perf report top symbols exyonq
rep = ev / "perf-record" / "exyonq-report.txt"
top_symbols = []
if rep.exists():
    for line in rep.read_text().splitlines():
        m = re.search(r'\s+([\d.]+)%\s+.*\s(\S+)$', line)
        if m and float(m.group(1)) >= 0.3:
            top_symbols.append({"percent": float(m.group(1)), "symbol": m.group(2)})
summary["top_exyonq_symbols"] = top_symbols[:20]

# smaps
for tag, ctr in (("exyonq", "exyonq"), ("nginx", "nginx"), ("ols", "ols")):
    p = ev / "memory" / f"{tag}-smaps.txt"
    if p.exists():
        d = {}
        for line in p.read_text().splitlines():
            if "=" in line:
                k, v = line.strip().split("=", 1)
                d[k] = int(v)
        summary[f"memory_{tag}"] = {k: round(v / (1024*1024), 2) for k, v in d.items() if k.endswith("bytes") or k in ("Rss","Pss","Private_Clean","Private_Dirty")}

# wire headers
hdr = ev / "wire" / "exyonq-response-headers.txt"
if hdr.exists():
    text = hdr.read_text().lower()
    if "transfer-encoding: chunked" in text:
        summary["exyonq_transfer_encoding"] = "CHUNKED"
    elif "content-length:" in text:
        summary["exyonq_transfer_encoding"] = "CONTENT_LENGTH"
    else:
        summary["exyonq_transfer_encoding"] = "OTHER"
for rival in ("nginx", "ols"):
    h = ev / "wire" / f"{rival}-response-headers.txt"
    if h.exists():
        text = h.read_text().lower()
        if "transfer-encoding: chunked" in text:
            summary[f"{rival}_transfer_encoding"] = "CHUNKED"
        elif "content-length:" in text:
            summary[f"{rival}_transfer_encoding"] = "CONTENT_LENGTH"
        else:
            summary[f"{rival}_transfer_encoding"] = "OTHER"

(out := ev / "reports" / "summary.json").write_text(json.dumps(summary, indent=2))
print(f"wrote {out}")
PY
}

write_meta
log "ensure stack"
ensure_stack

EXY_C=$(ctr exyonq)
NGX_C=$(ctr nginx)
OLS_C=$(ctr ols)
EXY_MAIN=$(main_pid "$EXY_C")
NGX_MAIN=$(main_pid "$NGX_C")
OLS_MAIN=$(main_pid "$OLS_C")
EXY_PIDS=$(resolve_exyonq_pids "$EXY_MAIN")
NGX_PIDS=$(resolve_nginx_workers "$NGX_MAIN")
OLS_PIDS=$(resolve_ols_workers "$OLS_MAIN")
EXY_WORKER=$(first_pid "$EXY_PIDS")
NGX_WORKER=$(first_pid "$NGX_PIDS")
OLS_WORKER=$(first_pid "$OLS_PIDS")

{
  echo "EXY_PIDS=$EXY_PIDS"
  echo "NGX_PIDS=$NGX_PIDS"
  echo "OLS_PIDS=$OLS_PIDS"
  echo "EXY_WORKER=$EXY_WORKER NGX_WORKER=$NGX_WORKER OLS_WORKER=$OLS_WORKER"
} | tee "$EV/container_pids.txt"

EXY_URL=$(url_for exyonq)
NGX_URL=$(url_for nginx)
OLS_URL=$(url_for ols)

log "perf stat trio"
perf_stat_one exyonq "$EXY_URL" "$EXY_PIDS"
sleep 5
perf_stat_one nginx "$NGX_URL" "$NGX_PIDS"
sleep 5
perf_stat_one ols "$OLS_URL" "$OLS_PIDS"
sleep 5

log "strace trio"
strace_one exyonq "$EXY_URL" "$EXY_WORKER"
sleep 5
strace_one nginx "$NGX_URL" "$NGX_WORKER"
sleep 5
strace_one ols "$OLS_URL" "$OLS_WORKER"
sleep 5

log "perf record exyonq"
record_one exyonq "$EXY_URL" "$EXY_WORKER"
sleep 5

log "smaps"
smaps_one exyonq "$EXY_C"
smaps_one nginx "$NGX_C"
smaps_one ols "$OLS_C"

log "wire capture"
wire_capture exyonq "$EXY_URL"

log "upstream socket observation under load"
run_warmup "$EXY_URL"
lp=$(run_load_bg "$EXY_URL"); sleep 3
upstream_socket_obs
wait "$lp" 2>/dev/null || true

log "upstream connect/close strace"
upstream_strace_connect exyonq "$EXY_WORKER" "$EXY_URL"
sleep 3
upstream_strace_connect nginx "$NGX_WORKER" "$NGX_URL"

log "aggregate"
aggregate
echo "DONE $EV" | tee "$EV/reports/done.txt"
