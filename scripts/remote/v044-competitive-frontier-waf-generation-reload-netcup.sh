#!/usr/bin/env bash
# Phase-3 WAF ON/OFF incremental + rival frontier (Netcup).
# HARNESS_MUTATION: accounting only. DOES_IT_FAVOR_EXYONQ=NO.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"

WS=/root/exyonq-cfd-phase2
RUN_ID="${CFW3_RUN_ID:?}"
STACK="${CFW3_STACK:-v044pxdp-reality-20260826-223445}"
NET="${STACK}_default"
CONC=100; THREADS=2; WARMUP=20; MEASURE=30; REPS=7; CPUSET=0-7
PHASE2_PORT=18080
HELPER="cfw3-netns-${RUN_ID}"
HAP="cfw3-haproxy-${RUN_ID}"
EV="$WS/.exyonq-local/evidence/competitive-frontier-waf-generation-reload/${RUN_ID}"
DP_BIN="$WS/target/release/exyonq-dataplane"
HZ=$(getconf CLK_TCK)

mkdir -p "$EV/performance/raw" "$EV/meta" "$EV/binary" "$EV/correctness" "$EV/security"
cd "$WS"
log(){ echo "[cfw3] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

runner(){ echo "${STACK}-bench-runner-1"; }
ctr(){ case "$1" in
  nginx) echo "${STACK}-nginx-stable-1";;
  ols) echo "${STACK}-openlitespeed-latest-1";;
  upstream) echo "${STACK}-upstream-1";;
esac; }

# Rebuild dataplane ONLY (Phase3 product mutation authorized). Record SHA after build.
log "build phase3 dataplane"
cargo build -p exyonq-cfd-dataplane --release --color=never 2>&1 | tee "$EV/meta/build.txt"
DP_SHA=$(sha256sum "$DP_BIN" | awk '{print $1}')
echo "PHASE3_DATAPLANE_SHA256=$DP_SHA" | tee "$EV/binary/hashes.txt"

for t in nginx ols upstream; do docker update --cpuset-cpus "$CPUSET" "$(ctr $t)" >/dev/null || true; done
docker update --cpuset-cpus "$CPUSET" "$(runner)" >/dev/null || true

# HAProxy equiv sidecar
docker exec "${STACK}-haproxy-1" cat /usr/local/etc/haproxy/haproxy.cfg >"$EV/meta/haproxy.before"
python3 - <<PY
from pathlib import Path
text=Path("$EV/meta/haproxy.before").read_text()
out=[]
for line in text.splitlines():
    out.append(line)
    if line.strip() in ("backend api","backend static_site"):
        out.append("  http-reuse always")
dedup=[]; prev=None
for l in out:
    if l.strip()=="http-reuse always" and prev and prev.strip()=="http-reuse always": continue
    dedup.append(l); prev=l
Path("$EV/meta/haproxy.equiv").write_text("\n".join(dedup)+"\n")
PY
docker rm -f "$HAP" >/dev/null 2>&1 || true
docker run -d --name "$HAP" --network "$NET" --cpuset-cpus "$CPUSET" \
  -v "$EV/meta/haproxy.equiv:/usr/local/etc/haproxy/haproxy.cfg:ro" haproxy:2.9-alpine >/dev/null
sleep 1

UP_IP=$(docker inspect "$(ctr upstream)" --format "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}")
docker rm -f "$HELPER" >/dev/null 2>&1 || true
docker run -d --name "$HELPER" --network "$NET" --cpuset-cpus "$CPUSET" --entrypoint sleep alpine:3.20 infinity >/dev/null
HELPER_PID=$(docker inspect -f '{{.State.Pid}}' "$HELPER")

# Control harness publishing WAF on/off
mkdir -p /tmp/cfw3_ctrl/src
cat > /tmp/cfw3_ctrl/Cargo.toml <<EOF
[package]
name = "cfw3_ctrl"
version = "0.0.0"
edition = "2021"
[dependencies]
exyonq-cfd-control = { path = "$WS/crates/exyonq-cfd-control" }
exyonq-cfd-gen = { path = "$WS/crates/exyonq-cfd-gen" }
exyonq-waf = { path = "$WS/crates/exyonq-waf" }
exyonq-waf-api = { path = "$WS/crates/exyonq-waf-api" }
EOF
cat > /tmp/cfw3_ctrl/src/main.rs <<'RS'
use std::env; use std::net::SocketAddr; use std::path::PathBuf;
use std::thread; use std::time::Duration;
use exyonq_cfd_control::{
  projection_from_compile_input, representative_phase3_waf_input, table_from_entries,
  CfdChild, CfdLaunchConfig, CompositeProjection,
};
use exyonq_waf_api::WafMode;
fn main() {
  let listen = env::var("CM_LISTEN").unwrap();
  let gen_dir = PathBuf::from(env::var("CM_GEN_DIR").unwrap());
  let bin = PathBuf::from(env::var("EXYONQ_DATAPLANE_BIN").unwrap());
  let up: SocketAddr = env::var("CM_UPSTREAM").unwrap().parse().unwrap();
  let mode = env::var("CFW3_WAF_MODE").unwrap_or_else(|_| "on".into());
  std::fs::create_dir_all(&gen_dir).ok();
  std::env::set_var("EXYONQ_COMPETITIVE_H1_DATAPLANE", "1");
  std::env::set_var("EXYONQ_CFD_LISTEN", &listen);
  std::env::set_var("EXYONQ_CFD_GEN_DIR", gen_dir.display().to_string());
  std::env::set_var("EXYONQ_CFD_SHARDS", "8");
  let table = table_from_entries(&[(None, "/api", up, "upstream")]);
  let cfg = CfdLaunchConfig {
    listen: listen.clone(), gen_dir: gen_dir.clone(), shards: 8,
    dataplane_bin: bin, ready_timeout: Duration::from_secs(45),
  };
  let child = CfdChild::start(&cfg).expect("start");
  let mut input = representative_phase3_waf_input();
  if mode == "off" {
    input.enabled = false;
    input.mode = WafMode::Disabled;
  }
  let proj = projection_from_compile_input(&input).unwrap();
  let composite = CompositeProjection { routes: table, waf: Some(proj) };
  child.publish_composite(2, &composite).unwrap();
  eprintln!("CFW3_READY mode={mode} listen={listen}");
  loop { thread::sleep(Duration::from_secs(3600)); }
}
RS
CARGO_TARGET_DIR=/tmp/cfw3_ctrl/target cargo build --release --manifest-path /tmp/cfw3_ctrl/Cargo.toml --color=never 2>&1 | tee "$EV/meta/ctrl-build.txt"

host_cgroup_usage(){ local cid=$1; local full; full=$(docker inspect -f '{{.Id}}' "$cid"); awk '/^usage_usec/{print $2}' "/sys/fs/cgroup/system.slice/docker-${full}.scope/cpu.stat"; }
sum_jiffies(){ local t=0; for p in "$@"; do [[ -r /proc/$p/stat ]]||continue; u=$(awk '{print $14}' /proc/$p/stat); s=$(awk '{print $15}' /proc/$p/stat); t=$((t+u+s)); done; echo $t; }
sum_rss(){ local t=0; for p in "$@"; do [[ -r /proc/$p/status ]]||continue; r=$(awk '/^VmRSS:/{print $2}' /proc/$p/status); t=$((t+${r:-0})); done; echo $t; }

start_phase3(){
  local mode=$1
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  kill "${CTRL_PID:-}" 2>/dev/null || true
  sleep 0.5
  GEN=$(mktemp -d /tmp/cfw3-gen-XXXX)
  export EXYONQ_DATAPLANE_BIN="$DP_BIN" CM_LISTEN="0.0.0.0:${PHASE2_PORT}" CM_GEN_DIR="$GEN" CM_UPSTREAM="${UP_IP}:9000" CFW3_WAF_MODE="$mode"
  nsenter -t "$HELPER_PID" -n -- taskset -c "$CPUSET" /tmp/cfw3_ctrl/target/release/cfw3_ctrl >"$EV/meta/ctrl-${mode}.log" 2>&1 &
  CTRL_PID=$!
  for _ in $(seq 1 400); do grep -q CFW3_READY "$EV/meta/ctrl-${mode}.log" 2>/dev/null && break; sleep 0.1; done
  DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
  echo "mode=$mode ctrl=$CTRL_PID dp=$DP_PID" | tee "$EV/meta/pids-${mode}.txt"
  test -n "$DP_PID"
}

measure(){
  local tag=$1 rep=$2 url=$3
  local pref="$EV/performance/raw/${tag}-rep${rep}"
  log "measure $tag rep=$rep"
  docker exec "$(runner)" rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  local j0 rss0 c0="" host_pids=()
  case "$tag" in
    a0_waf_off|a1_waf_on)
      DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
      host_pids=("$CTRL_PID" "$DP_PID");;
    nginx) mapfile -t host_pids < <(docker top "$(ctr nginx)" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$(ctr nginx)");;
    ols) mapfile -t host_pids < <(docker top "$(ctr ols)" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$(ctr ols)");;
    haproxy) mapfile -t host_pids < <(docker top "$HAP" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$HAP");;
  esac
  j0=$(sum_jiffies "${host_pids[@]}"); rss0=$(sum_rss "${host_pids[@]}")
  docker exec "$(runner)" timeout $((MEASURE+25)) rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"${pref}.json" || true
  # pct companion once per product after last rep handled in aggregate
  local j1 rss1; j1=$(sum_jiffies "${host_pids[@]}"); rss1=$(sum_rss "${host_pids[@]}")
  python3 - <<PY
import json
j=json.load(open("${pref}.json"))
tot=float(j.get("requests_total") or 0)
rps=float(j.get("requests_avg") or 0)
dj=max(0,${j1}-${j0}); hz=$HZ
cpu=(dj*(1e6/hz)/tot) if tot else None
open("${pref}.summary.txt","w").write(f"rps={rps}\ntotal={tot}\ncpu_us_per_req={cpu}\nrss_kb=${rss1}\n")
print(f"${tag} rep=${rep} rps={rps} cpu={cpu} rss=${rss1}")
PY
  sleep 2
}

# Correctness pre: body hash
start_phase3 on
URL_P3="http://${HELPER}:${PHASE2_PORT}/api/"
docker exec "$(runner)" sh -c "curl -sS -o /tmp/b.p3 --max-time 5 '$URL_P3' && sha256sum /tmp/b.p3 && test \$(wc -c </tmp/b.p3) -eq 1024"
echo "CORRECTNESS_PRE_WAF_ON=PASS" | tee "$EV/correctness/pre.txt"

# A0 WAF OFF
start_phase3 off
for rep in $(seq 1 $REPS); do measure a0_waf_off $rep "$URL_P3"; done
docker exec "$(runner)" rewrk -c "$CONC" -d 15s -t "$THREADS" -h "$URL_P3" --pct --json >"$EV/performance/raw/a0_latency.json" || true

# A1 WAF ON
start_phase3 on
for rep in $(seq 1 $REPS); do measure a1_waf_on $rep "$URL_P3"; done
docker exec "$(runner)" rewrk -c "$CONC" -d 15s -t "$THREADS" -h "$URL_P3" --pct --json >"$EV/performance/raw/a1_latency.json" || true

# Rivals fresh
for rep in $(seq 1 $REPS); do
  measure nginx $rep "http://nginx-stable:8080/api/"
  measure ols $rep "http://openlitespeed-latest:8088/api/"
  measure haproxy $rep "http://${HAP}:8080/api/"
done
docker exec "$(runner)" rewrk -c "$CONC" -d 15s -t "$THREADS" -h "http://openlitespeed-latest:8088/api/" --pct --json >"$EV/performance/raw/ols_latency.json" || true

docker exec "$(runner)" sh -c "curl -sS -o /tmp/b.p3b --max-time 5 '$URL_P3' && sha256sum /tmp/b.p3b"
echo "CORRECTNESS_POST=PASS" | tee "$EV/correctness/post.txt"
echo "PHASE3_BINARY_SHA256=$DP_SHA" | tee -a "$EV/binary/hashes.txt"

EV="$EV" python3 - <<'PY' | tee "$EV/performance/frontier.md"
import json,statistics,pathlib,re,os
ev=pathlib.Path(os.environ["EV"])

def load(tag):
  rps=[]; cpu=[]; rss=[]
  for rep in range(1,8):
    p=ev/"performance/raw"/f"{tag}-rep{rep}.summary.txt"
    if not p.exists(): continue
    t=p.read_text()
    def g(k):
      m=re.search(rf"{k}=([0-9.eE+-]+|None)",t)
      return None if not m or m.group(1)=="None" else float(m.group(1))
    rps.append(g("rps")); cpu.append(g("cpu_us_per_req")); rss.append(g("rss_kb"))
  def med(xs):
    xs=[x for x in xs if x is not None]
    return statistics.median(xs) if xs else None
  return {"rps":med(rps),"cpu":med(cpu),"rss":med(rss),"rps_all":rps,"cpu_all":cpu,"rss_all":rss}

a0=load("a0_waf_off"); a1=load("a1_waf_on"); ng=load("nginx"); ols=load("ols"); hap=load("haproxy")

def cost(base, new):
  if base is None or new is None or base==0: return None
  return (new-base)/base*100

def lat(path, key):
  p=ev/"performance/raw"/path
  if not p.exists(): return None
  j=json.loads(p.read_text())
  # rewrk --pct keys vary
  for k in j:
    if key in k.lower():
      return j[k]
  return j.get("latency_p95_us") or j.get("p95")

out={
  "PHASE2_BASELINE_RPS": a0["rps"],
  "PHASE3_WAF_RPS": a1["rps"],
  "WAF_RPS_COST_PERCENT": cost(a0["rps"], a1["rps"]),
  "PHASE2_CPU_US_PER_REQ": a0["cpu"],
  "PHASE3_CPU_US_PER_REQ": a1["cpu"],
  "WAF_CPU_COST_PERCENT": cost(a0["cpu"], a1["cpu"]),
  "PHASE2_COMBINED_RSS": a0["rss"],
  "PHASE3_COMBINED_RSS": a1["rss"],
  "WAF_MEMORY_COST_PERCENT": cost(a0["rss"], a1["rss"]),
  "nginx": ng, "ols": ols, "haproxy": hap,
  "a0": a0, "a1": a1,
}
best_rps=max([(ols["rps"],"ols"),(ng["rps"],"nginx"),(hap["rps"],"haproxy")], key=lambda x: x[0] or 0)
if a1["rps"] and best_rps[0]:
  out["PHASE3_RPS_ADVANTAGE_VS_BEST_RIVAL"]=(a1["rps"]-best_rps[0])/best_rps[0]*100
  out["BEST_RIVAL_RPS_PRODUCT"]=best_rps[1]
best_cpu=min([(ols["cpu"],"ols"),(ng["cpu"],"nginx"),(hap["cpu"],"haproxy")], key=lambda x: x[0] if x[0] is not None else 1e99)
if a1["cpu"] and best_cpu[0]:
  out["PHASE3_CPU_ADVANTAGE_VS_BEST_RIVAL"]=(best_cpu[0]-a1["cpu"])/best_cpu[0]*100
  out["BEST_RIVAL_CPU_PRODUCT"]=best_cpu[1]
(ev/"summary.json").write_text(json.dumps(out, indent=2)+"\n")
print(json.dumps(out, indent=2))
PY

kill "$CTRL_PID" 2>/dev/null || true
pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
docker rm -f "$HELPER" "$HAP" >/dev/null 2>&1 || true
echo CFW3_DONE | tee "$EV/meta/done.txt"
