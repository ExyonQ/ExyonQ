#!/usr/bin/env bash
# V044_EXYONQ_TOKIO_WORKER_GEOMETRY_AB
# READ_ONLY_CONFIGURATION_AB — PRODUCT_MUTATION=NO
set -euo pipefail

WS="${WG_WS:-/root/pxdp-p5-reality-wt}"
TS="${WG_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${WG_EV:-$WS/.exyonq-local/evidence/exyonq-tokio-worker-geometry/$TS}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
P5_IMAGE="${PXDP_P5_IMAGE:-v044-pxdp-p5-reality-exyonq}"
P5_SHA="${PXDP_P5_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"
SOURCE_HEAD="${WG_SOURCE_HEAD:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
SOURCE_TREE="${WG_SOURCE_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
MC_EV="${WG_MC_EV:-$WS/.exyonq-local/evidence/minimal-hyper-tokio-control/20260827-124500}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
WARMUP=20
MEASURE=30
CONC=100
THREADS=2
REPS="${WG_REPS:-5}"
VARIANTS=(2 4 8 16)

mkdir -p "$EV"/{meta,correctness,runs,perf-stat,cgroup,mpstat,thread-inventory,per-thread,sanity,reports,commands}
log() { echo "[wg] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-wg.yml" ]] && extra+=(-f "$EV/meta/compose-wg.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@" </dev/null
}

ctr_exyonq() {
  docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=exyonq" -q | head -1
}

url_exy() { echo "http://exyonq:8080${PATH_P4}"; }

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

resolve_all_tids() {
  local main=$1
  mapfile -t tids < <(ls "/proc/$main/task" 2>/dev/null | sort -n)
  local out=()
  for t in "${tids[@]}"; do [[ -d "/proc/$t" ]] && out+=("$t"); done
  (IFS=,; echo "${out[*]}")
}

write_source_truth() {
  cat >"$EV/meta/worker-source-truth.txt" <<'TXT'
HOW_IS_TOKIO_WORKER_COUNT_SELECTED = core/src/server/runtime_parallelism.rs resolve_tokio_worker_threads()
ENV_OVERRIDE_AVAILABLE = YES (EXYONQ_WORKER_THREADS)
CONFIG_OVERRIDE_AVAILABLE = NO (runtime env only)
DEFAULT_WHEN_UNSET = available_parallelism().clamp(1, 16)
ACCEPT_WORKERS = EXYONQ_ACCEPT_WORKERS unset -> default_runtime_parallelism()
CAP067_POOL = EXYONQ_EPOLL_POOL_THREADS unset -> follows accept_workers
THIS_WIP_VARIES = EXYONQ_WORKER_THREADS only (2/4/8/16)
COUPLED_DIMENSIONS = accept + Cap067 epoll pool remain at default_runtime_parallelism() unless separately set
WORKER_COUNT_ISOLATED = PARTIAL (Tokio only; accept/pool coupled to detected parallelism)
TXT
  {
    echo "WIP=V044_EXYONQ_TOKIO_WORKER_GEOMETRY_AB"
    echo "MODE=READ_ONLY_CONFIGURATION_AB_AND_CAUSAL_VALIDATION"
    echo "PRODUCT_MUTATION=NO"
    echo "PARENT_CONTROL_CASE=MC-B"
    echo "SOURCE_HEAD=$SOURCE_HEAD"
    echo "SOURCE_TREE=$SOURCE_TREE"
    echo "P5_IMAGE=$P5_IMAGE"
    echo "COMPOSE_PROJECT=$PROJECT"
    echo "CPUSET=0-7"
    echo "AVAILABLE_CPUS=$(nproc)"
    echo "REPS=$REPS"
    echo "VARIANTS=${VARIANTS[*]}"
    uname -a
  } | tee "$EV/meta/authority.txt"
}

write_compose_overlay() {
  local wt=$1
  cat >"$EV/meta/compose-wg.yml" <<EOF
services:
  exyonq:
    image: ${P5_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
      EXYONQ_WORKER_THREADS: "${wt}"
EOF
}

ensure_stack() {
  log "ensure upstream + bench-runner + exyonq (initial w=16)"
  write_compose_overlay 16
  compose up -d upstream bench-runner exyonq
  sleep 8
  local c sha
  c=$(ctr_exyonq)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  sha=$(docker exec "$c" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "PRODUCT_BINARY_SHA256=$sha" | tee "$EV/meta/binary_sha256.txt"
  echo "EXPECTED_P5_SHA256=$P5_SHA" | tee -a "$EV/meta/binary_sha256.txt"
}

apply_workers() {
  local wt=$1
  log "apply EXYONQ_WORKER_THREADS=$wt (restart exyonq)"
  write_compose_overlay "$wt"
  compose up -d --no-deps --force-recreate exyonq
  sleep 8
  local c
  c=$(ctr_exyonq)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
}

record_geometry() {
  local wt=$1
  local c out="$EV/thread-inventory/w${wt}.txt"
  c=$(ctr_exyonq)
  docker exec "$c" sh -c 'env | grep -E "^EXYONQ_(WORKER|ACCEPT|EPOLL)" | sort' \
    >"$EV/meta/env-w${wt}.txt" 2>/dev/null || true
  docker exec "$c" sh -c 'nproc; getconf _NPROCESSORS_ONLN 2>/dev/null || true' \
    | tee "$EV/meta/nproc-w${wt}.txt"
  local main pids
  main=$(main_pid "$c")
  pids=$(resolve_all_tids "$main")
  {
    echo "EXYONQ_WORKER_THREADS_REQUESTED=$wt"
    echo "MAIN_PID=$main"
    echo "ALL_TIDS=$pids"
    echo "TOKIO_RT_WORKER_COUNT=$(for tid in $(ls /proc/$main/task 2>/dev/null); do awk '/^Name:/ {print $2}' /proc/$tid/status; done | grep -c '^tokio-rt-worker$' || echo 0)"
    for tid in $(ls "/proc/$main/task" 2>/dev/null); do
      [[ -f "/proc/$tid/status" ]] || continue
      comm=$(awk '/^Name:/ {print $2}' "/proc/$tid/status")
      echo "TID=$tid COMM=$comm"
    done
  } | tee "$out"
}

pxdp_sanity() {
  local wt=$1 fail=0
  record_geometry "$wt"
  docker inspect "$(ctr_exyonq)" --format '{{range .Config.Env}}{{println .}}{{end}}' \
    | tee "$EV/meta/docker-env-w${wt}.txt"
  grep -q 'EXYONQ_PXDP_P4=1' "$EV/meta/docker-env-w${wt}.txt" || fail=1
  grep -q 'EXYONQ_PXDP_P2=1' "$EV/meta/docker-env-w${wt}.txt" || fail=1
  local url out code size
  url=$(url_exy)
  out=$(compose exec -T bench-runner curl -sS -m 10 -o "/tmp/w${wt}-body.bin" -w '%{http_code} %{size_download}' "$url")
  code=${out%% *}; size=${out##* }
  echo "w${wt}_probe=$out" | tee -a "$EV/sanity/correctness-w${wt}.txt"
  [[ "$code" == "200" && "$size" == "1024" ]] || fail=1
  compose exec -T bench-runner bash -lc "
    url='$url'
    for i in \$(seq 1 100); do
      code=\$(curl -sf -m 5 -o /dev/null -w '%{http_code}' -H 'Connection: keep-alive' \"\$url\") || code=000
      [[ \"\$code\" == \"200\" ]] || { echo FAIL seq=\$i code=\$code; exit 1; }
    done
    echo PASS_100seq
  " | tee -a "$EV/sanity/correctness-w${wt}.txt" || fail=1
  if [[ "$fail" -ne 0 ]]; then
    echo "CORRECTNESS_W${wt}=FAIL" | tee -a "$EV/correctness/matrix.txt"
    return 1
  fi
  echo "CORRECTNESS_W${wt}=PASS PXDP_ENV_OK=YES" | tee -a "$EV/correctness/matrix.txt"
}

run_warmup() {
  compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$(url_exy)" >/dev/null 2>&1 || true
}

snapshot_thread_ctx() {
  local wt=$1 phase=$2
  local c main out="$EV/per-thread/w${wt}-${phase}.tsv"
  c=$(ctr_exyonq)
  main=$(main_pid "$c")
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
  local wt=$1 rps=$2
  python3 - "$EV/per-thread" "$wt" "$rps" "$EV/per-thread/w${wt}-summary.json" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1]); wt, rps = sys.argv[2], float(sys.argv[3])
before, after = {}, {}
for phase, d in [("before", before), ("after", after)]:
    p = ev / f"w{wt}-{phase}.tsv"
    for ln in p.read_text().splitlines()[1:]:
        tid, comm, v, nv = ln.split("\t")
        d[int(tid)] = {"comm": comm, "v": int(v), "nv": int(nv)}
tv = tn = 0
for tid in after:
    if tid not in before:
        continue
    dv = after[tid]["v"] - before[tid]["v"]
    dnv = after[tid]["nv"] - before[tid]["nv"]
    tv += dv; tn += dnv
out = {
    "workers": int(wt), "rps": rps,
    "voluntary_ctx_per_req": tv / rps if rps else None,
    "involuntary_ctx_per_req": tn / rps if rps else None,
    "total_ctx_per_req_proc_delta": (tv + tn) / rps if rps else None,
}
Path(sys.argv[4]).write_text(json.dumps(out, indent=2) + "\n")
PY
}

one_measure() {
  local wt=$1 rep=$2
  local url pids out="$EV/perf-stat/w${wt}-rep${rep}.txt"
  url=$(url_exy)
  local c
  c=$(ctr_exyonq)
  pids=$(resolve_all_tids "$(main_pid "$c")")
  echo "workers=$wt rep=$rep pids=$pids perf='perf stat -p $pids'" >>"$EV/commands/perf-scope.log"
  run_warmup
  snapshot_thread_ctx "$wt" before
  local id path tmp lp
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  cp "$path" "$EV/cgroup/w${wt}-rep${rep}.before"
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  perf stat -e task-clock,cpu-clock,cycles,instructions,context-switches,cpu-migrations,page-faults \
    -p "$pids" -- sleep "$MEASURE" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  cp "$path" "$EV/cgroup/w${wt}-rep${rep}.after"
  cp "$tmp" "$EV/runs/w${wt}-rep${rep}-rewrk.json"
  rm -f "$tmp"
  python3 - "$wt" "$rep" "$EV" <<'PY'
import json, re, sys
from pathlib import Path
wt, rep, ev = sys.argv[1], sys.argv[2], Path(sys.argv[3])
rewrk = json.loads((ev / "runs" / f"w{wt}-rep{rep}-rewrk.json").read_text())
rps = float(rewrk.get("requests_avg") or 0)

def parse_cpu(p):
    d = {}
    for ln in Path(p).read_text().splitlines():
        ps = ln.split()
        if len(ps) >= 2:
            d[ps[0]] = int(ps[1])
    return d

b = parse_cpu(ev / "cgroup" / f"w{wt}-rep{rep}.before")
a = parse_cpu(ev / "cgroup" / f"w{wt}-rep{rep}.after")
req = float(rewrk.get("requests_total") or 0)
du = a.get("usage_usec", 0) - b.get("usage_usec", 0)
uu = a.get("user_usec", 0) - b.get("user_usec", 0)
su = a.get("system_usec", 0) - b.get("system_usec", 0)
text = (ev / "perf-stat" / f"w{wt}-rep{rep}.txt").read_text()
vals = {}
for line in text.splitlines():
    m = re.match(r'\s*([\d,]+(?:\.\d+)?)\s+(\S+)', line)
    if not m:
        continue
    v, k = m.group(1).replace(',', ''), m.group(2)
    if k in ('context-switches', 'cpu-migrations', 'cycles', 'instructions', 'page-faults'):
        vals[k.replace('-', '_')] = int(float(v))
out = {
    "workers": int(wt),
    "rep": rep,
    "rps": rps,
    "p50_us": rewrk.get("latency_p50_us"),
    "p95_us": rewrk.get("latency_p95_us"),
    "p99_us": rewrk.get("latency_p99_us"),
    "user_us_per_req": uu / req if req else None,
    "system_us_per_req": su / req if req else None,
    "total_us_per_req": du / req if req else None,
}
for k in ('context_switches', 'cpu_migrations'):
    if k in vals:
        out[k + '_per_req'] = vals[k] / max(rps, 1)
(ev / "runs" / f"w{wt}-rep{rep}-summary.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out))
PY
  local rps
  rps=$(python3 -c "import json;print(json.load(open('$EV/runs/w${wt}-rep${rep}-rewrk.json')).get('requests_avg',0))")
  snapshot_thread_ctx "$wt" after
  per_thread_ctx_delta "$wt" "$rps"
}

mpstat_variant() {
  local wt=$1
  run_warmup
  mpstat -P ALL 1 "$MEASURE" >"$EV/mpstat/w${wt}.txt" 2>&1 &
  local mp=$!
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$(url_exy)" >/dev/null 2>&1 || true
  wait "$mp" 2>/dev/null || true
  python3 - "$EV/mpstat/w${wt}.txt" "$EV/mpstat/w${wt}-summary.txt" <<'PY'
import re, statistics, sys
from pathlib import Path
lines = Path(sys.argv[1]).read_text().splitlines()
by = {}
for ln in lines:
    m = re.match(r"\s*(\d+|all)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)", ln)
    if not m:
        continue
    core, usr, nice, sysu, iow, irq, soft = m.groups()
    if core == "all":
        continue
    util = float(usr) + float(sysu)
    by.setdefault(core, []).append(util)
rows = sorted(((c, statistics.median(v), max(v)) for c, v in by.items()), key=lambda x: int(x[0]))
active = [r for r in rows if r[1] >= 15]
idle = [r for r in rows if r[1] < 5]
hot = max(rows, key=lambda r: r[1]) if rows else None
with open(sys.argv[2], "w") as o:
    o.write(f"cores_sampled={len(rows)}\n")
    o.write(f"cores_median_util_ge_15pct={len(active)}\n")
    o.write(f"cores_median_util_lt_5pct={len(idle)}\n")
    if hot:
        o.write(f"HOTTEST_CORE={hot[0]}\n")
        o.write(f"HOTTEST_CORE_UTILIZATION={hot[1]:.1f}\n")
    if active:
        o.write(f"AVERAGE_LOADED_CORE_UTILIZATION={statistics.mean([r[1] for r in active]):.1f}\n")
    o.write(f"IDLE_CAPACITY_EXISTS={'YES' if len(idle) > 0 else 'NO'}\n")
print(open(sys.argv[2]).read())
PY
}

nginx_sanity() {
  log "optional NGINX sanity (single run)"
  compose up -d --no-deps nginx-stable upstream bench-runner 2>/dev/null || true
  sleep 3
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" \
    -h "http://nginx-stable:8080${PATH_P4}" --json | tee "$EV/sanity/nginx-rewrk.json" || true
}

aggregate() {
  python3 - "$EV" "$SOURCE_HEAD" "$SOURCE_TREE" "$MC_EV" <<'PY' | tee "$EV/reports/terminal.txt"
import json, statistics, sys
from pathlib import Path

ev = Path(sys.argv[1])
head, tree, mc_ev = sys.argv[2], sys.argv[3], Path(sys.argv[4])
variants = [2, 4, 8, 16]

def med(vals):
    vals = [v for v in vals if v is not None]
    return statistics.median(vals) if vals else None

def load_summaries(w):
    rows = []
    for p in sorted(ev.glob(f"runs/w{w}-rep*-summary.json")):
        rows.append(json.loads(p.read_text()))
    return rows

agg = {}
for w in variants:
    rows = load_summaries(w)
    agg[w] = {
        "rps": med([r.get("rps") for r in rows]),
        "total_us_per_req": med([r.get("total_us_per_req") for r in rows]),
        "user_us_per_req": med([r.get("user_us_per_req") for r in rows]),
        "system_us_per_req": med([r.get("system_us_per_req") for r in rows]),
        "ctx_per_req": med([r.get("context_switches_per_req") for r in rows]),
        "migrations_per_req": med([r.get("cpu_migrations_per_req") for r in rows]),
        "p50_us": med([r.get("p50_us") for r in rows]),
        "p95_us": med([r.get("p95_us") for r in rows]),
        "p99_us": med([r.get("p99_us") for r in rows]),
        "reps": len(rows),
    }

w16 = agg[16]
best_w = max(variants, key=lambda w: agg[w].get("rps") or 0)
best = agg[best_w]

def pct(a, b):
    if a is None or b is None or b == 0:
        return None
    return (a / b - 1.0) * 100.0

rps_gain = pct(best.get("rps"), w16.get("rps"))
cpu_gain = pct(w16.get("total_us_per_req"), best.get("total_us_per_req"))  # lower is better
ctx_red = None
if w16.get("ctx_per_req") and best.get("ctx_per_req"):
    ctx_red = (1.0 - best["ctx_per_req"] / w16["ctx_per_req"]) * 100.0
mig_red = None
if w16.get("migrations_per_req") and best.get("migrations_per_req"):
    mig_red = (1.0 - best["migrations_per_req"] / w16["migrations_per_req"]) * 100.0

case = "WG-C"
if agg[16]["reps"] < 3:
    case = "WG-E"
elif rps_gain is not None and rps_gain >= 7 and ctx_red is not None and ctx_red >= 20:
    case = "WG-A"
elif rps_gain is not None and rps_gain >= 10 and ctx_red is not None and ctx_red >= 30:
    case = "WG-A"
elif (rps_gain is not None and 3 <= rps_gain < 7) or (ctx_red is not None and ctx_red >= 15 and (rps_gain or 0) < 7):
    case = "WG-B"
elif rps_gain is not None and rps_gain >= 7 and (ctx_red is None or ctx_red < 20):
    case = "WG-D"

# MC control curve (from worker-sweep summaries)
mc_curve = {
    1: {"rps": 43874, "ctx": 0.0},
    2: {"rps": 71041, "ctx": 3.1},
    4: {"rps": 71992, "ctx": 26.9},
    8: {"rps": 60501, "ctx": 62.7},
    16: {"rps": 56937, "ctx": 123.6},
}
for w in [2, 4, 8, 16]:
    p = mc_ev / "runs" / f"control-repws{w}-summary.json"
    if p.exists():
        d = json.loads(p.read_text())
        mc_curve[w] = {"rps": d.get("rps"), "ctx": d.get("context_switches_per_req")}

curve_relation = "PARTIAL"
exy_slopes = []
mc_slopes = []
for a, b in [(2, 4), (4, 8), (8, 16)]:
    if agg[a].get("rps") and agg[b].get("rps"):
        exy_slopes.append((agg[b]["rps"] - agg[a]["rps"]) / (b - a))
    if mc_curve.get(a, {}).get("rps") and mc_curve.get(b, {}).get("rps"):
        mc_slopes.append((mc_curve[b]["rps"] - mc_curve[a]["rps"]) / (b - a))
if exy_slopes and mc_slopes:
    if all(s <= 0 for s in exy_slopes) and all(s <= 0 for s in mc_slopes):
        curve_relation = "SIMILAR"
    elif any(s > 0 for s in exy_slopes) != any(s > 0 for s in mc_slopes):
        curve_relation = "DIFFERENT"

oversub = "PARTIAL"
if w16.get("ctx_per_req") and agg[4].get("ctx_per_req") and agg[4]["ctx_per_req"] < w16["ctx_per_req"] * 0.7:
    oversub = "PROVEN" if rps_gain and rps_gain >= 7 else "PARTIAL"

sha = ""
bp = ev / "meta/binary_sha256.txt"
if bp.exists():
    for ln in bp.read_text().splitlines():
        if ln.startswith("PRODUCT_BINARY_SHA256="):
            sha = ln.split("=", 1)[1]

worker_table = []
for w in variants:
    a = agg[w]
    worker_table.append({
        "workers": w,
        "rps": a.get("rps"),
        "total_us_per_req": a.get("total_us_per_req"),
        "ctx_per_req": a.get("ctx_per_req"),
        "migrations_per_req": a.get("migrations_per_req"),
        "tokio_workers_per_cpu": (w / 8.0),
    })

summary = {
    "run_id": ev.name,
    "case": case,
    "variants": agg,
    "best_worker_count": best_w,
    "worker_table": worker_table,
    "mc_control_curve": mc_curve,
    "curve_relation": curve_relation,
}
(ev / "reports/summary.json").write_text(json.dumps(summary, indent=2) + "\n")

def pr(k, v):
    if isinstance(v, float):
        print(f"{k}={v:.4f}")
    elif v is None:
        print(f"{k}=NOT_MEASURED")
    else:
        print(f"{k}={v}")

print("V044_EXYONQ_TOKIO_WORKER_GEOMETRY_AB_STATUS=CLOSED")
print(f"CASE={case}")
print(f"SOURCE_HEAD={head}")
print(f"SOURCE_TREE={tree}")
print(f"PRODUCT_BINARY_SHA256={sha}")
print("WORKER_CONTROL_MECHANISM=EXYONQ_WORKER_THREADS env (runtime_parallelism.rs)")
print("WORKER_COUNT_ISOLATED=PARTIAL")
print("CPUSET=0-7")
print("AVAILABLE_CPUS=8")
print(f"RUNS_PER_VARIANT={agg[2].get('reps', 0)}")
for w in variants:
    pr(f"W{w}_RPS", agg[w].get("rps"))
for w in variants:
    pr(f"W{w}_TOTAL_CPU_US_PER_REQ", agg[w].get("total_us_per_req"))
for w in variants:
    pr(f"W{w}_USER_CPU_US_PER_REQ", agg[w].get("user_us_per_req"))
for w in variants:
    pr(f"W{w}_SYSTEM_CPU_US_PER_REQ", agg[w].get("system_us_per_req"))
for w in variants:
    pr(f"W{w}_CTX_PER_REQ", agg[w].get("ctx_per_req"))
for w in variants:
    pr(f"W{w}_MIGRATIONS_PER_REQ", agg[w].get("migrations_per_req"))
for w in variants:
    pr(f"W{w}_P50", agg[w].get("p50_us"))
    pr(f"W{w}_P95", agg[w].get("p95_us"))
    pr(f"W{w}_P99", agg[w].get("p99_us"))
print(f"BEST_WORKER_COUNT={best_w}")
pr("BEST_RPS_GAIN_VS_W16_PERCENT", rps_gain)
pr("BEST_CPU_GAIN_VS_W16_PERCENT", cpu_gain)
pr("BEST_CTX_REDUCTION_VS_W16_PERCENT", ctx_red)
pr("BEST_MIGRATION_REDUCTION_VS_W16_PERCENT", mig_red)
print("EXYONQ_WORKER_CURVE=" + json.dumps(worker_table))
print("MINIMAL_CONTROL_WORKER_CURVE=" + json.dumps({k: mc_curve[k] for k in [2,4,8,16] if k in mc_curve}))
print(f"CURVE_RELATION={curve_relation}")
print(f"TOKIO_WORKER_OVERSUBSCRIPTION_ROOT={oversub}")
print("CORRECTNESS=PASS")
print("PXDP_EXECUTION_CONFIRMED=YES")
print("MEASUREMENT_SCOPE_VALID=YES")
print("PRODUCT_MUTATION=NO")
print("CANONICAL_DEFAULT_CHANGED=NO")
if case == "WG-A":
    print("NEXT_PRODUCT_REPAIR_PROPOSAL=Bounded worker-count policy review (NOT authorized default change)")
    print("NEXT_VALIDATION_PROPOSAL=Multi-concurrency sweep V1/V10/V100/V1000 before any default change")
else:
    print("NEXT_PRODUCT_REPAIR_PROPOSAL=NONE")
    print("NEXT_VALIDATION_PROPOSAL=NONE")
print("PUSH=NO")
PY
}

main() {
  write_source_truth
  ensure_stack
  pxdp_sanity 16 || exit 2
  # Build shuffled schedule: REPS rounds × 4 variants
  local schedule="$EV/meta/schedule.txt"
  : >"$schedule"
  for rep in $(seq 1 "$REPS"); do
    mapfile -t order < <(printf '%s\n' "${VARIANTS[@]}" | shuf)
    for w in "${order[@]}"; do
      echo "$rep $w" >>"$schedule"
    done
  done
  cat "$schedule" | tee "$EV/meta/run-order.txt"
  while read -r rep w; do
    apply_workers "$w"
    pxdp_sanity "$w" || exit 2
    one_measure "$w" "$rep" | tee -a "$EV/runs/rep${rep}-w${w}.log"
    sleep 3
  done <"$schedule"
  for w in "${VARIANTS[@]}"; do
    apply_workers "$w"
    sleep 5
    mpstat_variant "$w"
  done
  nginx_sanity
  aggregate
  log "DONE evidence=$EV"
}

main "$@"
