#!/usr/bin/env bash
# R3E competitive ExyonQ vs OpenLiteSpeed stable — run ON Linux evidence host.
# Host-native upstream + ExyonQ; OLS via docker --network host (same loopback path).
# PRODUCT_CODE_CHANGED=NO — measurement only. Not for publication until compiler ACCEPT.
set -euo pipefail

ARCH_LABEL="${ARCH_LABEL:?}"
WRK2_BIN="${WRK2_BIN:?}"
ROOT="${ROOT:-$(pwd)}"
EV="${EV:-$ROOT/.exyonq-local/tmp/r3-20260803/${ARCH_LABEL}-r3e}"
EXYONQ_REVISION="${EXYONQ_REVISION:-f0b2d67}"
TREE_HEAD="${TREE_HEAD:-unknown}"
DURATION_SEC="${DURATION_SEC:-30}"
WARMUP_SEC="${WARMUP_SEC:-10}"
CONNECTIONS="${CONNECTIONS:-1000}"
THREADS="${THREADS:-8}"
SAMPLES="${SAMPLES:-3}"
OLS_IMAGE="${OLS_IMAGE:-litespeedtech/openlitespeed:1.8.5-lsphp82}"
OLS_NAME="${OLS_NAME:-r3e-ols-stable}"

if [[ "$ARCH_LABEL" == "amd64" ]]; then
  REQUESTED_RPS="${REQUESTED_RPS:-40000}"
else
  REQUESTED_RPS="${REQUESTED_RPS:-20000}"
fi

mkdir -p "$EV/samples" "$EV/run"
cd "$ROOT"

# Fair FD ceiling for both implementations (docker OLS + host ExyonQ)
ulimit -n 1048576 || ulimit -n 65536 || true
echo "nofile=$(ulimit -n)" | tee "$EV/ulimit.txt"

{
  echo "R3E_COMPETITIVE"
  echo "R3_PRODUCT_BASELINE=$EXYONQ_REVISION"
  echo "TREE_HEAD=$TREE_HEAD"
  echo "ARCH=$ARCH_LABEL"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "USES_REAL_DATA=YES"
  echo "USES_SIMULATED_DATA=NO
USES_KNOWN_TEST_UPSTREAM_RESPONSE=YES (real independent HTTP peer only)"
  echo "NOT_PRODUCT_CORRECTNESS_EVIDENCE=YES"
  echo "PRODUCT_CODE_CHANGED=NO"
  echo "R3E_PRIOR_RUN=INVALID_HARNESS_EMFILE (re-run with raised nofile)"
  echo "REQUESTED_RPS=$REQUESTED_RPS"
  echo "DURATION_SEC=$DURATION_SEC"
  echo "WARMUP_SEC=$WARMUP_SEC"
  echo "CONNECTIONS=$CONNECTIONS"
  echo "THREADS=$THREADS"
  echo "SAMPLES=$SAMPLES"
  echo "OLS_IMAGE=$OLS_IMAGE"
  echo "WRK2_BIN=$WRK2_BIN"
  uname -a
  nproc
  free -h | head -2
} | tee "$EV/run_meta.txt"

pick_port() {
  local start="$1"
  python3 - "$start" <<'PY'
import socket, sys
start = int(sys.argv[1])
for port in range(start, start + 100):
    s = socket.socket()
    try:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("0.0.0.0", port))
    except OSError:
        s.close()
        continue
    s.close()
    print(port)
    raise SystemExit(0)
raise SystemExit("no free port")
PY
}

EXYONQ_BIN="$ROOT/target/release/exyonq"
UPSTREAM_BIN="$ROOT/tools/upstream/target/release/exyonq-upstream"
if [[ ! -x "$UPSTREAM_BIN" && -x "$ROOT/target/release/exyonq-upstream" ]]; then
  UPSTREAM_BIN="$ROOT/target/release/exyonq-upstream"
fi
test -x "$UPSTREAM_BIN" || { echo "missing upstream binary (checked tools/... and target/release)" >&2; exit 2; }
test -x "$EXYONQ_BIN" || { echo "missing exyonq binary $EXYONQ_BIN" >&2; exit 2; }
test -x "$WRK2_BIN" || { echo "missing wrk2 $WRK2_BIN" >&2; exit 2; }

"$EXYONQ_BIN" --version 2>&1 | tee "$EV/exyonq_version.txt" || true

UPSTREAM_PORT="$(pick_port 29300)"
EXYONQ_PORT="$(pick_port 29400)"
OLS_PORT="$(pick_port 29500)"
echo "UPSTREAM_PORT=$UPSTREAM_PORT EXYONQ_PORT=$EXYONQ_PORT OLS_PORT=$OLS_PORT" | tee "$EV/ports.txt"

RUN_DIR="$EV/run"
WWW_DIR="$RUN_DIR/www"
mkdir -p "$WWW_DIR"
cp -f "$ROOT/scripts/r3/payloads/health.txt" "$RUN_DIR/health.txt"
cp -f "$ROOT/scripts/r3/payloads/www/index.html" "$WWW_DIR/index.html" 2>/dev/null || echo ok >"$WWW_DIR/index.html"

# ExyonQ config
sed -e "s/__EXYONQ_PORT__/${EXYONQ_PORT}/g" -e "s/__UPSTREAM_PORT__/${UPSTREAM_PORT}/g" \
  "$ROOT/scripts/r3/exyonq-r3e-proxy.toml" >"$RUN_DIR/exyonq.toml"
cp "$RUN_DIR/exyonq.toml" "$EV/exyonq.toml"

# OLS configs (host network → loopback upstream; dedicated listen port)
sed -e "s/upstream:9000/127.0.0.1:${UPSTREAM_PORT}/g" \
    -e "s/maxConns                100/maxConns                10000/" \
  "$ROOT/scripts/r3/ols/vhosts/bench/vhconf.conf" >"$RUN_DIR/vhconf.conf"
# Point docRoot/health to mounted /bench paths (docker volume)
sed -e "s|\\*:8088|*:${OLS_PORT}|g" \
  "$ROOT/scripts/r3/ols/httpd_config.conf" >"$RUN_DIR/httpd_config.conf"
# Disable gzip compress for fair proxy compare (vh already enableGzip 0; harden server tuning)
# Keep official vhconf semantics otherwise.
cp "$RUN_DIR/vhconf.conf" "$EV/ols_vhconf.conf"
cp "$RUN_DIR/httpd_config.conf" "$EV/ols_httpd_config.conf"

pkill -f '[e]xyonq-upstream' 2>/dev/null || true
docker rm -f "$OLS_NAME" 2>/dev/null || true
# Do not kill unrelated exyonq; only our tagged pid file later
sleep 0.3

BENCH_WWW="$WWW_DIR" UPSTREAM_HOST=0.0.0.0 UPSTREAM_PORT="$UPSTREAM_PORT" "$UPSTREAM_BIN" \
  >"$EV/upstream.log" 2>&1 &
UPSTREAM_PID=$!
echo "$UPSTREAM_PID" >"$EV/upstream.pid"

EXYONQ_CONFIG="$RUN_DIR/exyonq.toml" EXYONQ_CONTROL_SOCKET="$RUN_DIR/exyonq.sock" \
  "$EXYONQ_BIN" serve --config "$RUN_DIR/exyonq.toml" \
  >"$EV/exyonq.log" 2>&1 &
EXYONQ_PID=$!
echo "$EXYONQ_PID" >"$EV/exyonq.pid"

docker run -d --name "$OLS_NAME" --network host \
  --ulimit nofile=1048576:1048576 \
  -v "$RUN_DIR/httpd_config.conf:/usr/local/lsws/conf/httpd_config.conf:ro" \
  -v "$RUN_DIR/vhconf.conf:/usr/local/lsws/conf/vhosts/bench/vhconf.conf:ro" \
  -v "$WWW_DIR:/bench/www:ro" \
  -v "$RUN_DIR/health.txt:/bench/health.txt:ro" \
  "$OLS_IMAGE" \
  >"$EV/ols_docker_run.txt" 2>&1

cleanup() {
  kill "$EXYONQ_PID" 2>/dev/null || true
  kill "$UPSTREAM_PID" 2>/dev/null || true
  docker rm -f "$OLS_NAME" 2>/dev/null || true
}
trap cleanup EXIT

wait_http() {
  local url="$1" label="$2" tries="${3:-80}"
  local i=0
  for i in $(seq 1 "$tries"); do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      echo "${label}_ready attempt=$i" | tee -a "$EV/readiness.txt"
      return 0
    fi
    sleep 0.25
  done
  echo "${label}_NOT_READY url=$url" | tee -a "$EV/readiness.txt"
  return 1
}

wait_http "http://127.0.0.1:${UPSTREAM_PORT}/health" upstream 40
wait_http "http://127.0.0.1:${EXYONQ_PORT}/api/" exyonq 80
wait_http "http://127.0.0.1:${OLS_PORT}/api/" ols 120

integrity_check() {
  local url="$1" label="$2"
  local body len head status
  status="$(curl -sS -o "$EV/body_${label}.bin" -w '%{http_code}' --max-time 5 "$url" || echo ERR)"
  len="$(wc -c <"$EV/body_${label}.bin" | tr -d ' ')"
  head="$(head -c 16 "$EV/body_${label}.bin" | xxd -p)"
  echo "${label}_status=$status len=$len head=$head" | tee -a "$EV/integrity.txt"
  [[ "$status" == "200" && "$len" == "1024" && "$head" == "78787878787878787878787878787878" ]]
}

integrity_check "http://127.0.0.1:${UPSTREAM_PORT}/api/" upstream
integrity_check "http://127.0.0.1:${EXYONQ_PORT}/api/" exyonq
integrity_check "http://127.0.0.1:${OLS_PORT}/api/" ols

# Warmup both (discarded)
for impl_url in \
  "exyonq|http://127.0.0.1:${EXYONQ_PORT}/api/" \
  "ols|http://127.0.0.1:${OLS_PORT}/api/"
do
  impl="${impl_url%%|*}"
  url="${impl_url#*|}"
  "$WRK2_BIN" -t"$THREADS" -c"$CONNECTIONS" -d"${WARMUP_SEC}s" -R"$REQUESTED_RPS" \
    "$url" >"$EV/warmup_${impl}.txt" 2>&1 || true
done

parse_wrk() {
  local out="$1"
  local achieved non2xx timeouts p50 p95 p99 sock
  achieved="$(rg -o 'Requests/sec:\s*[0-9.]+' "$out" | awk '{print $2}' | head -1 || true)"
  non2xx="$(rg -o 'Non-2xx or 3xx responses:\s*[0-9]+' "$out" | awk '{print $NF}' | head -1 || true)"
  timeouts="$(rg -o 'Socket errors:.*timeout\s+[0-9]+' "$out" | rg -o 'timeout\s+[0-9]+' | awk '{print $2}' | head -1 || true)"
  if [[ -z "$timeouts" ]]; then
    timeouts="$(rg -o 'timeout\s+[0-9]+' "$out" | awk '{print $2}' | head -1 || true)"
  fi
  p50="$(rg -o '50%\s+[0-9.]+[a-z]+' "$out" | awk '{print $2}' | head -1 || true)"
  p95="$(rg -o '95%\s+[0-9.]+[a-z]+' "$out" | awk '{print $2}' | head -1 || true)"
  p99="$(rg -o '99%\s+[0-9.]+[a-z]+' "$out" | awk '{print $2}' | head -1 || true)"
  sock="$(rg -n 'Socket errors' "$out" | head -1 || true)"
  echo "achieved=${achieved:-0}"
  echo "non2xx=${non2xx:-0}"
  echo "timeouts=${timeouts:-0}"
  echo "p50=${p50:-NA}"
  echo "p95=${p95:-NA}"
  echo "p99=${p99:-NA}"
  echo "socket_errors_line=${sock:-none}"
}

rss_kb() {
  local pid="$1"
  ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo NA
}

cpu_pct() {
  local pid="$1"
  ps -o %cpu= -p "$pid" 2>/dev/null | tr -d ' ' || echo NA
}

ols_pid() {
  # best-effort: main openlitespeed process on host network
  pgrep -f 'openlitespeed|litespeed' | head -1 || true
}

SUMMARY="$EV/competitive_summary.txt"
: >"$SUMMARY"
echo "INTERLEAVED_ORDER=exyonq,ols alternating per sample index" | tee -a "$SUMMARY"

order=0
for sample in $(seq 1 "$SAMPLES"); do
  for impl in exyonq ols; do
    order=$((order + 1))
    if [[ "$impl" == "exyonq" ]]; then
      url="http://127.0.0.1:${EXYONQ_PORT}/api/"
      impl_pid="$EXYONQ_PID"
    else
      url="http://127.0.0.1:${OLS_PORT}/api/"
      impl_pid="$(ols_pid)"
    fi
    out="$EV/samples/${sample}_${impl}_order${order}.txt"
    sample_env="$EV/samples/${sample}_${impl}_order${order}.env"
    echo "=== sample=$sample impl=$impl order=$order requested=$REQUESTED_RPS ===" | tee -a "$SUMMARY"
    upstream_rss="$(rss_kb "$UPSTREAM_PID")"
    impl_rss="$(rss_kb "${impl_pid:-0}")"
    upstream_cpu="$(cpu_pct "$UPSTREAM_PID")"
    set +e
    "$WRK2_BIN" -t"$THREADS" -c"$CONNECTIONS" -d"${DURATION_SEC}s" -R"$REQUESTED_RPS" --latency \
      "$url" >"$out" 2>&1
    ec=$?
    set -e
    upstream_rss_after="$(rss_kb "$UPSTREAM_PID")"
    impl_rss_after="$(rss_kb "${impl_pid:-0}")"
    # integrity after sample
    set +e
    integrity_check "$url" "post_${sample}_${impl}"
    integ_ec=$?
    set -e
    {
      echo "implementation=$impl"
      echo "architecture=$ARCH_LABEL"
      echo "run_index=$sample"
      echo "order=$order"
      echo "revision=$EXYONQ_REVISION"
      echo "tree_head=$TREE_HEAD"
      echo "config_identity=exyonq-r3e-proxy.toml+ols-vhconf-hostloopback"
      echo "requested_rps=$REQUESTED_RPS"
      parse_wrk "$out"
      echo "wrk2_exit=$ec"
      echo "integrity_ok=$integ_ec"
      echo "errors_non2xx_see_above=1"
      echo "upstream_rss_kb_before=$upstream_rss"
      echo "upstream_rss_kb_after=$upstream_rss_after"
      echo "impl_rss_kb_before=$impl_rss"
      echo "impl_rss_kb_after=$impl_rss_after"
      echo "upstream_cpu_sample_pct=$upstream_cpu"
      echo "duration_sec=$DURATION_SEC"
      echo "connections=$CONNECTIONS"
      echo "threads=$THREADS"
      echo "url=$url"
      echo "environment_identity=$(hostname)-$(uname -m)"
    } | tee "$sample_env" | tee -a "$SUMMARY"
  done
done

# Aggregate medians of achieved RPS for valid samples (integrity_ok=0, wrk2_exit=0, non2xx=0)
python3 - "$EV" "$ARCH_LABEL" "$REQUESTED_RPS" <<'PY' | tee -a "$EV/competitive_summary.txt"
import pathlib, re, statistics, sys
ev = pathlib.Path(sys.argv[1])
arch = sys.argv[2]
req = float(sys.argv[3])
by = {"exyonq": [], "ols": []}
invalid = []
for env in sorted(ev.glob("samples/*.env")):
    text = env.read_text()
    def g(k, default=""):
        m = re.search(rf"^{k}=(.*)$", text, re.M)
        return m.group(1).strip() if m else default
    impl = g("implementation")
    achieved = float(g("achieved", "0") or 0)
    non2xx = int(float(g("non2xx", "0") or 0))
    timeouts = int(float(g("timeouts", "0") or 0))
    wrk2_exit = int(g("wrk2_exit", "1") or 1)
    integ = int(g("integrity_ok", "1") or 1)
    valid = wrk2_exit == 0 and non2xx == 0 and timeouts == 0 and integ == 0 and achieved > 0
    rec = f"{env.name} impl={impl} achieved={achieved} valid={valid}"
    if valid:
        by.setdefault(impl, []).append(achieved)
    else:
        invalid.append(rec)
    print(rec)
for impl, vals in by.items():
    if vals:
        print(f"{impl}_n={len(vals)} median_rps={statistics.median(vals):.1f} mean_rps={statistics.mean(vals):.1f} min={min(vals):.1f} max={max(vals):.1f}")
    else:
        print(f"{impl}_n=0 INVALID")
eq = by.get("exyonq") or []
ols = by.get("ols") or []
print(f"requested_rps={req}")
print(f"invalid_count={len(invalid)}")
if eq and ols:
    em, om = statistics.median(eq), statistics.median(ols)
    # 5% band parity
    hi = max(em, om)
    lo = min(em, om)
    if hi <= 0:
        cls = "INCONCLUSIVE"
    elif abs(em - om) / hi <= 0.05:
        cls = "PARITY"
    elif em > om:
        cls = "WIN"
    else:
        cls = "LOSS"
    print(f"R3E_{arch.upper()}_EXYONQ_MEDIAN_RPS={em:.1f}")
    print(f"R3E_{arch.upper()}_OLS_MEDIAN_RPS={om:.1f}")
    print(f"R3E_{arch.upper()}_CLASSIFICATION={cls}")
else:
    print(f"R3E_{arch.upper()}_CLASSIFICATION=INCONCLUSIVE")
    print("REASON=insufficient_valid_samples")
PY

echo "R3E_REMOTE_DONE arch=$ARCH_LABEL" | tee -a "$SUMMARY"
