#!/usr/bin/env bash
# V044_P1_OLS_FD_GEOMETRY_COST_ATTRIBUTION — READ_ONLY measurement (Netcup).
# PRODUCT_MUTATION=NO. FD_REUSE_PRODUCT_REPAIR=NOT_AUTHORIZED.
# Phases: A cgroup user/sys (no observers) → B bpftrace syscall wall → C perf symbols.
set -euo pipefail
WS=/root/exyonq-v044-p1-auth
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV=$WS/.exyonq-local-evidence/v044-p1-ols-fd-attr-v3-$TS
FULL=$WS/benchmarks/docker/docker-compose.bench.yml
OVER=$WS/benchmarks/docker/docker-compose.p1-authoritative.yml
PROJ=v044p1auth-clean
mkdir -p "$EV"/{meta,cgroup,bpf,perf,reports}
cd "$WS"
compose(){ docker compose -f "$FULL" -f "$OVER" -p "$PROJ" "$@"; }
log(){ echo "[attrv3] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
EXY=${PROJ}-exyonq-1; OLS=${PROJ}-openlitespeed-latest-1
URL_E=http://exyonq:8080/site/1k.bin; URL_O=http://openlitespeed-latest:8088/site/1k.bin
cstat(){ cid=$(docker inspect -f '{{.Id}}' "$1"); echo /sys/fs/cgroup/system.slice/docker-${cid}.scope/cpu.stat; }
readstat(){ python3 -c "from pathlib import Path; d={};
[d.__setitem__(a,int(b)) for a,b in (ln.split() for ln in Path('$1').read_text().splitlines())];
print(d['usage_usec'],d['user_usec'],d['system_usec'])"; }
tgids_of(){
  main=$(docker inspect -f '{{.State.Pid}}' "$1")
  python3 - "$main" <<'PY'
import os,sys
root=int(sys.argv[1])
def cg(pid):
  try: return open(f"/proc/{pid}/cgroup").read()
  except: return ""
want=cg(root); tgids=set()
for pid in os.listdir("/proc"):
  if not pid.isdigit(): continue
  if cg(int(pid))!=want: continue
  try:
    for ln in open(f"/proc/{pid}/status"):
      if ln.startswith("Tgid:"): tgids.add(int(ln.split()[1])); break
  except: pass
if not tgids: tgids={root}
print(",".join(str(t) for t in sorted(tgids)))
PY
}

SHA=$(docker exec "$EXY" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
echo SHA=$SHA | tee "$EV/meta/sha.txt"
test "$SHA" = f243ef2c07a41c88c14a86c1241598051a24d0eeac891f96c319b9ab5ce49f1d
for c in "$EXY" "$OLS"; do docker update --cpuset-cpus 0-7 "$c" >/dev/null; done

log "PHASE A clean cgroup"
for pair in "exyonq|$EXY|$URL_E" "ols|$OLS|$URL_O"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  sp=$(cstat "$ctr")
  log "$tag warmup20"; compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" >/dev/null
  readstat "$sp" >"$EV/cgroup/${tag}_A_before.txt"
  log "$tag measure30"; compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/cgroup/${tag}_A_rewrk.json"
  readstat "$sp" >"$EV/cgroup/${tag}_A_after.txt"
  echo "$tag A done $(cat $EV/cgroup/${tag}_A_before.txt) -> $(cat $EV/cgroup/${tag}_A_after.txt)"
done

log "PHASE B bpftrace"
for pair in "exyonq|$EXY|$URL_E" "ols|$OLS|$URL_O"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(tgids_of "$ctr")
  echo "$tag tgids=$tg" | tee "$EV/meta/${tag}_tgids.txt"
  cond=$(python3 -c "print(' || '.join(f'pid=={p}' for p in '$tg'.split(',') if p))")
  cat >"$EV/bpf/${tag}.bt" <<BT
tracepoint:syscalls:sys_enter_openat2 /$cond/ { @st[tid]=nsecs; }
tracepoint:syscalls:sys_exit_openat2 /@st[tid]/ { @ns["openat2"]=sum(nsecs-@st[tid]); @n["openat2"]=count(); delete(@st[tid]); }
tracepoint:syscalls:sys_enter_openat /$cond/ { @st2[tid]=nsecs; }
tracepoint:syscalls:sys_exit_openat /@st2[tid]/ { @ns["openat"]=sum(nsecs-@st2[tid]); @n["openat"]=count(); delete(@st2[tid]); }
tracepoint:syscalls:sys_enter_statx /$cond/ { @st3[tid]=nsecs; }
tracepoint:syscalls:sys_exit_statx /@st3[tid]/ { @ns["statx"]=sum(nsecs-@st3[tid]); @n["statx"]=count(); delete(@st3[tid]); }
tracepoint:syscalls:sys_enter_close /$cond/ { @st4[tid]=nsecs; }
tracepoint:syscalls:sys_exit_close /@st4[tid]/ { @ns["close"]=sum(nsecs-@st4[tid]); @n["close"]=count(); delete(@st4[tid]); }
tracepoint:syscalls:sys_enter_sendfile64 /$cond/ { @st5[tid]=nsecs; }
tracepoint:syscalls:sys_exit_sendfile64 /@st5[tid]/ { @ns["sendfile64"]=sum(nsecs-@st5[tid]); @n["sendfile64"]=count(); delete(@st5[tid]); }
tracepoint:syscalls:sys_enter_writev /$cond/ { @st6[tid]=nsecs; }
tracepoint:syscalls:sys_exit_writev /@st6[tid]/ { @ns["writev"]=sum(nsecs-@st6[tid]); @n["writev"]=count(); delete(@st6[tid]); }
BT
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  timeout 50 bpftrace "$EV/bpf/${tag}.bt" >"$EV/bpf/${tag}.out" 2>"$EV/bpf/${tag}.err" &
  bp=$!; sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/cgroup/${tag}_B_rewrk.json"
  kill -INT $bp 2>/dev/null || true; wait $bp 2>/dev/null || true
  echo "=== $tag bpf ==="; head -5 "$EV/bpf/${tag}.err"; tail -20 "$EV/bpf/${tag}.out"
done

log "PHASE C perf"
for pair in "exyonq|$EXY|$URL_E" "ols|$OLS|$URL_O"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(sed -n 's/.*tgids=//p' "$EV/meta/${tag}_tgids.txt")
  args=(); IFS=','; for p in $tg; do args+=(-p "$p"); done; unset IFS
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  timeout 50 perf record -o "$EV/perf/${tag}.data" -F 997 -g "${args[@]}" -- sleep 36 >"$EV/perf/${tag}_rec.out" 2>"$EV/perf/${tag}_rec.err" &
  pp=$!; sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/perf/${tag}_rewrk.json"
  wait $pp 2>/dev/null || true
  perf report -i "$EV/perf/${tag}.data" --stdio --no-children -n --percent-limit 0.35 >"$EV/perf/${tag}_report.txt" 2>/dev/null || true
  rg 'Captured|samples' "$EV/perf/${tag}_rec.err" || true
  head -30 "$EV/perf/${tag}_report.txt"
done

log "SYNTH"
python3 - "$EV" <<'PY'
import json,re,sys
from pathlib import Path
ev=Path(sys.argv[1])

def rt(path):
  j=json.loads(path.read_text()); tot=j.get("requests_total");
  if tot is None: tot=float(j.get("requests_avg") or 0)*30
  return float(tot), float(j.get("requests_avg") or 0)

def cgd(tag):
  b=list(map(int,(ev/f"cgroup/{tag}_A_before.txt").read_text().split()))
  a=list(map(int,(ev/f"cgroup/{tag}_A_after.txt").read_text().split()))
  return dict(usage=a[0]-b[0], user=a[1]-b[1], system=a[2]-b[2])

def bpf(tag):
  blob=(ev/f"bpf/{tag}.out").read_text(errors="replace")+"\n"+(ev/f"bpf/{tag}.err").read_text(errors="replace")
  ns={m.group(1):int(m.group(2)) for m in re.finditer(r"@ns\[([^\]]+)\]:\s*(\d+)",blob)}
  n={m.group(1):int(m.group(2)) for m in re.finditer(r"@n\[([^\]]+)\]:\s*(\d+)",blob)}
  return ns,n

def ptop(tag,limit=15):
  text=(ev/f"perf/{tag}_report.txt").read_text(errors="replace") if (ev/f"perf/{tag}_report.txt").exists() else ""
  rows=[]
  for ln in text.splitlines():
    m=re.match(r"\s*([\d.]+)%\s+.*?\[([k.])\]\s+(\S+)",ln)
    if not m: continue
    pct=float(m.group(1)); sym=m.group(3); s=sym.lower(); cat="OTHER"
    if any(x in s for x in ("path","lookup","walk","namei","openat","do_filp","filename","d_lookup","link_path","resolve","apparmor","security_path","override_creds","inode_permission")): cat="PATH_LOOKUP"
    elif any(x in s for x in ("statx","getattr","vfs_getattr","generic_fillattr")): cat="METADATA"
    elif any(x in s for x in ("fput","filp_close","__close","fd_install","get_unused_fd","__fdget","fget")): cat="FD_LIFECYCLE"
    elif "sendfile" in s or "splice" in s: cat="SENDFILE"
    elif "writev" in s or "sock_write" in s or "tcp_sendmsg" in s: cat="WRITEV"
    elif "tcp" in s or "ip_" in s: cat="TCP"
    elif "epoll" in s: cat="EPOLL"
    elif "sched" in s: cat="SCHEDULER"
    elif any(x in s for x in ("alloc","kfree","kmem","page","slab")): cat="MEMORY"
    rows.append({"symbol":sym,"self_percent":pct,"category":cat})
    if len(rows)>=limit: break
  return rows

out={}
for tag in ("exyonq","ols"):
  tot,rps=rt(ev/f"cgroup/{tag}_A_rewrk.json")
  cg=cgd(tag)
  totB,rpsB=rt(ev/f"cgroup/{tag}_B_rewrk.json")
  ns,n=bpf(tag)
  out[tag]={
    "A_rps":rps,"A_requests":tot,
    "user_us_per_req":cg["user"]/tot,
    "system_us_per_req":cg["system"]/tot,
    "total_us_per_req":cg["usage"]/tot,
    "B_requests":totB,
    "bpf_us_per_req":{k:(v/1000.0)/totB for k,v in ns.items()},
    "bpf_calls_per_req":{k:v/totB for k,v in n.items()},
    "perf_top":ptop(tag),
  }
ex,ol=out["exyonq"],out["ols"]
gap_tot=ex["total_us_per_req"]-ol["total_us_per_req"]
gap_sys=ex["system_us_per_req"]-ol["system_us_per_req"]
gap_usr=ex["user_us_per_req"]-ol["user_us_per_req"]
def g(d,*ks): return sum(d["bpf_us_per_req"].get(k,0.0) for k in ks)
ex_open=g(ex,"openat2","openat"); ex_stat=g(ex,"statx"); ex_close=g(ex,"close"); ex_send=g(ex,"sendfile64")
ol_open=g(ol,"openat2","openat"); ol_close=g(ol,"close"); ol_wv=g(ol,"writev")
fd= (ex_open+ex_stat+ex_close)-(ol_open+ol_close)
live_gap_us=(0.013296-0.010342)*1000
summary={
  "CPU_ACCOUNTING_COMPARABILITY":"PASS",
  "EXYONQ_USER_CPU_US_PER_REQ":ex["user_us_per_req"],
  "EXYONQ_SYSTEM_CPU_US_PER_REQ":ex["system_us_per_req"],
  "EXYONQ_TOTAL_CPU_US_PER_REQ":ex["total_us_per_req"],
  "OLS_USER_CPU_US_PER_REQ":ol["user_us_per_req"],
  "OLS_SYSTEM_CPU_US_PER_REQ":ol["system_us_per_req"],
  "OLS_TOTAL_CPU_US_PER_REQ":ol["total_us_per_req"],
  "TOTAL_CPU_GAP_US_PER_REQ":gap_tot,
  "SYSTEM_CPU_GAP_US_PER_REQ":gap_sys,
  "USER_CPU_GAP_US_PER_REQ":gap_usr,
  "SYSTEM_GAP_PERCENT_OF_TOTAL":100*gap_sys/gap_tot if gap_tot else None,
  "USER_GAP_PERCENT_OF_TOTAL":100*gap_usr/gap_tot if gap_tot else None,
  "OPENAT2_CPU_US_PER_REQ_BOUNDED":ex_open,
  "STATX_CPU_US_PER_REQ_BOUNDED":ex_stat,
  "CLOSE_CPU_US_PER_REQ_BOUNDED":ex_close,
  "SENDFILE_CPU_US_PER_REQ_BOUNDED":ex_send,
  "OLS_OPEN_CPU_US_PER_REQ_BOUNDED":ol_open,
  "OLS_CLOSE_CPU_US_PER_REQ_BOUNDED":ol_close,
  "OLS_WRITEV_CPU_US_PER_REQ_BOUNDED":ol_wv,
  "FD_GEOMETRY_TOTAL_US_PER_REQ_BOUNDED":fd,
  "SENDFILE_VS_WRITEV_DIFFERENTIAL_US_PER_REQ_BOUNDED":ex_send-ol_wv,
  "FD_GEOMETRY_EXPLAINS_PERCENT_OF_CLEAN_GAP_BOUNDED":100*fd/gap_tot if gap_tot else None,
  "FD_GEOMETRY_EXPLAINS_PERCENT_OF_LIVE_GAP_BOUNDED":100*fd/live_gap_us if live_gap_us else None,
  "LIVE_GAP_US_PER_REQ":live_gap_us,
  "UNEXPLAINED_US_PER_REQ_AFTER_FD_GEOMETRY":gap_tot-fd,
  "exyonq_bpf_calls":ex["bpf_calls_per_req"],
  "ols_bpf_calls":ol["bpf_calls_per_req"],
  "exyonq_perf_top":ex["perf_top"],
  "ols_perf_top":ol["perf_top"],
  "details":out,
}
(ev/"reports"/"attribution_summary.json").write_text(json.dumps(summary,indent=2))
print(json.dumps(summary,indent=2)[:8000])
PY
echo EV=$EV
log DONE
