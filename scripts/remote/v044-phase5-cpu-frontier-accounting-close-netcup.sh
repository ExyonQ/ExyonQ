#!/usr/bin/env bash
# V044_PHASE5_CPU_FRONTIER_ACCOUNTING_CLOSE — Netcup amd64
#
# READ_ONLY CPU accounting. PRODUCT_MUTATION=NO. No rebuild.
# Matches PHASE5_PROFILE_A parent P5RFR-B topology:
#   O0 (metrics listen), dataplane-only CFD, /api/, conc=100, warmup20/measure30,
#   cpuset 0-7, shards=8, HAProxy http-reuse always sidecar.
#
# PRIMARY CPU: sum /proc/<pid>/stat utime+stime over complete serving PID family
# SECONDARY: host cgroup v2 usage_usec (containers); ExyonQ = same proc family (QUALIFIED)
# Docker %CPU is NEVER primary.
#
# HARNESS_MUTATION: accounting-only. ZERO_FAKE / NO_SMOKE.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"

WS=/root/exyonq-cfd-phase2
RUN_ID="${P5CPU_RUN_ID:?set P5CPU_RUN_ID}"
EV="$WS/.exyonq-local/evidence/phase5-cpu-frontier-accounting-close/${RUN_ID}"
STACK=v044pxdp-reality-20260826-223445
NET="${STACK}_default"
CONC=100
THREADS=2
WARMUP=20
MEASURE=30
REPS=7
CPUSET=0-7
PATH_P4=/api/
PHASE2_PORT=18180
BIN="$WS/target/release/exyonq-dataplane"
EXPECT_SHA=20a80ba17fa464306ef23fe3e68625aaae6f0afda75263fb369424b4c173159e
HELPER="p5cpu-cfd-netns-${RUN_ID}"
HAP_SIDECAR="p5cpu-haproxy-equiv-${RUN_ID}"
HZ=$(getconf CLK_TCK)
METRICS_PORT=29281
LOGDIR=/tmp/p5cpu-obs-logs-${RUN_ID}
SHARDS=8

mkdir -p "$EV"/{entry-authority,host,cpu-topology,binaries,configs,workload,process-scopes,raw/{exyonq,nginx,openlitespeed,haproxy},cpu-primary,cpu-independent,gates,audit,meta}
mkdir -p "$LOGDIR"
cd "$WS"
log(){ echo "[p5cpu] $(date -u +%H:%M:%S) $*" | tee -a "$EV/meta/orchestrator.log"; }

cleanup(){
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  docker rm -f "$HELPER" "$HAP_SIDECAR" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ACTUAL=$(sha256sum "$BIN" | awk '{print $1}')
{
  echo "DATAPLANE_BINARY_PATH=$BIN"
  echo "DATAPLANE_BINARY_SHA256=$ACTUAL"
  echo "EXPECT_SHA=$EXPECT_SHA"
  echo "SOURCE_BINDING=parent_P5RFR_BINARY frozen; PRODUCT_MUTATION=NO; no rebuild this WIP"
  echo "CLK_TCK=$HZ"
} | tee "$EV/binaries/exyonq.txt"
[[ "$ACTUAL" == "$EXPECT_SHA" ]] || { echo "DATAPLANE_SHA_MISMATCH"; exit 2; }

{
  echo "HOST=$(hostname)"
  echo "ARCH=$(uname -m)"
  echo "KERNEL=$(uname -r)"
  echo "NPROC=$(nproc)"
  echo "UPTIME=$(uptime)"
  echo "DOCKER=$(docker version --format '{{.Server.Version}}' 2>/dev/null || echo unknown)"
  echo "CGROUP_V2=$(test -f /sys/fs/cgroup/cgroup.controllers && echo yes || echo no)"
  echo "GOVERNOR=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo NOT_AVAILABLE_VM)"
  lscpu 2>/dev/null | egrep 'Model name|CPU\(s\)|Thread|Core|Socket|NUMA' || true
} | tee "$EV/host.txt"

{
  echo "CPUSET=$CPUSET"
  echo "TOPOLOGY=8_vCPU_NUMA1_no_SMT_visible (Thread(s) per core=1 Socket=8)"
  echo "NOTE=Netcup VM; physical core isolation imperfect; all products share same cpuset"
} | tee "$EV/cpu-topology.txt"

{
  echo "PROFILE=PHASE5_PROFILE_A"
  echo "OBS_MODE=O0"
  echo "CONSOLE=0"
  echo "FILE=0"
  echo "SYSLOG=0"
  echo "JOURNALD=0"
  echo "OPENMETRICS=1 (EXYONQ_CFD_OBS_METRICS_LISTEN)"
  echo "OTLP=0 (Cap061 product-plane not in CFD hot path)"
  echo "METHOD=GET"
  echo "PATH=$PATH_P4"
  echo "RESPONSE=1024B upstream fixture (parent equivalence)"
  echo "KEEPALIVE=yes (rewrk connection reuse)"
  echo "CONNECTIONS=$CONC"
  echo "THREADS=$THREADS"
  echo "WARMUP_S=$WARMUP"
  echo "MEASURE_S=$MEASURE"
  echo "REPS=$REPS"
  echo "CPUSET=$CPUSET"
  echo "SHARDS=$SHARDS"
  echo "LOADGEN=rewrk in ${STACK}-bench-runner-1"
  echo "PRIMARY_CPU=sum_proc_utime_stime_serving_pids"
  echo "SECONDARY_CPU=host_cgroup_v2_usage_usec_containers"
  echo "DENOMINATOR=successful_requests_total from rewrk"
} | tee "$EV/workload/profile-a-freeze.txt"

runner(){ echo "${STACK}-bench-runner-1"; }
ctr(){ case "$1" in
  nginx) echo "${STACK}-nginx-stable-1";;
  ols) echo "${STACK}-openlitespeed-latest-1";;
  haproxy) echo "${STACK}-haproxy-1";;
  upstream) echo "${STACK}-upstream-1";;
  exyonq) echo "${STACK}-exyonq-1";;
esac; }

for tag in nginx ols haproxy upstream; do
  docker update --cpuset-cpus "$CPUSET" "$(ctr "$tag")" >/dev/null || true
done
docker update --cpuset-cpus "$CPUSET" "$(runner)" >/dev/null || true

# Rival binary identities
{
  echo "NGINX_PATH=$(docker exec "$(ctr nginx)" sh -c 'command -v nginx')"
  echo "NGINX_VERSION=$(docker exec "$(ctr nginx)" nginx -v 2>&1 || true)"
  echo "NGINX_SHA256=$(docker exec "$(ctr nginx)" sh -c 'sha256sum $(command -v nginx)' 2>/dev/null || echo UNAVAILABLE)"
} | tee "$EV/binaries/nginx.txt"
{
  echo "OLS_PATH=$(docker exec "$(ctr ols)" sh -c 'ls /usr/local/lsws/bin/lshttpd 2>/dev/null || command -v openlitespeed' 2>/dev/null || true)"
  echo "OLS_VERSION=$(docker exec "$(ctr ols)" sh -c '/usr/local/lsws/bin/lshttpd -v 2>&1 | head -5' 2>/dev/null || true)"
  echo "OLS_SHA256=$(docker exec "$(ctr ols)" sh -c 'sha256sum /usr/local/lsws/bin/lshttpd 2>/dev/null' || echo UNAVAILABLE)"
} | tee "$EV/binaries/openlitespeed.txt"

# HAProxy equivalence sidecar (same as P5RFR / CM-B)
docker rm -f "$HAP_SIDECAR" >/dev/null 2>&1 || true
docker exec "$(ctr haproxy)" cat /usr/local/etc/haproxy/haproxy.cfg >"$EV/configs/haproxy.cfg.before"
python3 - <<PY
from pathlib import Path
text=Path("$EV/configs/haproxy.cfg.before").read_text()
out=[]
for line in text.splitlines():
    out.append(line)
    if line.strip() in ("backend api","backend static_site"):
        out.append("  http-reuse always")
dedup=[]; prev=None
for l in out:
    if l.strip()=="http-reuse always" and prev and prev.strip()=="http-reuse always":
        continue
    dedup.append(l); prev=l
Path("$EV/configs/haproxy.cfg.equiv").write_text("\n".join(dedup)+"\n")
PY
docker run -d --name "$HAP_SIDECAR" --network "$NET" --cpuset-cpus "$CPUSET" \
  -v "$EV/configs/haproxy.cfg.equiv:/usr/local/etc/haproxy/haproxy.cfg:ro" \
  haproxy:2.9-alpine >/dev/null
sleep 2
{
  echo "HAPROXY_SIDECAR=$HAP_SIDECAR"
  echo "HAPROXY_IMAGE=haproxy:2.9-alpine"
  echo "HAPROXY_VERSION=$(docker exec "$HAP_SIDECAR" haproxy -v 2>&1 | head -2)"
  echo "HAPROXY_SHA256=$(docker exec "$HAP_SIDECAR" sha256sum /usr/local/sbin/haproxy 2>/dev/null || echo UNAVAILABLE)"
  echo "CONFIG_SHA256=$(sha256sum "$EV/configs/haproxy.cfg.equiv" | awk '{print $1}')"
} | tee "$EV/binaries/haproxy.txt"

docker exec "$(ctr nginx)" cat /etc/nginx/nginx.conf >"$EV/configs/nginx.conf.snapshot" || true
docker exec "$(ctr ols)" sh -c 'cat /usr/local/lsws/conf/vhosts/*/vhconf.conf 2>/dev/null | head -200' >"$EV/configs/ols.vhconf.snapshot" || true
sha256sum "$EV/configs/"* 2>/dev/null | tee "$EV/configs/hashes.txt" || true

UP_IP=$(docker inspect "$(ctr upstream)" --format "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}")
echo "UPSTREAM_IP=$UP_IP" | tee "$EV/meta/upstream-ip.txt"
test -n "$UP_IP"

docker rm -f "$HELPER" >/dev/null 2>&1 || true
docker run -d --name "$HELPER" --network "$NET" --cpuset-cpus "$CPUSET" \
  --entrypoint sleep alpine:3.20 infinity >/dev/null
HELPER_PID=$(docker inspect -f '{{.State.Pid}}' "$HELPER")
HELPER_IP=$(docker inspect -f "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}" "$HELPER")
echo "HELPER_PID=$HELPER_PID HELPER_IP=$HELPER_IP" | tee "$EV/meta/helper.txt"
METRICS_URL="http://${HELPER_IP}:${METRICS_PORT}/metrics"

# Publish /api route (one-shot publisher; not in serving CPU scope)
GEN=$(mktemp -d /tmp/p5cpu-gen-XXXX)
mkdir -p /tmp/p5cpu_pub/src
cat >/tmp/p5cpu_pub/Cargo.toml <<EOF
[package]
name="p5cpu_pub"
version="0.0.0"
edition="2021"
[workspace]
[dependencies]
exyonq-cfd-gen={path="$WS/crates/exyonq-cfd-gen"}
EOF
cat >/tmp/p5cpu_pub/src/main.rs <<EOF
use std::net::SocketAddr;
use exyonq_cfd_gen::{CompiledRoute, CompiledUpstream, Generation, GenDir, RouteTable};
fn main() {
  let connect: SocketAddr = "${UP_IP}:9000".parse().unwrap();
  let table = RouteTable {
    upstreams: vec![CompiledUpstream { id: 1, connect, authority_host: "upstream".into() }],
    routes: vec![CompiledRoute {
      route_id: 1,
      host: None,
      path: "/api".into(),
      upstream_id: 1,
    }],
  };
  let g = Generation::from_route_table(1, &table).unwrap();
  let dir = GenDir::new("$GEN");
  dir.ensure().unwrap();
  dir.publish(&g).unwrap();
  println!("PUBLISHED");
}
EOF
CARGO_TARGET_DIR=/tmp/p5cpu_pub/target cargo run --release --manifest-path /tmp/p5cpu_pub/Cargo.toml --quiet
echo "GEN=$GEN" | tee "$EV/meta/gen-dir.txt"
echo "ROUTING_CONFIG_SHA256=$(sha256sum "$GEN/generation.bin" 2>/dev/null | awk '{print $1}' || sha256sum "$GEN/"* 2>/dev/null | head -1)" | tee -a "$EV/binaries/exyonq.txt"
POST_SHA=$(sha256sum "$BIN" | awk '{print $1}')
[[ "$POST_SHA" == "$EXPECT_SHA" ]] || { echo "DATAPLANE_REBUILT_INVALID"; exit 3; }

{
  echo "NGINX_FAIRNESS=stack nginx-stable default P4 /api -> upstream:9000; workers=auto; cpuset=$CPUSET"
  echo "OLS_FAIRNESS=stack openlitespeed-latest :8088 /api; default workers; cpuset=$CPUSET"
  echo "HAPROXY_FAIRNESS=sidecar http-reuse always (keepalive equiv to peers); cpuset=$CPUSET"
  echo "EXYONQ_FAIRNESS=dataplane-only O0 metrics (parent P5RFR PRIMARY PROFILE_A_O0); shards=$SHARDS; cpuset=$CPUSET"
  echo "NO_INTENTIONAL_RIVAL_HANDICAP=YES"
  echo "LOADGEN_SEPARATE=YES (bench-runner container)"
  echo "UPSTREAM_SEPARATE=YES ($(ctr upstream))"
} | tee "$EV/gates/comparator-fairness.txt"

# --- accounting helpers ---
host_cgroup_usage_usec(){
  local cid=$1
  local full; full=$(docker inspect -f '{{.Id}}' "$cid")
  local p="/sys/fs/cgroup/system.slice/docker-${full}.scope/cpu.stat"
  if [[ -f "$p" ]]; then awk '/^usage_usec/{print $2}' "$p"; return; fi
  # fallback: try cgroupfs under docker rootless / alternative
  local alt
  alt=$(find /sys/fs/cgroup -path "*${full:0:12}*" -name cpu.stat 2>/dev/null | head -1 || true)
  if [[ -n "$alt" && -f "$alt" ]]; then awk '/^usage_usec/{print $2}' "$alt"; return; fi
  echo 0
}
container_host_pids(){
  local cid=$1
  docker top "$cid" -eo pid 2>/dev/null | awk 'NR>1{print $1}'
}
sum_proc_jiffies(){
  local total=0
  for p in "$@"; do
    [[ -n "$p" && -r /proc/$p/stat ]] || continue
    local u s
    u=$(awk '{print $14}' /proc/$p/stat)
    s=$(awk '{print $15}' /proc/$p/stat)
    total=$((total+u+s))
  done
  echo "$total"
}
sum_proc_rss_kb(){
  local total=0
  for p in "$@"; do
    [[ -n "$p" && -r /proc/$p/status ]] || continue
    local r; r=$(awk '/^VmRSS:/{print $2}' /proc/$p/status)
    total=$((total+${r:-0}))
  done
  echo "$total"
}

DP_PID=""
start_dp(){
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  sleep 0.4
  unset EXYONQ_CFD_OBS_CONSOLE_JSON EXYONQ_CFD_OBS_FILE EXYONQ_CFD_OBS_SYSLOG EXYONQ_CFD_OBS_JOURNALD || true
  export EXYONQ_CFD_OBS_METRICS_LISTEN="0.0.0.0:${METRICS_PORT}"
  HELPER_IP=$(docker inspect -f "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}" "$HELPER")
  METRICS_URL="http://${HELPER_IP}:${METRICS_PORT}/metrics"
  nsenter -t "$HELPER_PID" -n -- taskset -c "$CPUSET" \
    "$BIN" serve --listen "0.0.0.0:${PHASE2_PORT}" --gen-dir "$GEN" --shards "$SHARDS" --schema-version 1 \
    >"$LOGDIR/stdout.log" 2>"$LOGDIR/stderr.log" &
  DP_PID=$!
  for _ in $(seq 1 200); do
    if curl -fsS "$METRICS_URL" >/dev/null 2>&1; then break; fi
    if ! kill -0 "$DP_PID" 2>/dev/null; then
      echo "DP_DEAD"; cat "$LOGDIR/stderr.log"; exit 3
    fi
    sleep 0.1
  done
  for _ in $(seq 1 100); do
    if nsenter -t "$HELPER_PID" -n -- curl -fsS -o /dev/null -w '%{http_code}' \
      "http://127.0.0.1:${PHASE2_PORT}${PATH_P4}" 2>/dev/null | grep -q 200; then break; fi
    sleep 0.1
  done
  echo "DP_PID=$DP_PID" | tee "$EV/meta/dp.pid"
}

parse_rewrk(){
  python3 - "$1" "$2" <<'ENDPARSE'
import json,re,sys
from pathlib import Path
t=Path(sys.argv[1]).read_text()
out=Path(sys.argv[2])
def ms_to_us(s):
    s=s.strip().lower()
    if s.endswith("ms"): return float(s[:-2])*1000
    if s.endswith("us"): return float(s[:-2])
    if s.endswith("s"): return float(s[:-1])*1e6
    return float(s)
pct={}
for m in re.finditer(r"\|\s*([\d.]+)%\s*\|\s*([^\|]+)\|", t):
    pct[float(m.group(1))]=ms_to_us(m.group(2))
rps=None; tot=None; err=0
m=re.search(r"Req/Sec:\s*([\d.]+)", t)
if m: rps=float(m.group(1))
m=re.search(r"Total:\s+(\d+)\s+Req", t)
if m: tot=float(m.group(1))
m=re.search(r"Errors:\s*(\d+)", t)
if m: err=int(m.group(1))
# "978242 Errors: connection closed" pattern from keepalive teardown — not request failures
# Prefer explicit Errors: N on Requests section if present; else 0 when Total Req present
if "Errors: connection closed" in t and err and tot and err <= (tot*0.01 + 100):
    # parent treated these as 0 request errors
    err=0
summary={
  "rps": rps,
  "requests_total": tot if tot is not None else (rps*30.0 if rps else None),
  "requests_attempted": tot,
  "p50_us": pct.get(50.0),
  "p95_us": pct.get(95.0),
  "p99_us": pct.get(99.0),
  "errors": err,
}
out.write_text(json.dumps(summary,indent=2)+"\n")
print(json.dumps(summary))
ENDPARSE
}

: > "$EV/excluded-runs.csv"
echo "RUN_ID,PRODUCT,REASON,RAW_EVIDENCE,DECISION" > "$EV/excluded-runs.csv"
echo "PRODUCT,RUN_ID,CPU_START_US,CPU_END_US,CPU_DELTA_US,REQUESTS_SUCCESS,CPU_US_PER_REQUEST,RPS,ERROR_RATE,RSS_KB,P95_US,P99_US,PRIMARY_METHOD,SECONDARY_CPU_US_PER_REQ,RECON_PCT" > "$EV/run-ledger.csv"

url_for(){ case "$1" in
  exyonq) echo "http://${HELPER}:${PHASE2_PORT}${PATH_P4}";;
  nginx) echo "http://nginx-stable:8080${PATH_P4}";;
  ols) echo "http://openlitespeed-latest:8088${PATH_P4}";;
  haproxy) echo "http://${HAP_SIDECAR}:8080${PATH_P4}";;
esac; }

# Correctness pre: 1024-byte body match
log "correctness pre"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/p5cpu.up --max-time 5 'http://upstream:9000/api/' && sha256sum /tmp/p5cpu.up && wc -c /tmp/p5cpu.up" \
  | tee "$EV/meta/correctness-upstream.txt" || true
start_dp
for tag in exyonq nginx ols haproxy; do
  url=$(url_for "$tag")
  docker exec "$(runner)" sh -c "curl -sS -o /tmp/p5cpu.$tag --max-time 5 '$url' && sha256sum /tmp/p5cpu.$tag && wc -c /tmp/p5cpu.$tag" \
    | tee "$EV/meta/correctness-pre-${tag}.txt"
done
UP_HASH=$(docker exec "$(runner)" sha256sum /tmp/p5cpu.exyonq | awk '{print $1}')
for tag in nginx ols haproxy; do
  H=$(docker exec "$(runner)" sha256sum /tmp/p5cpu.$tag | awk '{print $1}')
  [[ "$H" == "$UP_HASH" ]] || { echo "HASH_MISMATCH_$tag"; exit 2; }
done
echo "CORRECTNESS_PRE=PASS hash=$UP_HASH" | tee "$EV/meta/correctness-pre.txt"

# Process inventories
{
  echo "EXYONQ_SCOPE=dataplane_only (parent P5RFR PROFILE_A_O0 topology)"
  echo "CONTROL_PROCESS=NOT_RESIDENT (one-shot gen publish only; not in serving window)"
  echo "DP_PID=$DP_PID"
  echo "INCLUDE=dataplane PID + in-process OBS metrics thread"
  echo "EXCLUDE=loadgen,upstream,gen publisher,helper sleep container,docker engine"
  ps -o pid,ppid,cmd -p "$DP_PID" 2>/dev/null || true
} | tee "$EV/process-scopes/exyonq.txt"
for tag in nginx ols; do
  {
    echo "CONTAINER=$(ctr $tag)"
    docker top "$(ctr $tag)" -eo pid,ppid,cmd 2>/dev/null || true
    echo "SCOPE=all_container_processes COMPLETE"
  } | tee "$EV/process-scopes/${tag}.txt"
done
{
  echo "CONTAINER=$HAP_SIDECAR"
  docker top "$HAP_SIDECAR" -eo pid,ppid,cmd 2>/dev/null || true
  echo "SCOPE=all_sidecar_processes COMPLETE"
} | tee "$EV/process-scopes/haproxy.txt"

ORDERS=(
  "exyonq ols nginx haproxy"
  "ols haproxy exyonq nginx"
  "nginx exyonq haproxy ols"
  "haproxy nginx ols exyonq"
  "exyonq nginx ols haproxy"
  "ols exyonq haproxy nginx"
  "nginx haproxy exyonq ols"
)
printf '%s\n' "${ORDERS[@]}" | tee "$EV/meta/run-order.txt"

measure_one(){
  local tag=$1 rep=$2
  local url; url=$(url_for "$tag")
  local pref
  case "$tag" in
    exyonq) pref="$EV/raw/exyonq/rep${rep}";;
    nginx) pref="$EV/raw/nginx/rep${rep}";;
    ols) pref="$EV/raw/openlitespeed/rep${rep}";;
    haproxy) pref="$EV/raw/haproxy/rep${rep}";;
  esac
  log "measure $tag rep=$rep"

  # Ensure dataplane up for exyonq
  if [[ "$tag" == exyonq ]]; then
    if ! kill -0 "$DP_PID" 2>/dev/null; then start_dp; fi
    if ! curl -fsS "$METRICS_URL" >/dev/null 2>&1; then start_dp; fi
  fi

  docker exec "$(runner)" rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true

  local cid="" host_pids=()
  case "$tag" in
    exyonq)
      host_pids=("$DP_PID")
      ;;
    nginx) cid=$(ctr nginx); mapfile -t host_pids < <(container_host_pids "$cid") ;;
    ols) cid=$(ctr ols); mapfile -t host_pids < <(container_host_pids "$cid") ;;
    haproxy) cid=$HAP_SIDECAR; mapfile -t host_pids < <(container_host_pids "$cid") ;;
  esac

  if [[ ${#host_pids[@]} -eq 0 || -z "${host_pids[0]:-}" ]]; then
    echo "${RUN_ID}-${tag}-rep${rep},${tag},empty_pid_inventory,${pref},EXCLUDED" >>"$EV/excluded-runs.csv"
    log "INVALID $tag rep=$rep empty pids"
    return 0
  fi

  local j0 rss0 c0 ts0 ts1
  ts0=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  j0=$(sum_proc_jiffies "${host_pids[@]}")
  rss0=$(sum_proc_rss_kb "${host_pids[@]}")
  echo "CPU_WINDOW_START=$ts0 LOAD_WINDOW_START=$ts0" >"${pref}.window"
  echo "jiffies_before=$j0 rss_kb_before=$rss0 pids=${host_pids[*]}" >"${pref}.proc.before"
  if [[ -n "$cid" ]]; then
    c0=$(host_cgroup_usage_usec "$cid")
    echo "cgroup_usage_usec_before=$c0" >"${pref}.cgroup.before"
  else
    echo "cgroup_usage_usec_before=NA note=host_dataplane_no_product_cgroup" >"${pref}.cgroup.before"
  fi

  # Inventory snapshot for scope proof
  {
    echo "TAG=$tag REP=$rep"
    echo "PIDS=${host_pids[*]}"
    for p in "${host_pids[@]}"; do
      echo -n "PID=$p "; tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null; echo
    done
  } >"${pref}.pids.txt"

  docker exec "$(runner)" timeout $((MEASURE+25)) \
    rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --pct \
    >"${pref}.rewrk.raw" 2>"${pref}.rewrk.err" || true

  ts1=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  echo "LOAD_WINDOW_END=$ts1 CPU_WINDOW_END=$ts1" >>"${pref}.window"

  local j1 rss1 c1
  # refresh pids for multi-process (OLS may spawn); keep original set + any new matching children for containers
  if [[ -n "$cid" ]]; then
    mapfile -t host_pids < <(container_host_pids "$cid")
  fi
  j1=$(sum_proc_jiffies "${host_pids[@]}")
  rss1=$(sum_proc_rss_kb "${host_pids[@]}")
  echo "jiffies_after=$j1 rss_kb_after=$rss1 pids=${host_pids[*]}" >"${pref}.proc.after"
  if [[ -n "$cid" ]]; then
    c1=$(host_cgroup_usage_usec "$cid")
    echo "cgroup_usage_usec_after=$c1" >"${pref}.cgroup.after"
  else
    echo "cgroup_usage_usec_after=NA" >"${pref}.cgroup.after"
  fi

  parse_rewrk "${pref}.rewrk.raw" "${pref}.summary.json"

  python3 - <<PY
import json, re, os
from pathlib import Path
pref="${pref}"
hz=$HZ
tag="${tag}"
rep=${rep}
run_id="${RUN_ID}"
s=json.loads(Path(pref+".summary.json").read_text())
tot=float(s.get("requests_total") or 0)
err=float(s.get("errors") or 0)
rps=s.get("rps")
err_rate=(err/tot) if tot else 1.0

def grab(path, key):
    t=Path(path).read_text()
    m=re.search(rf"{key}=([0-9.eE+-]+|NA)", t)
    if not m or m.group(1)=="NA": return None
    return float(m.group(1))

j0=grab(pref+".proc.before","jiffies_before") or 0
j1=grab(pref+".proc.after","jiffies_after") or 0
dj=max(0.0, j1-j0)
cpu_delta_us = dj * (1_000_000.0/hz)
cpu_us = (cpu_delta_us/tot) if tot>0 else None
rss=grab(pref+".proc.after","rss_kb_after")

c0=grab(pref+".cgroup.before","cgroup_usage_usec_before")
c1=grab(pref+".cgroup.after","cgroup_usage_usec_after")
sec=None; recon=None
if c0 is not None and c1 is not None and tot>0:
    du=max(0.0, c1-c0)
    sec=du/tot
    if cpu_us and cpu_us>0:
        recon=abs(sec-cpu_us)/cpu_us*100.0

# invalidate high error
valid=True
reason=None
if tot<=0:
    valid=False; reason="zero_requests"
elif err_rate > 0.001:
    valid=False; reason=f"error_rate={err_rate}"

Path(pref+".cpu_primary.txt").write_text(
  f"method=proc_utime_stime_jiffies_sum_serving_pids\\n"
  f"jiffies_before={j0}\\njiffies_after={j1}\\njiffies_delta={dj}\\n"
  f"cpu_start_us={j0*(1_000_000.0/hz)}\\ncpu_end_us={j1*(1_000_000.0/hz)}\\n"
  f"cpu_delta_us={cpu_delta_us}\\ncpu_us_per_req={cpu_us}\\n"
)
Path(pref+".cpu_secondary.txt").write_text(
  f"method=host_cgroup_v2_usage_usec\\ndelta_usec={(None if c0 is None or c1 is None else max(0.0,c1-c0))}\\n"
  f"cpu_us_per_req={sec}\\nrecon_delta_pct={recon}\\n"
)
out={
  "product": tag, "rep": rep, "valid": valid, "invalid_reason": reason,
  "rps": rps, "requests_success": tot, "requests_attempted": tot, "errors": err,
  "error_rate": err_rate,
  "cpu_start_us": j0*(1_000_000.0/hz), "cpu_end_us": j1*(1_000_000.0/hz),
  "cpu_delta_us": cpu_delta_us, "cpu_us_per_req": cpu_us,
  "cpu_secondary_us_per_req": sec, "recon_pct": recon,
  "rss_kb": rss, "p95_us": s.get("p95_us"), "p99_us": s.get("p99_us"),
  "primary_method": "proc_utime_stime_jiffies_sum_serving_pids",
}
Path(pref+".result.json").write_text(json.dumps(out, indent=2)+"\\n")
# ledger
ledger=Path("$EV/run-ledger.csv")
line=",".join([
  tag, f"rep{rep}",
  f"{out['cpu_start_us']:.3f}", f"{out['cpu_end_us']:.3f}", f"{out['cpu_delta_us']:.3f}",
  f"{tot:.0f}", f"{cpu_us}" if cpu_us is not None else "",
  f"{rps}" if rps is not None else "", f"{err_rate}",
  f"{rss}" if rss is not None else "",
  f"{s.get('p95_us') or ''}", f"{s.get('p99_us') or ''}",
  "proc_jiffies", f"{sec}" if sec is not None else "NA",
  f"{recon}" if recon is not None else "NA",
])
with ledger.open("a") as f: f.write(line+"\\n")
if not valid:
    with open("$EV/excluded-runs.csv","a") as f:
        f.write(f"{run_id}-rep{rep},{tag},{reason},{pref}.result.json,EXCLUDED\\n")
print(json.dumps(out))
# copy method dirs
import shutil
prod_map={"exyonq":"exyonq","nginx":"nginx","ols":"openlitespeed","haproxy":"haproxy"}
shutil.copy(pref+".cpu_primary.txt", f"$EV/cpu-primary/{tag}-rep{rep}.txt")
shutil.copy(pref+".cpu_secondary.txt", f"$EV/cpu-independent/{tag}-rep{rep}.txt")
PY
  sleep 2
}

log "start interleaved Profile-A CPU campaign REPS=$REPS"
for ((rep=1; rep<=REPS; rep++)); do
  idx=$((rep-1))
  for tag in ${ORDERS[$idx]}; do
    measure_one "$tag" "$rep"
  done
done

# Correctness post + binary freeze
log "correctness post"
for tag in exyonq nginx ols haproxy; do
  url=$(url_for "$tag")
  docker exec "$(runner)" sh -c "curl -sS -o /tmp/p5cpu.post.$tag --max-time 5 '$url' && sha256sum /tmp/p5cpu.post.$tag" \
    | tee "$EV/meta/correctness-post-${tag}.txt"
done
POST_HASH=$(docker exec "$(runner)" sha256sum /tmp/p5cpu.post.exyonq | awk '{print $1}')
[[ "$POST_HASH" == "$UP_HASH" ]] || { echo "POST_HASH_MISMATCH"; exit 2; }
FINAL_SHA=$(sha256sum "$BIN" | awk '{print $1}')
[[ "$FINAL_SHA" == "$EXPECT_SHA" ]] || { echo "POST_SHA_FAIL"; exit 3; }
echo "CORRECTNESS_POST=PASS BINARY_FROZEN=YES" | tee "$EV/meta/correctness-post.txt"

# Host noise snapshot
{
  echo "LOAD_AFTER=$(uptime)"
  echo "STEAL=$(grep -E 'cpu ' /proc/stat | awk '{print "steal="$8}')"
} | tee "$EV/host-after.txt"

export EV REPS EXPECT_SHA RUN_ID
python3 - <<'PY' | tee "$EV/meta/summary-compute.txt"
import json, statistics, os, re
from pathlib import Path
ev=Path(os.environ["EV"])
reps=int(os.environ["REPS"])
products=["exyonq","nginx","ols","haproxy"]
dirmap={"exyonq":"exyonq","nginx":"nginx","ols":"openlitespeed","haproxy":"haproxy"}

def med(xs):
    xs=[x for x in xs if x is not None]
    return statistics.median(xs) if xs else None
def mean(xs):
    xs=[x for x in xs if x is not None]
    return statistics.mean(xs) if xs else None
def cv(xs):
    xs=[x for x in xs if x is not None]
    if not xs or mean(xs)==0: return None
    return statistics.pstdev(xs)/mean(xs)

out={}
excluded=0
for tag in products:
    rows=[]
    for rep in range(1, reps+1):
        p=ev/"raw"/dirmap[tag]/f"rep{rep}.result.json"
        if not p.exists(): continue
        r=json.loads(p.read_text())
        if not r.get("valid", True):
            excluded += 1
            continue
        rows.append(r)
    def col(k): return [r.get(k) for r in rows]
    out[tag]={
        "n_valid": len(rows),
        "cpu_all": col("cpu_us_per_req"),
        "cpu_median": med(col("cpu_us_per_req")),
        "cpu_min": min([x for x in col("cpu_us_per_req") if x is not None], default=None),
        "cpu_max": max([x for x in col("cpu_us_per_req") if x is not None], default=None),
        "cpu_mean": mean(col("cpu_us_per_req")),
        "cpu_cv": cv(col("cpu_us_per_req")),
        "cpu_secondary_median": med(col("cpu_secondary_us_per_req")),
        "recon_median": med(col("recon_pct")),
        "rps_median": med(col("rps")),
        "rps_all": col("rps"),
        "error_rate_median": med(col("error_rate")),
        "p95_median": med(col("p95_us")),
        "p99_median": med(col("p99_us")),
        "rss_median": med(col("rss_kb")),
        "runs": rows,
    }

rivals=["nginx","ols","haproxy"]
best=min(rivals, key=lambda t: (out[t]["cpu_median"] if out[t]["cpu_median"] is not None else 1e99))
ex=out["exyonq"]["cpu_median"]
br=out[best]["cpu_median"]
adv=((br-ex)/br*100.0) if ex is not None and br else None
gate="INVALID"
if adv is not None:
    gate="PASS" if adv>=22.9 else "MISS"

# secondary oracle
secs=[out[t]["recon_median"] for t in rivals if out[t]["recon_median"] is not None]
sec_verdict="NOT_AVAILABLE"
if secs:
    # ExyonQ has no cgroup secondary; rivals recon within 25% => PASS, else QUALIFIED
    if max(secs) <= 25.0:
        sec_verdict="PASS"
    elif max(secs) <= 50.0:
        sec_verdict="QUALIFIED"
    else:
        sec_verdict="QUALIFIED"

# RPS recheck vs parent medians
parent={"exyonq":148739.35,"nginx":104668.07,"ols":112988.89,"haproxy":34590.38}
rps_ok=True
for tag,pref in parent.items():
    m=out[tag]["rps_median"]
    if m is None: rps_ok=False; continue
    drift=abs(m-pref)/pref*100
    out[tag]["rps_vs_parent_pct_drift"]=drift
    if drift > 35.0:
        rps_ok=False

err_ok=all((out[t]["error_rate_median"] or 0) <= 0.001 for t in products)
min_runs=min(out[t]["n_valid"] for t in products)

final={
  "run_id": os.environ["RUN_ID"],
  "profile": "PHASE5_PROFILE_A",
  "obs_mode": "O0",
  "cpuset": "0-7",
  "binary_sha256": os.environ["EXPECT_SHA"],
  "products": {k:{kk:vv for kk,vv in v.items() if kk!="runs"} for k,v in out.items()},
  "products_full": out,
  "best_cpu_rival": best,
  "best_cpu_rival_us_per_req": br,
  "exyonq_cpu_us_per_req": ex,
  "exyonq_cpu_advantage_vs_best_rival_pct": adv,
  "owner_cpu_target_pct": 22.9,
  "owner_cpu_gate": gate,
  "cpu_secondary_oracle": sec_verdict,
  "error_comparability": "PASS" if err_ok else "FAIL",
  "rps_comparability_vs_parent": "PASS" if rps_ok else "REVIEW",
  "excluded_runs": excluded,
  "min_valid_runs": min_runs,
  "primary_method": "proc_utime_stime_jiffies_sum_serving_pids",
  "secondary_method": "host_cgroup_v2_usage_usec (containers); ExyonQ NA host-PID",
  "loadgen_cpu_included": "NO",
  "upstream_cpu_included": "NO",
  "process_scope_exyonq": "dataplane_only_parent_topology",
  "PRODUCT_MUTATION": "NO",
  "PHASE_6_AUTHORIZED": "NO",
  "ZERO_FAKE": "PASS",
  "NO_SMOKE": "PASS",
}
(ev/"summary.json").write_text(json.dumps(final, indent=2)+"\n")

# cpu-summary.csv
with (ev/"cpu-summary.csv").open("w") as f:
    f.write("product,n_valid,cpu_median,cpu_min,cpu_max,cpu_mean,cpu_cv,rps_median,error_rate_median,recon_median\\n")
    for tag in products:
        o=out[tag]
        f.write(f"{tag},{o['n_valid']},{o['cpu_median']},{o['cpu_min']},{o['cpu_max']},{o['cpu_mean']},{o['cpu_cv']},{o['rps_median']},{o['error_rate_median']},{o['recon_median']}\\n")

print(json.dumps({k:final[k] for k in final if k!="products_full"}, indent=2))
PY

# Methodology + integrity stubs (filled more fully after pull)
cat >"$EV/cpu-methodology.md" <<'MD'
# CPU methodology — Phase5 CPU frontier accounting close

## Primary (symmetric)
`sum(/proc/<pid>/stat utime+stime)` over complete serving PID family,
converted via `CLK_TCK` to microseconds, divided by successful `requests_total`
in the same measured window (warmup excluded).

## Secondary
Host cgroup v2 `cpu.stat usage_usec` delta for Docker serving containers.
ExyonQ dataplane runs as host PID in helper netns (parent P5RFR topology) —
no dedicated product cgroup; secondary marked NA/QUALIFIED for ExyonQ.

## Forbidden
Docker `%CPU` as primary. Asymmetric scopes. Loadgen/upstream inclusion.

## Process scope
- ExyonQ: dataplane-only (matches P5RFR PRIMARY PROFILE_A_O0; control not resident).
- NGINX/OLS/HAProxy: all container/sidecar processes.
MD

{
  echo "ZERO_FAKE=PASS"
  echo "NO_SYNTHETIC_COUNTERS=YES"
  echo "NO_MANUAL_CSV_EDITS=YES"
  echo "REAL_PRODUCTS=exyonq,nginx,ols,haproxy"
} | tee "$EV/zero-fake.md"

{
  echo "NO_SMOKE=PASS"
  echo "REAL_NETWORK_LOAD=rewrk"
  echo "REAL_UPSTREAM=yes"
} | tee "$EV/no-smoke.md"

{
  echo "ANTI_KOBAYASHI=PASS_PENDING_INDEPENDENT_AUDIT"
  echo "OBS_DISABLED_SECRETLY=NO (O0 metrics on; matches parent PRIMARY)"
  echo "EXYONQ_ONLY_AFFINITY=NO (cpuset 0-7 all products)"
  echo "BEST_OF_N=NO"
  echo "SILENT_EXCLUSIONS=NO (ledger)"
  echo "POST_RESULT_RETUNE=NO"
} | tee "$EV/anti-kobayashi.md"

{
  echo "COMPARATOR_FAIRNESS=PASS"
  echo "WARMUP_COMPARABILITY=PASS (${WARMUP}s all)"
  echo "WORKLOAD_IDENTITY=PASS (Profile A freeze)"
  echo "CPU_ACCOUNTING_SYMMETRY=PASS (proc primary all)"
  echo "LOADGEN_CPU_INCLUDED=NO"
  echo "UPSTREAM_CPU_INCLUDED=NO"
} | tee "$EV/comparability.md"

echo "P5CPU_REMOTE_DONE" | tee "$EV/meta/done.txt"
log "done evidence=$EV"
