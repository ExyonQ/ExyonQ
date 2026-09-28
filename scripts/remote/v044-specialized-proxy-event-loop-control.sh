#!/usr/bin/env bash
# V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL
# ISOLATED_NON_PRODUCT_ARCHITECTURE_PROTOTYPE — PRODUCT_MUTATION=NO
set -euo pipefail

WS="${EP_WS:-/root/pxdp-p5-reality-wt}"
TS="${EP_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${EP_EV:-$WS/.exyonq-local/evidence/specialized-proxy-event-loop/$TS}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
P5_IMAGE="${PXDP_P5_IMAGE:-v044-pxdp-p5-reality-exyonq}"
P5_SHA="${PXDP_P5_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"
SOURCE_HEAD="${EP_SOURCE_HEAD:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
SOURCE_TREE="${EP_SOURCE_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
PROTO_DIR="$WS/.exyonq-local/experiments/specialized-proxy-event-loop"
CONTROL_DIR="$WS/.exyonq-local/experiments/minimal-hyper-tokio-proxy"
PROTO_IMAGE="${EP_PROTO_IMAGE:-v044-specialized-proxy-event-loop:$TS}"
CONTROL_IMAGE="${EP_CONTROL_IMAGE:-v044-minimal-hyper-tokio-control:ep-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
WARMUP=20
MEASURE=30
CONC=100
THREADS=2
REPS="${EP_REPS:-5}"
SWEEP_REPS="${EP_SWEEP_REPS:-3}"
# Fair geometry: w4 from WG-B for ExyonQ + Hyper control
TOKIO_W=4
ACCEPT_W=8

mkdir -p "$EV"/{meta,correctness,upstream,runs,perf-stat,cgroup,mpstat,thread-inventory,shard-sweep,reports,commands,proto-source}
log() { echo "[ep] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-ep.yml" ]] && extra+=(-f "$EV/meta/compose-ep.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@" </dev/null
}

ctr() {
  case "$1" in
    exyonq) docker ps --filter "label=com.docker.compose.project=$PROJECT" --format '{{.ID}} {{.Names}}' | awk '/-exyonq-1/{print $1; exit}' ;;
    proto) docker ps --filter "label=com.docker.compose.project=$PROJECT" --format '{{.ID}} {{.Names}}' | awk '/specialized-proxy/{print $1; exit}' ;;
    control) docker ps --filter "label=com.docker.compose.project=$PROJECT" --format '{{.ID}} {{.Names}}' | awk '/minimal-control/{print $1; exit}' ;;
    nginx) docker ps --filter "label=com.docker.compose.project=$PROJECT" --format '{{.ID}} {{.Names}}' | awk '/nginx-stable/{print $1; exit}' ;;
    ols) docker ps --format '{{.ID}} {{.Names}}' | awk -v p="$PROJECT" '$0 ~ p && /openlitespeed-latest/{print $1; exit}' ;;
    haproxy) docker ps --filter "label=com.docker.compose.project=$PROJECT" --format '{{.ID}} {{.Names}}' | awk '/haproxy/{print $1; exit}' ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    proto) echo "http://specialized-proxy:8080${PATH_P4}" ;;
    control) echo "http://minimal-control:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
    haproxy) echo "http://haproxy:8080${PATH_P4}" ;;
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

pids_for() {
  local tag=$1 c main
  c=$(ctr "$tag")
  main=$(main_pid "$c")
  if [[ "$tag" == "nginx" ]]; then resolve_nginx_pids "$main"; else resolve_all_tids "$main"; fi
}

write_authority() {
  {
    echo "WIP=V044_SPECIALIZED_PROXY_EVENT_LOOP_CONTROL"
    echo "MODE=ISOLATED_NON_PRODUCT_ARCHITECTURE_PROTOTYPE"
    echo "PARENT_CASE=GA-B"
    echo "SELECTED_MODEL=C_THREAD_PER_CORE_PLUS_MIO"
    echo "PRODUCT_MUTATION=NO"
    echo "SOURCE_EXYONQ_HEAD=$SOURCE_HEAD"
    echo "SOURCE_EXYONQ_TREE=$SOURCE_TREE"
    echo "P5_IMAGE=$P5_IMAGE"
    echo "PROTO_IMAGE=$PROTO_IMAGE"
    echo "CONTROL_IMAGE=$CONTROL_IMAGE"
    echo "TOKIO_W=$TOKIO_W"
    echo "ACCEPT_W=$ACCEPT_W"
    echo "CPUSET=0-7"
    echo "REPS=$REPS"
    uname -a
  } | tee "$EV/meta/authority.txt"
  cp -a "$PROTO_DIR/ADR-021.md" "$PROTO_DIR/Cargo.toml" "$PROTO_DIR/Cargo.lock" "$PROTO_DIR/src" "$EV/proto-source/" 2>/dev/null || true
  find "$PROTO_DIR/src" -name '*.rs' | xargs wc -l | tee "$EV/meta/proto-loc.txt"
}

build_images() {
  log "build prototype image"
  docker build -t "$PROTO_IMAGE" "$PROTO_DIR" 2>&1 | tee "$EV/meta/proto-docker-build.log"
  docker run --rm --entrypoint sha256sum "$PROTO_IMAGE" /usr/local/bin/specialized-proxy-event-loop \
    | tee "$EV/meta/proto-binary-sha256.txt"
  log "build Hyper/Tokio control image"
  docker build -t "$CONTROL_IMAGE" "$CONTROL_DIR" 2>&1 | tee "$EV/meta/control-docker-build.log"
  docker run --rm --entrypoint sha256sum "$CONTROL_IMAGE" /usr/local/bin/minimal-hyper-tokio-proxy \
    | tee "$EV/meta/control-binary-sha256.txt"
}

write_compose() {
  local shards="${1:-4}"
  cat >"$EV/meta/compose-ep.yml" <<EOF
services:
  exyonq:
    image: ${P5_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
      EXYONQ_WORKER_THREADS: "${TOKIO_W}"
      EXYONQ_ACCEPT_WORKERS: "${ACCEPT_W}"
      EXYONQ_EPOLL_STATIC: "0"
  minimal-control:
    image: ${CONTROL_IMAGE}
    cpuset: "0-7"
    environment:
      LISTEN: "0.0.0.0:8080"
      UPSTREAM: "http://upstream:9000"
      TOKIO_WORKER_THREADS: "${TOKIO_W}"
    depends_on:
      upstream:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://127.0.0.1:8080/health | grep -q ok"]
      interval: 5s
      timeout: 3s
      retries: 8
      start_period: 8s
  specialized-proxy:
    image: ${PROTO_IMAGE}
    cpuset: "0-7"
    environment:
      LISTEN: "0.0.0.0:8080"
      UPSTREAM: "http://upstream:9000"
      SHARDS: "${shards}"
      BUFFER_SIZE: "16384"
    depends_on:
      upstream:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://127.0.0.1:8080/health | grep -q ok"]
      interval: 5s
      timeout: 3s
      retries: 8
      start_period: 8s
EOF
  cp "$EV/meta/compose-ep.yml" "$EV/meta/compose-shards-${shards}.yml"
}

ensure_stack() {
  local shards="${1:-4}"
  write_compose "$shards"
  log "compose up shards=$shards"
  compose up -d upstream bench-runner nginx-stable openlitespeed-latest haproxy exyonq minimal-control specialized-proxy
  sleep 10
  for tag in exyonq control proto nginx ols haproxy; do
    local c
    c=$(ctr "$tag" || true)
    [[ -n "$c" ]] && docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
  done
}

proto_correctness() {
  local fail=0 url
  url=$(url_for proto)
  log "correctness on prototype"
  compose exec -T bench-runner curl -sS -m 10 -o /tmp/ep-body.bin -w '%{http_code} %{size_download}\n' "$url" \
    | tee "$EV/correctness/single.txt"
  grep -q '^200 1024$' "$EV/correctness/single.txt" || fail=1
  compose exec -T bench-runner sha256sum /tmp/ep-body.bin | tee "$EV/correctness/body-sha.txt"
  compose exec -T bench-runner bash -lc '
    url="http://specialized-proxy:8080/api/"
    expect=$(sha256sum /tmp/ep-body.bin | awk "{print \$1}")
    ok=0
    for i in $(seq 1 1000); do
      code=$(curl -sf -m 5 -o /tmp/ep-ka.bin -w "%{http_code}" -H "Connection: keep-alive" "$url") || code=000
      [[ "$code" == "200" ]] || { echo "FAIL i=$i code=$code"; exit 1; }
      got=$(sha256sum /tmp/ep-ka.bin | awk "{print \$1}")
      [[ "$got" == "$expect" ]] || { echo "HASH_MISMATCH i=$i"; exit 1; }
      ok=$((ok+1))
    done
    echo "KEEPALIVE_1000=PASS hash=$expect ok=$ok"
  ' | tee "$EV/correctness/keepalive-1000.txt" || fail=1
  compose exec -T bench-runner curl -sS -o /dev/null -w '%{http_code}\n' -X POST "$url" \
    | tee "$EV/correctness/post-reject.txt"
  grep -qE '^(405|501)$' "$EV/correctness/post-reject.txt" || fail=1
  compose exec -T bench-runner bash -lc '
    url="http://specialized-proxy:8080/api/"
    for i in $(seq 1 200); do
      code=$(curl -sf -m 5 -o /dev/null -w "%{http_code}" "$url") || code=000
      [[ "$code" == "200" ]] || { echo "FAIL openclose i=$i code=$code"; exit 1; }
    done
    echo "FD_OPEN_CLOSE_200=PASS"
  ' | tee "$EV/correctness/fd-lifecycle.txt" || fail=1
  # concurrent clients via background curls
  compose exec -T bench-runner bash -lc '
    url="http://specialized-proxy:8080/api/"
    fail=0
    for i in $(seq 1 50); do
      curl -sf -m 8 -o /dev/null "$url" &
    done
    wait || fail=1
    [[ $fail -eq 0 ]] && echo "CONCURRENT_50=PASS" || echo "CONCURRENT_50=FAIL"
    exit $fail
  ' | tee "$EV/correctness/concurrent.txt" || fail=1
  if [[ "$fail" -ne 0 ]]; then
    echo "CORRECTNESS=FAIL" | tee "$EV/correctness/matrix.txt"
    return 1
  fi
  echo "CORRECTNESS=PASS" | tee "$EV/correctness/matrix.txt"
}

capture_upstream() {
  compose exec -T bench-runner curl -sS -D - -o /tmp/up-body.bin "http://upstream:9000/api/" \
    | tee "$EV/upstream/response-headers.txt"
  compose exec -T bench-runner sha256sum /tmp/up-body.bin | tee "$EV/upstream/body-sha256.txt"
}

run_warmup() { compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$1" >/dev/null 2>&1 || true; }

one_measure() {
  local tag=$1 rep=$2
  local url c out="$EV/perf-stat/${tag}-rep${rep}.txt"
  url=$(url_for "$tag")
  c=$(ctr "$tag")
  run_warmup "$url"
  local id path tmp lp main
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  main=$(main_pid "$c")
  cp "$path" "$EV/cgroup/${tag}-rep${rep}.before" 2>/dev/null || true
  awk '/^VmRSS:/ {print $2}' "/proc/$main/status" >"$EV/cgroup/${tag}-rep${rep}.rss_kb" || echo 0 >"$EV/cgroup/${tag}-rep${rep}.rss_kb"
  ls "/proc/$main/task" 2>/dev/null | wc -l >"$EV/cgroup/${tag}-rep${rep}.threads" || echo 0 >"$EV/cgroup/${tag}-rep${rep}.threads"
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  perf stat -e task-clock,cpu-clock,cycles,instructions,context-switches,cpu-migrations,page-faults \
    -p "$main" -- sleep "$MEASURE" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  cp "$path" "$EV/cgroup/${tag}-rep${rep}.after" 2>/dev/null || true
  cp "$tmp" "$EV/runs/${tag}-rep${rep}-rewrk.json"
  rm -f "$tmp"
  python3 - "$tag" "$rep" "$EV" <<'PY'
import json,re,sys
from pathlib import Path
tag,rep,ev=sys.argv[1],sys.argv[2],Path(sys.argv[3])
rewrk=json.loads((ev/"runs"/f"{tag}-rep{rep}-rewrk.json").read_text())
rps=float(rewrk.get("requests_avg") or 0)
req=float(rewrk.get("requests_total") or 0)
def parse_cpu(p):
    d={}
    try:
        for ln in Path(p).read_text().splitlines():
            ps=ln.split()
            if len(ps)>=2: d[ps[0]]=int(ps[1])
    except Exception:
        pass
    return d
b=parse_cpu(ev/"cgroup"/f"{tag}-rep{rep}.before")
a=parse_cpu(ev/"cgroup"/f"{tag}-rep{rep}.after")
du=a.get("usage_usec",0)-b.get("usage_usec",0)
uu=a.get("user_usec",0)-b.get("user_usec",0)
su=a.get("system_usec",0)-b.get("system_usec",0)
text=(ev/"perf-stat"/f"{tag}-rep{rep}.txt").read_text(errors="replace")
vals={}
for line in text.splitlines():
    m=re.match(r"\s*([\d,]+(?:\.\d+)?)\s+(\S+)", line)
    if not m: continue
    v,k=m.group(1).replace(",",""), m.group(2)
    if k in ("context-switches","cpu-migrations","cycles","instructions","page-faults"):
        vals[k.replace("-","_")]=int(float(v))
rss=int((ev/"cgroup"/f"{tag}-rep{rep}.rss_kb").read_text().strip() or "0")
threads=int((ev/"cgroup"/f"{tag}-rep{rep}.threads").read_text().strip() or "0")
out={
  "tag":tag,"rep":int(rep),"rps":rps,"requests_total":req,
  "total_us_per_req": (du/req) if req else None,
  "user_us_per_req": (uu/req) if req else None,
  "system_us_per_req": (su/req) if req else None,
  "ctx_per_req": (vals.get("context_switches",0)/req) if req else None,
  "migrations_per_req": (vals.get("cpu_migrations",0)/req) if req else None,
  "p50_us": rewrk.get("latency_p50_us"),
  "p95_us": rewrk.get("latency_p95_us"),
  "p99_us": rewrk.get("latency_p99_us"),
  "rss_kb": rss, "threads": threads, "perf": vals,
}
(ev/"runs"/f"{tag}-rep{rep}-summary.json").write_text(json.dumps(out,indent=2)+"\n")
print(json.dumps({"tag":tag,"rep":rep,"rps":round(rps,1)}))
PY
}

shard_sweep() {
  log "shard sweep 1 2 4 8"
  local best=4 best_rps=0
  for s in 1 2 4 8; do
    ensure_stack "$s"
    sleep 2
    local sum=0 n=0
    for r in $(seq 1 "$SWEEP_REPS"); do
      one_measure proto "$r"
      mv "$EV/runs/proto-rep${r}-summary.json" "$EV/shard-sweep/shards${s}-rep${r}-summary.json" 2>/dev/null || true
      mv "$EV/runs/proto-rep${r}-rewrk.json" "$EV/shard-sweep/shards${s}-rep${r}-rewrk.json" 2>/dev/null || true
      local rps
      rps=$(python3 -c "import json;print(json.load(open('$EV/shard-sweep/shards${s}-rep${r}-summary.json'))['rps'])")
      sum=$(python3 -c "print($sum+$rps)")
      n=$((n+1))
    done
    local med
    med=$(python3 - <<PY
import json,statistics
from pathlib import Path
vals=[]
for p in Path("$EV/shard-sweep").glob("shards${s}-rep*-summary.json"):
    vals.append(json.loads(p.read_text())["rps"])
print(statistics.median(vals) if vals else 0)
PY
)
    echo "shards=$s median_rps=$med" | tee -a "$EV/shard-sweep/summary.txt"
    python3 -c "import sys; sys.exit(0 if float('$med')>float('$best_rps') else 1)" && best=$s && best_rps=$med || true
  done
  echo "BEST_SHARDS=$best BEST_RPS=$best_rps" | tee "$EV/shard-sweep/best.txt"
  echo "$best" >"$EV/meta/best-shards.txt"
}

full_matrix() {
  local shards
  shards=$(cat "$EV/meta/best-shards.txt")
  ensure_stack "$shards"
  proto_correctness
  capture_upstream
  local products=(exyonq control proto nginx ols haproxy)
  # interleave: for each rep, shuffle products
  for rep in $(seq 1 "$REPS"); do
    local order
    order=$(python3 -c "import random; p=list('exyonq control proto nginx ols haproxy'.split()); random.seed(1000+$rep); random.shuffle(p); print(' '.join(p))")
    echo "rep=$rep order=$order" | tee -a "$EV/commands/run-order.txt"
    for tag in $order; do
      log "measure $tag rep=$rep"
      one_measure "$tag" "$rep"
    done
  done
}

summarize() {
  python3 - "$EV" <<'PY'
import json, statistics
from pathlib import Path
ev=Path(__import__("sys").argv[1])
tags=["exyonq","control","proto","nginx","ols","haproxy"]
fields=["rps","total_us_per_req","user_us_per_req","system_us_per_req","ctx_per_req","migrations_per_req","rss_kb","threads"]
out={"medians":{}, "deltas":{}}
for tag in tags:
    vals={f:[] for f in fields}
    for p in sorted(ev.glob(f"runs/{tag}-rep*-summary.json")):
        d=json.loads(p.read_text())
        for f in fields:
            if d.get(f) is not None: vals[f].append(d[f])
    out["medians"][tag]={f:(statistics.median(v) if v else None) for f,v in vals.items()}
ex=out["medians"]["exyonq"]["rps"] or 0
pr=out["medians"]["proto"]["rps"] or 0
ct=out["medians"]["control"]["rps"] or 0
ng=out["medians"]["nginx"]["rps"] or 0
ol=out["medians"]["ols"]["rps"] or 0
def pct(a,b):
    return None if not a else (b-a)/a*100.0
out["deltas"]={
  "proto_vs_exyonq_pct": pct(ex,pr),
  "proto_vs_control_pct": pct(ct,pr),
  "proto_vs_nginx_pct": pct(ng,pr),
  "proto_vs_ols_pct": pct(ol,pr),
}
gate=out["deltas"]["proto_vs_exyonq_pct"] or -999
# Case classification
ctx_ex=out["medians"]["exyonq"]["ctx_per_req"] or 0
ctx_pr=out["medians"]["proto"]["ctx_per_req"] or 0
cpu_ex=out["medians"]["exyonq"]["total_us_per_req"] or 0
cpu_pr=out["medians"]["proto"]["total_us_per_req"] or 0
near_nginx = ng and pr >= ng*0.9
if gate>=15 and cpu_pr and cpu_ex and cpu_pr < cpu_ex and ctx_pr < ctx_ex:
    case="EP-C" if near_nginx else "EP-A"
elif 7 <= gate < 15:
    case="EP-B"
elif gate < 7:
    case="EP-D"
else:
    case="EP-D"
out["case"]=case
out["architecture_value_gate"]="PASS" if gate>=15 else "FAIL"
best=(ev/"meta"/"best-shards.txt").read_text().strip() if (ev/"meta"/"best-shards.txt").exists() else "?"
out["best_shards"]=best
(ev/"reports"/"summary.json").write_text(json.dumps(out,indent=2)+"\n")
print(json.dumps(out,indent=2))
PY
}

main() {
  write_authority
  if [[ "${EP_RESUME:-}" == "matrix" ]]; then
    log "RESUME matrix using existing images / best shards"
    [[ -f "$EV/meta/best-shards.txt" ]] || echo 4 >"$EV/meta/best-shards.txt"
    # reuse images from prior run if env set
    full_matrix
    summarize
  else
    build_images
    shard_sweep
    full_matrix
    summarize
  fi
  log "DONE EV=$EV"
  echo "EVIDENCE_DIR=$EV"
}

main "$@"
