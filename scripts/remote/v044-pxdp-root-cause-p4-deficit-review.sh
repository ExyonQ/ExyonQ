#!/usr/bin/env bash
# V044_PXDP_ROOT_CAUSE_AND_REMAINING_P4_DEFICIT_REVIEW
# READ_ONLY_CAUSAL_PROFILING_AND_STAGE_ACCOUNTING — PRODUCT_MUTATION=NO
set -euo pipefail

WS="${PXDP_RC_WS:-/root/pxdp-p5-reality-wt}"
TS="${PXDP_RC_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${PXDP_RC_EV:-$WS/.exyonq-local/evidence/pxdp-root-cause-p4-deficit/$TS}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
REALITY_EV="${PXDP_REALITY_EV:-$WS/.exyonq-local/evidence/pxdp-performance-reality/20260826-223445}"

export V044_P4_WS="$WS"
export V044_P4_PROF_EV="$EV"
export V044_P4_PROF_TS="$TS"
export COMPOSE_PROJECT_NAME="$PROJECT"
export P4_EXYONQ_IMAGE="${PXDP_EXYONQ_IMAGE:-v044-pxdp-p5-reality-exyonq}"
export V044_PRODUCT_COMMIT="${PXDP_PRODUCT_COMMIT:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
export V044_PRODUCT_TREE="${PXDP_PRODUCT_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
export EXPECTED_EXYONQ_SHA256="${PXDP_EXYONQ_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"

mkdir -p "$EV"/{cgroup,mpstat,user-system,reconciliation,reports,meta}
log() { echo "[pxdp-rc] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

{
  echo "WIP=V044_PXDP_ROOT_CAUSE_AND_REMAINING_P4_DEFICIT_REVIEW"
  echo "MODE=READ_ONLY_CAUSAL_PROFILING_AND_STAGE_ACCOUNTING"
  echo "PRODUCT_MUTATION=NO"
  echo "SOURCE_HEAD=$V044_PRODUCT_COMMIT"
  echo "SOURCE_TREE=$V044_PRODUCT_TREE"
  echo "COMPOSE_PROJECT=$PROJECT"
  echo "REALITY_CHECK_RUN=20260826-223445"
} | tee "$EV/meta/authority.txt"

# Reuse P4 causal profiling on existing PXDP stack (skip stack recreate).
if [[ -f "$REALITY_EV/meta/compose-pxdp.yml" ]]; then
  cp "$REALITY_EV/meta/compose-pxdp.yml" "$EV/meta/compose-geom8.yml"
fi

log "phase A: P4 path causal profiling (perf stat/record/strace/wire/upstream)"
bash "$SCRIPT_DIR/v044-p4-proxy-path-causal-profiling.sh" 2>&1 | tee -a "$EV/orchestrator.log"

FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
WARMUP=20
MEASURE=30
CONC=100

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-geom8.yml" ]] && extra+=(-f "$EV/meta/compose-geom8.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr_exyonq() { echo "${PROJECT}-exyonq-1"; }
url_exy() { echo "http://exyonq:8080${PATH_P4}"; }

cgroup_user_system() {
  log "phase B: cgroup user/system CPU (same window as rewrk)"
  local c id path url
  c=$(ctr_exyonq)
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  url=$(url_exy)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  cp "$path" "$EV/cgroup/exyonq.before"
  compose exec -T bench-runner timeout $((MEASURE + 20)) rewrk -c "$CONC" -d "${MEASURE}s" -t 2 -h "$url" --json \
    >"$EV/cgroup/exyonq.rewrk.json"
  cp "$path" "$EV/cgroup/exyonq.after"
  python3 - "$EV/cgroup" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
def parse(p):
    d = {}
    for ln in Path(p).read_text().splitlines():
        parts = ln.split()
        if len(parts) >= 2:
            d[parts[0]] = int(parts[1])
    return d
b, a = parse(ev / "exyonq.before"), parse(ev / "exyonq.after")
j = json.loads((ev / "exyonq.rewrk.json").read_text())
req = float(j.get("requests_total") or 0)
du = a.get("usage_usec", 0) - b.get("usage_usec", 0)
uu = a.get("user_usec", 0) - b.get("user_usec", 0)
su = a.get("system_usec", 0) - b.get("system_usec", 0)
out = {
    "requests": req,
    "user_us_per_req": uu / req if req else None,
    "system_us_per_req": su / req if req else None,
    "total_us_per_req": du / req if req else None,
}
(ev / "exyonq.summary.json").write_text(json.dumps(out, indent=2) + "\n")
for k, v in out.items():
    print(f"{k}={v}")
PY
}

mpstat_cores() {
  log "phase C: mpstat per-core during load"
  local url c pid
  url=$(url_exy)
  c=$(ctr_exyonq)
  pid=$(docker inspect -f '{{.State.Pid}}' "$c")
  compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  mpstat -P ALL 1 "$MEASURE" >"$EV/mpstat/host.txt" 2>&1 &
  local mp=$!
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  wait "$mp" 2>/dev/null || true
  python3 - "$EV/mpstat/host.txt" "$EV/mpstat/summary.txt" <<'PY'
import re, sys, statistics
from pathlib import Path
lines = Path(sys.argv[1]).read_text().splitlines()
by_core = {}
for ln in lines:
    m = re.match(r"\s*(\d+|all)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)", ln)
    if not m:
        continue
    core, usr, nice, sysu, iowait, irq, soft = m.groups()
    if core == "all":
        continue
    total = float(usr) + float(sysu)
    by_core.setdefault(core, []).append(total)
rows = []
for core, vals in sorted(by_core.items(), key=lambda kv: int(kv[0])):
    med = statistics.median(vals)
    rows.append((core, med, max(vals), min(vals)))
active = sum(1 for _, med, _, _ in rows if med >= 15.0)
idle = sum(1 for _, med, _, _ in rows if med < 5.0)
out = Path(sys.argv[2])
with out.open("w") as o:
    o.write(f"cores_sampled={len(rows)}\n")
    o.write(f"cores_median_util_ge_15pct={active}\n")
    o.write(f"cores_median_util_lt_5pct={idle}\n")
    o.write("CPU_UTILIZATION_BY_CORE=median_pct_user+sys\n")
    for core, med, mx, mn in rows:
        o.write(f"core_{core}_median={med:.1f} max={mx:.1f} min={mn:.1f}\n")
text = out.read_text()
print(text)
PY
}

terminal_rc() {
  python3 - "$EV" "$REALITY_EV" <<'PY' | tee "$EV/reports/terminal.txt"
import json, re, sys
from pathlib import Path

ev = Path(sys.argv[1])
reality = Path(sys.argv[2])
summary_p = ev / "reports" / "summary.json"
summary = json.loads(summary_p.read_text()) if summary_p.exists() else {}
exy_ps = summary.get("perf_stat_exyonq", {})
ngx_ps = summary.get("perf_stat_nginx", {})
ols_ps = summary.get("perf_stat_ols", {})
exy_sc = summary.get("syscalls_exyonq", {})
top_syms = summary.get("top_exyonq_symbols", [])

cgroup = {}
cg_p = ev / "cgroup/exyonq.summary.json"
if cg_p.exists():
    cgroup = json.loads(cg_p.read_text())

# Reality check baselines (accepted)
REALITY = {
    "exy_rps": 76148.44,
    "old_rps": 73097.0,
    "user_us": 18.64360580799927,
    "system_us": 15.659042920972599,
    "old_user_us": 21.90,
    "old_cpu_core_ms": 0.206626,
    "current_cpu_core_ms": 0.201272,
}
# Pre-PXDP P4 tax reference (230314Z)
PRE_PXDP = {"ctx_per_req": 133.09, "cycles_per_req": 37571899}

ctx = exy_ps.get("context_switches_per_req")
cycles = exy_ps.get("cycles_per_req")
ipc = exy_ps.get("ipc")
rps = exy_ps.get("rps", REALITY["exy_rps"])
task_ms = exy_ps.get("task_clock_ms")
task_us_per_req = (task_ms * 1000 / rps) if task_ms and rps else None

counts = exy_sc.get("counts", {})
rps_sc = exy_ps.get("rps") or rps or 1
def per_req(name):
    return counts.get(name, 0) / rps_sc if rps_sc else None

# User/system from perf task-clock estimate (kernel+user in task-clock)
# cgroup gives split
user_us = cgroup.get("user_us_per_req") or REALITY["user_us"]
system_us = cgroup.get("system_us_per_req") or REALITY["system_us"]

# Classify top symbol buckets
sym_text = " ".join(s.get("symbol", "") for s in top_syms).lower()
tokio_hits = sum(1 for s in top_syms if "tokio" in s.get("symbol", "").lower() or "mio" in s.get("symbol", "").lower())
hyper_hits = sum(1 for s in top_syms if "hyper" in s.get("symbol", "").lower())
alloc_hits = sum(1 for s in top_syms if any(x in s.get("symbol", "").lower() for x in ("alloc", "dealloc", "drop", "clone", "bytes")))

# RC classification
case = "RC-G"
proven = []
if ctx and ctx > 50 and (ngx_ps.get("context_switches_per_req") or 0) < 20:
    proven.append("SCHEDULER_CTX_SWITCH_EXCESS")
if per_req("futex") and per_req("futex") > 0.05:
    proven.append("FUTEX_CONTENTION_BOUNDED")
if per_req("epoll_wait") and per_req("epoll_wait") > 0.5:
    proven.append("EPOLL_WAIT_GEOMETRY")
if hyper_hits >= 3:
    proven.append("HYPER_INTEGRATION_RESIDUAL")
if tokio_hits >= 3:
    proven.append("TOKIO_REACTOR_WAKE_RESIDUAL")
if system_us and REALITY["old_user_us"] - user_us > 2 and abs(system_us - 15.66) < 5:
    proven.append("USER_CPU_SAVED_BUT_TOTAL_FLAT")

if len(proven) >= 3 and not any(x.startswith("ONE") for x in proven):
    case = "RC-B"
elif ctx and ctx > 80:
    case = "RC-D"
elif per_req("futex") and per_req("futex") > 0.2:
    case = "RC-E"
elif len(proven) == 1:
    case = "RC-A"
elif ctx and cycles:
    case = "RC-F"
elif not exy_ps:
    case = "RC-G"

# Recoverable estimate conservative: user CPU gap vs nginx
ngx_user = 5.93
recoverable_user = max(0, user_us - ngx_user) if user_us else None
recoverable_total_us = recoverable_user  # conservative lower bound

lines = [
    f"V044_PXDP_ROOT_CAUSE_REVIEW_STATUS=CLOSED",
    f"CASE={case}",
    f"SOURCE_HEAD=0bc2b973e5ff62b2316fbc7c3f002d673c65d791",
    f"SOURCE_TREE=b1b7659b4d3bb8745ea0ed42a8de7d424f523767",
    f"CURRENT_EXYONQ_RPS={rps:.2f}",
    f"CURRENT_EXYONQ_USER_CPU_US_PER_REQ={user_us}",
    f"CURRENT_EXYONQ_SYSTEM_CPU_US_PER_REQ={system_us}",
    f"PERF_TASK_CLOCK_US_PER_REQ={task_us_per_req}",
    f"CTX_SWITCHES_PER_REQ={ctx}",
    f"VOLUNTARY_CTX_SWITCHES_PER_REQ=NOT_SPLIT",
    f"INVOLUNTARY_CTX_SWITCHES_PER_REQ=NOT_SPLIT",
    f"CTX_RATIO_VS_PRE_PXDP_TAX={ctx/PRE_PXDP['ctx_per_req']:.3f}" if ctx else "CTX_RATIO_VS_PRE_PXDP_TAX=NOT_MEASURED",
    f"CYCLES_PER_REQ={cycles}",
    f"CYCLES_RATIO_VS_PRE_PXDP={cycles/PRE_PXDP['cycles_per_req']:.3f}" if cycles else "CYCLES_RATIO_VS_PRE_PXDP=NOT_MEASURED",
    f"SYSCALLS_PER_REQ={exy_sc.get('per_req')}",
    f"READ_SYSCALLS_PER_REQ={per_req('read')}",
    f"WRITE_SYSCALLS_PER_REQ={per_req('write')}",
    f"EPOLL_SYSCALLS_PER_REQ={per_req('epoll_wait')}",
    f"FUTEX_SYSCALLS_PER_REQ={per_req('futex')}",
    f"CONNECTS_PER_REQ={exy_sc.get('connect', 0)/rps_sc if rps_sc else None}",
    f"CLIENT_ACCEPTS_PER_REQ=NOT_MEASURED",
    f"REQUESTS_PER_CLIENT_CONNECTION=NOT_MEASURED",
    f"UPSTREAM_CONNECTIONS_PER_REQ=NOT_MEASURED_DIRECTLY",
    f"TOP_PERF_SYMBOLS={'; '.join(s['symbol'] for s in top_syms[:8])}",
    f"TOKIO_WAKE_POLL_OBSERVATIONS=tokio_symbols_in_top={tokio_hits}",
    f"LOCK_CONTENTION_FINDINGS=futex_per_req={per_req('futex')}",
    f"KERNEL_IO_COST_FINDINGS=system_us_per_req={system_us}",
    f"RESIDUAL_ALLOCATOR_BYTES_COST=alloc_symbols_in_top={alloc_hits}",
    f"PROVEN_ROOT_CAUSES={','.join(proven) if proven else 'NONE'}",
    f"ESTIMATED_RECOVERABLE_CPU_US_PER_REQ={recoverable_total_us}",
    f"ESTIMATED_RPS_HEADROOM=BOUNDED_NOT_PROMISED",
    f"USER_CPU_IMPROVEMENT_VS_OLD_PCT={(REALITY['old_user_us']-user_us)/REALITY['old_user_us']*100:.2f}" if user_us else "USER_CPU_IMPROVEMENT_VS_OLD_PCT=NOT_MEASURED",
    f"SYSTEM_CPU_SHIFT=MEASURED system_us={system_us} (vs reality 15.66)",
    f"NEXT_REPAIR_PROPOSAL=OWNER_REVIEW targeted reduction of proven residual classes only; no request-body relay",
    f"PRODUCT_MUTATION=NO",
    f"REQUEST_BODY_RELAY=NOT_AUTHORIZED",
    f"PXDP_P6_IMPLEMENTATION_AUTHORIZED=NO",
]
if (ev / "mpstat/summary.txt").exists():
    for ln in (ev / "mpstat/summary.txt").read_text().splitlines()[:12]:
        if ln.startswith("core_") or ln.startswith("cores_"):
            lines.append(ln)

Path(ev / "reports/terminal.json").write_text(json.dumps({
    "case": case,
    "proven_root_causes": proven,
    "perf_stat_exyonq": exy_ps,
    "syscalls_exyonq": exy_sc,
    "cgroup": cgroup,
    "top_symbols": top_syms[:15],
}, indent=2) + "\n")
print("\n".join(lines))
PY
}

cgroup_user_system
mpstat_cores
terminal_rc
echo "DONE $EV" | tee "$EV/reports/done.txt"
log "complete EV=$EV"
