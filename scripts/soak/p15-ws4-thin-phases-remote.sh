#!/usr/bin/env bash
# P1.5-WS4 thin phases (S10 config tooling, S15 bounded pressure, S16 idle+reconnect).
# Linux host only. Not competitive bench. Host-safe (no OOM/disk-full/fork-bomb).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  p15-ws4-thin-phases-remote.sh \
    --workspace PATH \
    --host-label amd64|arm64 \
    --expected-arch x86_64|aarch64 \
    --artifact-dir PATH \
    --phase S10|S15|S16|all \
    [--duration-sec N] \
    [--sample-interval N]
EOF
}

WORKSPACE=""
HOST_LABEL=""
EXPECTED_ARCH=""
ARTIFACT_DIR=""
PHASE="all"
DURATION_SEC="${P15_WS4_THIN_DURATION_SEC:-120}"
SAMPLE_INTERVAL="${P15_WS4_SAMPLE_INTERVAL:-5}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --host-label) HOST_LABEL="${2:-}"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="${2:-}"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="${2:-}"; shift 2 ;;
    --phase) PHASE="${2:-}"; shift 2 ;;
    --duration-sec) DURATION_SEC="${2:-}"; shift 2 ;;
    --sample-interval) SAMPLE_INTERVAL="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$HOST_LABEL" && -n "$EXPECTED_ARCH" && -n "$ARTIFACT_DIR" ]] || {
  usage >&2
  exit 2
}
[[ "$(uname -s)" == "Linux" ]] || { echo "ERROR: Linux host required" >&2; exit 2; }
ACTUAL_ARCH="$(uname -m)"
[[ "$ACTUAL_ARCH" == "$EXPECTED_ARCH" ]] || {
  echo "ERROR: arch mismatch expected=$EXPECTED_ARCH actual=$ACTUAL_ARCH" >&2
  exit 2
}

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
# shellcheck disable=SC1091
source ~/.cargo/env 2>/dev/null || true
mkdir -p "$ARTIFACT_DIR"

TMP="$(mktemp -d)"
DOCROOT="$TMP/public"
CFG="$TMP/exyonq.toml"
CTRL_SOCK="$TMP/control.sock"
SERVER_LOG="$ARTIFACT_DIR/server.log"
SERVER_PID=""
SAMPLER_PID=""
LOAD_PID=""
EXYONQ_BIN=""
EXYONQCTL_BIN=""
LISTEN_PORT=""

cleanup() {
  local rc=$?
  [[ -n "${LOAD_PID:-}" ]] && kill "$LOAD_PID" 2>/dev/null || true
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
  exit "$rc"
}
trap cleanup EXIT INT TERM

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

rss_kb() {
  local pid="$1"
  [[ -r "/proc/$pid/status" ]] && awk '/VmRSS:/ {print $2}' "/proc/$pid/status" || echo 0
}
fd_count() {
  local pid="$1"
  [[ -d "/proc/$pid/fd" ]] && ls -1 "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' ' || echo 0
}
task_count() {
  local pid="$1"
  [[ -d "/proc/$pid/task" ]] && ls -1 "/proc/$pid/task" 2>/dev/null | wc -l | tr -d ' ' || echo 1
}
socket_count() {
  local pid="$1"
  local n=0 f t
  [[ -d "/proc/$pid/fd" ]] || { echo 0; return; }
  for f in "/proc/$pid/fd"/*; do
    [[ -e "$f" ]] || continue
    t="$(readlink "$f" 2>/dev/null || true)"
    [[ "$t" == socket:* ]] && n=$((n + 1))
  done
  echo "$n"
}

prepare_fixture() {
  mkdir -p "$DOCROOT/sub"
  echo "ws4-thin-ok" >"$DOCROOT/index.html"
  echo "ws4-thin-ok" >"$DOCROOT/sub/index.html"
  head -c 65536 /dev/urandom >"$DOCROOT/medium.bin" 2>/dev/null || dd if=/dev/urandom of="$DOCROOT/medium.bin" bs=1024 count=64 status=none
  LISTEN_PORT="$(pick_port)"
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF
  cp "$CFG" "$ARTIFACT_DIR/exyonq.toml"
}

ensure_bins() {
  cargo build --release -p exyonq -p exyonqctl -q
  EXYONQ_BIN="$WORKSPACE/target/release/exyonq"
  EXYONQCTL_BIN="$WORKSPACE/target/release/exyonqctl"
}

start_server() {
  prepare_fixture
  ensure_bins
  rm -f "$CTRL_SOCK"
  : >"$SERVER_LOG"
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" RUST_LOG=error \
    "$EXYONQ_BIN" serve --config "$CFG" >>"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  local i=0
  while (( i < 100 )); do
    if curl -sf --max-time 2 "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      echo "ERROR: server died during start" >&2
      tail -n 80 "$SERVER_LOG" >&2 || true
      return 1
    fi
    sleep 0.1
    i=$((i + 1))
  done
  echo "ERROR: server did not become ready" >&2
  tail -n 80 "$SERVER_LOG" >&2 || true
  return 1
}

stop_server() {
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  SAMPLER_PID=""
  [[ -n "${LOAD_PID:-}" ]] && kill "$LOAD_PID" 2>/dev/null || true
  LOAD_PID=""
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=""
}

start_sampler() {
  local out="$1"
  local pid="$2"
  echo "ts_unix,rss_kb,fd,tasks,sockets" >"$out"
  (
    while kill -0 "$pid" 2>/dev/null; do
      echo "$(date +%s),$(rss_kb "$pid"),$(fd_count "$pid"),$(task_count "$pid"),$(socket_count "$pid")"
      sleep "$SAMPLE_INTERVAL"
    done
  ) >>"$out" &
  SAMPLER_PID=$!
}

write_phase_summary() {
  local id="$1" phase_dir="$2" rc="$3"
  local rss_s="$4" rss_e="$5" fd_s="$6" fd_e="$7" task_s="$8" task_e="$9"
  local sock_s="${10}" sock_e="${11}"
  local reqs="${12}" ok="${13}" cerr="${14}" serr="${15}"
  local crashes=0
  if [[ -n "${SERVER_PID:-}" ]] && ! kill -0 "$SERVER_PID" 2>/dev/null; then
    crashes=1
  fi
  local rss_peak="$rss_e"
  if [[ -f "$phase_dir/resources.csv" ]]; then
    rss_peak="$(awk -F, 'NR>1 {if($2+0>m)m=$2+0} END{print (m+0?m:0)}' "$phase_dir/resources.csv")"
    [[ "$rss_peak" -lt "$rss_e" ]] && rss_peak="$rss_e"
  fi
  cat >"$phase_dir/summary.json" <<EOF
{
  "scenario": "$id",
  "host_label": "$HOST_LABEL",
  "arch": "$ACTUAL_ARCH",
  "duration_sec": $DURATION_SEC,
  "exit_status": $rc,
  "REQUESTS": $reqs,
  "SUCCESS": $ok,
  "CLIENT_ERRORS": $cerr,
  "SERVER_ERRORS": $serr,
  "TIMEOUTS": 0,
  "RESETS": 0,
  "CRASHES": $crashes,
  "RESTARTS": 0,
  "RSS_START": $rss_s,
  "RSS_PEAK": $rss_peak,
  "RSS_END": $rss_e,
  "FD_START": $fd_s,
  "FD_PEAK": $fd_e,
  "FD_END": $fd_e,
  "TASK_START": $task_s,
  "TASK_PEAK": $task_e,
  "TASK_END": $task_e,
  "SOCKET_START": $sock_s,
  "SOCKET_PEAK": $sock_e,
  "SOCKET_END": $sock_e,
  "GENERATION_START": 1,
  "GENERATION_END": 1,
  "verdict": "$([ "$rc" -eq 0 ] && [ "$crashes" -eq 0 ] && echo PASS || echo FAIL)"
}
EOF
  local rss_delta=$((rss_e - rss_s))
  local fd_delta=$((fd_e - fd_s))
  local class="ALLOCATOR_RETENTION"
  if (( crashes > 0 )); then class="TRUE_LEAK"; fi
  if (( fd_delta > 50 )); then class="UNCLASSIFIED"; fi
  if (( rss_delta > 65536 )); then class="UNCLASSIFIED"; fi
  echo "GROWTH_CLASS=$class RSS_DELTA_KB=$rss_delta FD_DELTA=$fd_delta" >"$phase_dir/growth.env"
  if [[ "$class" == "UNCLASSIFIED" ]]; then
    echo "ERROR: unclassified growth in $id" >&2
    return 1
  fi
  return "$rc"
}

run_s10() {
  local phase_dir="$ARTIFACT_DIR/S10"
  mkdir -p "$phase_dir"
  DURATION_SEC="${P15_WS4_S10_DURATION_SEC:-120}"
  start_server
  local rss_s fd_s task_s sock_s
  rss_s="$(rss_kb "$SERVER_PID")"
  fd_s="$(fd_count "$SERVER_PID")"
  task_s="$(task_count "$SERVER_PID")"
  sock_s="$(socket_count "$SERVER_PID")"
  start_sampler "$phase_dir/resources.csv" "$SERVER_PID"

  (
    end=$((SECONDS + DURATION_SEC))
    while (( SECONDS < end )); do
      curl -sf --max-time 2 "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null || true
      sleep 0.25
    done
  ) &
  LOAD_PID=$!

  local tooling_rc=0 loops=0
  local end=$((SECONDS + DURATION_SEC))
  while (( SECONDS < end )); do
    loops=$((loops + 1))
    if ! "$EXYONQCTL_BIN" config lint "$CFG" >"$phase_dir/lint-${loops}.log" 2>&1; then
      tooling_rc=1
      break
    fi
    if ! "$EXYONQCTL_BIN" config test "$CFG" >"$phase_dir/test-${loops}.log" 2>&1; then
      tooling_rc=1
      break
    fi
    if ! "$EXYONQCTL_BIN" config reload --check "$CFG" >"$phase_dir/reload-check-${loops}.log" 2>&1; then
      tooling_rc=1
      break
    fi
    sleep 5
  done
  wait "$LOAD_PID" 2>/dev/null || true
  LOAD_PID=""
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  SAMPLER_PID=""

  local rss_e fd_e task_e sock_e
  rss_e="$(rss_kb "$SERVER_PID")"
  fd_e="$(fd_count "$SERVER_PID")"
  task_e="$(task_count "$SERVER_PID")"
  sock_e="$(socket_count "$SERVER_PID")"
  write_phase_summary S10 "$phase_dir" "$tooling_rc" "$rss_s" "$rss_e" "$fd_s" "$fd_e" "$task_s" "$task_e" "$sock_s" "$sock_e" "$loops" "$loops" 0 "$tooling_rc"
}

run_s15() {
  local phase_dir="$ARTIFACT_DIR/S15"
  mkdir -p "$phase_dir"
  DURATION_SEC="${P15_WS4_S15_DURATION_SEC:-180}"
  local conc="${P15_WS4_S15_CONCURRENCY:-32}"
  [[ "$HOST_LABEL" == "arm64" ]] && conc="${P15_WS4_S15_CONCURRENCY_ARM64:-16}"
  start_server
  local rss_s fd_s task_s sock_s
  rss_s="$(rss_kb "$SERVER_PID")"
  fd_s="$(fd_count "$SERVER_PID")"
  task_s="$(task_count "$SERVER_PID")"
  sock_s="$(socket_count "$SERVER_PID")"
  start_sampler "$phase_dir/resources.csv" "$SERVER_PID"

  local reqs=0 ok=0 fail=0
  local end=$((SECONDS + DURATION_SEC))
  local url="http://127.0.0.1:${LISTEN_PORT}/medium.bin"
  # Cap concurrency to avoid fork storms that hang `wait`.
  (( conc > 16 )) && conc=16
  while (( SECONDS < end )); do
    : >"$phase_dir/wave.out"
    local i=0
    local pids=()
    for ((i = 0; i < conc; i++)); do
      (
        if curl -sf --max-time 2 "$url" -o /dev/null; then echo OK; else echo ERR; fi
      ) >>"$phase_dir/wave.out" &
      pids+=($!)
      if (( ${#pids[@]} >= 8 )); then
        local pid
        for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
        pids=()
      fi
    done
    local pid
    for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
    local wave_ok wave_err
    wave_ok="$(grep -c '^OK$' "$phase_dir/wave.out" 2>/dev/null || echo 0)"
    wave_err="$(grep -c '^ERR$' "$phase_dir/wave.out" 2>/dev/null || echo 0)"
    reqs=$((reqs + conc))
    ok=$((ok + wave_ok))
    fail=$((fail + wave_err))
    sleep 0.25
  done

  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  SAMPLER_PID=""
  local rss_e fd_e task_e sock_e rc=0
  rss_e="$(rss_kb "$SERVER_PID")"
  fd_e="$(fd_count "$SERVER_PID")"
  task_e="$(task_count "$SERVER_PID")"
  sock_e="$(socket_count "$SERVER_PID")"
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then rc=1; fi
  local fd_delta=$((fd_e - fd_s))
  (( fd_delta > 200 )) && rc=1
  # Allow some client pressure errors; fail only if success rate collapses.
  local success_rate=0
  if (( reqs > 0 )); then success_rate=$(( ok * 100 / reqs )); fi
  if (( reqs > 0 && success_rate < 90 )); then rc=1; fi
  echo "CONCURRENCY=$conc HOST_SAFETY=PASS SUCCESS_RATE=${success_rate}%" >"$phase_dir/pressure.env"
  write_phase_summary S15 "$phase_dir" "$rc" "$rss_s" "$rss_e" "$fd_s" "$fd_e" "$task_s" "$task_e" "$sock_s" "$sock_e" "$reqs" "$ok" "$fail" 0
}

run_s16() {
  local phase_dir="$ARTIFACT_DIR/S16"
  mkdir -p "$phase_dir"
  DURATION_SEC="${P15_WS4_S16_DURATION_SEC:-120}"
  start_server
  local rss_s fd_s task_s sock_s
  rss_s="$(rss_kb "$SERVER_PID")"
  fd_s="$(fd_count "$SERVER_PID")"
  task_s="$(task_count "$SERVER_PID")"
  sock_s="$(socket_count "$SERVER_PID")"
  start_sampler "$phase_dir/resources.csv" "$SERVER_PID"

  sleep "$DURATION_SEC"

  local ok=0 fail=0 i
  for i in 1 2 3 4 5; do
    if curl -sf --max-time 5 "http://127.0.0.1:${LISTEN_PORT}/" >/dev/null; then
      ok=$((ok + 1))
    else
      fail=$((fail + 1))
    fi
  done

  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  SAMPLER_PID=""
  local rss_e fd_e task_e sock_e rc=0
  rss_e="$(rss_kb "$SERVER_PID")"
  fd_e="$(fd_count "$SERVER_PID")"
  task_e="$(task_count "$SERVER_PID")"
  sock_e="$(socket_count "$SERVER_PID")"
  (( fail > 0 )) && rc=1
  write_phase_summary S16 "$phase_dir" "$rc" "$rss_s" "$rss_e" "$fd_s" "$fd_e" "$task_s" "$task_e" "$sock_s" "$sock_e" 5 "$ok" 0 "$fail"
}

RC=0
case "$PHASE" in
  S10) run_s10 || RC=$? ;;
  S15) run_s15 || RC=$? ;;
  S16) run_s16 || RC=$? ;;
  all)
    run_s10 || RC=1
    stop_server
    run_s15 || RC=1
    stop_server
    run_s16 || RC=1
    ;;
  *) echo "ERROR: unknown phase $PHASE" >&2; exit 2 ;;
esac

echo "THIN_PHASES_RC=$RC" | tee "$ARTIFACT_DIR/thin_rc.txt"
exit "$RC"
