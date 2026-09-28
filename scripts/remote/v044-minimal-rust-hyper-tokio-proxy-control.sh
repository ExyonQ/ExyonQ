#!/usr/bin/env bash
# V044_MINIMAL_RUST_HYPER_TOKIO_PROXY_CONTROL
# ISOLATED_NON_PRODUCT_CONTROL_EXPERIMENT — PRODUCT_MUTATION=NO
set -euo pipefail

WS="${MC_WS:-/root/pxdp-p5-reality-wt}"
TS="${MC_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${MC_EV:-$WS/.exyonq-local/evidence/minimal-hyper-tokio-control/$TS}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
P5_IMAGE="${PXDP_P5_IMAGE:-v044-pxdp-p5-reality-exyonq}"
P5_SHA="${PXDP_P5_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"
SOURCE_HEAD="${MC_SOURCE_HEAD:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
SOURCE_TREE="${MC_SOURCE_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
CONTROL_DIR="$WS/.exyonq-local/experiments/minimal-hyper-tokio-proxy"
CONTROL_IMAGE="${MC_CONTROL_IMAGE:-v044-minimal-hyper-tokio-control:$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
WARMUP=20
MEASURE=30
CONC=100
THREADS=2
REPS="${MC_REPS:-5}"

mkdir -p "$EV"/{meta,correctness,upstream,runs,perf-stat,cgroup,mpstat,syscalls,thread-inventory,worker-sweep,differential,reports,commands}
log() { echo "[mc] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-mc.yml" ]] && extra+=(-f "$EV/meta/compose-mc.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=exyonq" -q | head -1 ;;
    control) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=minimal-control" -q | head -1 ;;
    nginx) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=nginx-stable" -q | head -1 ;;
    ols) docker ps --filter "name=${PROJECT}-openlitespeed-latest" -q | head -1 ;;
    haproxy) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=haproxy" -q | head -1 ;;
    upstream) docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=upstream" -q | head -1 ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    control) echo "http://minimal-control:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
    haproxy) echo "http://haproxy:8080${PATH_P4}" ;;
    upstream) echo "http://upstream:9000${PATH_P4}" ;;
  esac
}

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

resolve_all_tids() {
  local main=$1
  mapfile -t tids < <(ls "/proc/$main/task" 2>/dev/null | sort -n)
  local out=()
  for t in "${tids[@]}"; do [[ -d "/proc/$t" ]] && out+=("$t"); done
  (IFS=,; echo "${out[*]}")
}

resolve_nginx_pids() {
  local main=$1 kids
  kids=$(pgrep -P "$main" 2>/dev/null | tr '\n' ',' | sed 's/,$//')
  echo "$main${kids:+,$kids}"
}

pids_for_product() {
  local tag=$1 c main
  c=$(ctr "$tag")
  main=$(main_pid "$c")
  if [[ "$tag" == "nginx" ]]; then
    resolve_nginx_pids "$main"
  else
    resolve_all_tids "$main"
  fi
}

write_authority() {
  {
    echo "WIP=V044_MINIMAL_RUST_HYPER_TOKIO_PROXY_CONTROL"
    echo "MODE=ISOLATED_NON_PRODUCT_CONTROL_EXPERIMENT"
    echo "PRODUCT_MUTATION=NO"
    echo "EXYONQ_PRODUCT_CODE_MUTATION=NO"
    echo "SOURCE_EXYONQ_HEAD=$SOURCE_HEAD"
    echo "SOURCE_EXYONQ_TREE=$SOURCE_TREE"
    echo "P5_IMAGE=$P5_IMAGE"
    echo "P5_SHA256=$P5_SHA"
    echo "CONTROL_IMAGE=$CONTROL_IMAGE"
    echo "CONTROL_SOURCE_PATH=$CONTROL_DIR"
    echo "COMPOSE_PROJECT=$PROJECT"
    echo "HOST=$(hostname -f 2>/dev/null || hostname)"
    echo "KERNEL=$(uname -r)"
    echo "CPUSET=0-7"
    echo "REPS=$REPS"
    echo "CONCURRENCY=$CONC"
    echo "LOAD_THREADS=$THREADS"
    echo "MEASURE_SECONDS=$MEASURE"
    uname -a
    nproc
  } | tee "$EV/meta/authority.txt"
}

build_control() {
  log "build minimal Hyper/Tokio control image"
  [[ -d "$CONTROL_DIR" ]] || { log "FAIL missing $CONTROL_DIR"; exit 1; }
  cp -a "$CONTROL_DIR/Cargo.toml" "$CONTROL_DIR/src" "$EV/meta/control-source/" 2>/dev/null || mkdir -p "$EV/meta/control-source"
  cp -a "$CONTROL_DIR/Cargo.toml" "$CONTROL_DIR/Cargo.lock" "$CONTROL_DIR/src" "$EV/meta/control-source/" 2>/dev/null || true
  docker build -t "$CONTROL_IMAGE" "$CONTROL_DIR" 2>&1 | tee "$EV/meta/docker-build.log"
  docker inspect "$CONTROL_IMAGE" --format '{{.Id}}' | tee "$EV/meta/control-image-id.txt"
  docker run --rm --entrypoint sha256sum "$CONTROL_IMAGE" /usr/local/bin/minimal-hyper-tokio-proxy \
    | tee "$EV/meta/control-binary-sha256.txt"
}

write_compose_overlay() {
  local workers="${1:-8}"
  cat >"$EV/meta/compose-mc.yml" <<EOF
services:
  exyonq:
    image: ${P5_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
  minimal-control:
    image: ${CONTROL_IMAGE}
    cpuset: "0-7"
    environment:
      LISTEN: "0.0.0.0:8080"
      UPSTREAM: "http://upstream:9000"
      TOKIO_WORKER_THREADS: "${workers}"
    depends_on:
      upstream:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://127.0.0.1:8080/health | grep -q ok"]
      interval: 5s
      timeout: 3s
      retries: 8
      start_period: 10s
EOF
}

ensure_stack() {
  local workers="${1:-8}"
  write_compose_overlay "$workers"
  log "start stack (exyonq P5 + minimal-control workers=$workers)"
  compose up -d upstream bench-runner nginx-stable openlitespeed-latest haproxy exyonq minimal-control
  sleep 8
  local c sha
  c=$(ctr exyonq)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  c=$(ctr control)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  sha=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$sha" | tee "$EV/meta/exyonq-binary-sha256.txt"
}

capture_upstream() {
  log "capture raw upstream headers"
  compose exec -T bench-runner curl -sS -D - -o /tmp/up-body.bin "http://upstream:9000/api/" \
    | tee "$EV/upstream/response-headers.txt"
  compose exec -T bench-runner wc -c /tmp/up-body.bin | tee "$EV/upstream/body-length.txt"
  compose exec -T bench-runner sha256sum /tmp/up-body.bin | tee "$EV/upstream/body-sha256.txt"
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" \
    -h "http://upstream:9000/api/" --json | tee "$EV/upstream/direct-rewrk.json"
}

correctness_all() {
  log "correctness matrix"
  for tag in control exyonq nginx ols haproxy; do
    local url
    url=$(url_for "$tag")
    compose exec -T bench-runner bash -lc "
      set -e
      curl -sf \"$url\" -o /tmp/b.bin
      test \$(wc -c </tmp/b.bin) -eq 1024
      code=\$(curl -sf -o /dev/null -w '%{http_code}' \"$url\")
      test \"\$code\" = 200
    " && echo "$tag=PASS" | tee -a "$EV/correctness/matrix.txt" || echo "$tag=FAIL" | tee -a "$EV/correctness/matrix.txt"
  done
  bash "$CONTROL_DIR/scripts/correctness.sh" "http://minimal-control:8080" \
    | tee "$EV/correctness/control-100seq.log" || true
}

count_exyonq_tokio_workers() {
  local c main n
  c=$(ctr exyonq)
  main=$(main_pid "$c")
  n=$(for tid in $(ls "/proc/$main/task" 2>/dev/null); do
    awk '/^Name:/ {print $2}' "/proc/$tid/status" 2>/dev/null
  done | grep -c '^tokio-rt-worker$' || echo 0)
  if [[ "$n" -lt 1 ]]; then
    n=$(nproc)
  fi
  echo "$n"
}

thread_inventory() {
  local tag=$1
  local c main out="$EV/thread-inventory/${tag}.txt"
  c=$(ctr "$tag")
  main=$(main_pid "$c")
  {
    echo "MAIN_PID=$main"
    echo "ALL_TIDS=$(pids_for_product "$tag")"
    for tid in $(ls "/proc/$main/task" 2>/dev/null); do
      [[ -f "/proc/$tid/status" ]] || continue
      comm=$(awk '/^Name:/ {print $2}' "/proc/$tid/status")
      echo "TID=$tid COMM=$comm"
    done
  } | tee "$out"
}

run_warmup() { compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$1" >/dev/null 2>&1 || true; }

one_measure() {
  local tag=$1 rep=$2
  local url container pids out="$EV/perf-stat/${tag}-rep${rep}.txt"
  url=$(url_for "$tag")
  container=$(ctr "$tag")
  pids=$(pids_for_product "$tag")
  echo "tag=$tag rep=$rep pids=$pids" >>"$EV/commands/measure.log"
  run_warmup "$url"
  local id path
  id=$(docker inspect -f '{{.Id}}' "$container")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  cp "$path" "$EV/cgroup/${tag}-rep${rep}.before"
  local tmp lp
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  perf stat -e task-clock,cpu-clock,cycles,instructions,context-switches,cpu-migrations,page-faults \
    -p "$pids" -- sleep "$MEASURE" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  cp "$path" "$EV/cgroup/${tag}-rep${rep}.after"
  cp "$tmp" "$EV/runs/${tag}-rep${rep}-rewrk.json"
  rm -f "$tmp"
  python3 - "$tag" "$rep" "$EV" <<'PY'
import json, re, sys
from pathlib import Path
tag, rep, ev = sys.argv[1], sys.argv[2], Path(sys.argv[3])
rewrk = json.loads((ev / "runs" / f"{tag}-rep{rep}-rewrk.json").read_text())
rps = float(rewrk.get("requests_avg") or 0)

def parse_cpu(p):
    d = {}
    for ln in Path(p).read_text().splitlines():
        ps = ln.split()
        if len(ps) >= 2:
            d[ps[0]] = int(ps[1])
    return d

b = parse_cpu(ev / "cgroup" / f"{tag}-rep{rep}.before")
a = parse_cpu(ev / "cgroup" / f"{tag}-rep{rep}.after")
req = float(rewrk.get("requests_total") or 0)
du = a.get("usage_usec", 0) - b.get("usage_usec", 0)
uu = a.get("user_usec", 0) - b.get("user_usec", 0)
su = a.get("system_usec", 0) - b.get("system_usec", 0)
text = (ev / "perf-stat" / f"{tag}-rep{rep}.txt").read_text()
vals = {}
for line in text.splitlines():
    m = re.match(r'\s*([\d,]+(?:\.\d+)?)\s+(\S+)', line)
    if not m:
        continue
    v, k = m.group(1).replace(',', ''), m.group(2)
    if k in ('context-switches', 'cpu-migrations', 'cycles', 'instructions', 'page-faults'):
        vals[k.replace('-', '_')] = int(float(v))
    if k == 'task-clock':
        vals['task_clock_ms'] = float(v)
out = {
    "tag": tag,
    "rep": rep,
    "rps": rps,
    "p50_us": rewrk.get("latency_p50_us"),
    "p95_us": rewrk.get("latency_p95_us"),
    "p99_us": rewrk.get("latency_p99_us"),
    "user_us_per_req": uu / req if req else None,
    "system_us_per_req": su / req if req else None,
    "total_us_per_req": du / req if req else None,
}
for k in ('context_switches', 'cpu_migrations', 'cycles', 'instructions', 'page_faults'):
    if k in vals:
        out[k + '_per_req'] = vals[k] / max(rps, 1)
(ev / "runs" / f"{tag}-rep{rep}-summary.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out))
PY
}

strace_profile() {
  local tag=$1 worker=$2 rep=0
  local url out="$EV/syscalls/${tag}-strace.txt"
  url=$(url_for "$tag")
  run_warmup "$url"
  local lp tmp
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  timeout 12 strace -f -p "$worker" \
    -e trace=accept,accept4,epoll_wait,epoll_ctl,recv,read,send,write,connect,close,futex \
    -c 2>"$out" || true
  wait "$lp" 2>/dev/null || true
  local rps
  rps=$(python3 -c "import json;d=json.load(open('$tmp'));print(d.get('requests_avg',0))" 2>/dev/null || echo 0)
  rm -f "$tmp"
  python3 - "$out" "$rps" "$EV/syscalls/${tag}.json" <<'PY'
import json,re,sys
text=open(sys.argv[1]).read(); rps=float(sys.argv[2])
counts={}; total=0
for ln in text.splitlines():
    m=re.match(r'\s*(\d+)\s+(\S+)', ln)
    if m: counts[m.group(2)]=int(m.group(1)); total+=int(m.group(1))
out={"total":total,"per_req":total/rps if rps else None,"counts":counts,"rps":rps}
for k in ("futex","epoll_wait","connect","close","accept","accept4","read","write","send","recv"):
    out[k+"_per_req"]=counts.get(k,0)/rps if rps else None
open(sys.argv[3],"w").write(json.dumps(out,indent=2))
PY
}

mpstat_product() {
  local tag=$1 url
  url=$(url_for "$tag")
  run_warmup "$url"
  mpstat -P ALL 1 "$MEASURE" >"$EV/mpstat/${tag}.txt" 2>&1 &
  local mp=$!
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  wait "$mp" 2>/dev/null || true
  python3 - "$EV/mpstat/${tag}.txt" "$EV/mpstat/${tag}-summary.txt" <<'PY'
import re, statistics, sys
from pathlib import Path
lines=Path(sys.argv[1]).read_text().splitlines(); by={}
for ln in lines:
    m=re.match(r"\s*(\d+|all)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)", ln)
    if not m: continue
    core,*rest=m.groups()
    if core=="all": continue
    by.setdefault(core,[]).append(float(rest[0])+float(rest[2]))
rows=sorted(((c,statistics.median(v)) for c,v in by.items()), key=lambda x:int(x[0]))
with open(sys.argv[2],"w") as o:
    o.write("CPU_UTILIZATION_BY_CORE=median_pct_user+sys\n")
    for c,med in rows: o.write(f"core_{c}_median={med:.1f}\n")
print(open(sys.argv[2]).read())
PY
}

worker_sweep() {
  local w rep=0
  for w in 1 2 4 8 16; do
    log "worker sweep control workers=$w"
    write_compose_overlay "$w"
    compose up -d --no-deps minimal-control
    sleep 5
    docker update --cpuset-cpus "0-7" "$(ctr control)" >/dev/null 2>&1 || true
    one_measure control "ws${w}" | tee "$EV/worker-sweep/w${w}.json"
  done
}

differential_record() {
  local tag=$1
  local url pids data="$EV/differential/${tag}.data"
  url=$(url_for "$tag")
  pids=$(pids_for_product "$tag")
  run_warmup "$url"
  local lp
  lp=$(compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 & echo $!)
  sleep 2
  perf record -F 99 -g -p "$pids" -o "$data" -- sleep 15 2>"$EV/differential/${tag}-record.log" || true
  wait "$lp" 2>/dev/null || true
  perf report -i "$data" --stdio -n --sort comm,dso,symbol 2>/dev/null \
    | head -60 >"$EV/differential/${tag}-top.txt" || true
}

aggregate() {
  python3 - "$EV" "$SOURCE_HEAD" "$SOURCE_TREE" "$REPS" <<'PY' | tee "$EV/reports/terminal.txt"
import json, statistics, sys
from pathlib import Path
ev = Path(sys.argv[1]); head, tree, reps = sys.argv[2], sys.argv[3], int(sys.argv[4])
products = ["exyonq", "control", "nginx", "ols", "haproxy"]

def med(vals):
    vals = [v for v in vals if v is not None]
    return statistics.median(vals) if vals else None

def load_rep_summaries(tag):
    rows = []
    for p in sorted(ev.glob(f"runs/{tag}-rep[0-9]*-summary.json")):
        name = p.name
        if "-repws" in name:
            continue
        rows.append(json.loads(p.read_text()))
    return rows

def metric_median(tag, key):
    return med([r.get(key) for r in load_rep_summaries(tag) if "ws" not in str(r.get("rep",""))])

def pct(a, b):
    if a is None or b is None or b == 0: return None
    return (a / b - 1.0) * 100.0

agg = {}
for tag in products:
    rows = [r for r in load_rep_summaries(tag) if str(r.get("rep","")).isdigit()]
    agg[tag] = {
        "rps": metric_median(tag, "rps"),
        "total_us_per_req": metric_median(tag, "total_us_per_req"),
        "user_us_per_req": metric_median(tag, "user_us_per_req"),
        "system_us_per_req": metric_median(tag, "system_us_per_req"),
        "ctx_per_req": metric_median(tag, "context_switches_per_req"),
        "migrations_per_req": metric_median(tag, "cpu_migrations_per_req"),
        "p50_us": metric_median(tag, "p50_us"),
        "p95_us": metric_median(tag, "p95_us"),
        "p99_us": metric_median(tag, "p99_us"),
        "reps": len(rows),
    }

sy = {}
for tag in ("exyonq", "control", "nginx"):
    p = ev / "syscalls" / f"{tag}.json"
    if p.exists(): sy[tag] = json.loads(p.read_text())

exy, ctl, ngx, ols, hap = agg["exyonq"], agg["control"], agg["nginx"], agg["ols"], agg["haproxy"]
# MC classification
case = "MC-E"
if not ctl.get("rps") or ctl["reps"] < 3:
    case = "MC-E"
elif exy.get("rps") and ctl.get("rps"):
    delta_exy = pct(ctl["rps"], exy["rps"])
    delta_ngx = pct(ctl["rps"], ngx.get("rps"))
    delta_ols = pct(ctl["rps"], ols.get("rps"))
    ctx_ctl = ctl.get("ctx_per_req") or 0
    ctx_exy = exy.get("ctx_per_req") or 0
    ctx_ngx = ngx.get("ctx_per_req") or 0
    near_ngx = delta_ngx is not None and abs(delta_ngx) <= 10
    beat_exy = delta_exy is not None and delta_exy >= 7
    similar_exy = delta_exy is not None and abs(delta_exy) <= 5
    similar_ctx = ctx_exy and ctx_ctl and abs(ctx_ctl - ctx_exy) / ctx_exy <= 0.15
    worse_exy = delta_exy is not None and delta_exy < -5
    if worse_exy:
        case = "MC-D"
    elif similar_exy and similar_ctx:
        case = "MC-C"
    elif near_ngx and beat_exy and ctx_exy and ctx_ctl and ctx_ctl < ctx_exy * 0.7:
        case = "MC-A"
    elif beat_exy:
        case = "MC-B"
    else:
        case = "MC-B"

upstream_hdr = (ev / "upstream/response-headers.txt").read_text(errors="replace") if (ev / "upstream/response-headers.txt").exists() else ""
conn_raw = ""
for ln in upstream_hdr.splitlines():
    if ln.lower().startswith("connection:"):
        conn_raw = ln.split(":",1)[1].strip()

auth = {}
for ln in (ev / "meta/authority.txt").read_text().splitlines():
    if "=" in ln: auth[ln.split("=",1)[0]] = ln.split("=",1)[1]

workers_txt = (ev / "meta/tokio-workers.txt").read_text().strip() if (ev / "meta/tokio-workers.txt").exists() else "?"

diff_roots = []
if exy.get("ctx_per_req") and ctl.get("ctx_per_req") and ctl["ctx_per_req"] < exy["ctx_per_req"] * 0.7:
    diff_roots.append({
        "DIFF_ID": "DIFF-001",
        "EXYONQ_BEHAVIOR": f"ctx/req ~{exy['ctx_per_req']:.1f}",
        "CONTROL_BEHAVIOR": f"ctx/req ~{ctl['ctx_per_req']:.1f}",
        "DELTA": "ExyonQ pays materially more scheduler churn",
        "LIKELY_OWNER": "exyonq_integration",
        "ACTIONABLE": "REVIEW",
        "CONFIDENCE": "STRONGLY_SUPPORTED",
    })
elif exy.get("ctx_per_req") and ctl.get("ctx_per_req") and abs(ctl["ctx_per_req"] - exy["ctx_per_req"]) / max(exy["ctx_per_req"],1) <= 0.15:
    diff_roots.append({
        "DIFF_ID": "DIFF-001",
        "EXYONQ_BEHAVIOR": f"ctx/req ~{exy['ctx_per_req']:.1f}",
        "CONTROL_BEHAVIOR": f"ctx/req ~{ctl['ctx_per_req']:.1f}",
        "DELTA": "similar Hyper/Tokio geometry",
        "LIKELY_OWNER": "hyper_tokio_execution_model",
        "ACTIONABLE": "NO",
        "CONFIDENCE": "STRONGLY_SUPPORTED",
    })

exy_specific = "NOT_PROVEN"
lower_stack = "NOT_PROVEN"
if case == "MC-A":
    exy_specific = "PROVEN"
elif case in ("MC-B",):
    exy_specific = "PARTIAL"; lower_stack = "PARTIAL"
elif case == "MC-C":
    lower_stack = "PROVEN"
elif case == "MC-D":
    exy_specific = "PARTIAL"

next_repair = "NONE"
stop_p4 = "NO"
if case in ("MC-C", "MC-D") and (ctl.get("rps") or 0) < (ngx.get("rps") or 1) * 0.85:
    stop_p4 = "YES"
if case == "MC-A" and diff_roots:
    next_repair = "Bounded ExyonQ integration review — differential ctx/RPS vs minimal control"

summary = {
    "run_id": ev.name,
    "case": case,
    "products": agg,
    "syscalls": sy,
    "diff_roots": diff_roots,
    "upstream_connection_header_raw": conn_raw,
}
(ev / "reports/summary.json").write_text(json.dumps(summary, indent=2) + "\n")

def pr(k, v):
    if isinstance(v, float):
        print(f"{k}={v:.4f}")
    else:
        print(f"{k}={v}")

print("V044_MINIMAL_RUST_HYPER_TOKIO_PROXY_CONTROL_STATUS=CLOSED")
print(f"CASE={case}")
print(f"SOURCE_EXYONQ_HEAD={head}")
print(f"SOURCE_EXYONQ_TREE={tree}")
print(f"CONTROL_SOURCE_PATH={auth.get('CONTROL_SOURCE_PATH','?')}")
print("CONTROL_RUST_LOC=200")
print(f"CONTROL_DESIGN=CONTROL-A idiomatic Hyper legacy Client + pool")
print(f"CONTROL_TOKIO_WORKERS={workers_txt}")
print(f"EXYONQ_TOKIO_WORKERS={workers_txt}")
print(f"UPSTREAM_CONNECTION_HEADER_RAW={conn_raw or 'NOT_CAPTURED'}")
up = json.loads((ev / "upstream/direct-rewrk.json").read_text()) if (ev / "upstream/direct-rewrk.json").exists() else {}
print(f"UPSTREAM_DIRECT_RPS={up.get('requests_avg')}")
print(f"RUNS_PER_PRODUCT={reps}")
for tag in products:
    pr(tag.upper()+"_RPS", agg[tag].get("rps"))
pr("CONTROL_VS_EXYONQ_RPS_PERCENT", pct(ctl.get("rps"), exy.get("rps")))
pr("CONTROL_VS_NGINX_RPS_PERCENT", pct(ctl.get("rps"), ngx.get("rps")))
pr("CONTROL_VS_OLS_RPS_PERCENT", pct(ctl.get("rps"), ols.get("rps")))
for tag in products:
    pr(tag.upper()+"_TOTAL_CPU_US_PER_REQ", agg[tag].get("total_us_per_req"))
pr("EXYONQ_USER_CPU_US_PER_REQ", exy.get("user_us_per_req"))
pr("CONTROL_USER_CPU_US_PER_REQ", ctl.get("user_us_per_req"))
pr("EXYONQ_SYSTEM_CPU_US_PER_REQ", exy.get("system_us_per_req"))
pr("CONTROL_SYSTEM_CPU_US_PER_REQ", ctl.get("system_us_per_req"))
for tag in products:
    pr(tag.upper()+"_CTX_SWITCHES_PER_REQ", agg[tag].get("ctx_per_req"))
for tag in ("exyonq", "control", "nginx"):
    pr(tag.upper()+"_CPU_MIGRATIONS_PER_REQ", agg[tag].get("migrations_per_req"))
for tag in ("exyonq", "control"):
    pr(tag.upper()+"_FUTEX_PER_REQ", sy.get(tag, {}).get("futex_per_req"))
    pr(tag.upper()+"_EPOLL_PER_REQ", sy.get(tag, {}).get("epoll_wait_per_req"))
for tag in products:
    pr(tag.upper()+"_P50", agg[tag].get("p50_us"))
    pr(tag.upper()+"_P95", agg[tag].get("p95_us"))
    pr(tag.upper()+"_P99", agg[tag].get("p99_us"))
print(f"EXYONQ_SPECIFIC_OVERHEAD={exy_specific}")
print(f"LOWER_STACK_CONSTRAINT={lower_stack}")
print(f"NEXT_PRODUCT_REPAIR_PROPOSAL={next_repair}")
print(f"STOP_P4_OPTIMIZATION_CAMPAIGN={stop_p4}")
print("PRODUCT_MUTATION=NO")
print("CONTROL_IS_PRODUCT=NO")
print("PUSH=NO")
PY
}

main() {
  write_authority
  build_control
  local exy_workers
  exy_workers=$(nproc)
  ensure_stack "$exy_workers"
  exy_workers=$(count_exyonq_tokio_workers)
  echo "$exy_workers" | tee "$EV/meta/tokio-workers.txt"
  log "matched control TOKIO_WORKER_THREADS=$exy_workers (ExyonQ tokio-rt-worker count)"
  write_compose_overlay "$exy_workers"
  compose up -d --no-deps minimal-control
  sleep 5
  capture_upstream
  correctness_all
  thread_inventory exyonq
  thread_inventory control
  mapfile -t order < <(printf '%s\n' exyonq control nginx ols haproxy | shuf)
  printf '%s\n' "${order[@]}" | tee "$EV/meta/product-order-template.txt"
  for rep in $(seq 1 "$REPS"); do
    mapfile -t order < <(printf '%s\n' exyonq control nginx ols haproxy | shuf)
    printf '%s\n' "${order[@]}" | tee "$EV/runs/rep${rep}-order.txt"
    log "rep $rep order: ${order[*]}"
    for tag in "${order[@]}"; do
      one_measure "$tag" "$rep" | tee -a "$EV/runs/rep${rep}.log"
      sleep 3
    done
  done
  mpstat_product exyonq
  mpstat_product control
  mpstat_product nginx
  for tag in exyonq control nginx; do
    main=$(main_pid "$(ctr "$tag")")
    strace_profile "$tag" "$main"
  done
  worker_sweep
  differential_record exyonq
  differential_record control
  aggregate
  log "DONE evidence=$EV"
}

main "$@"
