#!/usr/bin/env bash
# R3C load-generator capacity — infrastructure only (wrk2 vs exyonq-upstream).
# Proves generator headroom; NOT product correctness.
set -euo pipefail

ARCH_LABEL="${ARCH_LABEL:?}"
COMPETITIVE_TARGET_RPS="${COMPETITIVE_TARGET_RPS:?}"
MARGIN_RATIO="${MARGIN_RATIO:-1.25}"
DURATION_SEC="${DURATION_SEC:-20}"
CONNECTIONS="${CONNECTIONS:-1000}"
THREADS="${THREADS:-8}"
WRK2_BIN="${WRK2_BIN:?}"
ROOT="${ROOT:-$(pwd)}"
EV="${EV:-$ROOT/.exyonq-local/tmp/r3-20260803/${ARCH_LABEL}-r3c}"
mkdir -p "$EV"

echo "USES_SIMULATED_DATA=NO
USES_KNOWN_TEST_UPSTREAM_RESPONSE=YES (real independent HTTP peer; loadgen capacity only)" | tee "$EV/baseline.txt"
echo "R3_PRODUCT_BASELINE=${R3_PRODUCT_BASELINE:-f0b2d67}" | tee -a "$EV/baseline.txt"
echo "NOT_PRODUCT_CORRECTNESS_EVIDENCE=YES" | tee -a "$EV/baseline.txt"

pick_port() {
  python3 - <<'PY'
import socket
for port in range(29301, 29401):
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

BIN="$ROOT/tools/upstream/target/release/exyonq-upstream"
[[ -x "$BIN" ]] || cargo build --release --manifest-path "$ROOT/tools/upstream/Cargo.toml"

pkill -f '[e]xyonq-upstream' 2>/dev/null || true
sleep 0.3
UPSTREAM_PORT="$(pick_port)"
UPSTREAM_HOST=0.0.0.0 UPSTREAM_PORT="$UPSTREAM_PORT" "$BIN" >"$EV/upstream.log" 2>&1 &
UPSTREAM_PID=$!
cleanup() { kill "$UPSTREAM_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:${UPSTREAM_PORT}/health" >/dev/null 2>&1 && break
  sleep 0.1
done

URL="http://127.0.0.1:${UPSTREAM_PORT}/api/"
MARGIN_RATE="$(python3 -c "print(int(float('$COMPETITIVE_TARGET_RPS')*float('$MARGIN_RATIO')))")"
SUMMARY="$EV/loadgen_summary.txt"
{
  echo "ARCH=$ARCH_LABEL"
  echo "COMPETITIVE_TARGET_RPS=$COMPETITIVE_TARGET_RPS"
  echo "MARGIN_RATIO=$MARGIN_RATIO"
  echo "MARGIN_RATE=$MARGIN_RATE"
  echo "DURATION_SEC=$DURATION_SEC"
  echo "CONNECTIONS=$CONNECTIONS"
  echo "THREADS=$THREADS"
  echo "WRK2_BIN=$WRK2_BIN"
  uname -a
  date -u +%Y-%m-%dT%H:%M:%SZ
} | tee "$SUMMARY"

SAFE_RPS=0
for RATE in "$COMPETITIVE_TARGET_RPS" "$MARGIN_RATE"; do
  OUT="$EV/wrk2_${RATE}.txt"
  echo "=== RATE=$RATE ===" | tee -a "$SUMMARY"
  # Sample wrk2 CPU while running
  (
    sleep 5
    pgrep -a wrk2 | head -3 | tee "$EV/wrk2_ps_${RATE}.txt" || true
    # average %cpu over a few samples if available
    for _ in 1 2 3 4; do
      ps -C wrk2 -o %cpu=,rss= 2>/dev/null | head -1 || true
      sleep 2
    done
  ) >"$EV/wrk2_cpu_${RATE}.txt" 2>&1 &
  SAMPLER=$!
  set +e
  "$WRK2_BIN" -t"$THREADS" -c"$CONNECTIONS" -d"${DURATION_SEC}s" -R"$RATE" --latency "$URL" \
    >"$OUT" 2>&1
  EC=$?
  set -e
  wait "$SAMPLER" 2>/dev/null || true
  ACHIEVED="$(rg -o 'Requests/sec:\s*[0-9.]+' "$OUT" | awk '{print $2}' | head -1 || true)"
  ACHIEVED="${ACHIEVED:-0}"
  NON2XX="$(rg -o 'Non-2xx or 3xx responses:\s*[0-9]+' "$OUT" | awk '{print $NF}' | head -1 || true)"
  NON2XX="${NON2XX:-0}"
  TIMEOUTS="$(rg -o 'Socket errors:.*timeout\s+[0-9]+' "$OUT" | awk '{print $NF}' | head -1 || true)"
  TIMEOUTS="${TIMEOUTS:-0}"
  RATIO="$(python3 -c "print(float('$ACHIEVED')/float('$RATE') if float('$RATE') else 0)")"
  CPU_AVG="$(awk '{s+=$1;n++} END{if(n) printf "%.1f", s/n; else print "NA"}' "$EV/wrk2_cpu_${RATE}.txt" 2>/dev/null || echo NA)"
  echo "rate=$RATE achieved=$ACHIEVED ratio=$RATIO non2xx=$NON2XX timeouts=$TIMEOUTS exit=$EC wrk2_cpu_avg_pct=$CPU_AVG" | tee -a "$SUMMARY"
  if python3 - <<PY
rate=float("$RATE"); ach=float("$ACHIEVED"); non=int("$NON2XX"); to=int("$TIMEOUTS" or 0)
ok = ach >= 0.95*rate and non == 0 and to == 0
raise SystemExit(0 if ok else 1)
PY
  then
    SAFE_RPS="$RATE"
    echo "valid=1" | tee -a "$SUMMARY"
  else
    echo "valid=0 FIRST_INVALID=$RATE" | tee -a "$SUMMARY"
    break
  fi
done

echo "SAFE_RPS=$SAFE_RPS" | tee -a "$SUMMARY"
# Pass if margin rate validated (headroom above competitive target)
if python3 -c "raise SystemExit(0 if float('$SAFE_RPS') >= float('$MARGIN_RATE') else 1)"; then
  echo "R3C_${ARCH_LABEL}=PASS" | tee -a "$SUMMARY"
  echo "MARGIN_OK=YES target=$COMPETITIVE_TARGET_RPS safe_ceiling=$SAFE_RPS" | tee -a "$SUMMARY"
  exit 0
fi
echo "R3C_${ARCH_LABEL}=FAIL" | tee -a "$SUMMARY"
exit 1
