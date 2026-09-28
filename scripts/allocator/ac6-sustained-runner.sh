#!/usr/bin/env bash
# AC6 sustained memory/stability — rotated system/jemalloc/mimalloc.
# Load is open-loop ReWrk in 30s windows (×30 = 900s). No rivals.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
COMPOSE="$ROOT/benchmarks/docker/docker-compose.bench.yml"
COMPOSE_DIR="$ROOT/benchmarks/docker"
SAMPLE="$ROOT/benchmarks/scripts/sample_resources.sh"
CONVERTER="$ROOT/benchmarks/scenarios/perf/rewrk-report-to-json.py"
RESULTS="${AC6_RESULTS:-$ROOT/benchmarks/results/allocator-ac6-sustained}"
RAW_ROOT="${AC6_RAW_ROOT:-$ROOT/docs/benchmarks/allocators/raw/ac6}"
ID_DIR="$ROOT/docs/benchmarks/allocators/identities/linux"
HOST_CPUS="${BENCH_HOST_CPUS:-$(nproc)}"
REWRK_THREADS=2
LOADGEN_STOP=85
EXPECTED_REWRK_SHA=bb4102ab40a98d69683285bf10d6dae3ce2fdb04f698d91c4a0c4b59717c11a7
SOURCE_COMMIT_EXPECTED=bee1d68414fed48b4e4d7138eb12b8fe1a696558
HOST_ID=NETCUP_AMD64

WARMUP_S=60
MEASURE_S=900
RECOVERY_S=180
WINDOW_S=30
WINDOWS=$((MEASURE_S / WINDOW_S))

export BENCH_HOST_CPUS="$HOST_CPUS"
mkdir -p "$RESULTS" "$RAW_ROOT" "$RESULTS/runs" "$RESULTS/rejected" "$RESULTS/calibration"
PROFILES="$RESULTS/frozen-profiles.json"
LOG="$RESULTS/ac6-runner.log"
MANIFEST="$RESULTS/runs-manifest.tsv"

log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" | tee -a "$LOG"; }

stop_rivals() {
  docker compose -f "$COMPOSE" stop \
    nginx-stable nginx-mainline haproxy envoy traefik caddy apache \
    openlitespeed-stable openlitespeed-latest >/dev/null 2>&1 || true
}

session_certify() {
  log "AC6.4 session certify"
  stop_rivals
  docker compose -f "$COMPOSE" --profile bench up -d --no-deps bench-runner upstream >/dev/null
  stop_rivals
  local rewrk_sha
  rewrk_sha="$(docker compose -f "$COMPOSE" --profile bench exec -T bench-runner sha256sum /usr/local/bin/rewrk | awk '{print $1}')"
  [[ "$rewrk_sha" == "$EXPECTED_REWRK_SHA" ]] || { log "FAIL rewrk sha"; exit 2; }
  for v in system jemalloc mimalloc; do
    local expected actual
    expected="$(grep '^CONTAINER_IMAGE_ID=' "$ID_DIR/identity-$v.txt" | cut -d= -f2)"
    actual="$(docker image inspect "exyonq-alloc-$v:local" --format '{{.Id}}')"
    [[ "$expected" == "$actual" ]] || { log "FAIL image $v"; exit 2; }
  done
  pgrep -af 'cargo build|rustc ' >/dev/null 2>&1 && { log "FAIL build process"; exit 2; }
  log "certify OK"
}

scenario_config() {
  case "$1" in
    p11) echo /bench/bench-tls.toml ;;
    *) echo /bench/bench.toml ;;
  esac
}

scenario_path() {
  case "$1" in
    p1|p11) echo /site/1k.bin ;;
    p3) echo /site/1m.bin ;;
    p4|p6) echo /api/ ;;
    *) echo /site/1k.bin ;;
  esac
}

init_profiles() {
  # Targets = 70% of min AC5 median; start_conns from AC5; calibrate down.
  python3 - "$PROFILES" <<'PY'
import json, sys
from pathlib import Path
profiles = {
  "p1":  {"target_rps": 120064.1, "connections": 70, "ac5_connections": 100,
          "warmup_s": 60, "measure_s": 900, "recovery_s": 180, "window_s": 30,
          "scheme": "http", "http2": False, "extra": "", "config": "/bench/bench.toml",
          "frozen_by": "pending_calibration"},
  "p3":  {"target_rps": 3725.9, "connections": 70, "ac5_connections": 100,
          "warmup_s": 60, "measure_s": 900, "recovery_s": 180, "window_s": 30,
          "scheme": "http", "http2": False, "extra": "", "config": "/bench/bench.toml",
          "frozen_by": "pending_calibration"},
  "p4":  {"target_rps": 113373.8, "connections": 70, "ac5_connections": 100,
          "warmup_s": 60, "measure_s": 900, "recovery_s": 180, "window_s": 30,
          "scheme": "http", "http2": False, "extra": "", "config": "/bench/bench.toml",
          "frozen_by": "pending_calibration"},
  "p6":  {"target_rps": 105330.6, "connections": 140, "ac5_connections": 200,
          "warmup_s": 60, "measure_s": 900, "recovery_s": 180, "window_s": 30,
          "scheme": "http", "http2": False, "extra": "", "config": "/bench/bench.toml",
          "frozen_by": "pending_calibration", "class": "DIAGNOSTIC_HIGH_VARIANCE"},
  "p11": {"target_rps": 70029.4, "connections": 35, "ac5_connections": 50,
          "warmup_s": 60, "measure_s": 900, "recovery_s": 180, "window_s": 30,
          "scheme": "https", "http2": True, "extra": "", "config": "/bench/bench-tls.toml",
          "frozen_by": "pending_calibration"},
}
Path(sys.argv[1]).write_text(json.dumps(profiles, indent=2) + "\n")
PY
}

load_profile_field() {
  python3 -c "import json; p=json.load(open('$PROFILES'))['$1']; print(p['$2'])"
}

recreate_variant() {
  local variant="$1" config="$2" mode="$3"
  cd "$COMPOSE_DIR"
  docker tag "exyonq-alloc-${variant}:local" docker-exyonq:latest
  local expected actual
  expected="$(grep '^CONTAINER_IMAGE_ID=' "$ID_DIR/identity-$variant.txt" | cut -d= -f2)"
  actual="$(docker image inspect docker-exyonq:latest --format '{{.Id}}')"
  [[ "$expected" == "$actual" ]] || { echo IDENTITY_MISMATCH; return 3; }
  EXYONQ_CONFIG="$config" docker compose -f docker-compose.bench.yml up -d --no-deps --force-recreate exyonq >/dev/null
  stop_rivals
  local ok=0
  for _ in $(seq 1 50); do
    if [[ "$mode" == "https" ]]; then
      curl -skf --max-time 2 "https://127.0.0.1:8443/site/1k.bin" -o /dev/null && ok=1 && break
    else
      curl -sf --max-time 2 "http://127.0.0.1:8080/health" >/dev/null && ok=1 && break
      curl -sf --max-time 2 "http://127.0.0.1:8080/site/1k.bin" -o /dev/null && ok=1 && break
    fi
    sleep 1
  done
  [[ "$ok" -eq 1 ]] || return 1
  sleep 3
  return 0
}

# Continuous sampler: writes CSV with phase tags via external phase file.
# IMPORTANT: do not capture this via $(...) — bash waits for background jobs
# inside command substitutions, which would block the whole warm-up/load path.
start_timeseries() {
  local cid_srv="$1" cid_lg="$2" out_csv="$3" phase_file="$4" duration="$5" pid_file="$6"
  python3 - "$cid_srv" "$cid_lg" "$out_csv" "$phase_file" "$duration" "$HOST_CPUS" <<'PY' &
import json, os, subprocess, time, sys
from pathlib import Path

cid_srv, cid_lg, out, phase_file, duration, host = sys.argv[1:7]
duration = int(duration); host = int(host)
Path(out).write_text(
  "ts,phase,server_cpu_pct,server_rss_mib,loadgen_cpu_pct,loadgen_rss_mib,"
  "minor_faults,major_faults,voluntary_ctxt,nonvoluntary_ctxt,open_fds,threads\n"
)

def mem_mib(s):
    s = (s or "0").split("/")[0].strip().upper()
    if s.endswith("GIB"): return float(s[:-3]) * 1024
    if s.endswith("MIB"): return float(s[:-3])
    if s.endswith("KIB"): return float(s[:-3]) / 1024
    if s.endswith("B"): return float(s[:-1]) / (1024*1024)
    return 0.0

def cpu_norm(perc):
    try:
        c = float(str(perc).replace("%",""))
    except Exception:
        return None
    if c > 100: c = c / host
    return round(c, 2)

def stats(cid):
    try:
        line = subprocess.check_output(
            ["docker","stats","--no-stream","--format","{{json .}}", cid],
            text=True, stderr=subprocess.DEVNULL).strip()
        row = json.loads(line)
        return cpu_norm(row.get("CPUPerc")), mem_mib(row.get("MemUsage"))
    except Exception:
        return None, None

def proc(cid):
    try:
        pid = subprocess.check_output(
            ["docker","inspect","-f","{{.State.Pid}}", cid], text=True).strip()
        pid = int(pid)
        data = {"minor_faults":None,"major_faults":None,"voluntary_ctxt":None,
                "nonvoluntary_ctxt":None,"open_fds":None,"threads":None}
        st = Path(f"/proc/{pid}/status")
        if st.exists():
            for line in st.read_text().splitlines():
                if line.startswith("voluntary_ctxt_switches:"):
                    data["voluntary_ctxt"]=int(line.split()[1])
                elif line.startswith("nonvoluntary_ctxt_switches:"):
                    data["nonvoluntary_ctxt"]=int(line.split()[1])
                elif line.startswith("Threads:"):
                    data["threads"]=int(line.split()[1])
                elif line.startswith("FDSize:"):
                    data["open_fds"]=int(line.split()[1])
        stat = Path(f"/proc/{pid}/stat")
        if stat.exists():
            parts = stat.read_text().split()
            if len(parts)>12:
                data["minor_faults"]=int(parts[9]); data["major_faults"]=int(parts[11])
        # better fd count
        fd = Path(f"/proc/{pid}/fd")
        if fd.exists():
            try: data["open_fds"]=len(list(fd.iterdir()))
            except Exception: pass
        return data
    except Exception:
        return {}

end = time.time() + duration
with open(out, "a", encoding="utf-8") as f:
    while time.time() < end:
        t0 = time.time()
        phase = Path(phase_file).read_text().strip() if Path(phase_file).exists() else "unknown"
        sc, sr = stats(cid_srv)
        lc, lr = stats(cid_lg)
        pr = proc(cid_srv)
        f.write(
          f"{int(t0)},{phase},{sc},{sr},{lc},{lr},"
          f"{pr.get('minor_faults')},{pr.get('major_faults')},"
          f"{pr.get('voluntary_ctxt')},{pr.get('nonvoluntary_ctxt')},"
          f"{pr.get('open_fds')},{pr.get('threads')}\n"
        )
        f.flush()
        sleep = 1.0 - (time.time() - t0)
        if sleep > 0: time.sleep(sleep)
PY
  echo $! >"$pid_file"
  disown $! 2>/dev/null || true
}

rewrk_once() {
  local url="$1" conns="$2" dur="$3" http2="$4" out="$5"
  local args=(-c "$conns" -d "${dur}s" -h "$url" --json -t "$REWRK_THREADS")
  [[ "$http2" == "1" || "$http2" == "True" || "$http2" == "true" ]] && args+=(--http2)
  if ! docker compose -f "$COMPOSE" exec -T bench-runner \
    env -u NO_COLOR rewrk "${args[@]}" >"$out" 2>/dev/null; then
    echo '{}' >"$out"
    return 1
  fi
  return 0
}

url_for() {
  local sid="$1" path
  path="$(scenario_path "$sid")"
  if [[ "$(load_profile_field "$sid" scheme)" == "https" ]]; then
    echo "https://exyonq:8443${path}"
  else
    echo "http://exyonq:8080${path}"
  fi
}

calibrate_one() {
  local sid="$1"
  local target conns config mode url http2
  target="$(load_profile_field "$sid" target_rps)"
  conns="$(load_profile_field "$sid" connections)"
  config="$(load_profile_field "$sid" config)"
  http2="$(load_profile_field "$sid" http2)"
  [[ "$(load_profile_field "$sid" scheme)" == "https" ]] && mode=https || mode=http
  url="$(url_for "$sid")"
  log "CALIBRATE $sid target_rps=$target start_conns=$conns"

  local ac5c best_conns best_rps best_delta
  ac5c="$(load_profile_field "$sid" ac5_connections)"
  best_conns="$conns"; best_rps=0; best_delta=1e18
  # Broad ladder: open-loop ReWrk often stays near ceiling until conns are low.
  # Compact ladder (30s probes). Open-loop often plateaus; include low conns.
  local candidates
  candidates="$(python3 - "$ac5c" <<'PY'
ac5=int(__import__('sys').argv[1])
fracs=[0.15,0.25,0.35,0.50,0.70,1.0]
vals=sorted({max(10, int(ac5*f)) for f in fracs})
print('\n'.join(str(v) for v in vals))
PY
)"
  local c
  local rows_file="$RESULTS/calibration/${sid}-rows.tsv"
  : >"$rows_file"
  for c in $candidates; do
    recreate_variant system "$config" "$mode" || continue
    local raw="$RESULTS/calibration/${sid}-c${c}.json"
    rewrk_once "$url" "$c" 30 "$([[ $http2 == True || $http2 == true ]] && echo 1 || echo 0)" "$raw" || true
    python3 "$CONVERTER" "$raw" -o "${raw%.json}-conv.json" 2>/dev/null || continue
    local rps
    rps="$(python3 - "${raw%.json}-conv.json" <<'PY'
import json,sys
conv=json.loads(open(sys.argv[1]).read() or "{}")
print((conv.get("summary") or {}).get("requestsPerSec") or 0)
PY
)"
    local delta
    delta="$(python3 -c "print(abs(float('$rps')-float('$target')))")"
    log "  cand conns=$c rps=$rps delta=$delta"
    echo -e "$c\t$rps\t$delta" >>"$rows_file"
  done
  read -r best_conns best_rps pick_note < <(python3 - "$rows_file" "$target" <<'PY'
import sys
from pathlib import Path
rows=[]
for line in Path(sys.argv[1]).read_text().splitlines():
    if not line.strip(): continue
    c,r,d=line.split('\t')
    rows.append((int(c), float(r), float(d)))
target=float(sys.argv[2])
if not rows:
    print(10, 0, "empty"); raise SystemExit
# prefer closest to target among positive rps
pos=[x for x in rows if x[1]>0]
best=min(pos, key=lambda x: x[2])
note="closest_to_target"
if best[1] > target*1.25:
    min_r=min(x[1] for x in pos)
    foot=[x for x in pos if x[1] <= min_r*1.12]
    # among foot, prefer still reasonable load (>=50% of min_r already) and lowest conns
    # Actually prefer conns that get closest BELOW or near target if any
    below=[x for x in pos if x[1] <= target*1.10]
    if below:
        best=min(below, key=lambda x: (abs(x[1]-target), x[0]))
        note="near_or_below_target"
    elif foot:
        best=min(foot, key=lambda x: x[0])
        note="open_loop_plateau_foot"
print(best[0], best[1], note)
PY
)
  log "  pick conns=$best_conns rps=$best_rps note=$pick_note"
  # confirm pick with 120s
  recreate_variant system "$config" "$mode" || true
  local craw="$RESULTS/calibration/${sid}-confirm.json"
  rewrk_once "$url" "$best_conns" 120 "$([[ $http2 == True || $http2 == true ]] && echo 1 || echo 0)" "$craw" || true
  python3 "$CONVERTER" "$craw" -o "${craw%.json}-conv.json" 2>/dev/null || true
  best_rps="$(python3 - "${craw%.json}-conv.json" <<'PY'
import json,sys
conv=json.loads(open(sys.argv[1]).read() or "{}")
print((conv.get("summary") or {}).get("requestsPerSec") or 0)
PY
)"
  log "  confirm 120s conns=$best_conns rps=$best_rps"

  # verify jemalloc+mimalloc short stability at best_conns (60s)
  for v in jemalloc mimalloc; do
    recreate_variant "$v" "$config" "$mode" || { log "calib verify fail $v"; return 1; }
    local raw="$RESULTS/calibration/${sid}-verify-${v}.json"
    rewrk_once "$url" "$best_conns" 60 "$([[ $http2 == True || $http2 == true ]] && echo 1 || echo 0)" "$raw" || true
    python3 "$CONVERTER" "$raw" -o "${raw%.json}-conv.json" 2>/dev/null || true
    local ok
    ok="$(python3 - "${raw%.json}-conv.json" <<'PY'
import json,sys
c=json.loads(open(sys.argv[1]).read() or "{}")
s=c.get("summary") or {}
print(1 if float(s.get("successRate") or 0)>=1.0 and float(s.get("requestsPerSec") or 0)>0 else 0)
PY
)"
    [[ "$ok" == "1" ]] || { log "calib verify FAIL $sid $v"; return 1; }
    log "  verify $v OK"
  done

  python3 - "$PROFILES" "$sid" "$best_conns" "$best_rps" "$target" "$pick_note" <<'PY'
import json,sys
from pathlib import Path
path, sid, conns, rps, target, note = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5]), sys.argv[6]
p=json.loads(path.read_text())
p[sid]["connections"]=conns
p[sid]["calibrated_rps_system_120s"]=rps
p[sid]["target_rps"]=target
p[sid]["calibration_note"]=note
p[sid]["load_mode"]="rewrk_open_loop_connections"
p[sid]["frozen_by"]="ac6_calibration"
p[sid]["rewrk_threads"]=2
p[sid]["server_workers"]=4
p[sid]["accept_workers"]=4
path.write_text(json.dumps(p, indent=2)+"\n")
print(f"FROZEN {sid} conns={conns} calib_rps={rps:.1f} target={target:.1f} note={note}")
PY
  cp "$PROFILES" "$RAW_ROOT/frozen-profiles.json"
}

calibrate_all() {
  if [[ -f "$PROFILES" ]] && grep -q '"frozen_by": "ac6_calibration"' "$PROFILES"; then
    log "profiles already frozen — skipping recalibration"
    return 0
  fi
  init_profiles
  for sid in p1 p3 p4 p6 p11; do
    calibrate_one "$sid" || { log "CALIBRATION FAILED $sid"; exit 3; }
  done
  log "ALL PROFILES FROZEN"
  cat "$PROFILES" | tee -a "$LOG"
}

analyze_run() {
  local rundir="$1"
  python3 - "$rundir" "$HOST_CPUS" "$LOADGEN_STOP" <<'PY'
import csv, json, math, statistics, sys
from pathlib import Path

rundir = Path(sys.argv[1])
host, stop = int(sys.argv[2]), float(sys.argv[3])

def safe_div(a,b):
    return None if b in (0,0.0,None) or a is None else a/b

# timeseries
rows=[]
with open(rundir/"timeseries.csv", newline="", encoding="utf-8") as f:
    for r in csv.DictReader(f):
        rows.append(r)

def fnum(x):
    try:
        if x in (None,"","None"): return None
        return float(x)
    except Exception:
        return None

by_phase={}
for r in rows:
    by_phase.setdefault(r["phase"], []).append(r)

def rss_stats(phase):
    xs=[fnum(r["server_rss_mib"]) for r in by_phase.get(phase,[]) if fnum(r["server_rss_mib"]) is not None]
    if not xs: return {}
    return {"n":len(xs),"min":min(xs),"max":max(xs),"mean":statistics.mean(xs),"median":statistics.median(xs),"final":xs[-1],"first":xs[0]}

idle = rss_stats("idle")
warmup = rss_stats("warmup")
load = rss_stats("load")
rec = [fnum(r["server_rss_mib"]) for r in by_phase.get("recovery",[]) if fnum(r["server_rss_mib"]) is not None]

def at_recovery(sec):
    # approx: recovery samples at 1Hz from start of recovery
    if not rec: return None
    idx=min(len(rec)-1, max(0, sec-1))
    return rec[idx]

rss_idle = idle.get("first") or idle.get("median")
rss_after_wu = warmup.get("final") or warmup.get("median")
rss_load_final = load.get("final")
rss_load_max = load.get("max")
rss_r30 = at_recovery(30)
rss_r60 = at_recovery(60)
rss_r180 = at_recovery(180) if rec else None
if rss_r180 is None and rec: rss_r180 = rec[-1]

growth = None if None in (rss_load_final, rss_after_wu) else rss_load_final - rss_after_wu
retained = None if None in (rss_r180, rss_idle) else rss_r180 - rss_idle
denom = None if None in (rss_load_max, rss_idle) else (rss_load_max - rss_idle)
recovery_ratio = safe_div(None if None in (rss_load_max, rss_r180) else (rss_load_max - rss_r180), denom)

# monotonic: linear slope on second half of load RSS
load_rss=[fnum(r["server_rss_mib"]) for r in by_phase.get("load",[]) if fnum(r["server_rss_mib"]) is not None]
slope = None
if len(load_rss) >= 20:
    half=load_rss[len(load_rss)//2:]
    xs=list(range(len(half)))
    mx=statistics.mean(xs); my=statistics.mean(half)
    num=sum((x-mx)*(y-my) for x,y in zip(xs,half))
    den=sum((x-mx)**2 for x in xs) or 1
    slope=num/den  # MiB per sample (~per second)

# loadgen peak
lg=[fnum(r["loadgen_cpu_pct"]) for r in rows if fnum(r["loadgen_cpu_pct"]) is not None]
lg_peak=max(lg) if lg else None

# windows — only canonical wNN.json (exclude wNN-raw.json / wNN-conv.json)
windows=[]
for p in sorted(rundir.glob("windows/w[0-9][0-9].json")):
    windows.append(json.loads(p.read_text()))

rps_list=[w["rps"] for w in windows if w.get("rps")]
p99_list=[w["p99_ms"] for w in windows if w.get("p99_ms") is not None]
success_all=all(float(w.get("success_rate") or 0)>=1.0 for w in windows) if windows else False
err_sum=sum(int(w.get("errors") or 0) for w in windows)

def cv(xs):
    if not xs or statistics.mean(xs)==0: return None
    return statistics.pstdev(xs)/statistics.mean(xs)*100 if len(xs)>1 else 0.0

half=len(rps_list)//2
first_rps=statistics.mean(rps_list[:half]) if half else None
second_rps=statistics.mean(rps_list[half:]) if half else None
first_p99=statistics.mean(p99_list[:half]) if half and p99_list else None
second_p99=statistics.mean(p99_list[half:]) if half and p99_list else None

degrade_rps = first_rps and second_rps and second_rps < first_rps * 0.95
degrade_p99 = first_p99 and second_p99 and second_p99 > first_p99 * 1.10

# memory class
mem_class="STABLE"
if slope is not None and slope > 0.01 and growth and growth > 0 and (rss_after_wu and rss_load_final and rss_load_final > rss_after_wu*1.10):
    mem_class="MONOTONIC_GROWTH"
elif retained is not None and rss_idle and retained > 0.10*rss_idle:
    mem_class="RETAINED_MEMORY"
elif growth is not None and rss_after_wu and growth > 0.10*rss_after_wu and retained is not None and rss_idle and retained <= 0.10*rss_idle:
    mem_class="TRANSIENT_GROWTH_RECOVERED"
elif rss_load_max and rss_idle and rss_load_max > rss_idle * 3 and recovery_ratio and recovery_ratio < 0.3:
    mem_class="ABNORMAL_SPIKE"

creep_suspected=False
if rss_after_wu and rss_load_final and rss_load_final > rss_after_wu*1.10: creep_suspected=True
if rss_idle and rss_r180 and rss_r180 > rss_idle*1.10: creep_suspected=True
if slope is not None and slope > 0.005: creep_suspected=True

classification="VALID_RUN"
reason=""
if lg_peak is not None and lg_peak >= stop:
    classification="LOADGEN_SATURATED"; reason=f"lg_peak={lg_peak}"
elif not windows or not success_all or err_sum>0:
    classification="SERVER_FAILURE"; reason=f"success/windows errors={err_sum}"
elif not rps_list or statistics.mean(rps_list)<=0:
    classification="HARNESS_FAILURE"; reason="no rps"
# sampler gap
ts=[int(r["ts"]) for r in rows if r.get("ts")]
gaps=[b-a for a,b in zip(ts,ts[1:])]
if gaps and max(gaps)>10:
    classification="SAMPLER_INVALID"; reason=f"gap={max(gaps)}"

# target deviation from profile
prof=json.loads((rundir/"profile.json").read_text())
target=float(prof.get("target_rps") or 0)
achieved=statistics.median(rps_list) if rps_list else None
# Open-loop ReWrk may exceed TARGET_RPS (connection plateaus). Fail only if starved.
if target and achieved and achieved < target * 0.40:
    classification="TARGET_LOAD_INVALID"; reason=f"achieved={achieved} target={target}"

summary={
  "classification": classification, "reject_reason": reason,
  "achieved_rps_median": achieved, "target_rps": target,
  "rps_window_cv": cv(rps_list), "p99_window_cv": cv(p99_list),
  "first_half_rps": first_rps, "second_half_rps": second_rps,
  "first_half_p99": first_p99, "second_half_p99": second_p99,
  "degrade_rps": bool(degrade_rps), "degrade_p99": bool(degrade_p99),
  "loadgen_cpu_peak": lg_peak,
  "RSS_IDLE_INITIAL": rss_idle,
  "RSS_AFTER_WARMUP": rss_after_wu,
  "RSS_LOAD_MIN": load.get("min"), "RSS_LOAD_MEAN": load.get("mean"),
  "RSS_LOAD_MEDIAN": load.get("median"), "RSS_LOAD_MAX": rss_load_max,
  "RSS_LOAD_FINAL": rss_load_final,
  "RSS_RECOVERY_30S": rss_r30, "RSS_RECOVERY_60S": rss_r60, "RSS_RECOVERY_180S": rss_r180,
  "RSS_GROWTH_DURING_LOAD": growth,
  "RSS_RETAINED_AFTER_RECOVERY": retained,
  "RSS_RECOVERY_RATIO": recovery_ratio,
  "rss_slope_second_half": slope,
  "memory_class": mem_class,
  "RAM_CREEP_SUSPECTED": creep_suspected,
  "windows_n": len(windows),
  "success_all": success_all,
}
(rundir/"summary.json").write_text(json.dumps(summary, indent=2)+"\n")
(rundir/"classification.txt").write_text(classification+"\n")
print(classification, "rps_med=", achieved, "retained=", retained, "creep_sus=", creep_suspected, "mem=", mem_class)
sys.exit(0 if classification=="VALID_RUN" else 10)
PY
}

run_one() {
  local scenario="$1" variant="$2" round="$3" order="$4"
  local conns config scheme http2 target url mode
  conns="$(load_profile_field "$scenario" connections)"
  config="$(load_profile_field "$scenario" config)"
  scheme="$(load_profile_field "$scenario" scheme)"
  http2="$(load_profile_field "$scenario" http2)"
  target="$(load_profile_field "$scenario" target_rps)"
  [[ "$scheme" == "https" ]] && mode=https || mode=http
  url="$(url_for "$scenario")"
  local h2flag=0
  [[ "$http2" == "True" || "$http2" == "true" ]] && h2flag=1

  local tag="ac6-${scenario}-r${round}-${variant}-o${order}"
  local rundir="$RESULTS/runs/$tag"
  mkdir -p "$rundir/windows"
  log "RUN $tag conns=$conns"

  # identity
  {
    echo "ALLOCATOR_VARIANT=$variant"
    grep -E '^(BINARY_SHA256|CONTAINER_IMAGE_ID|SOURCE_COMMIT|RUSTC_VERSION|TARGET_TRIPLE|TARGET_CPU|CARGO_PROFILE|CARGO_FEATURES|BINARY_SIZE)=' \
      "$ID_DIR/identity-$variant.txt"
    echo "REWRK_SHA256=$EXPECTED_REWRK_SHA"
    echo "HOST_ID=$HOST_ID"
    echo "SCENARIO=$scenario"
    echo "RUN_INDEX=$round"
    echo "RUN_ORDER=$order"
  } >"$rundir/identity.txt"
  grep SOURCE_COMMIT "$rundir/identity.txt" | grep -q "$SOURCE_COMMIT_EXPECTED" || {
    echo IDENTITY_MISMATCH >"$rundir/classification.txt"
    return 1
  }
  cp "$PROFILES" "$rundir/profiles-all.json"
  python3 -c "import json; p=json.load(open('$PROFILES'))['$scenario']; json.dump(p, open('$rundir/profile.json','w'), indent=2)"

  if ! recreate_variant "$variant" "$config" "$mode"; then
    echo SERVER_FAILURE >"$rundir/classification.txt"
    return 1
  fi

  local cid_srv cid_lg
  cid_srv="$(docker compose -f "$COMPOSE" ps -q exyonq)"
  cid_lg="$(docker compose -f "$COMPOSE" ps -q bench-runner)"
  local phase_file="$rundir/phase.txt"
  local sampler_pid_file="$rundir/sampler.pid"
  echo idle >"$phase_file"
  # windows may overrun WINDOW_S slightly; keep sampler alive with slack
  local total=$((10 + WARMUP_S + MEASURE_S + RECOVERY_S + WINDOWS * 5 + 120))
  local sampler_pid
  start_timeseries "$cid_srv" "$cid_lg" "$rundir/timeseries.csv" "$phase_file" "$total" "$sampler_pid_file"
  sampler_pid="$(cat "$sampler_pid_file")"
  sleep 10  # idle baseline

  echo warmup >"$phase_file"
  rewrk_once "$url" "$conns" "$WARMUP_S" "$h2flag" "$rundir/warmup-rewrk.json" || true
  python3 "$CONVERTER" "$rundir/warmup-rewrk.json" -o "$rundir/warmup-converted.json" 2>/dev/null || true

  echo load >"$phase_file"
  local w
  for w in $(seq 1 "$WINDOWS"); do
    local wraw="$rundir/windows/w$(printf '%02d' "$w")-raw.json"
    rewrk_once "$url" "$conns" "$WINDOW_S" "$h2flag" "$wraw" || true
    python3 "$CONVERTER" "$wraw" -o "$rundir/windows/w$(printf '%02d' "$w")-conv.json" 2>/dev/null || echo '{}' >"$rundir/windows/w$(printf '%02d' "$w")-conv.json"
    python3 - "$rundir/windows/w$(printf '%02d' "$w")-conv.json" "$rundir/windows/w$(printf '%02d' "$w").json" "$w" <<'PY'
import json,sys
from pathlib import Path
conv=json.loads(Path(sys.argv[1]).read_text() or "{}")
out=Path(sys.argv[2]); idx=int(sys.argv[3])
s=conv.get("summary") or {}; lat=conv.get("latencyPercentiles") or {}; r=conv.get("rewrk") or {}
payload={
  "window": idx,
  "rps": s.get("requestsPerSec"),
  "success_rate": s.get("successRate"),
  "total": s.get("total"),
  "p50_ms": (lat["p50"]*1000 if lat.get("p50") is not None else None),
  "p95_ms": (lat["p95"]*1000 if lat.get("p95") is not None else None),
  "p99_ms": (lat["p99"]*1000 if lat.get("p99") is not None else None),
  "throughput_bytes_s": r.get("transfer_rate"),
  "errors": 0 if float(s.get("successRate") or 0)>=1.0 else 1,
  "timeouts": 0,
}
out.write_text(json.dumps(payload, indent=2)+"\n")
print(f"  window {idx} rps={payload['rps']}")
PY
  done

  echo recovery >"$phase_file"
  sleep "$RECOVERY_S"

  # stop sampler
  kill "$sampler_pid" 2>/dev/null || true
  wait "$sampler_pid" 2>/dev/null || true

  if analyze_run "$rundir"; then
    :
  else
    cp -a "$rundir" "$RESULTS/rejected/$tag" 2>/dev/null || true
    docker compose -f "$COMPOSE" stop exyonq >/dev/null 2>&1 || true
    sleep 5
    return 1
  fi

  # manifest
  python3 - "$MANIFEST" "$tag" "$scenario" "$variant" "$round" "$order" "$rundir/summary.json" <<'PY'
import json,sys
from pathlib import Path
mani, tag, sid, var, rnd, order, summ = sys.argv[1:8]
s=json.loads(Path(summ).read_text())
hdr = not Path(mani).exists()
if hdr:
  Path(mani).write_text(
    "tag\tscenario\tvariant\tround\torder\tclassification\trps_med\tp99_cv\trss_retained\tcreep_sus\tmem_class\treason\n"
  )
line=(f"{tag}\t{sid}\t{var}\t{rnd}\t{order}\t{s.get('classification')}\t{s.get('achieved_rps_median')}\t"
      f"{s.get('p99_window_cv')}\t{s.get('RSS_RETAINED_AFTER_RECOVERY')}\t{s.get('RAM_CREEP_SUSPECTED')}\t"
      f"{s.get('memory_class')}\t{s.get('reject_reason')}\n")
open(mani,"a").write(line)
PY

  mkdir -p "$RAW_ROOT/$scenario/$variant"
  cp -a "$rundir" "$RAW_ROOT/$scenario/$variant/r${round}-o${order}"

  docker compose -f "$COMPOSE" stop exyonq >/dev/null 2>&1 || true
  sleep 5
  return 0
}

ROUND_ORDERS=(
  "system jemalloc mimalloc"
  "jemalloc mimalloc system"
  "mimalloc system jemalloc"
)

run_matrix() {
  local scenarios=(p1 p3 p4 p6 p11)
  for sid in "${scenarios[@]}"; do
    log "=== SCENARIO $sid ==="
    local round=1
    for order_line in "${ROUND_ORDERS[@]}"; do
      local order=1
      for variant in $order_line; do
        local tries=0 ok=0
        while [[ $tries -lt 2 ]]; do
          tries=$((tries+1))
          if run_one "$sid" "$variant" "$round" "$order"; then ok=1; break; fi
          log "retry $sid $variant r$round"
          sleep 10
        done
        [[ "$ok" -eq 1 ]] || log "FAILED $sid $variant r$round"
        order=$((order+1))
      done
      round=$((round+1))
    done
  done
}

PHASE="${1:-all}"
session_certify

case "$PHASE" in
  calibrate)
    calibrate_all
    ;;
  matrix)
    [[ -f "$PROFILES" ]] || { log "missing profiles — run calibrate first"; exit 1; }
    grep -q '"frozen_by": "ac6_calibration"' "$PROFILES" || {
      log "profiles not frozen — refuse matrix"; exit 1
    }
    run_matrix
    ;;
  all)
    calibrate_all
    grep -q '"frozen_by": "ac6_calibration"' "$PROFILES" || {
      log "profiles not frozen after calibrate"; exit 1
    }
    run_matrix
    ;;
  *)
    echo "usage: $0 [all|calibrate|matrix]" >&2
    exit 1
    ;;
esac

log "AC6 phase=$PHASE DONE"
cp "$PROFILES" "$RAW_ROOT/frozen-profiles.json" 2>/dev/null || true
cp "$MANIFEST" "$RAW_ROOT/runs-manifest.tsv" 2>/dev/null || true
