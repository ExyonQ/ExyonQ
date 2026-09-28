#!/usr/bin/env bash
# Phase-4 body streaming + rival frontier (Netcup AMD64).
# HARNESS_MUTATION: accounting + POST body loadgen. DOES_IT_FAVOR_EXYONQ=NO.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"

WS=/root/exyonq-cfd-phase2
RUN_ID="${CFB4_RUN_ID:?}"
STACK="${CFB4_STACK:-v044pxdp-reality-20260826-223445}"
NET="${STACK}_default"
CONC=100; THREADS=2; WARMUP=15; MEASURE=30; REPS=7; CPUSET=0-7
PORT=18080
HELPER="cfb4-netns-${RUN_ID}"
HAP="cfb4-haproxy-${RUN_ID}"
EV="$WS/.exyonq-local/evidence/competitive-frontier-request-bodies-backpressure/${RUN_ID}"
DP_BIN="$WS/target/release/exyonq-dataplane"
HZ=$(getconf CLK_TCK)

mkdir -p "$EV"/{performance/raw,meta,binary,correctness,security,slow-client,slow-upstream}
cd "$WS"
log(){ echo "[cfb4] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

runner(){ echo "${STACK}-bench-runner-1"; }
ctr(){ case "$1" in
  nginx) echo "${STACK}-nginx-stable-1";;
  ols) echo "${STACK}-openlitespeed-latest-1";;
  upstream) echo "${STACK}-upstream-1";;
esac; }

log "build phase4 dataplane"
cargo build -p exyonq-cfd-dataplane --release --color=never 2>&1 | tee "$EV/meta/build.txt"
DP_SHA=$(sha256sum "$DP_BIN" | awk '{print $1}')
echo "PHASE4_DATAPLANE_SHA256=$DP_SHA" | tee "$EV/binary/hashes.txt"

for t in nginx ols upstream; do docker update --cpuset-cpus "$CPUSET" "$(ctr $t)" >/dev/null || true; done
docker update --cpuset-cpus "$CPUSET" "$(runner)" >/dev/null || true

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

mkdir -p /tmp/cfb4_ctrl/src
cat > /tmp/cfb4_ctrl/Cargo.toml <<EOF
[package]
name = "cfb4_ctrl"
version = "0.0.0"
edition = "2021"
[dependencies]
exyonq-cfd-control = { path = "$WS/crates/exyonq-cfd-control" }
exyonq-cfd-gen = { path = "$WS/crates/exyonq-cfd-gen" }
exyonq-waf = { path = "$WS/crates/exyonq-waf" }
exyonq-waf-api = { path = "$WS/crates/exyonq-waf-api" }
EOF
cat > /tmp/cfb4_ctrl/src/main.rs <<'RS'
use std::env; use std::net::SocketAddr; use std::path::PathBuf;
use std::thread; use std::time::Duration;
use exyonq_cfd_control::{
  projection_from_compile_input, representative_phase3_waf_input, table_from_entries,
  CfdChild, CfdLaunchConfig, CompositeProjection,
};
fn main() {
  let listen = env::var("CM_LISTEN").unwrap();
  let gen_dir = PathBuf::from(env::var("CM_GEN_DIR").unwrap());
  let bin = PathBuf::from(env::var("EXYONQ_DATAPLANE_BIN").unwrap());
  let up: SocketAddr = env::var("CM_UPSTREAM").unwrap().parse().unwrap();
  std::fs::create_dir_all(&gen_dir).ok();
  std::env::set_var("EXYONQ_COMPETITIVE_H1_DATAPLANE", "1");
  let table = table_from_entries(&[(None, "/api", up, "upstream")]);
  let cfg = CfdLaunchConfig {
    listen: listen.clone(), gen_dir: gen_dir.clone(), shards: 8,
    dataplane_bin: bin, ready_timeout: Duration::from_secs(45),
  };
  let child = CfdChild::start(&cfg).expect("start");
  let input = representative_phase3_waf_input();
  let proj = projection_from_compile_input(&input).unwrap();
  let composite = CompositeProjection { routes: table, waf: Some(proj) };
  child.publish_composite(2, &composite).unwrap();
  eprintln!("CFB4_READY listen={listen}");
  loop { thread::sleep(Duration::from_secs(3600)); }
}
RS
CARGO_TARGET_DIR=/tmp/cfb4_ctrl/target cargo build --release --manifest-path /tmp/cfb4_ctrl/Cargo.toml --color=never 2>&1 | tee "$EV/meta/ctrl-build.txt"

host_cgroup_usage(){ local cid=$1; local full; full=$(docker inspect -f '{{.Id}}' "$cid"); awk '/^usage_usec/{print $2}' "/sys/fs/cgroup/system.slice/docker-${full}.scope/cpu.stat"; }
sum_jiffies(){ local t=0; for p in "$@"; do [[ -r /proc/$p/stat ]]||continue; u=$(awk '{print $14}' /proc/$p/stat); s=$(awk '{print $15}' /proc/$p/stat); t=$((t+u+s)); done; echo $t; }
sum_rss(){ local t=0; for p in "$@"; do [[ -r /proc/$p/status ]]||continue; r=$(awk '/^VmRSS:/{print $2}' /proc/$p/status); t=$((t+${r:-0})); done; echo $t; }

start_dp(){
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PORT}" 2>/dev/null || true
  kill "${CTRL_PID:-}" 2>/dev/null || true
  sleep 0.5
  GEN=$(mktemp -d /tmp/cfb4-gen-XXXX)
  export EXYONQ_DATAPLANE_BIN="$DP_BIN" CM_LISTEN="0.0.0.0:${PORT}" CM_GEN_DIR="$GEN" CM_UPSTREAM="${UP_IP}:9000"
  nsenter -t "$HELPER_PID" -n -- taskset -c "$CPUSET" /tmp/cfb4_ctrl/target/release/cfb4_ctrl >"$EV/meta/ctrl.log" 2>&1 &
  CTRL_PID=$!
  for _ in $(seq 1 400); do grep -q CFB4_READY "$EV/meta/ctrl.log" 2>/dev/null && break; sleep 0.1; done
  DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
  echo "ctrl=$CTRL_PID dp=$DP_PID" | tee "$EV/meta/pids.txt"
  test -n "$DP_PID"
}

# Python HTTP POST loadgen (Content-Length). Runs inside bench-runner.
# Writes JSON: requests, bytes, elapsed_s, errors, p50/p95/p99 ms
post_loadgen(){
  local url=$1 bytes=$2 conc=$3 duration=$4 out=$5
  docker exec "$(runner)" python3 - <<PY >"$out"
import concurrent.futures, socket, time, statistics, json, urllib.parse
url="$url"; body_len=int("$bytes"); conc=int("$conc"); duration=float("$duration")
u=urllib.parse.urlparse(url)
host, port = u.hostname, u.port or 80
path = u.path or "/"
body = b"A" * body_len
hdr = (
  f"POST {path} HTTP/1.1\\r\\nHost: {host}\\r\\nContent-Length: {body_len}\\r\\n"
  f"Connection: keep-alive\\r\\n\\r\\n"
).encode() + body

def one_session(stop_at):
  ok=err=0; lat=[]; nbytes=0
  try:
    s=socket.create_connection((host, port), timeout=5)
    s.settimeout(5)
  except Exception:
    return 0,1,[],0
  while time.time() < stop_at:
    t0=time.perf_counter()
    try:
      s.sendall(hdr)
      buf=b""
      while b"\\r\\n\\r\\n" not in buf:
        chunk=s.recv(65536)
        if not chunk: raise ConnectionError("eof")
        buf+=chunk
      # drain fixed 1024 body if present (stack upstream) else Content-Length
      head,rest=buf.split(b"\\r\\n\\r\\n",1)
      cl=None
      for line in head.split(b"\\r\\n")[1:]:
        if line.lower().startswith(b"content-length:"):
          cl=int(line.split(b":",1)[1].strip())
      need = 0 if cl is None else max(0, cl-len(rest))
      while need>0:
        chunk=s.recv(min(65536, need))
        if not chunk: break
        need-=len(chunk)
        nbytes+=len(chunk)
      nbytes += len(rest) if cl else 0
      if b" 200 " in head.split(b"\\r\\n",1)[0]:
        ok+=1
      else:
        err+=1
      lat.append((time.perf_counter()-t0)*1000)
    except Exception:
      err+=1
      try: s.close()
      except Exception: pass
      try: s=socket.create_connection((host, port), timeout=5); s.settimeout(5)
      except Exception: break
  try: s.close()
  except Exception: pass
  return ok, err, lat, nbytes

stop=time.time()+duration
with concurrent.futures.ThreadPoolExecutor(max_workers=conc) as ex:
  futs=[ex.submit(one_session, stop) for _ in range(conc)]
  oks=errs=0; lats=[]; nbytes=0
  for f in concurrent.futures.as_completed(futs):
    o,e,l,n=f.result(); oks+=o; errs+=e; lats+=l; nbytes+=n
elapsed=duration
lats.sort()
def pct(p):
  if not lats: return None
  i=min(len(lats)-1, int(round((p/100)*(len(lats)-1))))
  return lats[i]
out={
  "requests": oks, "errors": errs, "elapsed_s": elapsed,
  "rps": oks/elapsed if elapsed else 0,
  "bytes": nbytes, "throughput_Bps": nbytes/elapsed if elapsed else 0,
  "p50_ms": pct(50), "p95_ms": pct(95), "p99_ms": pct(99),
}
print(json.dumps(out))
PY
}

measure_get(){
  local tag=$1 rep=$2 url=$3
  local pref="$EV/performance/raw/${tag}-rep${rep}"
  log "measure GET $tag rep=$rep"
  docker exec "$(runner)" rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  local j0 rss0 c0="" host_pids=()
  case "$tag" in
    p4b_small_phase4)
      DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
      host_pids=("$CTRL_PID" "$DP_PID");;
    nginx) mapfile -t host_pids < <(docker top "$(ctr nginx)" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$(ctr nginx)");;
    ols) mapfile -t host_pids < <(docker top "$(ctr ols)" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$(ctr ols)");;
    haproxy) mapfile -t host_pids < <(docker top "$HAP" -eo pid | awk 'NR>1{print}'); c0=$(host_cgroup_usage "$HAP");;
  esac
  j0=$(sum_jiffies "${host_pids[@]}"); rss0=$(sum_rss "${host_pids[@]}")
  docker exec "$(runner)" timeout $((MEASURE+25)) rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"${pref}.json" || true
  local j1 rss1; j1=$(sum_jiffies "${host_pids[@]}"); rss1=$(sum_rss "${host_pids[@]}")
  python3 - <<PY
import json
j=json.load(open("${pref}.json"))
tot=float(j.get("requests_total") or 0)
rps=float(j.get("requests_avg") or 0)
dj=max(0,${j1}-${j0}); hz=$HZ
cpu=(dj*(1e6/hz)/tot) if tot else None
open("${pref}.summary.txt","w").write(f"rps={rps}\\ntotal={tot}\\ncpu_us_per_req={cpu}\\nrss_kb=${rss1}\\n")
print(f"${tag} rep=${rep} rps={rps} cpu={cpu} rss=${rss1}")
PY
  sleep 1
}

measure_post(){
  local tag=$1 rep=$2 url=$3 body=$4
  local pref="$EV/performance/raw/${tag}-rep${rep}"
  log "measure POST $tag body=$body rep=$rep"
  post_loadgen "$url" "$body" "$CONC" "$WARMUP" /dev/null || true
  local j0 rss0 host_pids=()
  DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
  case "$tag" in
    p4b_*_phase4)
      host_pids=("$CTRL_PID" "$DP_PID");;
    *_nginx) mapfile -t host_pids < <(docker top "$(ctr nginx)" -eo pid | awk 'NR>1{print}');;
    *_ols) mapfile -t host_pids < <(docker top "$(ctr ols)" -eo pid | awk 'NR>1{print}');;
    *_haproxy) mapfile -t host_pids < <(docker top "$HAP" -eo pid | awk 'NR>1{print}');;
  esac
  j0=$(sum_jiffies "${host_pids[@]}"); rss0=$(sum_rss "${host_pids[@]}")
  post_loadgen "$url" "$body" "$CONC" "$MEASURE" "${pref}.json"
  local j1 rss1; j1=$(sum_jiffies "${host_pids[@]}"); rss1=$(sum_rss "${host_pids[@]}")
  python3 - <<PY
import json
j=json.load(open("${pref}.json"))
rps=float(j.get("rps") or 0); tot=float(j.get("requests") or 0)
tps=float(j.get("throughput_Bps") or 0)
dj=max(0,${j1}-${j0}); hz=$HZ
cpu=(dj*(1e6/hz)/tot) if tot else None
mib=max(tot*(${body})/(1024*1024), 1e-9)
cpu_mib=(dj*(1e6/hz)/mib) if tot else None
open("${pref}.summary.txt","w").write(
  f"rps={rps}\\ntotal={tot}\\nthroughput_Bps={tps}\\ncpu_us_per_req={cpu}\\ncpu_us_per_mib={cpu_mib}\\nrss_kb=${rss1}\\np50={j.get('p50_ms')}\\np95={j.get('p95_ms')}\\np99={j.get('p99_ms')}\\n")
print(f"${tag} rep=${rep} rps={rps} thr={tps} cpu={cpu} rss=${rss1} p95={j.get('p95_ms')}")
PY
  sleep 1
}

start_dp
URL_P4="http://${HELPER}:${PORT}/api/"

# Correctness: POST body accepted (upstream may ignore; expect 200)
BODY=$(python3 - <<'PY'
print("X"*1024, end="")
PY
)
docker exec "$(runner)" sh -c "printf '%s' '$BODY' | curl -sS -o /tmp/b.p4 -w '%{http_code}' --max-time 5 -X POST --data-binary @- -H 'Content-Length: 1024' '$URL_P4'" | tee "$EV/correctness/post_status.txt"
echo | tee -a "$EV/correctness/post_status.txt"
# Smuggling reject
ST=$(docker exec "$(runner)" curl -sS -o /dev/null -w '%{http_code}' --max-time 5 -H 'Content-Length: 5' -H 'Transfer-Encoding: chunked' "$URL_P4" || true)
echo "CL_TE_STATUS=$ST" | tee "$EV/security/cl_te.txt"

# P4B-SMALL GET (response streaming) Phase4 + rivals
for rep in $(seq 1 $REPS); do
  measure_get p4b_small_phase4 $rep "$URL_P4"
  measure_get nginx $rep "http://nginx-stable:8080/api/"
  measure_get ols $rep "http://openlitespeed-latest:8088/api/"
  measure_get haproxy $rep "http://${HAP}:8080/api/"
done

# Body profiles Phase4 + OLS (best rival proxy class)
for rep in $(seq 1 $REPS); do
  measure_post p4b_medium_phase4 $rep "$URL_P4" 65536
  measure_post p4b_medium_ols $rep "http://openlitespeed-latest:8088/api/" 65536
done
for rep in $(seq 1 $REPS); do
  measure_post p4b_large_phase4 $rep "$URL_P4" 1048576
  measure_post p4b_large_ols $rep "http://openlitespeed-latest:8088/api/" 1048576
done

# Slow-client RSS: large response drain slowly (GET 1KiB repeated is weak; stream many)
DP_PID=$(pgrep -P "$CTRL_PID" -f exyonq-dataplane | head -1)
RSS0=$(sum_rss "$CTRL_PID" "$DP_PID")
python3 - <<PY | tee "$EV/slow-client/rss.txt"
import os, time, socket, urllib.parse
url="$URL_P4"
u=urllib.parse.urlparse(url)
host,port=u.hostname,u.port
# open many slow consumers
socks=[]
for i in range(50):
  s=socket.create_connection((host,port),timeout=5)
  s.setblocking(False)
  s.send(b"GET /api/ HTTP/1.1\\r\\nHost: x\\r\\nConnection: keep-alive\\r\\n\\r\\n")
  socks.append(s)
time.sleep(5)
print(f"RSS_START_KB={os.environ.get('RSS0','$RSS0')}")
PY
RSS1=$(sum_rss "$CTRL_PID" "$DP_PID")
# drain slowly
python3 - <<PY
import socket, time, urllib.parse, select
url="$URL_P4"; u=urllib.parse.urlparse(url)
host,port=u.hostname,u.port
s=socket.create_connection((host,port),timeout=5)
s.sendall(b"GET /api/ HTTP/1.1\\r\\nHost: x\\r\\nConnection: close\\r\\n\\r\\n")
# read 1 byte at a time with delay — small response so only proves no hang
while True:
  r,_,_=select.select([s],[],[],1)
  if not r: break
  b=s.recv(1)
  if not b: break
  time.sleep(0.002)
s.close()
print("SLOW_CLIENT_DRAIN=DONE")
PY
RSS2=$(sum_rss "$CTRL_PID" "$DP_PID")
{
  echo "RSS_START=$RSS0"
  echo "RSS_PEAK=$RSS1"
  echo "RSS_END=$RSS2"
  echo "BUFFER_BOUND_EXPECTED=32768_per_conn"
  echo "NOTE=1KiB upstream response; bound proof primarily Darwin phase4_bodies slow_client"
} | tee "$EV/slow-client/summary.txt"

EV="$EV" python3 - <<'PY' | tee "$EV/performance/frontier.md"
import json,statistics,pathlib,re,os
ev=pathlib.Path(os.environ["EV"])

def load(tag):
  rps=[]; cpu=[]; rss=[]; p95=[]; thr=[]; cpu_mib=[]
  for rep in range(1,8):
    p=ev/"performance/raw"/f"{tag}-rep{rep}.summary.txt"
    if not p.exists(): continue
    t=p.read_text()
    def g(k):
      m=re.search(rf"{k}=([0-9.eE+-]+|None)",t)
      return None if not m or m.group(1)=="None" else float(m.group(1))
    rps.append(g("rps")); cpu.append(g("cpu_us_per_req")); rss.append(g("rss_kb"))
    p95.append(g("p95")); thr.append(g("throughput_Bps")); cpu_mib.append(g("cpu_us_per_mib"))
  def med(xs):
    xs=[x for x in xs if x is not None]
    return statistics.median(xs) if xs else None
  return {"rps":med(rps),"cpu":med(cpu),"rss":med(rss),"p95":med(p95),"thr":med(thr),"cpu_mib":med(cpu_mib),
          "rps_all":rps,"cpu_all":cpu,"rss_all":rss}

out={}
for tag in ["p4b_small_phase4","nginx","ols","haproxy","p4b_medium_phase4","p4b_medium_ols","p4b_large_phase4","p4b_large_ols"]:
  out[tag]=load(tag)
def adv(a,b):
  if a is None or b is None or b==0: return None
  return (a-b)/b*100
s=out["p4b_small_phase4"]; o=out["ols"]
out["P4B_SMALL_RPS_ADVANTAGE_VS_OLS"]=adv(s["rps"], o["rps"])
out["P4B_SMALL_CPU_ADVANTAGE_VS_OLS"]=None if not s["cpu"] or not o["cpu"] else (o["cpu"]-s["cpu"])/o["cpu"]*100
(ev/"summary.json").write_text(json.dumps(out, indent=2)+"\n")
print(json.dumps(out, indent=2))
PY

echo "PHASE4_BINARY_SHA256=$DP_SHA" | tee -a "$EV/binary/hashes.txt"
kill "$CTRL_PID" 2>/dev/null || true
pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PORT}" 2>/dev/null || true
docker rm -f "$HELPER" "$HAP" >/dev/null 2>&1 || true
echo CFB4_DONE | tee "$EV/meta/done.txt"
