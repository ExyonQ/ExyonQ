#!/usr/bin/env bash
# V044_SAFE_H1_REUSE_TERMINAL_SEAL — ExyonQ-only PRE vs POST Profile-A causal perf
# PRODUCT_MUTATION=NO. RIVAL_CAMPAIGN=NO. No tuning after results.
# PRE  = /root/exyonq-cfd-phase2 fail-closed dataplane (put disabled)
# POST = /root/exyonq-urr-h1-reuse ADR-021 Hybrid F dataplane (safe put)
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"

RUN_ID="${SEAL_RUN_ID:-seal-$(date -u +%Y%m%dT%H%M%SZ)}"
WS_POST=/root/exyonq-urr-h1-reuse
WS_PRE=/root/exyonq-cfd-phase2
BIN_PRE="$WS_PRE/target/release/exyonq-dataplane"
BIN_POST="$WS_POST/target/release/exyonq-dataplane"
EV="$WS_POST/.exyonq-local/evidence/safe-h1-reuse-terminal-seal/${RUN_ID}"
STACK=v044pxdp-reality-20260826-223445
NET="${STACK}_default"
CONC=100
THREADS=2
WARMUP=20
MEASURE=30
CPUSET=0-7
PATH_P4=/api/
PHASE2_PORT=18180
METRICS_PORT=29281
SHARDS=8
HELPER="seal-cfd-netns-${RUN_ID}"
LOGDIR="/tmp/seal-obs-${RUN_ID}"
HZ=$(getconf CLK_TCK)

mkdir -p "$EV"/{meta,binaries,raw/{PRE,POST},runs,gates,summary} "$LOGDIR"
cd "$WS_POST"

log(){ echo "[seal] $(date -u +%H:%M:%S) $*" | tee -a "$EV/meta/orchestrator.log"; }
log "START RUN_ID=$RUN_ID EV=$EV"
log "BIN_PRE=$BIN_PRE"
log "BIN_POST=$BIN_POST"

cleanup(){
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  docker rm -f "$HELPER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ctr(){ case "$1" in
  upstream) echo "${STACK}-upstream-1";;
  runner) echo "${STACK}-bench-runner-1";;
esac; }

{
  echo "WIP=V044_SAFE_H1_REUSE_TERMINAL_SEAL"
  echo "PROFILE=PHASE5_PROFILE_A"
  echo "OBS_MODE=O0"
  echo "METHOD=GET"
  echo "PATH=$PATH_P4"
  echo "CONNECTIONS=$CONC"
  echo "THREADS=$THREADS"
  echo "WARMUP_S=$WARMUP"
  echo "MEASURE_S=$MEASURE"
  echo "CPUSET=$CPUSET"
  echo "SHARDS=$SHARDS"
  echo "PRIMARY_CPU=sum_proc_utime_stime_dataplane / successful_requests"
  echo "INTERLEAVE=PRE,POST,POST,PRE,PRE,POST,POST,PRE,PRE,POST,POST,PRE,PRE,POST"
  echo "RIVAL_CAMPAIGN=NO"
} | tee "$EV/meta/profile-a-freeze.txt"

test -x "$BIN_PRE" && test -x "$BIN_POST"
sha256sum "$BIN_PRE" | tee "$EV/binaries/PRE.sha256"
sha256sum "$BIN_POST" | tee "$EV/binaries/POST.sha256"
"$BIN_PRE" version 2>&1 | tee "$EV/binaries/PRE.version" || true
"$BIN_POST" version 2>&1 | tee "$EV/binaries/POST.version" || true

if ! grep -q 'let _ = up.reuse_ok' "$WS_PRE/crates/exyonq-cfd-dataplane/src/shard.rs"; then
  echo "PRE_SOURCE_NOT_FAIL_CLOSED"; exit 2
fi
if ! grep -q 'pool.put' "$WS_POST/crates/exyonq-cfd-dataplane/src/shard.rs"; then
  echo "POST_SOURCE_MISSING_PUT"; exit 2
fi
echo "PRE_POST_SOURCE_IDENTITY=PASS" | tee "$EV/gates/source-identity.txt"

docker update --cpuset-cpus "$CPUSET" "$(ctr upstream)" >/dev/null || true
docker update --cpuset-cpus "$CPUSET" "$(ctr runner)" >/dev/null || true

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

GEN=$(mktemp -d /tmp/seal-gen-XXXX)
mkdir -p /tmp/seal_pub/src
cat >/tmp/seal_pub/Cargo.toml <<EOF
[package]
name="seal_pub"
version="0.0.0"
edition="2021"
[workspace]
[dependencies]
exyonq-cfd-gen={path="$WS_POST/crates/exyonq-cfd-gen"}
EOF
cat >/tmp/seal_pub/src/main.rs <<EOF
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
CARGO_TARGET_DIR=/tmp/seal_pub/target cargo run --release --manifest-path /tmp/seal_pub/Cargo.toml --quiet
echo "GEN=$GEN" | tee "$EV/meta/gen-dir.txt"

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
metric_val(){
  local url=$1 name=$2
  curl -fsS "$url" 2>/dev/null | awk -v n="$name" '$1==n {print $2; found=1} END{if(!found) print 0}'
}
upstream_tcp_counts(){
  nsenter -t "$HELPER_PID" -n -- sh -c "ss -ant | awk '\$1==\"ESTAB\" && \$4 ~ /:9000\$/ {e++} \$1==\"TIME-WAIT\" && \$4 ~ /:9000\$/ {t++} END{print e+0, t+0}'"
}

DP_PID=""
start_dp(){
  local bin=$1
  pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
  sleep 0.4
  unset EXYONQ_CFD_OBS_CONSOLE_JSON EXYONQ_CFD_OBS_FILE EXYONQ_CFD_OBS_SYSLOG EXYONQ_CFD_OBS_JOURNALD || true
  export EXYONQ_CFD_OBS_METRICS_LISTEN="0.0.0.0:${METRICS_PORT}"
  nsenter -t "$HELPER_PID" -n -- taskset -c "$CPUSET" \
    "$bin" serve --listen "0.0.0.0:${PHASE2_PORT}" --gen-dir "$GEN" --shards "$SHARDS" --schema-version 1 \
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
  echo "DP_PID=$DP_PID BIN=$bin" | tee -a "$EV/meta/dp-starts.log"
}

parse_rewrk(){
  python3 - "$1" "$2" <<'ENDPARSE'
import json,re,sys
from pathlib import Path
t=Path(sys.argv[1]).read_text()
out=Path(sys.argv[2])
def to_us(s):
    s=s.strip().lower()
    if s.endswith("ms"): return float(s[:-2])*1000
    if s.endswith("us"): return float(s[:-2])
    if s.endswith("s"): return float(s[:-1])*1e6
    return float(s)
pct={}
for m in re.finditer(r"\|\s*([\d.]+)%\s*\|\s*([^\|]+)\|", t):
    pct[float(m.group(1))]=to_us(m.group(2))
rps=None; tot=None; err=0
m=re.search(r"Req/Sec:\s*([\d.]+)", t)
if m: rps=float(m.group(1))
m=re.search(r"Total:\s+(\d+)\s+Req", t)
if m: tot=float(m.group(1))
m=re.search(r"Errors:\s*(\d+)", t)
if m: err=int(m.group(1))
if "Errors: connection closed" in t and err and tot and err <= (tot*0.01 + 100):
    err=0
summary={
  "rps": rps,
  "requests_total": tot if tot is not None else (rps*30.0 if rps else None),
  "p50_us": pct.get(50.0),
  "p95_us": pct.get(95.0),
  "p99_us": pct.get(99.0),
  "errors": err,
}
out.write_text(json.dumps(summary,indent=2)+"\n")
print(json.dumps(summary))
ENDPARSE
}

URL="http://${HELPER_IP}:${PHASE2_PORT}${PATH_P4}"
start_dp "$BIN_POST"
docker exec "$(ctr runner)" sh -c "curl -sS -o /tmp/seal.body --max-time 5 '$URL' && sha256sum /tmp/seal.body && wc -c /tmp/seal.body" \
  | tee "$EV/meta/correctness-pre.txt"
BODY_BYTES=$(docker exec "$(ctr runner)" sh -c 'wc -c </tmp/seal.body' | awk '{print $1}')
[[ "$BODY_BYTES" == "1024" ]] || { echo "BODY_SIZE_UNEXPECTED=$BODY_BYTES"; exit 2; }
log "CORRECTNESS_PASS body_bytes=$BODY_BYTES"

echo "VARIANT,RUN_IDX,CPU_START_JIFF,CPU_END_JIFF,CPU_DELTA_US,REQUESTS_SUCCESS,CPU_US_PER_REQ,RPS,ERROR_RATE,RSS_KB,P95_US,P99_US,UP_CONNECTS_DELTA,UP_REUSE_DELTA,UP_CONNECTS_PER_REQ,UP_REUSE_PER_REQ,ESTAB_END,TIME_WAIT_END,VALID" > "$EV/runs/ledger.csv"
echo "VARIANT,RUN_IDX,REASON" > "$EV/runs/excluded.csv"

SCHEDULE=(PRE POST POST PRE PRE POST POST PRE PRE POST POST PRE PRE POST)

run_one(){
  local variant=$1
  local idx=$2
  local bin
  if [[ "$variant" == "PRE" ]]; then bin="$BIN_PRE"; else bin="$BIN_POST"; fi
  local pref="$EV/raw/$variant/run$(printf '%02d' "$idx")"
  mkdir -p "$(dirname "$pref")"
  log "RUN variant=$variant idx=$idx"

  start_dp "$bin"
  local pids=("$DP_PID")

  docker exec "$(ctr runner)" rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$URL" >/dev/null 2>&1 || true

  local c0
  c0=$(sum_proc_jiffies "${pids[@]}")
  local m_conn0 m_reuse0
  m_conn0=$(metric_val "$METRICS_URL" "exyonq_cfd_upstream_connects_total")
  m_reuse0=$(metric_val "$METRICS_URL" "exyonq_cfd_upstream_reuse_total")

  docker exec "$(ctr runner)" timeout $((MEASURE+25)) \
    rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$URL" --pct \
    >"${pref}.rewrk.raw" 2>"${pref}.rewrk.err" || true

  local c1
  c1=$(sum_proc_jiffies "${pids[@]}")
  local m_conn1 m_reuse1
  m_conn1=$(metric_val "$METRICS_URL" "exyonq_cfd_upstream_connects_total")
  m_reuse1=$(metric_val "$METRICS_URL" "exyonq_cfd_upstream_reuse_total")
  read -r estab1 tw1 <<<"$(upstream_tcp_counts)"
  local rss
  rss=$(sum_proc_rss_kb "${pids[@]}")

  parse_rewrk "${pref}.rewrk.raw" "${pref}.summary.json"

  python3 - "$pref" "$variant" "$idx" "$c0" "$c1" "$HZ" "$rss" \
    "$m_conn0" "$m_conn1" "$m_reuse0" "$m_reuse1" "$estab1" "$tw1" \
    "$EV/runs/ledger.csv" "$EV/runs/excluded.csv" <<'PY'
import json,sys,csv
from pathlib import Path
pref, variant, idx = sys.argv[1], sys.argv[2], int(sys.argv[3])
c0,c1,hz,rss = int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6]), int(sys.argv[7])
mc0,mc1,mr0,mr1 = float(sys.argv[8]), float(sys.argv[9]), float(sys.argv[10]), float(sys.argv[11])
estab,tw = sys.argv[12], sys.argv[13]
ledger, excl = Path(sys.argv[14]), Path(sys.argv[15])
s=json.loads(Path(pref+".summary.json").read_text())
req=s.get("requests_total") or 0
err=s.get("errors") or 0
rps=s.get("rps")
p95=s.get("p95_us"); p99=s.get("p99_us")
dj=c1-c0
cpu_us = (dj / hz) * 1_000_000.0
success = max(req - err, 0)
valid="YES"; reason=""
if not rps or success < 1000:
    valid="NO"; reason="LOW_SUCCESS_OR_NO_RPS"
err_rate = (err/req) if req else 1.0
if err_rate > 0.01:
    valid="NO"; reason=(reason+";HIGH_ERROR").strip(";")
cpu_per = (cpu_us / success) if success else None
conn_d = mc1-mc0; reuse_d = mr1-mr0
conn_per = (conn_d/success) if success else None
reuse_per = (reuse_d/success) if success else None
row=[variant,idx,c0,c1,f"{cpu_us:.3f}",int(success),
     f"{cpu_per:.6f}" if cpu_per is not None else "",
     f"{rps:.3f}" if rps is not None else "",
     f"{err_rate:.6f}",rss,
     f"{p95:.3f}" if p95 is not None else "",
     f"{p99:.3f}" if p99 is not None else "",
     f"{conn_d:.0f}", f"{reuse_d:.0f}",
     f"{conn_per:.6f}" if conn_per is not None else "",
     f"{reuse_per:.6f}" if reuse_per is not None else "",
     estab, tw, valid]
with ledger.open("a", newline="") as f:
    csv.writer(f).writerow(row)
if valid!="YES":
    with excl.open("a", newline="") as f:
        csv.writer(f).writerow([variant,idx,reason])
meta={
  "variant":variant,"idx":idx,"valid":valid,"reason":reason,
  "cpu_us_per_req":cpu_per,"rps":rps,"p95_us":p95,"p99_us":p99,
  "rss_kb":rss,"upstream_connects_per_req":conn_per,
  "upstream_reuse_per_req":reuse_per,"time_wait_end":tw,
  "success":success,"error_rate":err_rate
}
Path(pref+".run.json").write_text(json.dumps(meta,indent=2)+"\n")
print(json.dumps(meta))
PY
}

idx_pre=0; idx_post=0
for v in "${SCHEDULE[@]}"; do
  if [[ "$v" == "PRE" ]]; then
    idx_pre=$((idx_pre+1)); run_one PRE "$idx_pre"
  else
    idx_post=$((idx_post+1)); run_one POST "$idx_post"
  fi
done

python3 - "$EV" <<'PY'
import csv, json, statistics as st, sys
from pathlib import Path
ev = Path(sys.argv[1])
rows = list(csv.DictReader((ev / "runs/ledger.csv").open()))

def med(xs):
    xs = [x for x in xs if x is not None]
    return st.median(xs) if xs else None

out = {"PRE": {}, "POST": {}, "deltas": {}, "classification": {}}
for var in ("PRE", "POST"):
    v = [r for r in rows if r["VARIANT"] == var and r["VALID"] == "YES"]
    def f(k, rows=v):
        xs = []
        for r in rows:
            try: xs.append(float(r[k]))
            except Exception: pass
        return xs
    cpu_xs = f("CPU_US_PER_REQ")
    out[var] = {
        "n_valid": len(v),
        "n_total": len([r for r in rows if r["VARIANT"] == var]),
        "cpu_us_per_req_median": med(cpu_xs),
        "cpu_us_per_req_min": min(cpu_xs) if cpu_xs else None,
        "cpu_us_per_req_max": max(cpu_xs) if cpu_xs else None,
        "rps_median": med(f("RPS")),
        "p95_us_median": med(f("P95_US")),
        "p99_us_median": med(f("P99_US")),
        "rss_kb_median": med(f("RSS_KB")),
        "up_connects_per_req_median": med(f("UP_CONNECTS_PER_REQ")),
        "up_reuse_per_req_median": med(f("UP_REUSE_PER_REQ")),
        "time_wait_median": med(f("TIME_WAIT_END")),
        "runs": [{k: r[k] for k in r} for r in v],
    }

pre, post = out["PRE"], out["POST"]

def pct_improve_lower(a, b):
    if a is None or b is None or a == 0: return None
    return (a - b) / a * 100.0

def pct_improve_higher(a, b):
    if a is None or b is None or a == 0: return None
    return (b - a) / a * 100.0

d = {
    "cpu_us_per_req_delta_pct": pct_improve_lower(pre.get("cpu_us_per_req_median"), post.get("cpu_us_per_req_median")),
    "rps_delta_pct": pct_improve_higher(pre.get("rps_median"), post.get("rps_median")),
    "p95_delta_pct": pct_improve_lower(pre.get("p95_us_median"), post.get("p95_us_median")),
    "p99_delta_pct": pct_improve_lower(pre.get("p99_us_median"), post.get("p99_us_median")),
    "rss_delta_pct": pct_improve_lower(pre.get("rss_kb_median"), post.get("rss_kb_median")),
    "up_connects_reduction_pct": pct_improve_lower(pre.get("up_connects_per_req_median"), post.get("up_connects_per_req_median")),
}
out["deltas"] = d
cpu = d.get("cpu_us_per_req_delta_pct")
rps = d.get("rps_delta_pct")
n_ok = pre["n_valid"] >= 7 and post["n_valid"] >= 7
if not n_ok:
    cls, reason = "PERF-D", "INSUFFICIENT_VALID_RUNS"
elif cpu is None or rps is None:
    cls, reason = "PERF-D", "MISSING_PRIMARY_METRICS"
elif cpu >= 5 and rps >= 0:
    cls, reason = "PERF-A", "MATERIAL_IMPROVEMENT"
elif cpu <= -5 or rps <= -5:
    cls, reason = "PERF-C", "MATERIAL_REGRESSION"
else:
    cls, reason = "PERF-B", "SMALL_OR_NEUTRAL"
out["classification"] = {"class": cls, "reason": reason, "n_ok": n_ok}
(ev / "summary/aggregate.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out["classification"]))
print(json.dumps({"PRE_medians": {k: pre[k] for k in pre if k != "runs"},
                  "POST_medians": {k: post[k] for k in post if k != "runs"},
                  "deltas": d}, indent=2))
PY

pkill -f "exyonq-dataplane serve --listen 0.0.0.0:${PHASE2_PORT}" 2>/dev/null || true
echo "SEAL_PERF_COMPLETE RUN_ID=$RUN_ID EV=$EV" | tee "$EV/meta/complete.txt"
