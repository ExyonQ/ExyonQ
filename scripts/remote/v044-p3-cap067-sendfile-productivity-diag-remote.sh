#!/usr/bin/env bash
# V044_P3_CAP067_SENDFILE_PATH_PRODUCTIVITY_DIAG — DEVELOPMENT (Netcup).
# PRODUCT_MUTATION=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Memcg packaging A/B DEFERRED: root cgroup.subtree_control includes memory on
# this host — every cgroup still has memory controller (escape infeasible).
#
# This WIP: sendfile64-only bpf geometry + cgroup µs/req + perf SENDFILE_IO share
# under KEEP (cork + EXYONQ_SENDFILE_CHUNK=131072 + geom 8/8/8).
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

WS="${V044_P3_SFPROD_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P3_SFPROD_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P3_SFPROD_EV:-$WS/.exyonq-local-evidence/v044-p3-cap067-sendfile-productivity-$TS}"
FULL="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJ="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
IMAGE="${P1_EXYONQ_IMAGE:-v044p1auth-exyonq-chunkab}"
CHUNK="${EXYONQ_SENDFILE_CHUNK:-131072}"
PATH_P3="/site/1m.bin"
EXPECTED_SHA="${EXPECTED_EXYONQ_SHA256:-fefe547d13a056de36fc43a5fab1feac8bc7ab957e3a65db318689806aa49d7c}"

mkdir -p "$EV"/{meta,bpf,cgroup,perf,reports}
cd "$WS"
compose(){ docker compose -f "$FULL" -f "$OVER" -p "$PROJ" "$@"; }
log(){ echo "[sfprod $(date -u +%H:%M:%S)] $*" | tee -a "$EV/orchestrator.log"; }

EXY=${PROJ}-exyonq-1
NGX=${PROJ}-nginx-stable-1
URL_E="http://exyonq:8080${PATH_P3}"
URL_N="http://nginx-stable:8080${PATH_P3}"

tgids_of(){
  main=$(docker inspect -f '{{.State.Pid}}' "$1")
  python3 - "$main" <<'PY'
import os,sys
root=int(sys.argv[1])
def cg(pid):
  try: return open(f"/proc/{pid}/cgroup").read()
  except OSError: return ""
want=cg(root); tgids=set()
for pid in os.listdir("/proc"):
  if not pid.isdigit(): continue
  if cg(int(pid))!=want: continue
  try:
    for ln in open(f"/proc/{pid}/status"):
      if ln.startswith("Tgid:"):
        tgids.add(int(ln.split()[1])); break
  except OSError: pass
print(",".join(str(t) for t in sorted(tgids)))
PY
}

cstat(){ cid=$(docker inspect -f '{{.Id}}' "$1"); echo "/sys/fs/cgroup/system.slice/docker-${cid}.scope/cpu.stat"; }
readstat(){
  python3 -c "from pathlib import Path; d={};
[d.__setitem__(a,int(b)) for a,b in (ln.split() for ln in Path('$1').read_text().splitlines() if ln.strip())];
print(d.get('usage_usec',0), d.get('user_usec',0), d.get('system_usec',0))"
}

{
  echo "WIP=V044_P3_CAP067_SENDFILE_PATH_PRODUCTIVITY_DIAG"
  echo "PRODUCT_MUTATION=NO"
  echo "MEMCG_PACKAGING=DEFERRED_INFEASIBLE_ROOT_MEMORY_CONTROLLER"
  echo "KEEP_CLAMP=EXYONQ_SENDFILE_CHUNK=$CHUNK"
  echo "GEOMETRY=8/8/8"
  echo "PATH=$PATH_P3"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
} | tee "$EV/meta/contract.txt"

# Restore KEEP geometry (prior geom A/B may have left 4/4/4).
export P1_EXYONQ_IMAGE="$IMAGE"
export EXYONQ_ACCEPT_WORKERS=8
export EXYONQ_WORKER_THREADS=8
export EXYONQ_EPOLL_POOL_THREADS=8
export EXYONQ_EPOLL_LISTEN=1
export EXYONQ_SENDFILE_CHUNK="$CHUNK"
cd "$WS/benchmarks/docker"
log "recreate exyonq KEEP geom8 chunk=$CHUNK"
compose up -d --no-deps --force-recreate exyonq
for i in $(seq 1 90); do
  if docker exec "$EXY" curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then
    log "exyonq healthy (try $i)"; break
  fi
  sleep 2
done
SHA=$(docker exec "$EXY" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$EV/meta/sha.txt"
if [[ -n "$EXPECTED_SHA" && "$SHA" != "$EXPECTED_SHA" ]]; then
  log "WARN sha $SHA != expected $EXPECTED_SHA"
fi
docker exec "$EXY" env | grep -E 'EXYONQ_(ACCEPT|WORKER|EPOLL|SENDFILE|EDGE)' | sort | tee "$EV/meta/exyonq_env.txt"
for c in "$EXY" "$NGX"; do docker update --cpuset-cpus 0-7 "$c" >/dev/null; done

# Correctness probe
for pair in "exyonq|$URL_E" "nginx|$URL_N"; do
  IFS='|' read -r tag url <<<"$pair"
  out=$(compose exec -T bench-runner curl -sS -m 15 -o "/tmp/${tag}-1m.bin" -w "%{http_code} %{size_download}" "$url")
  echo "${tag}_probe=$out" | tee -a "$EV/meta/http_probe.txt"
done

# --- Phase A: clean cgroup µs/req ---
log "PHASE A cgroup clean window"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  sp=$(cstat "$ctr")
  compose exec -T bench-runner rewrk -c 100 -d 20s -t 2 -h "$url" >/dev/null
  readstat "$sp" >"$EV/cgroup/${tag}_before.txt"
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/cgroup/${tag}_rewrk.json"
  readstat "$sp" >"$EV/cgroup/${tag}_after.txt"
  log "$tag A $(cat "$EV/cgroup/${tag}_before.txt") -> $(cat "$EV/cgroup/${tag}_after.txt")"
done

# --- Phase B: sendfile64-only bpf (never attach sys_enter_sendfile) ---
log "PHASE B sendfile64-only bpf"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(tgids_of "$ctr")
  echo "$tag tgids=$tg" | tee "$EV/meta/${tag}_tgids.txt"
  cond=$(python3 -c "print(' || '.join(f'pid=={p}' for p in '$tg'.split(',') if p))")
  cat >"$EV/bpf/${tag}_sf64.bt" <<BT
tracepoint:syscalls:sys_enter_sendfile64 /$cond/ {
  @n = count();
  @bytes = sum(args->count);
  @cnt_hist = hist(args->count);
}
interval:s:36 {
  print(@n);
  print(@bytes);
  print(@cnt_hist);
  exit();
}
BT
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  bpftrace "$EV/bpf/${tag}_sf64.bt" >"$EV/bpf/${tag}_sf64.out" 2>"$EV/bpf/${tag}_sf64.err" &
  bp=$!
  sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/bpf/${tag}_rewrk.json"
  wait $bp || true
  log "bpf $tag done"
  tail -40 "$EV/bpf/${tag}_sf64.out" | tee -a "$EV/orchestrator.log" || true
done

# --- Phase C: perf top symbols ---
log "PHASE C perf"
for pair in "exyonq|$EXY|$URL_E" "nginx|$NGX|$URL_N"; do
  IFS='|' read -r tag ctr url <<<"$pair"
  tg=$(sed -n 's/.*tgids=//p' "$EV/meta/${tag}_tgids.txt")
  args=(); IFS=','; for p in $tg; do [[ -n "$p" ]] && args+=(-p "$p"); done; unset IFS
  compose exec -T bench-runner rewrk -c 100 -d 8s -t 2 -h "$url" >/dev/null
  timeout 55 perf record -o "$EV/perf/${tag}.data" -F 997 -g "${args[@]}" -- sleep 36 \
    >"$EV/perf/${tag}_rec.out" 2>"$EV/perf/${tag}_rec.err" &
  pp=$!
  sleep 1
  compose exec -T bench-runner rewrk -c 100 -d 30s -t 2 -h "$url" --json >"$EV/perf/${tag}_rewrk.json"
  wait $pp 2>/dev/null || true
  perf report -i "$EV/perf/${tag}.data" --stdio --no-children -n --percent-limit 0.4 \
    >"$EV/perf/${tag}_report.txt" 2>/dev/null || true
  log "perf $tag done"
  head -35 "$EV/perf/${tag}_report.txt" | tee -a "$EV/orchestrator.log" || true
done

log "SYNTH"
python3 - "$EV" <<'PY'
import json, re, sys
from pathlib import Path
ev = Path(sys.argv[1])

def rewrk_rps(p):
    d = json.loads(p.read_text())
    return float(d.get("requests_avg") or 0), int(d.get("requests_total") or 0)

def cgroup_delta(tag):
    b = list(map(int, (ev/"cgroup"/f"{tag}_before.txt").read_text().split()))
    a = list(map(int, (ev/"cgroup"/f"{tag}_after.txt").read_text().split()))
    usage, user, system = (a[i]-b[i] for i in range(3))
    rps, total = rewrk_rps(ev/"cgroup"/f"{tag}_rewrk.json")
    # 30s measure window
    us_per_req = (usage/total) if total else None
    user_us = (user/total) if total else None
    sys_us = (system/total) if total else None
    return {
        "rps": rps, "requests_total": total,
        "usage_usec": usage, "user_usec": user, "system_usec": system,
        "us_per_req": us_per_req, "user_us_per_req": user_us, "system_us_per_req": sys_us,
    }

def parse_bpf(tag):
    text = (ev/"bpf"/f"{tag}_sf64.out").read_text() if (ev/"bpf"/f"{tag}_sf64.out").exists() else ""
    n = None; nbytes = None
    m = re.search(r"@n:\s*(\d+)", text)
    if m: n = int(m.group(1))
    m = re.search(r"@bytes:\s*(\d+)", text)
    if m: nbytes = int(m.group(1))
    hist = {}
    for line in text.splitlines():
        m = re.match(r"\[([^]]+)\)\s+(\d+)", line.strip())
        if m: hist[m.group(1)] = int(m.group(2))
    rps, total = rewrk_rps(ev/"bpf"/f"{tag}_rewrk.json") if (ev/"bpf"/f"{tag}_rewrk.json").exists() else (0,0)
    sf_per_req = (n/total) if (n and total) else None
    avg_count = (nbytes/n) if (n and nbytes) else None
    return {
        "sendfile64_n": n, "sendfile64_bytes_sum_count": nbytes,
        "count_hist": hist, "rewrk_rps": rps, "rewrk_total": total,
        "sendfile64_per_req": sf_per_req, "avg_count_arg": avg_count,
    }

def categorize(sym):
    s = sym.lower()
    if any(x in s for x in ("spin", "queued_spin", "raw_spin")): return "SPINLOCK"
    if any(x in s for x in ("page_counter", "charge", "memcg", "try_charge")): return "MEMCG"
    if any(x in s for x in ("skb_split", "skb_release", "tcp_send", "tcp_write", "skb_append", "tcp_push")): return "TCP_SKB"
    if any(x in s for x in ("sendfile", "splice", "filemap", "do_iter", "iov_iter", "copy_page_to")): return "SENDFILE_IO"
    return "OTHER"

def parse_perf(tag):
    p = ev/"perf"/f"{tag}_report.txt"
    if not p.exists(): return {"categories": {}, "top": []}
    cats = {}
    top = []
    for line in p.read_text(errors="ignore").splitlines():
        # e.g. "     3.21%  ..."
        m = re.search(r"([\d.]+)%\s+\S+\s+\S+\s+\[k\]\s+(\S+)", line)
        if not m:
            m = re.search(r"([\d.]+)%\s+.*?\[k\]\s+(\S+)", line)
        if not m: continue
        pct = float(m.group(1)); sym = m.group(2)
        cat = categorize(sym)
        cats[cat] = cats.get(cat, 0.0) + pct
        if len(top) < 12:
            top.append({"symbol": sym, "self_percent": pct, "category": cat})
    return {"categories": cats, "top": top}

out = {
    "suite": ev.name,
    "keep": {"chunk": 131072, "geometry": "8/8/8", "cork": True},
    "memcg_packaging": "DEFERRED_INFEASIBLE_ROOT_MEMORY_CONTROLLER",
    "exyonq": {
        "cgroup": cgroup_delta("exyonq"),
        "bpf": parse_bpf("exyonq"),
        "perf": parse_perf("exyonq"),
    },
    "nginx": {
        "cgroup": cgroup_delta("nginx"),
        "bpf": parse_bpf("nginx"),
        "perf": parse_perf("nginx"),
    },
}
ex = out["exyonq"]["cgroup"]; ng = out["nginx"]["cgroup"]
if ex.get("system_us_per_req") and ng.get("system_us_per_req"):
    out["gap"] = {
        "system_us_per_req": ex["system_us_per_req"] - ng["system_us_per_req"],
        "user_us_per_req": (ex.get("user_us_per_req") or 0) - (ng.get("user_us_per_req") or 0),
        "rps_delta_pct": 100.0 * (ex["rps"] - ng["rps"]) / ng["rps"] if ng["rps"] else None,
        "gap_class": "SYSTEM_DOMINATED" if (ex["system_us_per_req"] - ng["system_us_per_req"]) > abs((ex.get("user_us_per_req") or 0) - (ng.get("user_us_per_req") or 0)) else "MIXED",
    }
eb = out["exyonq"]["bpf"]; nb = out["nginx"]["bpf"]
out["geometry_read"] = {
    "exyonq_sf_per_req": eb.get("sendfile64_per_req"),
    "exyonq_avg_count": eb.get("avg_count_arg"),
    "nginx_sf_per_req": nb.get("sendfile64_per_req"),
    "nginx_avg_count": nb.get("avg_count_arg"),
}
# Escalation hint
ex_sf = (out["exyonq"]["perf"]["categories"] or {}).get("SENDFILE_IO", 0)
ng_sf = (out["nginx"]["perf"]["categories"] or {}).get("SENDFILE_IO", 0)
ex_sp = (out["exyonq"]["perf"]["categories"] or {}).get("SPINLOCK", 0)
hint = "UNKNOWN"
if eb.get("sendfile64_per_req") and eb["sendfile64_per_req"] >= 7 and (nb.get("sendfile64_per_req") or 0) <= 2:
    hint = "EXYONQ_MORE_SENDFILE_STEPS_THAN_NGINX"
elif ex_sf < ng_sf * 0.5 and ex_sp >= 5:
    hint = "LOW_PRODUCTIVE_SENDFILE_SHARE_ESCALATE_SPLICE_ADR"
elif eb.get("sendfile64_n") == 0:
    hint = "NO_SENDFILE64_UNEXPECTED"
out["escalation_hint"] = hint

(ev/"reports"/"productivity_summary.json").write_text(json.dumps(out, indent=2))
lines = [
    f"SUITE={ev.name}",
    f"GAP_CLASS={out.get('gap',{}).get('gap_class')}",
    f"EX_RPS={ex.get('rps')} NG_RPS={ng.get('rps')} dRPS%={out.get('gap',{}).get('rps_delta_pct')}",
    f"EX_SYS_US={ex.get('system_us_per_req')} NG_SYS_US={ng.get('system_us_per_req')}",
    f"EX_SF_PER_REQ={eb.get('sendfile64_per_req')} NG_SF_PER_REQ={nb.get('sendfile64_per_req')}",
    f"EX_AVG_COUNT={eb.get('avg_count_arg')} NG_AVG_COUNT={nb.get('avg_count_arg')}",
    f"EX_SENDFILE_IO%={ex_sf} NG_SENDFILE_IO%={ng_sf}",
    f"ESCALATION_HINT={hint}",
    f"MEMCG_PACKAGING={out['memcg_packaging']}",
]
(ev/"reports"/"productivity_terminal.txt").write_text("\n".join(lines)+"\n")
print("\n".join(lines))
PY

echo "SUITE_DIR=$EV" >"$EV/DONE"
log "SUITE_COMPLETE $EV"
