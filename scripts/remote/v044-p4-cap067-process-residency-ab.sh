#!/usr/bin/env bash
# V044_P4_CAP067_PROCESS_RESIDENCY_COST_AB
# READ_ONLY_CONFIGURATION_AB — PRODUCT_MUTATION=NO
# Independent variable: EXYONQ_EPOLL_STATIC default ON (A0) vs =0 (A1)
# Fixed: EXYONQ_WORKER_THREADS=4, EXYONQ_ACCEPT_WORKERS=8
set -euo pipefail

WS="${CP_WS:-/root/pxdp-p5-reality-wt}"
TS="${CP_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${CP_EV:-$WS/.exyonq-local/evidence/p4-cap067-process-residency/$TS}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-20260826-223445}"
P5_IMAGE="${PXDP_P5_IMAGE:-v044-pxdp-p5-reality-exyonq}"
P5_SHA="${PXDP_P5_SHA256:-96aa8c4f483e98fc59dfeec42b31bc0d4986db2a3f00be10617178d561fc4e05}"
SOURCE_HEAD="${CP_SOURCE_HEAD:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
SOURCE_TREE="${CP_SOURCE_TREE:-b1b7659b4d3bb8745ea0ed42a8de7d424f523767}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PATH_P4="/api/"
# Cap067 lazy-start trigger (static path); not the measured workload
PATH_STATIC_TRIGGER="/site/"
WARMUP=20
MEASURE=30
CONC=100
THREADS=2
REPS="${CP_REPS:-5}"
TOKIO_W=4
ACCEPT_W=8

mkdir -p "$EV"/{meta,correctness,runs,perf-stat,cgroup,mpstat,thread-inventory,sanity,reports,commands,schedule}
log() { echo "[cp067] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  [[ -f "$EV/meta/compose-cp.yml" ]] && extra+=(-f "$EV/meta/compose-cp.yml")
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@" </dev/null
}

ctr_exyonq() {
  docker ps --filter "label=com.docker.compose.project=$PROJECT" --filter "name=exyonq" -q | head -1
}

url_exy() { echo "http://exyonq:8080${PATH_P4}"; }
url_static() { echo "http://exyonq:8080${PATH_STATIC_TRIGGER}"; }

main_pid() { docker inspect -f '{{.State.Pid}}' "$1"; }

resolve_all_tids() {
  local main=$1
  mapfile -t tids < <(ls "/proc/$main/task" 2>/dev/null | sort -n)
  local out=()
  for t in "${tids[@]}"; do [[ -d "/proc/$t" ]] && out+=("$t"); done
  (IFS=,; echo "${out[*]}")
}

write_authority() {
  cat >"$EV/meta/source-control-semantics.txt" <<'TXT'
CAP067_POOL_ENABLE_CONTROL = EXYONQ_EPOLL_STATIC
  default = ON for cleartext (unset / empty / anything except disabled tokens)
  kill-switch = 0 | off | false | no  (core/src/server/epoll_start.rs epoll_static_env_enabled)
  process-lifetime cache; requires process restart to flip

CAP067_POOL_THREAD_CONTROL = EXYONQ_EPOLL_POOL_THREADS
  unset → follow accept_workers
  EXYONQ_EPOLL_POOL_THREADS=0 → parse fails positive → falls back to accept (DOES NOT DISABLE)

IS_ZERO_ALLOWED_ON_EPOLL_STATIC = YES (as disable token)
WHAT_DOES_ZERO_MEAN_ON_EPOLL_STATIC = disable Cap067 epoll static/keepalive mechanism
DOES_ZERO_DISABLE_POOL_CREATION = YES (epoll_keepalive_active=false → no prepare/start)
DOES_ZERO_FALL_BACK_TO_DEFAULT = NO
DOES_ZERO_DISABLE_CAP067_PRODUCT_CAPABILITY = YES (static/sendfile Cap067 path off)
  This WIP measures P4 /api/ only; static Cap067 value is out of scope.

EXYONQ_EPOLL_STATIC_THREADS = DOES_NOT_EXIST (no such env)
TXT
  {
    echo "WIP=V044_P4_CAP067_PROCESS_RESIDENCY_COST_AB"
    echo "MODE=READ_ONLY_CONFIGURATION_AB_AND_CAUSAL_VALIDATION"
    echo "PRODUCT_MUTATION=NO"
    echo "PARENT_CASE=RT-C"
    echo "SOURCE_HEAD=$SOURCE_HEAD"
    echo "SOURCE_TREE=$SOURCE_TREE"
    echo "P5_IMAGE=$P5_IMAGE"
    echo "COMPOSE_PROJECT=$PROJECT"
    echo "CPUSET=0-7"
    echo "TOKIO_WORKER_THREADS=$TOKIO_W"
    echo "ACCEPT_WORKERS=$ACCEPT_W"
    echo "INDEPENDENT_VARIABLE=EXYONQ_EPOLL_STATIC A0=default_ON A1=0"
    echo "REPS=$REPS"
    uname -a
  } | tee "$EV/meta/authority.txt"
}

# variant: a0 | a1
write_compose_overlay() {
  local variant=$1
  local epoll_static=""
  if [[ "$variant" == "a1" ]]; then
    epoll_static="0"
  fi
  cat >"$EV/meta/compose-cp.yml" <<EOF
services:
  exyonq:
    image: ${P5_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
      EXYONQ_WORKER_THREADS: "${TOKIO_W}"
      EXYONQ_ACCEPT_WORKERS: "${ACCEPT_W}"
      EXYONQ_EPOLL_POOL_THREADS: "${ACCEPT_W}"
      EXYONQ_EPOLL_STATIC: "${epoll_static}"
      EXYONQ_EPOLL_LISTEN: ""
      EXYONQ_SYNC_ACCEPT: ""
EOF
  cp "$EV/meta/compose-cp.yml" "$EV/meta/compose-${variant}.yml"
}

apply_variant() {
  local variant=$1
  log "apply variant=$variant (WT=$TOKIO_W AW=$ACCEPT_W EPOLL_STATIC=$([[ $variant == a1 ]] && echo 0 || echo default_ON))"
  write_compose_overlay "$variant"
  compose up -d --no-deps --force-recreate exyonq
  sleep 8
  local c
  c=$(ctr_exyonq)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null 2>&1 || true
}

# Classify threads: Cap067 OS workers sit in ep_poll; Tokio workers typically futex when idle.
# Cap067 inherits name tokio-rt-worker (RT-C).
record_thread_inventory() {
  local variant=$1 tag=$2
  local c main out="$EV/thread-inventory/${variant}-${tag}.txt"
  c=$(ctr_exyonq)
  main=$(main_pid "$c")
  {
    echo "VARIANT=$variant TAG=$tag MAIN_PID=$main"
    echo "EXYONQ_WORKER_THREADS=$TOKIO_W"
    echo "EXYONQ_ACCEPT_WORKERS=$ACCEPT_W"
    docker exec "$c" sh -c 'env | grep -E "^EXYONQ_(WORKER|ACCEPT|EPOLL)" | sort'
    echo "---tasks---"
    local total=0 tokio_named=0 ep_poll=0 futex=0 notify=0 other=0
    for tid in $(ls "/proc/$main/task" 2>/dev/null | sort -n); do
      [[ -f "/proc/$tid/status" ]] || continue
      local name wchan
      name=$(awk '/^Name:/ {print $2}' "/proc/$tid/status")
      wchan=$(cat "/proc/$tid/wchan" 2>/dev/null || echo "?")
      echo "TID=$tid COMM=$name WCHAN=$wchan"
      total=$((total + 1))
      case "$name" in
        notify-rs*) notify=$((notify + 1)) ;;
        tokio-rt-worker)
          tokio_named=$((tokio_named + 1))
          if [[ "$wchan" == *ep_poll* ]] || [[ "$wchan" == "ep_poll" ]]; then
            ep_poll=$((ep_poll + 1))
          elif [[ "$wchan" == *futex* ]]; then
            futex=$((futex + 1))
          fi
          ;;
        *) other=$((other + 1)) ;;
      esac
    done
    echo "TOTAL_OS_THREADS=$total"
    echo "TOKIO_NAMED_COUNT=$tokio_named"
    echo "WCHAN_EP_POLL_COUNT=$ep_poll"
    echo "WCHAN_FUTEX_COUNT=$futex"
    echo "NOTIFY_COUNT=$notify"
    echo "OTHER_COUNT=$other"
    # Cap067 OS workers inherit name tokio-rt-worker (RT-C). Tokio itself may also
    # sit in ep_poll, so wchan alone is NOT Cap067-proof. Use named surplus vs WT.
    local cap067_est=0
    if [[ "$tokio_named" -gt "$TOKIO_W" ]]; then
      cap067_est=$((tokio_named - TOKIO_W))
    fi
    echo "CAP067_THREAD_ESTIMATE=$cap067_est"
    echo "TOKIO_CORE_ESTIMATE=$TOKIO_W"
  } | tee "$out"

  # Listen log snippet (fresh container only — truncate-safe via since recreate)
  docker logs "$(ctr_exyonq)" 2>&1 | grep -E "exyonq listening|Cap067 keepalive|epoll" | tail -20 \
    | tee "$EV/thread-inventory/${variant}-${tag}-logs.txt" || true
}

# Ensure Cap067 pool is actually started for A0 (lazy on first static handoff).
maybe_activate_cap067() {
  local variant=$1
  if [[ "$variant" != "a0" ]]; then
    return 0
  fi
  log "A0: trigger Cap067 lazy start via static path ${PATH_STATIC_TRIGGER}"
  compose exec -T bench-runner curl -sS -m 5 -o /dev/null -w '%{http_code}\n' "$(url_static)" \
    | tee -a "$EV/commands/cap067-activate.log" || true
  # Also try index under site if needed
  compose exec -T bench-runner curl -sS -m 5 -o /dev/null -w '%{http_code}\n' "http://exyonq:8080/site/index.html" \
    | tee -a "$EV/commands/cap067-activate.log" || true
  sleep 1
}

assert_pool_state() {
  local variant=$1
  record_thread_inventory "$variant" "post-start"
  local inv="$EV/thread-inventory/${variant}-post-start.txt"
  local cap067 named total
  cap067=$(awk -F= '/^CAP067_THREAD_ESTIMATE=/ {print $2}' "$inv")
  named=$(awk -F= '/^TOKIO_NAMED_COUNT=/ {print $2}' "$inv")
  total=$(awk -F= '/^TOTAL_OS_THREADS=/ {print $2}' "$inv")
  local listen_log="$EV/thread-inventory/${variant}-post-start-logs.txt"
  if [[ "$variant" == "a0" ]]; then
    if ! grep -q 'epoll_keepalive=true' "$listen_log" 2>/dev/null; then
      log "ERROR: a0 listen log missing epoll_keepalive=true"
      echo "A0_CAP067_POOL_PRESENT=NO" | tee "$EV/meta/pool-assert-a0.txt"
      return 1
    fi
    if ! grep -q 'Cap067 keepalive pool geometry' "$listen_log" 2>/dev/null; then
      log "ERROR: a0 missing Cap067 geometry log"
      echo "A0_CAP067_POOL_PRESENT=NO" | tee "$EV/meta/pool-assert-a0.txt"
      return 1
    fi
    # Expect ~ACCEPT_W Cap067 OS workers (named surplus)
    if [[ "${cap067:-0}" -lt "$ACCEPT_W" ]]; then
      log "ERROR: A0 Cap067 pool undersized estimate=$cap067 want>=$ACCEPT_W named=$named total=$total"
      echo "A0_CAP067_POOL_PRESENT=NO THREADS=$cap067" | tee "$EV/meta/pool-assert-a0.txt"
      return 1
    fi
    echo "A0_CAP067_POOL_PRESENT=YES A0_CAP067_THREADS=$cap067 A0_TOTAL=$total A0_TOKIO_NAMED=$named" \
      | tee "$EV/meta/pool-assert-a0.txt"
  else
    # A1: kill-switch must clear keepalive flag and Cap067 geometry log.
    # Do NOT use wchan=ep_poll alone — Tokio workers also epoll.
    if grep -q 'epoll_keepalive=true' "$listen_log" 2>/dev/null; then
      log "ERROR: A1 still has epoll_keepalive=true"
      echo "A1_CAP067_POOL_PRESENT=YES_UNEXPECTED" | tee "$EV/meta/pool-assert-a1.txt"
      return 1
    fi
    if ! grep -q 'epoll_keepalive=false' "$listen_log" 2>/dev/null; then
      log "ERROR: A1 missing epoll_keepalive=false"
      echo "A1_CAP067_POOL_PRESENT=UNKNOWN" | tee "$EV/meta/pool-assert-a1.txt"
      return 1
    fi
    if grep -q 'Cap067 keepalive pool geometry' "$listen_log" 2>/dev/null; then
      log "ERROR: A1 still logged Cap067 geometry"
      echo "A1_CAP067_POOL_PRESENT=YES_UNEXPECTED" | tee "$EV/meta/pool-assert-a1.txt"
      return 1
    fi
    if [[ "${named:-0}" -ne "$TOKIO_W" ]]; then
      log "ERROR: A1 tokio-named=$named want=$TOKIO_W (Cap067 surplus residual?)"
      echo "A1_CAP067_POOL_PRESENT=YES_UNEXPECTED NAMED=$named" | tee "$EV/meta/pool-assert-a1.txt"
      return 1
    fi
    if [[ "${cap067:-0}" -ne 0 ]]; then
      log "ERROR: A1 Cap067 estimate=$cap067 want=0"
      echo "A1_CAP067_POOL_PRESENT=YES_UNEXPECTED THREADS=$cap067" | tee "$EV/meta/pool-assert-a1.txt"
      return 1
    fi
    echo "A1_CAP067_POOL_PRESENT=NO A1_CAP067_THREADS=0 A1_TOTAL=$total A1_TOKIO_NAMED=$named" \
      | tee "$EV/meta/pool-assert-a1.txt"
  fi
}

pxdp_sanity() {
  local variant=$1 fail=0
  docker inspect "$(ctr_exyonq)" --format '{{range .Config.Env}}{{println .}}{{end}}' \
    | tee "$EV/meta/docker-env-${variant}.txt"
  grep -q 'EXYONQ_PXDP_P4=1' "$EV/meta/docker-env-${variant}.txt" || fail=1
  grep -q 'EXYONQ_PXDP_P2=1' "$EV/meta/docker-env-${variant}.txt" || fail=1
  grep -q "EXYONQ_WORKER_THREADS=${TOKIO_W}" "$EV/meta/docker-env-${variant}.txt" || fail=1
  grep -q "EXYONQ_ACCEPT_WORKERS=${ACCEPT_W}" "$EV/meta/docker-env-${variant}.txt" || fail=1
  if [[ "$variant" == "a1" ]]; then
    grep -q 'EXYONQ_EPOLL_STATIC=0' "$EV/meta/docker-env-${variant}.txt" || fail=1
  fi
  local url out code size
  url=$(url_exy)
  out=$(compose exec -T bench-runner curl -sS -m 10 -o "/tmp/${variant}-body.bin" -w '%{http_code} %{size_download}' "$url")
  code=${out%% *}; size=${out##* }
  echo "${variant}_probe=$out" | tee -a "$EV/sanity/correctness-${variant}.txt"
  [[ "$code" == "200" && "$size" == "1024" ]] || fail=1
  compose exec -T bench-runner sha256sum "/tmp/${variant}-body.bin" | tee -a "$EV/sanity/correctness-${variant}.txt"
  compose exec -T bench-runner bash -lc "
    url='$url'
    for i in \$(seq 1 100); do
      code=\$(curl -sf -m 5 -o /dev/null -w '%{http_code}' -H 'Connection: keep-alive' \"\$url\") || code=000
      [[ \"\$code\" == \"200\" ]] || { echo FAIL seq=\$i code=\$code; exit 1; }
    done
    echo PASS_100seq
  " | tee -a "$EV/sanity/correctness-${variant}.txt" || fail=1
  if [[ "$fail" -ne 0 ]]; then
    echo "CORRECTNESS_${variant}=FAIL" | tee -a "$EV/correctness/matrix.txt"
    return 1
  fi
  echo "CORRECTNESS_${variant}=PASS PXDP_ENV_OK=YES" | tee -a "$EV/correctness/matrix.txt"
}

run_warmup() {
  compose exec -T bench-runner rewrk -c "$CONC" -d "${WARMUP}s" -t "$THREADS" -h "$(url_exy)" >/dev/null 2>&1 || true
}

one_measure() {
  local variant=$1 rep=$2
  local url c pids out="$EV/perf-stat/${variant}-rep${rep}.txt"
  url=$(url_exy)
  c=$(ctr_exyonq)
  pids=$(resolve_all_tids "$(main_pid "$c")")
  echo "variant=$variant rep=$rep pids=$pids" >>"$EV/commands/perf-scope.log"
  run_warmup
  # refresh TID list after warmup (Cap067 already started for a0)
  pids=$(resolve_all_tids "$(main_pid "$c")")
  local id path tmp lp
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  cp "$path" "$EV/cgroup/${variant}-rep${rep}.before"
  # RSS
  awk '/^VmRSS:/ {print $2}' "/proc/$(main_pid "$c")/status" >"$EV/cgroup/${variant}-rep${rep}.rss_kb_before" || true
  tmp=$(mktemp)
  compose exec -T bench-runner rewrk -c "$CONC" -d "${MEASURE}s" -t "$THREADS" -h "$url" --json >"$tmp" 2>/dev/null &
  lp=$!
  sleep 2
  # Prefer process-wide: attach to main PID (all threads) to avoid stale TID lists
  perf stat -e task-clock,cpu-clock,cycles,instructions,context-switches,cpu-migrations,page-faults \
    -p "$(main_pid "$c")" -- sleep "$MEASURE" >"$out" 2>&1 || true
  wait "$lp" 2>/dev/null || true
  cp "$path" "$EV/cgroup/${variant}-rep${rep}.after"
  awk '/^VmRSS:/ {print $2}' "/proc/$(main_pid "$c")/status" >"$EV/cgroup/${variant}-rep${rep}.rss_kb_after" || true
  cp "$tmp" "$EV/runs/${variant}-rep${rep}-rewrk.json"
  rm -f "$tmp"
  # mpstat snapshot
  mpstat -P ALL 1 3 >"$EV/mpstat/${variant}-rep${rep}.txt" 2>&1 || true

  python3 - "$variant" "$rep" "$EV" <<'PY'
import json, re, sys
from pathlib import Path
variant, rep, ev = sys.argv[1], sys.argv[2], Path(sys.argv[3])
rewrk = json.loads((ev / "runs" / f"{variant}-rep{rep}-rewrk.json").read_text())
rps = float(rewrk.get("requests_avg") or 0)
req = float(rewrk.get("requests_total") or 0)

def parse_cpu(p):
    d = {}
    for ln in Path(p).read_text().splitlines():
        ps = ln.split()
        if len(ps) >= 2:
            d[ps[0]] = int(ps[1])
    return d

b = parse_cpu(ev / "cgroup" / f"{variant}-rep{rep}.before")
a = parse_cpu(ev / "cgroup" / f"{variant}-rep{rep}.after")
du = a.get("usage_usec", 0) - b.get("usage_usec", 0)
uu = a.get("user_usec", 0) - b.get("user_usec", 0)
su = a.get("system_usec", 0) - b.get("system_usec", 0)
text = (ev / "perf-stat" / f"{variant}-rep{rep}.txt").read_text()
vals = {}
for line in text.splitlines():
    m = re.match(r"\s*([\d,]+(?:\.\d+)?)\s+(\S+)", line)
    if not m:
        continue
    v, k = m.group(1).replace(",", ""), m.group(2)
    if k in ("context-switches", "cpu-migrations", "cycles", "instructions", "page-faults"):
        vals[k.replace("-", "_")] = int(float(v))
rss_b = int((ev / "cgroup" / f"{variant}-rep{rep}.rss_kb_before").read_text().strip() or "0")
rss_a = int((ev / "cgroup" / f"{variant}-rep{rep}.rss_kb_after").read_text().strip() or "0")
out = {
    "variant": variant,
    "rep": int(rep),
    "rps": rps,
    "requests_total": req,
    "p50_us": rewrk.get("latency_p50_us"),
    "p95_us": rewrk.get("latency_p95_us"),
    "p99_us": rewrk.get("latency_p99_us"),
    "total_us_per_req": (du / req) if req else None,
    "user_us_per_req": (uu / req) if req else None,
    "system_us_per_req": (su / req) if req else None,
    "ctx_per_req": (vals.get("context_switches", 0) / req) if req else None,
    "migrations_per_req": (vals.get("cpu_migrations", 0) / req) if req else None,
    "rss_kb_before": rss_b,
    "rss_kb_after": rss_a,
    "perf": vals,
}
(ev / "runs" / f"{variant}-rep{rep}-summary.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps({"variant": variant, "rep": rep, "rps": rps, "ctx_per_req": out["ctx_per_req"]}))
PY
}

build_schedule() {
  # Interleave A0 A1 A1 A0 ... for REPS each
  local a0=0 a1=0 i=0
  : >"$EV/schedule/run-order.txt"
  while [[ $a0 -lt $REPS || $a1 -lt $REPS ]]; do
    if (( i % 2 == 0 )); then
      if [[ $a0 -lt $REPS ]]; then
        echo "a0 $((a0 + 1))" >>"$EV/schedule/run-order.txt"
        a0=$((a0 + 1))
      elif [[ $a1 -lt $REPS ]]; then
        echo "a1 $((a1 + 1))" >>"$EV/schedule/run-order.txt"
        a1=$((a1 + 1))
      fi
    else
      if [[ $a1 -lt $REPS ]]; then
        echo "a1 $((a1 + 1))" >>"$EV/schedule/run-order.txt"
        a1=$((a1 + 1))
      elif [[ $a0 -lt $REPS ]]; then
        echo "a0 $((a0 + 1))" >>"$EV/schedule/run-order.txt"
        a0=$((a0 + 1))
      fi
    fi
    i=$((i + 1))
  done
  # Prefer pattern A0 A1 A1 A0 when starting: rewrite to classic nest
  python3 - "$EV/schedule/run-order.txt" "$REPS" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1]); reps = int(sys.argv[2])
# Classic interleave: A0,A1,A1,A0,A0,A1,...
order = []
a0 = a1 = 0
pattern = ["a0", "a1", "a1", "a0"]
pi = 0
while a0 < reps or a1 < reps:
    v = pattern[pi % 4]
    pi += 1
    if v == "a0" and a0 < reps:
        a0 += 1
        order.append(f"a0 {a0}")
    elif v == "a1" and a1 < reps:
        a1 += 1
        order.append(f"a1 {a1}")
    else:
        # skip if that side done
        continue
path.write_text("\n".join(order) + "\n")
print(path.read_text())
PY
}

summarize() {
  python3 - "$EV" <<'PY'
import json, statistics
from pathlib import Path
ev = Path(__import__("sys").argv[1])

def median_field(variant, field):
    vals = []
    for p in sorted(ev.glob(f"runs/{variant}-rep*-summary.json")):
        d = json.loads(p.read_text())
        if d.get(field) is not None:
            vals.append(d[field])
    if not vals:
        return None
    return statistics.median(vals)

def pct(a, b):
    if a is None or b is None or a == 0:
        return None
    return (b - a) / a * 100.0

out = {
    "a0": {k: median_field("a0", k) for k in [
        "rps", "total_us_per_req", "user_us_per_req", "system_us_per_req",
        "ctx_per_req", "migrations_per_req", "p50_us", "p95_us", "p99_us",
        "rss_kb_after"]},
    "a1": {k: median_field("a1", k) for k in [
        "rps", "total_us_per_req", "user_us_per_req", "system_us_per_req",
        "ctx_per_req", "migrations_per_req", "p50_us", "p95_us", "p99_us",
        "rss_kb_after"]},
}
a0, a1 = out["a0"], out["a1"]
deltas = {
    "rps_delta_percent": pct(a0["rps"], a1["rps"]),
    "total_cpu_delta_percent": pct(a0["total_us_per_req"], a1["total_us_per_req"]),
    "ctx_delta_percent": pct(a0["ctx_per_req"], a1["ctx_per_req"]),
    "migration_delta_percent": pct(a0["migrations_per_req"], a1["migrations_per_req"]),
}
out["deltas"] = deltas
rps_d = deltas["rps_delta_percent"] or 0
# Case gate (A1 vs A0: positive RPS = Cap067 residency hurt)
if rps_d >= 7:
    case = "CP-A"
elif rps_d >= 3:
    case = "CP-B"
elif rps_d <= -3:
    case = "CP-D"
else:
    case = "CP-C"
out["case_heuristic"] = case
(ev / "reports" / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
print(json.dumps(out, indent=2))
PY
}

main() {
  write_authority
  log "ensure stack"
  write_compose_overlay a0
  compose up -d upstream bench-runner
  sleep 3

  # Binary hash once
  apply_variant a0
  local c sha
  c=$(ctr_exyonq)
  sha=$(docker exec "$c" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "PRODUCT_BINARY_SHA256=$sha" | tee "$EV/meta/binary_sha256.txt"
  echo "EXPECTED_P5_SHA256=$P5_SHA" | tee -a "$EV/meta/binary_sha256.txt"

  # Prove A0 pool
  maybe_activate_cap067 a0
  assert_pool_state a0
  pxdp_sanity a0

  # Prove A1 pool absent
  apply_variant a1
  assert_pool_state a1
  pxdp_sanity a1

  build_schedule
  local current=""
  while read -r variant rep; do
    [[ -z "$variant" ]] && continue
    if [[ "$variant" != "$current" ]]; then
      apply_variant "$variant"
      maybe_activate_cap067 "$variant"
      record_thread_inventory "$variant" "pre-rep${rep}"
      current=$variant
    fi
    log "measure $variant rep=$rep"
    one_measure "$variant" "$rep"
  done <"$EV/schedule/run-order.txt"

  # Final inventories
  apply_variant a0
  maybe_activate_cap067 a0
  record_thread_inventory a0 final
  apply_variant a1
  record_thread_inventory a1 final

  summarize
  log "DONE EV=$EV"
  echo "EVIDENCE_DIR=$EV"
}

main "$@"
