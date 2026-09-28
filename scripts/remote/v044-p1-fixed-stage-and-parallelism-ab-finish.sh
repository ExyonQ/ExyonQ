#!/usr/bin/env bash
# Finish remaining phases of V044_P1_FIXED_STAGE_AND_PARALLELISM_A_B
# after arms completed; KA failed inside bench-runner (no python3).
set -euo pipefail
EV="${1:-/root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1-fixed-stage-parallelism-ab-20260822T024222Z}"
WS=/root/exyonq-v044-p1-seal-06742e1b
PROJECT=v044p1auth-clean
FULL_COMPOSE=$WS/benchmarks/docker/docker-compose.bench.yml
OVER=$WS/benchmarks/docker/docker-compose.p1-authoritative.yml
compose(){ docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" --profile bench "$@"; }
log(){ echo "[p1-ab-finish] $(date -u +%H:%M:%S) $*"|tee -a "$EV/orchestrator.log"; }

mkdir -p "$EV"/{ka,stages,p3,reports,restore,meta}

cat > "$EV/meta/compose-A0_finish.yml" <<EOF
services:
  exyonq:
    image: v044p1auth-exyonq
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_CONFIG: /bench/bench.toml
EOF
compose -f "$EV/meta/compose-A0_finish.yml" up -d --force-recreate --no-deps exyonq
sleep 8
compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
IP=$(docker inspect ${PROJECT}-exyonq-1 --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
echo "EXYONQ_IP=$IP" | tee "$EV/ka/exyonq_ip.txt"

log "A/B-B first vs keepalive host→$IP"
python3 - "$IP" "$EV/ka/close-vs-ka.txt" <<'PY'
import socket, time, statistics, sys
host = sys.argv[1]
out_path = sys.argv[2]
port = 8080
path = "/site/1k.bin"
req_close = (f"GET {path} HTTP/1.1\r\nHost: exyonq\r\nConnection: close\r\n\r\n").encode()
req_ka = (f"GET {path} HTTP/1.1\r\nHost: exyonq\r\nConnection: keep-alive\r\n\r\n").encode()

def read_response(s, expect_cl=1024):
    buf=b""
    while True:
        chunk=s.recv(65536)
        if not chunk: break
        buf+=chunk
        if b"\r\n\r\n" not in buf: continue
        cl=expect_cl
        for ln in buf.split(b"\r\n"):
            if ln.lower().startswith(b"content-length:"):
                cl=int(ln.split(b":",1)[1].strip())
        hdr=buf.find(b"\r\n\r\n")+4
        if len(buf)>=hdr+cl:
            return buf
    return buf

def one_shot(n=500):
    lat=[]
    for _ in range(n):
        t0=time.perf_counter()
        s=socket.create_connection((host,port), timeout=5)
        s.sendall(req_close)
        read_response(s)
        s.close()
        lat.append((time.perf_counter()-t0)*1e6)
    return lat

def keepalive_session(conns=25, reqs_per=50):
    first=[]; subsequent=[]
    for _ in range(conns):
        s=socket.create_connection((host,port), timeout=5)
        for i in range(reqs_per):
            t0=time.perf_counter()
            s.sendall(req_ka)
            read_response(s)
            dt=(time.perf_counter()-t0)*1e6
            (first if i==0 else subsequent).append(dt)
        s.close()
    return first, subsequent

def pct(xs,p):
    xs=sorted(xs)
    if not xs: return None
    return xs[int(round((p/100)*(len(xs)-1)))]

close_lat=one_shot(500)
first,sub=keepalive_session(25,50)
lines=[
f"CLOSE_N={len(close_lat)}",
f"CLOSE_P50_US={pct(close_lat,50)}",
f"CLOSE_P95_US={pct(close_lat,95)}",
f"CLOSE_P99_US={pct(close_lat,99)}",
f"CLOSE_MEAN_US={statistics.mean(close_lat)}",
f"KA_FIRST_N={len(first)}",
f"KA_FIRST_P50_US={pct(first,50)}",
f"KA_FIRST_P95_US={pct(first,95)}",
f"KA_FIRST_MEAN_US={statistics.mean(first)}",
f"KA_SUB_N={len(sub)}",
f"KA_SUB_P50_US={pct(sub,50)}",
f"KA_SUB_P95_US={pct(sub,95)}",
f"KA_SUB_MEAN_US={statistics.mean(sub)}",
f"HANDOFF_PROXY_DELTA_MEAN_US={statistics.mean(first)-statistics.mean(sub)}",
f"HANDOFF_PROXY_DELTA_P50_US={pct(first,50)-pct(sub,50)}",
f"CLOSE_VS_KA_SUB_P50_US={pct(close_lat,50)-pct(sub,50)}",
]
open(out_path,"w").write("\n".join(lines)+"\n")
print("\n".join(lines))
PY

log "stages attribution"
mkdir -p "$EV/stages"
cp -a "$EV/arms/A0/sample/." "$EV/stages/" || true
python3 - "$EV/stages" <<'PY'
from pathlib import Path
import re, sys
ev=Path(sys.argv[1])
report=(ev/"perf-report.txt").read_text(errors="replace") if (ev/"perf-report.txt").exists() else ""
stat=(ev/"perf-stat.txt").read_text(errors="replace") if (ev/"perf-stat.txt").exists() else ""
hits=(ev/"perf-hits.txt").read_text(errors="replace") if (ev/"perf-hits.txt").exists() else ""

def pct_for(pat):
    tot=0.0
    for ln in report.splitlines():
        if re.search(pat, ln, re.I):
            m=re.match(r"\s*([\d.]+)%", ln)
            if m: tot += float(m.group(1))
    return tot
keys={
  "FUTEX": r"futex|__futex",
  "RWLOCK": r"rwlock|RwLock|pthread_rwlock",
  "MUTEX": r"mutex|Mutex|parking_lot",
  "SENDFILE_REGISTRY": r"SendfileHandle|sendfile_fd_cache|HandleRegistry",
  "EPOLL_WAIT": r"epoll_wait",
  "SENDFILE": r"\bsendfile\b",
  "TOKIO": r"tokio::",
  "WAF": r"waf|NopWaf|evaluate_wire",
}
out=[]
out.append("PERF_SYMBOL_PERCENT_SUMS (self, may double-count)")
for k,pat in keys.items():
    out.append(f"{k}_PCT={pct_for(pat):.4f}")
out.append("--- perf-hits head ---")
out.extend(hits.splitlines()[:40])
out.append("--- perf-stat ---")
out.append(stat)
rwlock=pct_for(r"rwlock|RwLock")
reg=pct_for(r"SendfileHandle|HandleRegistry|sendfile_fd_cache")
out.append("ROOTS_RWLOCK_CONTENTION=" + ("NONE_OBSERVED" if rwlock < 0.5 else "POSSIBLE"))
out.append("HANDLE_REGISTRY_ROOT=" + ("REJECTED_NOT_MATERIAL" if reg < 1.0 else "VISIBLE_IN_PERF"))
(ev/"attribution.txt").write_text("\n".join(out)+"\n")
print("\n".join(out[:30]))
PY

: > "$EV/reports/thread_geometry.txt"
for arm in A0 A1 A_CAP067; do
  {
    echo "=== $arm threads (top) ==="
    head -40 "$EV/arms/$arm/sample/threads.txt" 2>/dev/null || true
    echo "=== $arm per_cpu ==="
    cat "$EV/arms/$arm/sample/per_cpu.txt" 2>/dev/null || true
  } >> "$EV/reports/thread_geometry.txt"
done

p3_arm(){
  local label=$1 wt=$2 aw=$3 pool=$4
  log "P3 sanity $label"
  mkdir -p "$EV/p3/$label"
  cat > "$EV/meta/compose-p3-$label.yml" <<EOF
services:
  exyonq:
    image: v044p1auth-exyonq
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "${wt}"
      EXYONQ_ACCEPT_WORKERS: "${aw}"
      EXYONQ_EPOLL_POOL_THREADS: "${pool}"
      EXYONQ_CONFIG: /bench/bench.toml
EOF
  compose -f "$EV/meta/compose-p3-$label.yml" up -d --force-recreate --no-deps exyonq
  sleep 8
  local url="http://exyonq:8080/site/1m.bin"
  local out c sp
  out=$(compose exec -T bench-runner curl -sS -m 15 -o /tmp/p3.bin -w "%{http_code} %{size_download}" "$url")
  echo "probe=$out" | tee "$EV/p3/$label/probe.txt"
  c=${PROJECT}-exyonq-1
  compose exec -T bench-runner rewrk -c 100 -d 10s -t 2 -h "$url" >/dev/null 2>&1 || true
  timeout 50 docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$EV/p3/$label/cpu.txt" 2>/dev/null &
  sp=$!
  compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" --json >"$EV/p3/$label/rewrk.json"
  wait "$sp" 2>/dev/null || true
  compose exec -T bench-runner bash -lc "rewrk -c 100 -d 20s -t 2 -h $url --pct 2>&1" | tee "$EV/p3/$label/pct.txt" || true
  python3 - "$EV/p3/$label" <<'PY'
import json,re,statistics,sys
from pathlib import Path
d=Path(sys.argv[1])
j=json.loads((d/"rewrk.json").read_text())
rps=j.get("requests_avg")
ansi=re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
cpus=[]
for ln in (d/"cpu.txt").read_text(errors="replace").splitlines():
    ln=ansi.sub("",ln).strip()
    m=re.search(r"([\d.]+)%", ln)
    if m: cpus.append(float(m.group(1)))
cpu=statistics.median(cpus) if cpus else None
core_ms=(cpu/100*8*1000/rps) if cpu and rps else None
ptxt=(d/"pct.txt").read_text(errors="replace") if (d/"pct.txt").exists() else ""
lines=[f"P3_RPS={rps}", f"P3_CPU_PERCENT={cpu}", f"P3_CORE_MS_PER_REQ={core_ms}"]
for p in ("50","95","99"):
    m=re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", ptxt)
    if m: lines.append(f"P3_P{p}={m.group(1)}")
(d/"summary.txt").write_text("\n".join(lines)+"\n")
print("\n".join(lines))
PY
}
p3_arm A0 4 4 4
p3_arm A1 8 8 8

compose -f "$EV/meta/compose-A0_finish.yml" up -d --force-recreate --no-deps exyonq
sleep 5
echo "restored $(date -u +%Y%m%dT%H%M%SZ)" | tee "$EV/restore/done.txt"

python3 - "$EV" <<'PY'
import json, sys
from pathlib import Path
ev=Path(sys.argv[1])
arms=["A0","A1","A2","A_TOKIO","A_CAP067"]
rows={}
for a in arms:
    p=ev/"arms"/a/"summary.json"
    if p.exists(): rows[a]=json.loads(p.read_text())

def delta(a,b,key):
    if a not in rows or b not in rows: return None
    va,vb=rows[a].get(key),rows[b].get(key)
    if va is None or vb is None or va==0: return None
    return (vb/va-1)*100

out=[]
out.append("=== ARM TABLE ===")
for a,r in rows.items():
    out.append(f"{a}: RPS={r.get('rps_median')} CV={r.get('rps_cv')} CPU%={r.get('cpu_percent_median')} core_ms={r.get('cpu_core_ms_per_req')} P50={r.get('p50')} P95={r.get('p95')} P99={r.get('p99')} RSS={r.get('rss_mib_median')}")
base="A0"
out.append("\n=== DELTAS vs A0 ===")
for a in arms:
    if a==base: continue
    out.append(f"{a}_RPS_DELTA_PCT={delta(base,a,'rps_median')}")
    out.append(f"{a}_CPU_CORE_MS_DELTA_PCT={delta(base,a,'cpu_core_ms_per_req')}")
    out.append(f"{a}_P50_DELTA_PCT={delta(base,a,'p50')}")
    out.append(f"{a}_P95_DELTA_PCT={delta(base,a,'p95')}")
    out.append(f"{a}_P99_DELTA_PCT={delta(base,a,'p99')}")
a1=delta(base,"A1","rps_median"); cap=delta(base,"A_CAP067","rps_median"); tok=delta(base,"A_TOKIO","rps_median")
out.append("\n=== PARALLELISM CLASSIFICATION ===")
out.append(f"A1_FULL8_RPS_DELTA_PCT={a1}")
out.append(f"A_CAP067_RPS_DELTA_PCT={cap}")
out.append(f"A_TOKIO_RPS_DELTA_PCT={tok}")
def material(d, thr=3.0):
    return d is not None and abs(d)>=thr
if material(a1) and a1>0:
    out.append("IS_WORKER_COUNT_LIMITING_RPS=YES")
    out.append("DO_MORE_WORKERS_IMPROVE_RPS=YES")
elif material(cap) and cap>0 and (tok is None or tok<3):
    out.append("IS_WORKER_COUNT_LIMITING_RPS=YES")
    out.append("DO_MORE_WORKERS_IMPROVE_RPS=YES")
elif a1 is not None and abs(a1)<3:
    out.append("IS_WORKER_COUNT_LIMITING_RPS=NOT_PROVEN")
    out.append("DO_MORE_WORKERS_IMPROVE_RPS=NO")
else:
    out.append("IS_WORKER_COUNT_LIMITING_RPS=NOT_PROVEN")
    out.append("DO_MORE_WORKERS_IMPROVE_RPS=UNKNOWN")
for lab in ["A0","A1"]:
    sp=ev/"p3"/lab/"summary.txt"
    if sp.exists():
        out.append(f"\n=== P3 {lab} ===")
        out.append(sp.read_text())
ka=ev/"ka"/"close-vs-ka.txt"
if ka.exists():
    out.append("\n=== KA ===")
    out.append(ka.read_text())
attr=ev/"stages"/"attribution.txt"
if attr.exists():
    out.append("\n=== STAGES ===")
    out.append(attr.read_text()[:4000])
(ev/"reports"/"ab_terminal.txt").write_text("\n".join(out)+"\n")
print("\n".join(out))
PY

log "FINISH DONE EV=$EV"
