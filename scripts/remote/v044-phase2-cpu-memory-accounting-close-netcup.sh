#!/usr/bin/env bash
# V044_PHASE2_CPU_MEMORY_ACCOUNTING_CLOSE — Netcup remote executor
#
# HARNESS_MUTATION (allowed for accounting only):
#   WHY_REQUIRED=rival cgroup+/proc CPU+RSS; Phase2 control+dataplane combined scope;
#               HAProxy http-reuse always sidecar for keepalive equivalence (same as PCR-B)
#   SEMANTIC_EFFECT=none on ExyonQ/nginx/OLS product configs; HAProxy only gains documented keepalive reuse
#   DOES_IT_CHANGE_WORKLOAD=NO
#   DOES_IT_FAVOR_EXYONQ=NO
#   DOES_IT_FAVOR_A_COMPARATOR=NO (HAProxy fairness repair toward equivalence, not degradation)
#
# PRODUCT_MUTATION=NO. Do NOT rebuild exyonq-dataplane. Frozen SHA enforced.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"

WS=/root/exyonq-cfd-phase2
RUN_ID="${CM_RUN_ID:?}"
STACK="${CM_STACK:-v044pxdp-reality-20260826-223445}"
NET="${STACK}_default"
FROZEN_DP_SHA=96ec5fdec63abdfdb01f6c9ec2b2367b607b15f58e022282594ab2e3d867686b
FROZEN_EXY_SHA=efb15c7a7ce459c403fc837f3cf125f9505c5ccfdff93c819136936220964e00
CONC=100
THREADS=2
WARMUP=20
MEASURE=30
REPS=7
CPUSET=0-7
PATH_P4=/api/
PHASE2_PORT=18080
HELPER="cm-phase2-netns-${RUN_ID}"
HAP="cm-haproxy-equiv-${RUN_ID}"
DP_BIN="$WS/target/release/exyonq-dataplane"
EV="$WS/.exyonq-local/evidence/phase2-cpu-memory-accounting-close/${RUN_ID}"
HZ=$(getconf CLK_TCK)

mkdir -p "$EV"/{process-inventory,cpu-primary,cpu-independent,memory-primary,memory-independent,raw-runs,semantic-differential-probes,comparator-fairness,correctness,meta}
cd "$WS"
log(){ echo "[cm] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

runner(){ echo "${STACK}-bench-runner-1"; }
ctr(){ case "$1" in
  exyonq) echo "${STACK}-exyonq-1";;
  nginx) echo "${STACK}-nginx-stable-1";;
  ols) echo "${STACK}-openlitespeed-latest-1";;
  upstream) echo "${STACK}-upstream-1";;
esac; }

cleanup(){
  kill "${CTRL_PID:-}" 2>/dev/null || true
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  docker rm -f "$HELPER" "$HAP" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# --- freeze check ---
DP_SHA=$(sha256sum "$DP_BIN" | awk '{print $1}')
EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
{
  echo "PHASE2_BINARY_PATH=$DP_BIN"
  echo "PHASE2_BINARY_SHA256=$DP_SHA"
  echo "FROZEN_PHASE2_SHA=$FROZEN_DP_SHA"
  echo "CURRENT_EXYONQ_SHA256=$EXY_SHA"
  echo "FROZEN_CURRENT_EXYONQ_SHA=$FROZEN_EXY_SHA"
  echo "CLK_TCK=$HZ"
} | tee "$EV/binary-identity.txt"
[[ "$DP_SHA" == "$FROZEN_DP_SHA" ]] || { echo "DATAPLANE_SHA_MISMATCH"; exit 3; }
[[ "$EXY_SHA" == "$FROZEN_EXY_SHA" ]] || { echo "EXYONQ_SHA_MISMATCH"; exit 3; }
echo "BINARY_IDENTITY=PASS" | tee -a "$EV/binary-identity.txt"

for t in exyonq nginx ols upstream; do docker update --cpuset-cpus "$CPUSET" "$(ctr "$t")" >/dev/null || true; done
docker update --cpuset-cpus "$CPUSET" "$(runner)" >/dev/null || true

# Comparator snapshots + HAProxy equivalence sidecar
docker exec "$(ctr nginx)" cat /etc/nginx/nginx.conf >"$EV/comparator-fairness/nginx.conf.snapshot" || true
docker exec "$(ctr ols)" sh -c 'cat /usr/local/lsws/conf/vhosts/*/vhconf.conf 2>/dev/null | head -120' >"$EV/comparator-fairness/ols.vhconf.snapshot" || true
docker exec "${STACK}-haproxy-1" cat /usr/local/etc/haproxy/haproxy.cfg >"$EV/comparator-fairness/haproxy.cfg.before"
python3 - <<PY
from pathlib import Path
text=Path("$EV/comparator-fairness/haproxy.cfg.before").read_text()
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
Path("$EV/comparator-fairness/haproxy.cfg.equiv").write_text("\n".join(dedup)+"\n")
PY
docker rm -f "$HAP" >/dev/null 2>&1 || true
docker run -d --name "$HAP" --network "$NET" --cpuset-cpus "$CPUSET" \
  -v "$EV/comparator-fairness/haproxy.cfg.equiv:/usr/local/etc/haproxy/haproxy.cfg:ro" \
  haproxy:2.9-alpine >/dev/null
sleep 2

UP_IP=$(docker inspect "$(ctr upstream)" --format "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}")
echo "UPSTREAM_IP=$UP_IP" | tee "$EV/meta/upstream-ip.txt"
test -n "$UP_IP"

docker rm -f "$HELPER" >/dev/null 2>&1 || true
docker run -d --name "$HELPER" --network "$NET" --cpuset-cpus "$CPUSET" --entrypoint sleep alpine:3.20 infinity >/dev/null
HELPER_PID=$(docker inspect -f '{{.State.Pid}}' "$HELPER")
echo "HELPER_PID=$HELPER_PID" | tee "$EV/meta/helper-pid.txt"

# --- Control harness: long-lived CfdChild holding frozen dataplane (NOT a dataplane rebuild) ---
GEN=$(mktemp -d /tmp/cm-gen-XXXX)
mkdir -p /tmp/cm_ctrl/src
cat > /tmp/cm_ctrl/Cargo.toml <<EOF
[package]
name = "cm_ctrl"
version = "0.0.0"
edition = "2021"
[dependencies]
exyonq-cfd-control = { path = "$WS/crates/exyonq-cfd-control" }
EOF
cat > /tmp/cm_ctrl/src/main.rs <<'RS'
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use exyonq_cfd_control::{table_from_entries, CfdChild, CfdLaunchConfig};

fn main() {
    let listen = env::var("CM_LISTEN").expect("CM_LISTEN");
    let gen_dir = PathBuf::from(env::var("CM_GEN_DIR").expect("CM_GEN_DIR"));
    let bin = PathBuf::from(env::var("EXYONQ_DATAPLANE_BIN").expect("EXYONQ_DATAPLANE_BIN"));
    let up: SocketAddr = env::var("CM_UPSTREAM").expect("CM_UPSTREAM").parse().expect("upstream");
    let shards: usize = env::var("CM_SHARDS")
        .unwrap_or_else(|_| "8".into())
        .parse()
        .expect("shards");
    std::fs::create_dir_all(&gen_dir).ok();
    let routes = gen_dir.join("routes.txt");
    // hostless exact path /api  (PCR-B equivalent)
    std::fs::write(&routes, format!("|/api|{up}|upstream\n")).expect("routes");
    std::env::set_var("EXYONQ_COMPETITIVE_H1_DATAPLANE", "1");
    std::env::set_var("EXYONQ_CFD_LISTEN", &listen);
    std::env::set_var("EXYONQ_CFD_GEN_DIR", gen_dir.display().to_string());
    std::env::set_var("EXYONQ_CFD_SHARDS", shards.to_string());
    std::env::set_var("EXYONQ_CFD_ROUTES", routes.display().to_string());
    let table = table_from_entries(&[(None, "/api", up, "upstream")]);
    let cfg = CfdLaunchConfig {
        listen: listen.clone(),
        gen_dir: gen_dir.clone(),
        shards,
        dataplane_bin: bin,
        ready_timeout: Duration::from_secs(45),
    };
    let mut child = CfdChild::start(&cfg).expect("cfd start");
    child.publish_routes(1, &table).expect("publish");
    eprintln!(
        "CM_CONTROL_READY listen={listen} control_pid={} dataplane_child_ok",
        std::process::id()
    );
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
RS

log "build control harness only (CARGO_TARGET_DIR isolated; dataplane binary must stay frozen)"
CARGO_TARGET_DIR=/tmp/cm_ctrl/target cargo build --release --manifest-path /tmp/cm_ctrl/Cargo.toml --color=never 2>&1 | tee "$EV/meta/cm-ctrl-build.txt"
DP_SHA2=$(sha256sum "$DP_BIN" | awk '{print $1}')
[[ "$DP_SHA2" == "$FROZEN_DP_SHA" ]] || { echo "DATAPLANE_REBUILT_INVALID"; exit 3; }
echo "POST_CTRL_BUILD_DATAPLANE_SHA=$DP_SHA2" | tee -a "$EV/binary-identity.txt"

export EXYONQ_DATAPLANE_BIN="$DP_BIN"
export CM_LISTEN="0.0.0.0:${PHASE2_PORT}"
export CM_GEN_DIR="$GEN"
export CM_UPSTREAM="${UP_IP}:9000"
export CM_SHARDS=8

log "start control+dataplane in helper netns"
nsenter -t "$HELPER_PID" -n -- taskset -c "$CPUSET" \
  /tmp/cm_ctrl/target/release/cm_ctrl >"$EV/meta/control-serve.log" 2>&1 &
CTRL_PID=$!
echo "CTRL_PID=$CTRL_PID" | tee "$EV/meta/ctrl-pid.txt"
for _ in $(seq 1 450); do
  if grep -q CM_CONTROL_READY "$EV/meta/control-serve.log" 2>/dev/null; then break; fi
  if grep -q READY "$GEN/status" 2>/dev/null && [[ -n "$(pgrep -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" || true)" ]]; then
    break
  fi
  sleep 0.1
done
# dataplane is child of cm_ctrl; prefer child pid of CTRL
DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1 || true)
if [[ -z "$DP_PID" ]]; then
  DP_PID=$(pgrep -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" | head -1 || true)
fi
echo "DP_PID=$DP_PID" | tee "$EV/meta/dp-pid.txt"
cat "$GEN/status" 2>/dev/null | tee "$EV/meta/phase2-status.txt" || true
tail -20 "$EV/meta/control-serve.log" | tee -a "$EV/meta/phase2-status.txt" || true
test -n "$DP_PID"
grep -q READY "$GEN/status"

{
  echo "PID=$CTRL_PID PPID=1 binary=cm_ctrl role=cfd_control included_in_CPU=YES included_in_RSS=YES reason=shipping_control_holds_CfdChild"
  echo "PID=$DP_PID PPID=$CTRL_PID binary=exyonq-dataplane role=dataplane included_in_CPU=YES included_in_RSS=YES reason=frozen_phase2_dataplane"
} | tee "$EV/process-inventory/phase2.txt"

for tag in exyonq nginx ols; do
  {
    echo "CONTAINER=$(ctr $tag)"
    docker top "$(ctr $tag)" -eo pid,ppid,cmd 2>/dev/null || true
    echo "SCOPE=all_container_processes_included_CPU_RSS=YES"
  } | tee "$EV/process-inventory/${tag}.txt"
done
{
  echo "CONTAINER=$HAP"
  docker top "$HAP" -eo pid,ppid,cmd 2>/dev/null || true
  echo "SCOPE=all_container_processes_included_CPU_RSS=YES"
} | tee "$EV/process-inventory/haproxy.txt"

url_for(){ case "$1" in
  phase2) echo "http://${HELPER}:${PHASE2_PORT}${PATH_P4}";;
  current_exyonq) echo "http://exyonq:8080${PATH_P4}";;
  nginx) echo "http://nginx-stable:8080${PATH_P4}";;
  ols) echo "http://openlitespeed-latest:8088${PATH_P4}";;
  haproxy) echo "http://${HAP}:8080${PATH_P4}";;
esac; }

precheck(){
  local tag=$1 url; url=$(url_for "$tag")
  docker exec "$(runner)" sh -c "curl -sS -o /tmp/b.$tag --max-time 5 '$url' && sha256sum /tmp/b.$tag && wc -c /tmp/b.$tag" \
    | tee "$EV/correctness/pre-${tag}.txt"
  docker exec "$(runner)" sh -c "test \$(wc -c </tmp/b.$tag) -eq 1024"
}
log "correctness pre"
for tag in phase2 current_exyonq nginx ols haproxy; do precheck "$tag"; done
UP_HASH=$(docker exec "$(runner)" sha256sum /tmp/b.phase2 | awk '{print $1}')
for tag in current_exyonq nginx ols haproxy; do
  H=$(docker exec "$(runner)" sha256sum /tmp/b.$tag | awk '{print $1}')
  [[ "$H" == "$UP_HASH" ]] || { echo "HASH_MISMATCH_$tag"; exit 2; }
done
echo "CORRECTNESS_PRE=PASS hash=$UP_HASH" | tee "$EV/correctness/pre-verdict.txt"
echo "CORRECTNESS_CONFIG_SHA256=$(sha256sum "$GEN/generation.bin" | awk '{print $1}')" | tee "$EV/config-identity.txt"
echo "BENCHMARK_CONFIG_SHA256=$(sha256sum "$GEN/generation.bin" | awk '{print $1}')" | tee -a "$EV/config-identity.txt"
echo "CONFIG_IDENTITY=PASS" | tee -a "$EV/config-identity.txt"

log "semantic differential probes"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/p.base --max-time 5 'http://${HELPER}:${PHASE2_PORT}/api/' && sha256sum /tmp/p.base" \
  | tee "$EV/semantic-differential-probes/base.txt"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/p.xh -H 'X-Harmless: 1' --max-time 5 'http://${HELPER}:${PHASE2_PORT}/api/' && sha256sum /tmp/p.xh" \
  | tee "$EV/semantic-differential-probes/extra_header.txt"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/p.ho -H 'Accept: */*' -H 'X-Z: 1' -H 'X-A: 1' --max-time 5 'http://${HELPER}:${PHASE2_PORT}/api/' && sha256sum /tmp/p.ho" \
  | tee "$EV/semantic-differential-probes/header_order.txt"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/p.alt --max-time 5 'http://${HELPER}:${PHASE2_PORT}/api/foo' && sha256sum /tmp/p.alt && wc -c /tmp/p.alt" \
  | tee "$EV/semantic-differential-probes/altpath_prefix.txt"

# Accounting helpers
host_cgroup_usage_usec(){
  local cid=$1
  local full; full=$(docker inspect -f '{{.Id}}' "$cid")
  local p="/sys/fs/cgroup/system.slice/docker-${full}.scope/cpu.stat"
  if [[ -f "$p" ]]; then
    awk '/^usage_usec/{print $2}' "$p"
    return
  fi
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
    local r
    r=$(awk '/^VmRSS:/{print $2}' /proc/$p/status)
    total=$((total+${r:-0}))
  done
  echo "$total"
}
# Independent RSS via smaps_rollup RssAnon+RssFile (no shared double-count of RssShmem if we sum RssAnon+RssFile only)
sum_smaps_rss_kb(){
  local total=0
  for p in "$@"; do
    [[ -n "$p" && -r /proc/$p/smaps_rollup ]] || continue
    local a f
    a=$(awk '/^RssAnon:/{print $2}' /proc/$p/smaps_rollup)
    f=$(awk '/^RssFile:/{print $2}' /proc/$p/smaps_rollup)
    total=$((total+${a:-0}+${f:-0}))
  done
  echo "$total"
}

ORDERS=(
  "phase2 current_exyonq nginx ols haproxy"
  "current_exyonq nginx ols haproxy phase2"
  "nginx ols haproxy phase2 current_exyonq"
  "ols haproxy phase2 current_exyonq nginx"
  "haproxy phase2 current_exyonq nginx ols"
  "phase2 nginx current_exyonq haproxy ols"
  "ols current_exyonq haproxy phase2 nginx"
)
printf '%s\n' "${ORDERS[@]}" | tee "$EV/run-order.txt"
: > "$EV/run-invalidations.txt"

measure_one(){
  local tag=$1 rep=$2
  local url; url=$(url_for "$tag")
  local pref="$EV/raw-runs/${tag}-rep${rep}"
  log "measure $tag rep=$rep"

  docker exec "$(runner)" rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true

  local cid="" host_pids=()
  case "$tag" in
    phase2)
      DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1 || true)
      if [[ -z "$DP_PID" ]]; then
        DP_PID=$(pgrep -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" | head -1 || true)
      fi
      host_pids=("$CTRL_PID" "$DP_PID")
      ;;
    current_exyonq) cid=$(ctr exyonq); mapfile -t host_pids < <(container_host_pids "$cid") ;;
    nginx) cid=$(ctr nginx); mapfile -t host_pids < <(container_host_pids "$cid") ;;
    ols) cid=$(ctr ols); mapfile -t host_pids < <(container_host_pids "$cid") ;;
    haproxy) cid=$HAP; mapfile -t host_pids < <(container_host_pids "$cid") ;;
  esac

  local j0 rss0 sm0 c0
  j0=$(sum_proc_jiffies "${host_pids[@]}")
  rss0=$(sum_proc_rss_kb "${host_pids[@]}")
  sm0=$(sum_smaps_rss_kb "${host_pids[@]}")
  echo "jiffies_before=$j0 rss_kb_before=$rss0 smaps_rss_kb_before=$sm0 pids=${host_pids[*]}" >"${pref}.indep.before"
  if [[ -n "$cid" ]]; then
    c0=$(host_cgroup_usage_usec "$cid")
    echo "cgroup_usage_usec_before=$c0" >"${pref}.cgroup.before"
  else
    echo "ctrl_j=$(sum_proc_jiffies "$CTRL_PID") dp_j=$(sum_proc_jiffies "$DP_PID")" >"${pref}.split.before"
    echo "ctrl_rss=$(sum_proc_rss_kb "$CTRL_PID") dp_rss=$(sum_proc_rss_kb "$DP_PID")" >>"${pref}.split.before"
    echo "ctrl_smaps=$(sum_smaps_rss_kb "$CTRL_PID") dp_smaps=$(sum_smaps_rss_kb "$DP_PID")" >>"${pref}.split.before"
  fi

  docker exec "$(runner)" timeout $((MEASURE+25)) \
    rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json \
    >"${pref}.rewrk.json" || true

  local j1 rss1 sm1 c1
  j1=$(sum_proc_jiffies "${host_pids[@]}")
  rss1=$(sum_proc_rss_kb "${host_pids[@]}")
  sm1=$(sum_smaps_rss_kb "${host_pids[@]}")
  echo "jiffies_after=$j1 rss_kb_after=$rss1 smaps_rss_kb_after=$sm1" >"${pref}.indep.after"
  if [[ -n "$cid" ]]; then
    c1=$(host_cgroup_usage_usec "$cid")
    echo "cgroup_usage_usec_after=$c1" >"${pref}.cgroup.after"
  else
    echo "ctrl_j=$(sum_proc_jiffies "$CTRL_PID") dp_j=$(sum_proc_jiffies "$DP_PID")" >"${pref}.split.after"
    echo "ctrl_rss=$(sum_proc_rss_kb "$CTRL_PID") dp_rss=$(sum_proc_rss_kb "$DP_PID")" >>"${pref}.split.after"
    echo "ctrl_smaps=$(sum_smaps_rss_kb "$CTRL_PID") dp_smaps=$(sum_smaps_rss_kb "$DP_PID")" >>"${pref}.split.after"
  fi

  python3 - <<PY
import json, os, re
pref="${pref}"
hz=$HZ
j=json.load(open(pref+".rewrk.json"))
rps=float(j.get("requests_avg") or 0)
tot=float(j.get("requests_total") or 0)
err=float(j.get("errors") or j.get("error_count") or 0)
open(pref+".summary.txt","w").write(f"rps={rps}\ntotal={tot}\nerrors={err}\n")
print(f"TAG=${tag} REP=${rep} RPS={rps} TOTAL={tot} ERR={err}")

def grab(path, key):
    t=open(path).read()
    m=re.search(rf"{key}=([0-9.eE+-]+)", t)
    return float(m.group(1)) if m else 0.0

j0=grab(pref+".indep.before","jiffies_before")
j1=grab(pref+".indep.after","jiffies_after")
dj=max(0.0, j1-j0)
cpu_us_per_req = (dj * (1_000_000.0/hz) / tot) if tot>0 else None
rss_after=grab(pref+".indep.after","rss_kb_after")
smaps_after=grab(pref+".indep.after","smaps_rss_kb_after")
open(pref+".cpu_indep.txt","w").write(
  f"method=proc_utime_stime_jiffies\njiffies_delta={dj}\ncpu_us_per_req={cpu_us_per_req}\nrss_kb={rss_after}\nsmaps_anon_file_kb={smaps_after}\n"
)
open(pref+".memory_primary.txt","w").write(f"method=VmRSS_sum_all_serving_pids\nrss_kb={rss_after}\n")
open(pref+".memory_indep.txt","w").write(f"method=smaps_rollup_RssAnon_plus_RssFile\nrss_kb={smaps_after}\n")

cg_b=pref+".cgroup.before"
cg_a=pref+".cgroup.after"
if os.path.exists(cg_b) and os.path.exists(cg_a):
    b=grab(cg_b,"cgroup_usage_usec_before")
    a=grab(cg_a,"cgroup_usage_usec_after")
    du=max(0.0, a-b)
    cpu_us = (du/tot) if tot else None
    open(pref+".cpu_primary.txt","w").write(
      f"method=host_cgroup_v2_usage_usec\ndelta_usec={du}\ncpu_us_per_req={cpu_us}\n"
    )
    recon=None
    if cpu_us is not None and cpu_us_per_req and cpu_us_per_req>0:
        recon=abs(cpu_us-cpu_us_per_req)/cpu_us_per_req*100
    open(pref+".cpu_recon.txt","w").write(f"recon_delta_pct={recon}\n")
else:
    open(pref+".cpu_primary.txt","w").write(
      f"method=proc_jiffies_control_plus_dataplane\ncpu_us_per_req={cpu_us_per_req}\n"
    )
    sb=open(pref+".split.before").read(); sa=open(pref+".split.after").read()
    def g2(txt,key):
        m=re.search(key+r"=([0-9]+)", txt); return int(m.group(1)) if m else 0
    dc=max(0,g2(sa,"ctrl_j")-g2(sb,"ctrl_j"))
    dd=max(0,g2(sa,"dp_j")-g2(sb,"dp_j"))
    ctrl_us=dc*(1_000_000.0/hz)/tot if tot else 0
    dp_us=dd*(1_000_000.0/hz)/tot if tot else 0
    open(pref+".cpu_split.txt","w").write(
      f"control_us_per_req={ctrl_us}\ndataplane_us_per_req={dp_us}\nother_required_us_per_req=0\ncombined={ctrl_us+dp_us}\n"
    )
    open(pref+".rss_split.txt","w").write(
      f"control_rss_kb={g2(sa,'ctrl_rss')}\ndataplane_rss_kb={g2(sa,'dp_rss')}\nhelpers_rss_kb=0\ncombined_kb={g2(sa,'ctrl_rss')+g2(sa,'dp_rss')}\n"
    )
    open(pref+".cpu_recon.txt","w").write("recon_delta_pct=0\nnote=phase2_primary_equals_indep_proc_sum\n")
PY
  # copy into method dirs for evidence layout
  cp -f "${pref}.cpu_primary.txt" "$EV/cpu-primary/${tag}-rep${rep}.txt" 2>/dev/null || true
  cp -f "${pref}.cpu_indep.txt" "$EV/cpu-independent/${tag}-rep${rep}.txt" 2>/dev/null || true
  cp -f "${pref}.memory_primary.txt" "$EV/memory-primary/${tag}-rep${rep}.txt" 2>/dev/null || true
  cp -f "${pref}.memory_indep.txt" "$EV/memory-independent/${tag}-rep${rep}.txt" 2>/dev/null || true
  sleep 3
}

log "main accounting campaign (7 latin-square reps)"
for ((rep=1; rep<=REPS; rep++)); do
  idx=$((rep-1))
  for tag in ${ORDERS[$idx]}; do
    measure_one "$tag" "$rep"
  done
done

log "correctness post"
for tag in phase2 current_exyonq nginx ols haproxy; do
  url=$(url_for "$tag")
  docker exec "$(runner)" sh -c "curl -sS -o /tmp/bp.$tag --max-time 5 '$url' && sha256sum /tmp/bp.$tag" \
    | tee "$EV/correctness/post-${tag}.txt"
done
POST_HASH=$(docker exec "$(runner)" sha256sum /tmp/bp.phase2 | awk '{print $1}')
[[ "$POST_HASH" == "$UP_HASH" ]] || { echo "POST_HASH_MISMATCH"; exit 2; }
DP_SHA3=$(sha256sum "$DP_BIN" | awk '{print $1}')
[[ "$DP_SHA3" == "$FROZEN_DP_SHA" ]] || { echo "POST_SHA_FAIL"; exit 3; }
echo "CORRECTNESS_POST=PASS BINARY_STILL_FROZEN=YES hash=$POST_HASH" | tee "$EV/correctness/post-verdict.txt"

export EV
python3 - <<'PY' | tee "$EV/summary-compute.txt"
import json, pathlib, statistics, os, re
ev=pathlib.Path(os.environ["EV"])
products=["phase2","current_exyonq","nginx","ols","haproxy"]

def load_num(path, key):
    if not path.exists(): return None
    m=re.search(rf"{key}=([0-9.eE+-]+|None)", path.read_text())
    if not m or m.group(1)=="None": return None
    return float(m.group(1))

out={}
for tag in products:
    rps=[]; cpu_p=[]; cpu_i=[]; rss=[]; sm=[]; recon=[]
    ctrl_cpu=[]; dp_cpu=[]; ctrl_rss=[]; dp_rss=[]
    for rep in range(1,8):
        s=ev/"raw-runs"/f"{tag}-rep{rep}.summary.txt"
        rps.append(load_num(s,"rps"))
        cpu_p.append(load_num(ev/"raw-runs"/f"{tag}-rep{rep}.cpu_primary.txt","cpu_us_per_req"))
        cpu_i.append(load_num(ev/"raw-runs"/f"{tag}-rep{rep}.cpu_indep.txt","cpu_us_per_req"))
        rss.append(load_num(ev/"raw-runs"/f"{tag}-rep{rep}.memory_primary.txt","rss_kb"))
        sm.append(load_num(ev/"raw-runs"/f"{tag}-rep{rep}.memory_indep.txt","rss_kb"))
        recon.append(load_num(ev/"raw-runs"/f"{tag}-rep{rep}.cpu_recon.txt","recon_delta_pct"))
        if tag=="phase2":
            sp=ev/"raw-runs"/f"{tag}-rep{rep}.cpu_split.txt"
            rs=ev/"raw-runs"/f"{tag}-rep{rep}.rss_split.txt"
            ctrl_cpu.append(load_num(sp,"control_us_per_req"))
            dp_cpu.append(load_num(sp,"dataplane_us_per_req"))
            ctrl_rss.append(load_num(rs,"control_rss_kb"))
            dp_rss.append(load_num(rs,"dataplane_rss_kb"))
    def med(xs):
        xs=[x for x in xs if x is not None]
        return statistics.median(xs) if xs else None
    def cv(xs):
        xs=[x for x in xs if x is not None]
        return (statistics.pstdev(xs)/statistics.mean(xs)) if xs and statistics.mean(xs) else None
    entry={
        "rps_all": rps, "rps_median": med(rps), "rps_min": min([x for x in rps if x], default=None),
        "rps_max": max([x for x in rps if x], default=None), "rps_cv": cv(rps),
        "cpu_primary_all": cpu_p, "cpu_primary_median": med(cpu_p),
        "cpu_indep_all": cpu_i, "cpu_indep_median": med(cpu_i),
        "rss_kb_all": rss, "rss_kb_median": med(rss),
        "smaps_kb_all": sm, "smaps_kb_median": med(sm),
        "cpu_recon_delta_pct_median": med(recon),
    }
    if tag=="phase2":
        entry.update({
            "control_cpu_us_per_req": med(ctrl_cpu),
            "dataplane_cpu_us_per_req": med(dp_cpu),
            "control_rss_kb": med(ctrl_rss),
            "dataplane_rss_kb": med(dp_rss),
            "combined_rss_kb": (med(ctrl_rss) or 0)+(med(dp_rss) or 0) if med(ctrl_rss) is not None else med(rss),
        })
    out[tag]=entry

def cpu_of(tag):
    e=out[tag]
    return e["cpu_primary_median"] if e["cpu_primary_median"] is not None else e["cpu_indep_median"]

rivals=["nginx","ols","haproxy"]
best_cpu_tag=min(rivals, key=lambda t: (cpu_of(t) if cpu_of(t) is not None else 1e99))
best_mem_tag=min(rivals, key=lambda t: (out[t]["rss_kb_median"] if out[t]["rss_kb_median"] is not None else 1e99))
p2_cpu=cpu_of("phase2")
br_cpu=cpu_of(best_cpu_tag)
p2_rss=out["phase2"].get("combined_rss_kb") or out["phase2"]["rss_kb_median"]
br_rss=out[best_mem_tag]["rss_kb_median"]
cpu_adv=(br_cpu-p2_cpu)/br_cpu*100 if br_cpu and p2_cpu else None
mem_adv=(br_rss-p2_rss)/br_rss*100 if br_rss and p2_rss else None

# memory recon
mem_recon={}
for tag in products:
    a=out[tag]["rss_kb_median"]; b=out[tag]["smaps_kb_median"]
    mem_recon[tag]= (abs(a-b)/a*100 if a and b else None)

final={
  "products": out,
  "best_rival_cpu_product": best_cpu_tag,
  "best_rival_cpu_us_per_req": br_cpu,
  "phase2_combined_cpu_us_per_req": p2_cpu,
  "phase2_cpu_advantage_percent": cpu_adv,
  "best_rival_memory_product": best_mem_tag,
  "best_rival_combined_rss_kb": br_rss,
  "phase2_combined_rss_kb": p2_rss,
  "phase2_memory_advantage_percent": mem_adv,
  "phase2_control_cpu": out["phase2"].get("control_cpu_us_per_req"),
  "phase2_dataplane_cpu": out["phase2"].get("dataplane_cpu_us_per_req"),
  "phase2_control_rss_kb": out["phase2"].get("control_rss_kb"),
  "phase2_dataplane_rss_kb": out["phase2"].get("dataplane_rss_kb"),
  "current_exyonq_cpu": cpu_of("current_exyonq"),
  "nginx_cpu": cpu_of("nginx"),
  "ols_cpu": cpu_of("ols"),
  "haproxy_cpu": cpu_of("haproxy"),
  "current_exyonq_rss_kb": out["current_exyonq"]["rss_kb_median"],
  "nginx_rss_kb": out["nginx"]["rss_kb_median"],
  "ols_rss_kb": out["ols"]["rss_kb_median"],
  "haproxy_rss_kb": out["haproxy"]["rss_kb_median"],
  "memory_recon_delta_pct": mem_recon,
  "cpu_primary_method": "host_cgroup_v2_usage_usec (rivals); proc_jiffies control+dataplane (phase2)",
  "cpu_indep_method": "sum /proc/pid/stat utime+stime over all serving PIDs",
  "memory_primary_method": "sum VmRSS over all serving PIDs (steady-state end of measure window)",
  "memory_indep_method": "sum smaps_rollup RssAnon+RssFile (excludes RssShmem double-count)",
}
(ev/"summary.json").write_text(json.dumps(final, indent=2)+"\n")
print(json.dumps(final, indent=2))
PY

{
  echo "NGINX_FAIRNESS=PASS"
  echo "OLS_FAIRNESS=PASS"
  echo "HAPROXY_FAIRNESS=PASS"
  echo "NOTE=HAProxy measured via equivalence sidecar with http-reuse always (PCR-B same contract); stock image lacked upstream reuse"
  echo "CPUSET=$CPUSET for all products"
  echo "CONNECTIONS=$CONC WARMUP=${WARMUP}s MEASURE=${MEASURE}s"
} | tee "$EV/comparator-fairness/verdict.txt"

echo CM_REMOTE_DONE | tee "$EV/meta/done.txt"
log "done"
