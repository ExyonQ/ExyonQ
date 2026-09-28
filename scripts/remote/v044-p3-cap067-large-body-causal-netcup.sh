#!/usr/bin/env bash
# V044_P3_CAP067_LARGE_BODY_CAUSAL_PROFILE — READ_ONLY (Netcup).
# PRODUCT_MUTATION=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
# Post cork + 128KiB clamp KEEP: locate residual ~4× core-ms vs NGINX on P3 1MiB.
# Phases: A cgroup user/sys → B bpftrace sendfile/send/epoll → C perf symbols.
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

WS="${V044_P3_CAUSAL_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P3_CAUSAL_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P3_CAUSAL_EV:-$WS/.exyonq-local-evidence/v044-p3-cap067-large-body-causal-$TS}"
FULL="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJ="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P3="/site/1m.bin"
EXPECTED_SHA="${EXPECTED_EXYONQ_SHA256:-fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c}"

mkdir -p "$EV"/{meta,cgroup,bpf,perf,reports}
cd "$WS"
compose(){ docker compose -f "$FULL" -f "$OVER" -p "$PROJ" "$@"; }
log(){ echo "[p3-causal $(date -u +%H:%M:%S)] $*" | tee -a "$EV/orchestrator.log"; }

EXY=${PROJ}-exyonq-1
NGX=${PROJ}-nginx-stable-1
URL_E="http://exyonq:8080${PATH_P3}"
URL_N="http://nginx-stable:8080${PATH_P3}"

cstat(){ cid=$(docker inspect -f '{{.Id}}' "$1"); echo "/sys/fs/cgroup/system.slice/docker-${cid}.scope/cpu.stat"; }
readstat(){
  python3 -c "from pathlib import Path; d={};
[d.__setitem__(a,int(b)) for a,b in (ln.split() for ln in Path('$1').read_text().splitlines() if ln.strip())];
print(d.get('usage_usec',0), d.get('user_usec',0), d.get('system_usec',0))"
}
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

{
  echo "WIP=V044_P3_CAP067_LARGE_BODY_CAUSAL_PROFILE"
  echo "PRODUCT_MUTATION=NO"
  echo "PATH=$PATH_P3"
  echo "KEEP_CLAMP=EXYONQ_SENDFILE_CHUNK=131072 (128KiB)"
  echo "GEOMETRY=8/8/8"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
} | tee "$EV/meta/contract.txt"

SHA=$(docker exec "$EXY" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$EV/meta/sha.txt"
if [[ -n "$EXPECTED_SHA" && "$SHA" != "$EXPECTED_SHA" ]]; then
  log "WARN sha $SHA != expected $EXPECTED_SHA (continuing — causal on live KEEP image)"
fi
docker exec "$EXY" env | grep -E 'EXYONQ_(ACCEPT|WORKER|EPOLL|SENDFILE|EDGE)' | sort | tee "$EV/meta/exyonq_env.txt"
for c in "$EXY" "$NGX"; do docker update --cpuset-cpus 0-7 "$c" >/dev/null; done

# Correctness probe
for pair in "exyonq|$URL_E" "nginx|$URL_N"; do
  IFS='|' read -r tag url <<<"$pair"
  out=$(compose exec -T bench-runner curl -sS -m 15 -o "/tmp/${tag}-1m.bin" -w "%{http_code} %{size_download}" "$url")
  echo "${tag}_probe=$out" | tee -a "$EV/meta/http_probe.txt"
done

log "PHASE A clean cgroup (no observers)"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  sp=$(cstat "$ctr")
  log "$tag warmup20"
  compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" >/dev/null
  readstat "$sp" >"$EV/cgroup/${tag}_A_before.txt"
  log "$tag measure30"
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/cgroup/${tag}_A_rewrk.json"
  readstat "$sp" >"$EV/cgroup/${tag}_A_after.txt"
  log "$tag A $(cat "$EV/cgroup/${tag}_A_before.txt") -> $(cat "$EV/cgroup/${tag}_A_after.txt")"
done

log "PHASE B bpftrace (sendfile/send/epoll/read)"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(tgids_of "$ctr")
  echo "$tag tgids=$tg" | tee "$EV/meta/${tag}_tgids.txt"
  cond=$(python3 -c "print(' || '.join(f'pid=={p}' for p in '$tg'.split(',') if p))")
  cat >"$EV/bpf/${tag}.bt" <<BT
tracepoint:syscalls:sys_enter_sendfile64 /$cond/ { @st_sf[tid]=nsecs; }
tracepoint:syscalls:sys_exit_sendfile64 /@st_sf[tid]/ { @ns["sendfile64"]=sum(nsecs-@st_sf[tid]); @n["sendfile64"]=count(); delete(@st_sf[tid]); }
tracepoint:syscalls:sys_enter_sendfile /$cond/ { @st_sf2[tid]=nsecs; }
tracepoint:syscalls:sys_exit_sendfile /@st_sf2[tid]/ { @ns["sendfile"]=sum(nsecs-@st_sf2[tid]); @n["sendfile"]=count(); delete(@st_sf2[tid]); }
tracepoint:syscalls:sys_enter_sendto /$cond/ { @st_s[tid]=nsecs; }
tracepoint:syscalls:sys_exit_sendto /@st_s[tid]/ { @ns["sendto"]=sum(nsecs-@st_s[tid]); @n["sendto"]=count(); delete(@st_s[tid]); }
tracepoint:syscalls:sys_enter_write /$cond/ { @st_w[tid]=nsecs; }
tracepoint:syscalls:sys_exit_write /@st_w[tid]/ { @ns["write"]=sum(nsecs-@st_w[tid]); @n["write"]=count(); delete(@st_w[tid]); }
tracepoint:syscalls:sys_enter_writev /$cond/ { @st_wv[tid]=nsecs; }
tracepoint:syscalls:sys_exit_writev /@st_wv[tid]/ { @ns["writev"]=sum(nsecs-@st_wv[tid]); @n["writev"]=count(); delete(@st_wv[tid]); }
tracepoint:syscalls:sys_enter_epoll_wait /$cond/ { @st_e[tid]=nsecs; }
tracepoint:syscalls:sys_exit_epoll_wait /@st_e[tid]/ { @ns["epoll_wait"]=sum(nsecs-@st_e[tid]); @n["epoll_wait"]=count(); delete(@st_e[tid]); }
tracepoint:syscalls:sys_enter_epoll_pwait /$cond/ { @st_ep[tid]=nsecs; }
tracepoint:syscalls:sys_exit_epoll_pwait /@st_ep[tid]/ { @ns["epoll_pwait"]=sum(nsecs-@st_ep[tid]); @n["epoll_pwait"]=count(); delete(@st_ep[tid]); }
tracepoint:syscalls:sys_enter_read /$cond/ { @st_r[tid]=nsecs; }
tracepoint:syscalls:sys_exit_read /@st_r[tid]/ { @ns["read"]=sum(nsecs-@st_r[tid]); @n["read"]=count(); delete(@st_r[tid]); }
tracepoint:syscalls:sys_enter_recvfrom /$cond/ { @st_rf[tid]=nsecs; }
tracepoint:syscalls:sys_exit_recvfrom /@st_rf[tid]/ { @ns["recvfrom"]=sum(nsecs-@st_rf[tid]); @n["recvfrom"]=count(); delete(@st_rf[tid]); }
tracepoint:syscalls:sys_enter_setsockopt /$cond/ { @st_so[tid]=nsecs; }
tracepoint:syscalls:sys_exit_setsockopt /@st_so[tid]/ { @ns["setsockopt"]=sum(nsecs-@st_so[tid]); @n["setsockopt"]=count(); delete(@st_so[tid]); }
BT
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  timeout 55 bpftrace "$EV/bpf/${tag}.bt" >"$EV/bpf/${tag}.out" 2>"$EV/bpf/${tag}.err" &
  bp=$!; sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/cgroup/${tag}_B_rewrk.json"
  kill -INT $bp 2>/dev/null || true; wait $bp 2>/dev/null || true
  log "bpf $tag done"
  tail -30 "$EV/bpf/${tag}.out" | tee -a "$EV/orchestrator.log" || true
done

log "PHASE C perf record"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(sed -n 's/.*tgids=//p' "$EV/meta/${tag}_tgids.txt")
  args=(); IFS=','; for p in $tg; do [[ -n "$p" ]] && args+=(-p "$p"); done; unset IFS
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  timeout 55 perf record -o "$EV/perf/${tag}.data" -F 997 -g "${args[@]}" -- sleep 36 \
    >"$EV/perf/${tag}_rec.out" 2>"$EV/perf/${tag}_rec.err" &
  pp=$!; sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/perf/${tag}_rewrk.json"
  wait $pp 2>/dev/null || true
  perf report -i "$EV/perf/${tag}.data" --stdio --no-children -n --percent-limit 0.4 \
    >"$EV/perf/${tag}_report.txt" 2>/dev/null || true
  log "perf $tag samples:"; rg -n 'Captured|samples' "$EV/perf/${tag}_rec.err" || true
  head -40 "$EV/perf/${tag}_report.txt" | tee -a "$EV/orchestrator.log" || true
done

log "SYNTH"
python3 - "$EV" <<'PY'
import json, re, sys
from pathlib import Path
ev = Path(sys.argv[1])

def rt(path):
    j = json.loads(path.read_text())
    tot = j.get("requests_total")
    if tot is None:
        tot = float(j.get("requests_avg") or 0) * 30
    return float(tot), float(j.get("requests_avg") or 0)

def cgd(tag):
    b = list(map(int, (ev / f"cgroup/{tag}_A_before.txt").read_text().split()))
    a = list(map(int, (ev / f"cgroup/{tag}_A_after.txt").read_text().split()))
    return dict(usage=a[0] - b[0], user=a[1] - b[1], system=a[2] - b[2])

def bpf(tag):
    blob = (ev / f"bpf/{tag}.out").read_text(errors="replace") + "\n" + (ev / f"bpf/{tag}.err").read_text(errors="replace")
    ns = {m.group(1): int(m.group(2)) for m in re.finditer(r"@ns\[([^\]]+)\]:\s*(\d+)", blob)}
    n = {m.group(1): int(m.group(2)) for m in re.finditer(r"@n\[([^\]]+)\]:\s*(\d+)", blob)}
    return ns, n

def ptop(tag, limit=20):
    p = ev / f"perf/{tag}_report.txt"
    text = p.read_text(errors="replace") if p.exists() else ""
    rows = []
    for ln in text.splitlines():
        m = re.match(r"\s*([\d.]+)%\s+.*?\[([k.])\]\s+(\S+)", ln)
        if not m:
            continue
        pct = float(m.group(1))
        sym = m.group(3)
        s = sym.lower()
        cat = "OTHER"
        if "sendfile" in s or "splice" in s or "do_sendfile" in s:
            cat = "SENDFILE"
        elif any(x in s for x in ("tcp_sendmsg", "sock_send", "tcp_write", "ip_queue", "skb", "tcp_push")):
            cat = "TCP_SEND"
        elif "epoll" in s:
            cat = "EPOLL"
        elif "sched" in s or "futex" in s:
            cat = "SCHEDULER"
        elif any(x in s for x in ("copy_page", "memcpy", "csum", "page_counter", "charge", "memcg", "try_charge")):
            cat = "MEM_COPY_CHARGE"
        elif any(x in s for x in ("alloc", "kfree", "kmem", "slab", "page_fault")):
            cat = "MEMORY"
        elif "read" in s or "recv" in s:
            cat = "READ"
        rows.append({"symbol": sym, "self_percent": pct, "category": cat})
        if len(rows) >= limit:
            break
    return rows

out = {}
for tag in ("exyonq", "nginx"):
    tot, rps = rt(ev / f"cgroup/{tag}_A_rewrk.json")
    cg = cgd(tag)
    totB, rpsB = rt(ev / f"cgroup/{tag}_B_rewrk.json")
    ns, n = bpf(tag)
    out[tag] = {
        "A_rps": rps,
        "A_requests": tot,
        "user_us_per_req": cg["user"] / tot if tot else None,
        "system_us_per_req": cg["system"] / tot if tot else None,
        "total_us_per_req": cg["usage"] / tot if tot else None,
        "B_requests": totB,
        "B_rps": rpsB,
        "bpf_us_per_req": {k: (v / 1000.0) / totB for k, v in ns.items()} if totB else {},
        "bpf_calls_per_req": {k: v / totB for k, v in n.items()} if totB else {},
        "perf_top": ptop(tag),
    }

ex, ng = out["exyonq"], out["nginx"]
gap_tot = ex["total_us_per_req"] - ng["total_us_per_req"]
gap_sys = ex["system_us_per_req"] - ng["system_us_per_req"]
gap_usr = ex["user_us_per_req"] - ng["user_us_per_req"]

# Rank syscall call density gaps
keys = sorted(set(ex["bpf_calls_per_req"]) | set(ng["bpf_calls_per_req"]))
call_gaps = []
for k in keys:
    de = ex["bpf_calls_per_req"].get(k, 0.0) - ng["bpf_calls_per_req"].get(k, 0.0)
    te = ex["bpf_us_per_req"].get(k, 0.0) - ng["bpf_us_per_req"].get(k, 0.0)
    call_gaps.append((te, de, k))
call_gaps.sort(reverse=True)

report = {
    "scenario": "P3",
    "path": "/site/1m.bin",
    "servers": out,
    "gap_us_per_req": {
        "total": gap_tot,
        "user": gap_usr,
        "system": gap_sys,
        "system_share_of_gap_pct": (100.0 * gap_sys / gap_tot) if gap_tot else None,
        "user_share_of_gap_pct": (100.0 * gap_usr / gap_tot) if gap_tot else None,
    },
    "bpf_time_gap_ranked": [
        {"syscall": k, "delta_us_per_req": te, "delta_calls_per_req": de} for te, de, k in call_gaps[:12]
    ],
}
(ev / "reports/causal_summary.json").write_text(json.dumps(report, indent=2))

lines = []
lines.append("P3_CAP067_LARGE_BODY_CAUSAL")
lines.append(f"EXYONQ_RPS={ex['A_rps']:.2f} NGINX_RPS={ng['A_rps']:.2f}")
lines.append(f"EXYONQ_USER_US={ex['user_us_per_req']:.3f} SYSTEM_US={ex['system_us_per_req']:.3f} TOTAL_US={ex['total_us_per_req']:.3f}")
lines.append(f"NGINX_USER_US={ng['user_us_per_req']:.3f} SYSTEM_US={ng['system_us_per_req']:.3f} TOTAL_US={ng['total_us_per_req']:.3f}")
lines.append(f"GAP_TOTAL_US={gap_tot:.3f} GAP_SYSTEM_US={gap_sys:.3f} GAP_USER_US={gap_usr:.3f}")
if gap_tot:
    lines.append(f"SYSTEM_SHARE_OF_GAP_PCT={100*gap_sys/gap_tot:.1f} USER_SHARE_OF_GAP_PCT={100*gap_usr/gap_tot:.1f}")
lines.append("BPF_CALLS_PER_REQ_EXYONQ=" + json.dumps(ex["bpf_calls_per_req"], sort_keys=True))
lines.append("BPF_CALLS_PER_REQ_NGINX=" + json.dumps(ng["bpf_calls_per_req"], sort_keys=True))
lines.append("BPF_US_PER_REQ_EXYONQ=" + json.dumps(ex["bpf_us_per_req"], sort_keys=True))
lines.append("BPF_US_PER_REQ_NGINX=" + json.dumps(ng["bpf_us_per_req"], sort_keys=True))
lines.append("TOP_BPF_TIME_GAPS=" + json.dumps(report["bpf_time_gap_ranked"][:6]))
lines.append("PERF_TOP_EXYONQ=" + json.dumps(ex["perf_top"][:10]))
lines.append("PERF_TOP_NGINX=" + json.dumps(ng["perf_top"][:10]))

# Heuristic next recommendation
primary = "INVESTIGATE"
if gap_sys > gap_usr * 1.2:
    primary = "SYSTEM_DOMINATED"
elif gap_usr > gap_sys * 1.2:
    primary = "USERSPACE_DOMINATED"
else:
    primary = "MIXED"

sf_ex = ex["bpf_calls_per_req"].get("sendfile64", 0) + ex["bpf_calls_per_req"].get("sendfile", 0)
sf_ng = ng["bpf_calls_per_req"].get("sendfile64", 0) + ng["bpf_calls_per_req"].get("sendfile", 0)
ep_ex = ex["bpf_calls_per_req"].get("epoll_wait", 0) + ex["bpf_calls_per_req"].get("epoll_pwait", 0)
ep_ng = ng["bpf_calls_per_req"].get("epoll_wait", 0) + ng["bpf_calls_per_req"].get("epoll_pwait", 0)

next_mut = "PROFILE_ONLY_NO_PRODUCT_MUTATION_YET"
if sf_ex > sf_ng * 1.5 and sf_ng > 0:
    next_mut = "REDUCE_SENDFILE_SYSCALLS_PER_REQ (larger effective drain / fewer Progress parks)"
elif ep_ex > ep_ng * 1.5 and ep_ng > 0:
    next_mut = "REDUCE_EPOLL_WAKEUPS_ON_LARGE_BODY"
elif any(r["category"] == "MEM_COPY_CHARGE" for r in ex["perf_top"][:8]):
    next_mut = "ATTACK_MEMCG_OR_SKB_PAGE_CHARGE_PATH"
elif primary == "USERSPACE_DOMINATED":
    next_mut = "PROFILE_USERSPACE_CAP067_FSM_HOT_PATH"
else:
    next_mut = "ATTACK_KERNEL_SEND_PATH_COST (sendfile/tcp_sendmsg) — no socket knob reintro"

lines.append(f"GAP_CLASS={primary}")
lines.append(f"NEXT_PRODUCT_HYPOTHESIS={next_mut}")
lines.append("PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN")

text = "\n".join(lines) + "\n"
(ev / "reports/causal_terminal.txt").write_text(text)
print(text)
PY

echo "SUITE_DIR=$EV" >"$EV/DONE"
log "SUITE_COMPLETE $EV"
