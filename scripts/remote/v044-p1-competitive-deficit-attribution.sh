#!/usr/bin/env bash
# V044_P1_COMPETITIVE_DEFICIT_ATTRIBUTION — measurement-only (NO product mutation).
# Reuses seal compose project v044p1auth-clean + product binary 09301dc8…
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN. P2=HOLD.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_ATTR_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_ATTR_EV:-$WS/.exyonq-local-evidence/v044-p1-competitive-deficit-attr-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P1="/site/1k.bin"
EXPECTED_SHA="${EXPECTED_EXYONQ_SHA256:-09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f}"
WARMUP_SEC=15
MEASURE_SEC=30

mkdir -p "$EV"/{meta,smaps,proc,load,cgroup,notes}
cd "$WS"
log() { echo "[attr] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
  esac
}
url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P1}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P1}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P1}" ;;
  esac
}

# Host PIDs for container main processes (for /proc access from host).
main_pids() {
  local tag=$1 c
  c=$(ctr "$tag")
  case "$tag" in
    exyonq)
      docker top "$c" -eo pid,cmd | awk 'NR>1 && $0 ~ /exyonq serve/ {print $1; exit}'
      ;;
    nginx)
      # sum workers separately; print master then workers
      docker top "$c" -eo pid,cmd | awk 'NR>1 && /nginx: master/ {print "MASTER",$1}
                                         NR>1 && /nginx: worker/ {print "WORKER",$1}'
      ;;
    ols)
      docker top "$c" -eo pid,cmd | awk 'NR>1 && /lshttpd - main/ {print "MAIN",$1}
                                         NR>1 && /lshttpd - #/ {print "WORKER",$1}'
      ;;
  esac
}

capture_pid_bundle() {
  local label=$1 pid=$2 outdir=$3
  mkdir -p "$outdir"
  [[ -d "/proc/$pid" ]] || { echo "missing pid $pid" >"$outdir/${label}.missing"; return 0; }
  {
    echo "pid=$pid label=$label"
    date -u +%Y-%m-%dT%H:%M:%SZ
    echo "=== status ==="
    grep -E '^(Name|Pid|PPid|Threads|VmRSS|VmHWM|VmSize|VmData|VmStk|VmExe|VmLib|VmPeak|voluntary_ctxt_switches|nonvoluntary_ctxt_switches|RssAnon|RssFile|RssShmem):' "/proc/$pid/status" || true
    echo "=== smaps_rollup ==="
    cat "/proc/$pid/smaps_rollup" 2>/dev/null || echo "NO_SMAPS_ROLLUP"
    echo "=== nfd ==="
    ls "/proc/$pid/fd" 2>/dev/null | wc -l
    echo "=== maps_count ==="
    wc -l <"/proc/$pid/maps" 2>/dev/null || echo 0
  } >"$outdir/${label}.txt"

  # Categorize smaps (anon vs file) coarsely if full smaps available
  if [[ -r "/proc/$pid/smaps" ]]; then
    awk '
      /^[0-9a-f]+-/ { path=$0; sub(/^[0-9a-f]+-[0-9a-f]+[[:space:]]+[^ ]+[[:space:]]+[^ ]+[[:space:]]+[^ ]+[[:space:]]+[^ ]+[[:space:]]*/,"",path); if(path=="") path="[anon]"; }
      /^Rss:/ { rss=$2; total+=rss;
        if (path ~ /^\[/ || path=="") anon+=rss; else file+=rss;
        if (path ~ /exyonq|nginx|lshttpd|openlitespeed/) textish+=rss;
        if (path ~ /\[heap\]/) heap+=rss;
        if (path ~ /\[stack/) stack+=rss;
        if (path ~ /\.so/) so+=rss;
        if (path ~ /jemalloc|allocator/) alloclib+=rss;
      }
      /^Private_Dirty:/ { pd+=$2 }
      /^Private_Clean:/ { pc+=$2 }
      /^Shared_Clean:/ { sc+=$2 }
      /^Shared_Dirty:/ { sd+=$2 }
      /^Anonymous:/ { an+=$2 }
      END {
        printf "Rss_kB_sum=%d anon_path_Rss_kB=%d file_path_Rss_kB=%d heap_Rss_kB=%d stack_Rss_kB=%d so_Rss_kB=%d textish_Rss_kB=%d Private_Dirty_kB=%d Private_Clean_kB=%d Shared_Clean_kB=%d Shared_Dirty_kB=%d Anonymous_kB=%d\n",
          total,anon,file,heap,stack,so,textish,pd,pc,sc,sd,an
      }
    ' "/proc/$pid/smaps" >"$outdir/${label}.smaps_cat.txt"
  fi
}

sum_workers_smaps() {
  local tag=$1 phase=$2
  local out="$EV/smaps/${tag}-${phase}"
  mkdir -p "$out"
  main_pids "$tag" >"$EV/meta/${tag}-pids-${phase}.txt"
  case "$tag" in
    exyonq)
      pid=$(awk 'NR==1{print $1}' "$EV/meta/${tag}-pids-${phase}.txt")
      capture_pid_bundle "main" "$pid" "$out"
      # threads listing
      if [[ -n "$pid" && -d "/proc/$pid/task" ]]; then
        ls "/proc/$pid/task" | wc -l >"$out/thread_count.txt"
        # per-thread names
        for t in /proc/$pid/task/*/comm; do
          echo "$(basename "$(dirname "$t")") $(cat "$t" 2>/dev/null)"
        done >"$out/threads.txt" || true
      fi
      ;;
    nginx|ols)
      while read -r kind pid; do
        [[ -n "${pid:-}" ]] || continue
        capture_pid_bundle "${kind}_${pid}" "$pid" "$out"
      done <"$EV/meta/${tag}-pids-${phase}.txt"
      # aggregate rollup Rss
      {
        echo "phase=$phase tag=$tag"
        awk '/^VmRSS:/ {rss+=$2} END{printf "sum_VmRSS_kB=%d\n", rss}' "$out"/*.txt 2>/dev/null || true
        awk '/^Rss:/ && /kB/ { if($1=="Rss:") rss+=$2 } END{printf "sum_smaps_Rss_kB=%d\n", rss}' "$out"/*.txt 2>/dev/null || true
        awk -F= '/^Rss_kB_sum=/ {s+=$2} END{printf "sum_smaps_cat_Rss_kB=%d\n", s}' "$out"/*.smaps_cat.txt 2>/dev/null || true
        awk -F= '
          /^Private_Dirty_kB=/ {pd+=$2}
          /^Private_Clean_kB=/ {pc+=$2}
          /^Shared_Clean_kB=/ {sc+=$2}
          /^Anonymous_kB=/ {an+=$2}
          /^heap_Rss_kB=/ {h+=$2}
          /^stack_Rss_kB=/ {st+=$2}
          /^so_Rss_kB=/ {so+=$2}
          /^textish_Rss_kB=/ {tx+=$2}
          END {
            printf "agg_Private_Dirty_kB=%d\nagg_Private_Clean_kB=%d\nagg_Shared_Clean_kB=%d\nagg_Anonymous_kB=%d\nagg_heap_Rss_kB=%d\nagg_stack_Rss_kB=%d\nagg_so_Rss_kB=%d\nagg_textish_Rss_kB=%d\n",
              pd,pc,sc,an,h,st,so,tx
          }
        ' "$out"/*.smaps_cat.txt 2>/dev/null || true
      } >"$out/AGGREGATE.txt"
      ;;
  esac
  if [[ "$tag" == "exyonq" ]]; then
    {
      echo "phase=$phase tag=exyonq"
      awk '/^VmRSS:/ {print}' "$out/main.txt" 2>/dev/null || true
      cat "$out/main.smaps_cat.txt" 2>/dev/null || true
      echo -n "thread_count="; cat "$out/thread_count.txt" 2>/dev/null || echo 0
    } >"$out/AGGREGATE.txt"
  fi
}

cgroup_cpu_window() {
  local tag=$1
  local c path before after
  c=$(ctr "$tag")
  # docker cgroup v2 path
  path=$(docker inspect -f '{{.HostConfig.CgroupParent}} {{.Id}}' "$c" 2>/dev/null || true)
  # Use docker stats cpu + memory during measure; also try memory.current via nsenter-less:
  docker stats --no-stream --format '{{.Name}} cpu={{.CPUPerc}} mem={{.MemUsage}}' "$c" | tee "$EV/cgroup/${tag}-docker-stats-snapshot.txt"
}

load_one() {
  local tag=$1
  local url c
  url=$(url_for "$tag")
  c=$(ctr "$tag")
  log "load $tag warmup ${WARMUP_SEC}s"
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true

  # ctxt before
  sum_workers_smaps "$tag" "pre_measure"
  # capture ctxt switches baseline
  : >"$EV/proc/${tag}-ctxt-before.txt"
  case "$tag" in
    exyonq)
      pid=$(awk 'NR==1{print $1}' "$EV/meta/${tag}-pids-pre_measure.txt")
      grep -E 'ctxt_switches' "/proc/$pid/status" >>"$EV/proc/${tag}-ctxt-before.txt" || true
      ;;
    *)
      while read -r kind pid; do
        [[ "$kind" == WORKER || "$kind" == "WORKER" ]] || continue
        echo "pid=$pid" >>"$EV/proc/${tag}-ctxt-before.txt"
        grep -E 'ctxt_switches' "/proc/$pid/status" >>"$EV/proc/${tag}-ctxt-before.txt" || true
      done <"$EV/meta/${tag}-pids-pre_measure.txt"
      ;;
  esac

  log "load $tag measure ${MEASURE_SEC}s + smaps mid"
  timeout $((MEASURE_SEC + 10)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$EV/load/${tag}.docker-stats.txt" 2>/dev/null &
  local sp=$!
  sleep 0.3
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/load/${tag}.rewrk.json" &
  local rp=$!
  sleep $((MEASURE_SEC / 2))
  sum_workers_smaps "$tag" "mid_load"
  wait "$rp" || true
  wait "$sp" 2>/dev/null || true

  # ctxt after
  : >"$EV/proc/${tag}-ctxt-after.txt"
  case "$tag" in
    exyonq)
      pid=$(awk 'NR==1{print $1}' "$EV/meta/${tag}-pids-mid_load.txt")
      grep -E 'ctxt_switches' "/proc/$pid/status" >>"$EV/proc/${tag}-ctxt-after.txt" || true
      # thread CPU distribution via top batch
      ps -L -p "$pid" -o pid,tid,pcpu,comm >"$EV/proc/${tag}-threads-cpu.txt" 2>/dev/null || true
      ;;
    *)
      while read -r kind pid; do
        echo "pid=$pid kind=$kind" >>"$EV/proc/${tag}-ctxt-after.txt"
        grep -E 'ctxt_switches' "/proc/$pid/status" >>"$EV/proc/${tag}-ctxt-after.txt" || true
      done <"$EV/meta/${tag}-pids-mid_load.txt"
      ;;
  esac
  sum_workers_smaps "$tag" "post_load"
  cgroup_cpu_window "$tag"
}

# --- main ---
log "WS=$WS EV=$EV"
EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
if [[ "$EXY_SHA" != "$EXPECTED_SHA" ]]; then
  echo "BINARY_MISMATCH expected=$EXPECTED_SHA got=$EXY_SHA" | tee "$EV/meta/BINARY_MISMATCH"
  exit 2
fi
for tag in exyonq nginx ols; do
  docker update --cpuset-cpus "0-7" "$(ctr "$tag")" >/dev/null
  docker inspect -f 'Cpuset={{.HostConfig.CpusetCpus}} CpuQuota={{.HostConfig.CpuQuota}}' "$(ctr "$tag")" \
    | tee -a "$EV/meta/cpuset.txt"
done
df -h / | tee "$EV/meta/disk.txt"
docker ps --format '{{.Names}}' | tee "$EV/meta/docker_ps_names.txt"
echo "HOST_INTERFERENCE=OBSERVED_IF_EXTRA_CONTAINERS" | tee "$EV/meta/interference_note.txt"

# Idle baseline RSS (no load)
for tag in exyonq nginx ols; do
  sum_workers_smaps "$tag" "idle"
done

for tag in exyonq nginx ols; do
  load_one "$tag"
done

# Summarize JSON for lab
python3 - <<'PY' "$EV"
import json,sys,re,statistics
from pathlib import Path
ev=Path(sys.argv[1])
out={"evidence":str(ev),"servers":{}}
for tag in ("exyonq","nginx","ols"):
  s={"phases":{}}
  for phase in ("idle","pre_measure","mid_load","post_load"):
    agg=ev/f"smaps/{tag}-{phase}/AGGREGATE.txt"
    d={"raw":agg.read_text() if agg.exists() else ""}
    if agg.exists():
      for line in d["raw"].splitlines():
        if "=" in line:
          k,v=line.split("=",1)
          try: d[k]=int(v)
          except: d[k]=v
    s["phases"][phase]=d
  thr=ev/f"smaps/{tag}-mid_load/thread_count.txt"
  if thr.exists():
    s["thread_count"]=int(thr.read_text().strip() or 0)
  thrlist=ev/f"smaps/{tag}-mid_load/threads.txt"
  if thrlist.exists():
    s["threads"]=thrlist.read_text().splitlines()
  rj=ev/f"load/{tag}.rewrk.json"
  if rj.exists():
    try:
      j=json.loads(rj.read_text())
      s["rps"]=j.get("requests_per_sec") or j.get("rps")
      # rewrk schema variants
      if s["rps"] is None and "latency" in j:
        s["rewrk_keys"]=list(j.keys())
      s["rewrk"]=j
    except Exception as e:
      s["rewrk_err"]=str(e)
  # docker stats avg mem
  st=ev/f"load/{tag}.docker-stats.txt"
  if st.exists():
    rss=[]
    cpu=[]
    for line in st.read_text().splitlines():
      parts=line.replace('%','').split()
      if len(parts)>=2:
        try: cpu.append(float(parts[0]))
        except: pass
        # MemUsage like 18.5MiB / 15GiB
        m=re.search(r'([0-9.]+)([KMGT]i?B)', parts[1])
        if m:
          val=float(m.group(1)); unit=m.group(2)
          mul={"B":1/1048576,"KiB":1/1024,"MiB":1,"GiB":1024}.get(unit,1)
          rss.append(val*mul)
    if cpu: s["cpu_avg"]=statistics.mean(cpu)
    if rss: s["rss_avg_mib_docker"]=statistics.mean(rss)
  out["servers"][tag]=s
(ev/"reports").mkdir(exist_ok=True)
(ev/"reports/attribution_summary.json").write_text(json.dumps(out,indent=2))
print("wrote", ev/"reports/attribution_summary.json")
PY

log "DONE EV=$EV"
