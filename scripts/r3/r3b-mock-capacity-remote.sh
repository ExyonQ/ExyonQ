#!/usr/bin/env bash
# R3B mock capacity proof — run ON Linux evidence host (Netcup/Oracle).
# USES_REAL_DATA=YES — measured wrk2 against live r3-p4-mock; NOT_FOR_RANKING until dual-arch PASS.
set -euo pipefail

ARCH_LABEL="${ARCH_LABEL:?}"
THRESHOLD_RPS="${THRESHOLD_RPS:?}"
DURATION_SEC="${DURATION_SEC:-30}"
CONNECTIONS="${CONNECTIONS:-1000}"
THREADS="${THREADS:-8}"
WRK2_BIN="${WRK2_BIN:?}"
ROOT="${ROOT:-$(pwd)}"
EV="${EV:-$ROOT/.exyonq-local/tmp/r3-20260803/${ARCH_LABEL}}"
mkdir -p "$EV"
echo "R3_PRODUCT_BASELINE=${R3_PRODUCT_BASELINE:-f0b2d67}" | tee "$EV/baseline.txt"
echo "USES_SIMULATED_DATA=YES (deterministic mock upstream; infrastructure-only)" | tee -a "$EV/baseline.txt"
echo "NOT_PRODUCT_CORRECTNESS_EVIDENCE=YES" | tee -a "$EV/baseline.txt"

pick_port() {
  python3 - <<'PY'
import socket
for port in range(29191, 29291):
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
raise SystemExit("no free port in 29191-29290")
PY
}

BIN="$ROOT/tools/r3-p4-mock/target/release/r3-p4-mock"
if [[ ! -x "$BIN" ]]; then
  cargo build --release --manifest-path "$ROOT/tools/r3-p4-mock/Cargo.toml" 2>&1 | tee "$EV/build.log"
fi

pkill -f '[r]3-p4-mock' 2>/dev/null || true
sleep 0.5

MOCK_PORT="$(pick_port)"
export MOCK_PORT
echo "MOCK_PORT=$MOCK_PORT" | tee "$EV/port.txt"

MOCK_HOST=0.0.0.0 MOCK_PORT="$MOCK_PORT" "$BIN" >"$EV/mock.log" 2>&1 &
MOCK_PID=$!
cleanup() { kill "$MOCK_PID" 2>/dev/null || true; }
trap cleanup EXIT

ready=0
for _ in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:${MOCK_PORT}/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  # Abort early if process died
  if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    echo "mock exited during startup" | tee -a "$EV/capacity_summary.txt"
    cat "$EV/mock.log" | tee -a "$EV/capacity_summary.txt"
    exit 2
  fi
  sleep 0.1
done
test "$ready" = "1"

URL="http://127.0.0.1:${MOCK_PORT}/api/"
BODY_LEN="$(curl -fsS "$URL" | wc -c | tr -d ' ')"
echo "preflight_body_len=$BODY_LEN" | tee "$EV/preflight.txt"
test "$BODY_LEN" = "1024"
# Ensure we are not hitting the foreign smoke fixture
BODY_HEAD="$(curl -fsS "$URL" | head -c 16 | xxd -p)"
echo "preflight_body_head_hex=$BODY_HEAD" | tee -a "$EV/preflight.txt"
# synthetic payload is 1024 x's → 78 hex for 'x'
test "$BODY_HEAD" = "78787878787878787878787878787878"

if [[ "$ARCH_LABEL" == "amd64" ]]; then
  RATES=(10000 25000 40000 50000 60000 75000)
else
  RATES=(10000 15000 20000 25000 30000 40000)
fi

MAX_VALID=0
SUMMARY="$EV/capacity_summary.txt"
{
  echo "ARCH=$ARCH_LABEL"
  echo "THRESHOLD_RPS=$THRESHOLD_RPS"
  echo "DURATION_SEC=$DURATION_SEC"
  echo "CONNECTIONS=$CONNECTIONS"
  echo "THREADS=$THREADS"
  echo "WRK2_BIN=$WRK2_BIN"
  echo "MOCK_PORT=$MOCK_PORT"
  echo "URL=$URL"
  echo "MOCK_PID=$MOCK_PID"
  uname -a
  date -u +%Y-%m-%dT%H:%M:%SZ
  cat "$EV/preflight.txt"
} | tee "$SUMMARY"

for RATE in "${RATES[@]}"; do
  OUT="$EV/wrk2_${RATE}.txt"
  echo "=== RATE=$RATE ===" | tee -a "$SUMMARY"
  MOCK_RSS_BEFORE="$(ps -o rss= -p "$MOCK_PID" 2>/dev/null | tr -d ' ' || echo NA)"
  set +e
  "$WRK2_BIN" -t"$THREADS" -c"$CONNECTIONS" -d"${DURATION_SEC}s" -R"$RATE" --latency "$URL" \
    >"$OUT" 2>&1
  EC=$?
  set -e
  MOCK_RSS_AFTER="$(ps -o rss= -p "$MOCK_PID" 2>/dev/null | tr -d ' ' || echo NA)"
  ACHIEVED="$(rg -o 'Requests/sec:\s*[0-9.]+' "$OUT" | awk '{print $2}' | head -1 || true)"
  ACHIEVED="${ACHIEVED:-0}"
  NON2XX="$(rg -o 'Non-2xx or 3xx responses:\s*[0-9]+' "$OUT" | awk '{print $NF}' | head -1 || true)"
  NON2XX="${NON2XX:-0}"
  TIMEOUTS="$(rg -o 'timeout\s+[0-9]+' "$OUT" | awk '{print $2}' | head -1 || true)"
  TIMEOUTS="${TIMEOUTS:-0}"
  echo "rate=$RATE achieved=$ACHIEVED non2xx=$NON2XX timeouts=$TIMEOUTS exit=$EC mock_rss_kb_before=$MOCK_RSS_BEFORE after=$MOCK_RSS_AFTER" | tee -a "$SUMMARY"

  # Valid: ≥92% of requested RPS, zero non-2xx, timeouts < 0.5% of completed requests
  if python3 - <<PY
rate=float("$RATE"); ach=float("$ACHIEVED"); non=int("$NON2XX"); to=int("$TIMEOUTS")
completed = max(ach * float("$DURATION_SEC"), 1.0)
ok = ach >= 0.92*rate and non == 0 and to <= 0.005*completed
raise SystemExit(0 if ok else 1)
PY
  then
    MAX_VALID="$RATE"
    echo "valid=1" | tee -a "$SUMMARY"
  else
    echo "valid=0 FIRST_INVALID_RATE=$RATE" | tee -a "$SUMMARY"
    break
  fi
done

echo "MAX_VALID_RPS=$MAX_VALID" | tee -a "$SUMMARY"
BODY_LEN_AFTER="$(curl -fsS "$URL" | wc -c | tr -d ' ')"
echo "post_body_len=$BODY_LEN_AFTER" | tee -a "$SUMMARY"
test "$BODY_LEN_AFTER" = "1024"

if python3 -c "raise SystemExit(0 if float('$MAX_VALID') >= float('$THRESHOLD_RPS') else 1)"; then
  echo "R3B_${ARCH_LABEL}=PASS" | tee -a "$SUMMARY"
  exit 0
fi
echo "R3B_${ARCH_LABEL}=FAIL" | tee -a "$SUMMARY"
exit 1
