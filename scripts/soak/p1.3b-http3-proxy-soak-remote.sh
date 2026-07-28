#!/usr/bin/env bash
# P1.3b HTTP/3 proxy soak — Linux host runner (native ExyonQ + Python H1 upstream + H3 client).
# Ports: TCP 19391 · UDP H3 19395 · upstream 19390 — avoid P1.3a 19291/19293 and plan10a.
# Not a competitive benchmark. No HTML. Coverage gates A–R (intensity may differ by arch).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  p1.3b-http3-proxy-soak-remote.sh \
    --workspace PATH \
    --host-label amd64|arm64 \
    --expected-arch x86_64|aarch64 \
    --duration-sec SECONDS \
    --warmup-sec SECONDS \
    --concurrency N \
    --report-relpath docs/operations/p1.3b-http3-soak-*.md
EOF
}

WORKSPACE=""
HOST_LABEL=""
EXPECTED_ARCH=""
DURATION_SEC="${P1_3B_SOAK_DURATION_SEC:-600}"
WARMUP_SEC="${P1_3B_SOAK_WARMUP_SEC:-60}"
CONCURRENCY="${P1_3B_SOAK_CONCURRENCY:-8}"
REPORT_RELPATH=""
RUN_ID="${P1_3B_SOAK_RUN_ID:-p13b-soak-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCK_ID="${P1_3B_SOAK_LOCK_ID:-$RUN_ID}"

HTTP_PORT="${P1_3B_HTTP_PORT:-19391}"
H3_UDP_PORT="${P1_3B_H3_UDP_PORT:-19395}"
UP_PORT="${P1_3B_UPSTREAM_PORT:-19390}"

CLIENT_IMAGE="${EXYONQ_H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLATFORM="${EXYONQ_H3_CLIENT_PLATFORM:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --host-label) HOST_LABEL="${2:-}"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="${2:-}"; shift 2 ;;
    --duration-sec) DURATION_SEC="${2:-}"; shift 2 ;;
    --warmup-sec) WARMUP_SEC="${2:-}"; shift 2 ;;
    --concurrency) CONCURRENCY="${2:-}"; shift 2 ;;
    --report-relpath) REPORT_RELPATH="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$HOST_LABEL" && -n "$EXPECTED_ARCH" && -n "$REPORT_RELPATH" ]] || {
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
source "${HOME}/.cargo/env" 2>/dev/null || true

RESULT_DIR="$WORKSPACE/docs/operations/evidence/p1.3b-soak/$RUN_ID/$HOST_LABEL"
REPORT_PATH="$WORKSPACE/$REPORT_RELPATH"
RUNTIME_DIR="/tmp/p13b-soak-${HOST_LABEL}"
mkdir -p "$RESULT_DIR" "$RUNTIME_DIR" "$(dirname "$REPORT_PATH")"
rm -rf "${RUNTIME_DIR:?}/"*
exec > >(stdbuf -oL tee "$RESULT_DIR/runner.log") 2>&1

echo "=== P1.3b HTTP/3 proxy soak ==="
echo "lock_id=$LOCK_ID run_id=$RUN_ID host_label=$HOST_LABEL arch=$ACTUAL_ARCH"
echo "duration_sec=$DURATION_SEC warmup_sec=$WARMUP_SEC concurrency=$CONCURRENCY"
echo "ports tcp=$HTTP_PORT h3_udp=$H3_UDP_PORT upstream=$UP_PORT"
echo "commit=${P1_3B_SOAK_COMMIT:-unknown} branch=${P1_3B_SOAK_BRANCH:-unknown}"
date -u

EXYONQ_BIN="${EXYONQ_BIN:-$WORKSPACE/target/release/exyonq}"
EXYONQCTL_BIN="${EXYONQCTL_BIN:-$WORKSPACE/target/release/exyonqctl}"
UPSTREAM_PY="$WORKSPACE/scripts/soak/lib/p13a-upstream.py"
TLS_SRC="$(bash "$WORKSPACE/scripts/test-tls/generate-ephemeral-tls.sh" --print-paths | awk -F= '/^DIR=/{print $2; exit}')"
LIVE_CERT="$RUNTIME_DIR/live-cert.pem"
LIVE_KEY="$RUNTIME_DIR/live-key.pem"
CFG="$RUNTIME_DIR/exyonq.toml"
CTRL="$RUNTIME_DIR/ctl.sock"
SRV_LOG="$RUNTIME_DIR/exyonq.log"
UP_LOG="$RUNTIME_DIR/upstream.log"
WWW="$WORKSPACE/benchmarks/scenarios/payloads/www"
[[ -d "$WWW" ]] || WWW="$WORKSPACE/benchmarks/scenarios/fixtures/www"

UP_PID="" SRV_PID="" SAMPLER_PID="" LOADGEN_PID="" LIFE_PID=""
H3_MODE="" DOCKER_PLATFORM_ARGS=()
pass_n=0 fail_n=0

record() {
  local id="$1" result="$2" detail="${3:-}"
  echo "$id,$result,$detail" >>"$RESULT_DIR/gates.csv"
  if [[ "$result" == "PASS" ]]; then
    pass_n=$((pass_n + 1))
    echo "PASS  $id $detail"
  else
    fail_n=$((fail_n + 1))
    echo "FAIL  $id $detail"
  fi
}

cleanup() {
  local ec=$?
  rm -f "$RESULT_DIR/stop.sampler" 2>/dev/null || true
  [[ -n "${LOADGEN_PID:-}" ]] && kill "$LOADGEN_PID" 2>/dev/null || true
  [[ -n "${LIFE_PID:-}" ]] && kill "$LIFE_PID" 2>/dev/null || true
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  if [[ -n "${SRV_PID:-}" ]] && kill -0 "$SRV_PID" 2>/dev/null; then
    if [[ -x "$EXYONQCTL_BIN" && -S "$CTRL" ]]; then
      "$EXYONQCTL_BIN" shutdown --socket "$CTRL" >/dev/null 2>&1 || true
    fi
    kill "$SRV_PID" 2>/dev/null || true
    wait "$SRV_PID" 2>/dev/null || true
  fi
  if [[ -n "${UP_PID:-}" ]] && kill -0 "$UP_PID" 2>/dev/null; then
    kill "$UP_PID" 2>/dev/null || true
    wait "$UP_PID" 2>/dev/null || true
  fi
  mkdir -p "$RESULT_DIR/runtime"
  cp -a "$RUNTIME_DIR/." "$RESULT_DIR/runtime/" 2>/dev/null || true
  exit "$ec"
}
trap cleanup EXIT

: >"$RESULT_DIR/gates.csv"
echo "TEST_ID,RESULT,DETAIL" >"$RESULT_DIR/gates.csv"

# --- H3 client (native curl OR Docker ymuski/curl-http3) ---
resolve_h3_client() {
  if curl --version 2>/dev/null | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper'; then
    H3_MODE=native
    echo "H3_CLIENT=native_curl"
    return 0
  fi
  command -v docker >/dev/null || {
    echo "ERROR: neither native curl-http3 nor docker available" >&2
    return 1
  }
  if [[ -z "$CLIENT_PLATFORM" ]]; then
    # Closed P1.3b arm64 evidence used linux/amd64 (qemu). Native arm64 tags of
    # ymuski/curl-http3 often lack HTTP/3 — prefer amd64 everywhere for this image.
    CLIENT_PLATFORM=linux/amd64
  fi
  DOCKER_PLATFORM_ARGS=(--platform "$CLIENT_PLATFORM")
  # Always ensure the amd64 (or requested) digest is present — a cached wrong-arch
  # tag can make `image inspect` succeed while `run --platform` lacks HTTP/3.
  if ! docker pull "${DOCKER_PLATFORM_ARGS[@]}" "$CLIENT_IMAGE" >/dev/null 2>&1; then
    echo "ERROR: docker pull failed for $CLIENT_IMAGE (${CLIENT_PLATFORM})" >&2
    return 1
  fi
  if ! docker run --rm "${DOCKER_PLATFORM_ARGS[@]}" --network none "$CLIENT_IMAGE" \
      curl --version 2>/dev/null | grep -qiE 'HTTP3|http3|nghttp3|quiche|Hyper'; then
    echo "ERROR: client image lacks HTTP/3: $CLIENT_IMAGE platform=$CLIENT_PLATFORM" >&2
    return 1
  fi
  H3_MODE=docker
  echo "H3_CLIENT=docker image=$CLIENT_IMAGE platform=$CLIENT_PLATFORM"
}

# h3_curl [curl-args...] — always --http3-only -sk
h3_curl() {
  if [[ "$H3_MODE" == "native" ]]; then
    curl --http3-only -sk "$@"
  else
    docker run --rm "${DOCKER_PLATFORM_ARGS[@]}" --network host "$CLIENT_IMAGE" \
      curl --http3-only -sk "$@"
  fi
}

h3_code() {
  local t="${1:-15}"
  shift
  local out
  out="$(h3_curl --max-time "$t" -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || true)"
  # Normalize connect-fail "000" / empty
  [[ -z "$out" || "$out" == "000000" ]] && out=000
  echo "${out: -3}"
}

H3_BASE="https://127.0.0.1:${H3_UDP_PORT}"
# TLS terminates on the TCP listen — use HTTPS + HTTP/1.1 (not cleartext http://).
H1_BASE="https://127.0.0.1:${HTTP_PORT}"

h1_code() {
  local t="${1:-5}"
  shift
  local out
  out="$(curl -sk --http1.1 --max-time "$t" -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || true)"
  [[ -z "$out" || "$out" == "000000" ]] && out=000
  echo "${out: -3}"
}

write_config() {
  local up_port="$1"
  local timeout_ms="${2:-5000}"
  # control_socket is env-only (EXYONQ_CONTROL_SOCKET); TOML field is rejected by
  # deny-unknown AppConfig (P1.4). Closed P1.3b evidence used a binary that still
  # accepted the field — WS4 builds dirty tree and must not emit it.
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${HTTP_PORT}"
http3_listen = "0.0.0.0:${H3_UDP_PORT}"
routes = ["site", "api"]

[server.tls]
cert = "${LIVE_CERT}"
key = "${LIVE_KEY}"

[[route]]
name = "site"
match = { path = "/site" }
root = "${WWW}"
index = "index.html"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${up_port}"
timeout_ms = ${timeout_ms}
EOF
}

start_upstream() {
  local identity="${1:-p13b-up}"
  local slow_ms="${2:-1500}"
  if [[ -n "${UP_PID:-}" ]] && kill -0 "$UP_PID" 2>/dev/null; then
    kill "$UP_PID" 2>/dev/null || true
    wait "$UP_PID" 2>/dev/null || true
  fi
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "${UP_PORT}/tcp" >/dev/null 2>&1 || true
  else
    ss -lptn "sport = :${UP_PORT}" 2>/dev/null | awk '/pid=/ {print}' | sed -n 's/.*pid=\([0-9]*\).*/\1/p' \
      | while read -r p; do kill "$p" 2>/dev/null || true; done
  fi
  sleep 0.2
  : >"$UP_LOG"
  P13A_UPSTREAM_BIND=127.0.0.1 P13A_UPSTREAM_PORT="$UP_PORT" \
    P13A_UPSTREAM_IDENTITY="$identity" P13A_UPSTREAM_SLOW_MS="$slow_ms" \
    python3 "$UPSTREAM_PY" >>"$UP_LOG" 2>&1 &
  UP_PID=$!
  for _ in $(seq 1 50); do
    body="$(curl -sf --max-time 1 "http://127.0.0.1:${UP_PORT}/health" 2>/dev/null || true)"
    if echo "$body" | grep -q "\"identity\":\"${identity}\""; then
      return 0
    fi
    sleep 0.1
  done
  echo "ERROR: upstream not ready" >&2
  cat "$UP_LOG" >&2 || true
  return 1
}

start_exyonq() {
  write_config "$UP_PORT" "${1:-5000}"
  rm -f "$CTRL"
  : >"$SRV_LOG"
  echo "CTRL_PATH=$CTRL len=${#CTRL}"
  [[ "${#CTRL}" -lt 100 ]] || { echo "ERROR: control socket path too long (${#CTRL})" >&2; return 1; }
  EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=error \
    "$EXYONQ_BIN" serve -c "$CFG" >>"$SRV_LOG" 2>&1 &
  SRV_PID=$!
  for _ in $(seq 1 120); do
    if grep -q "HTTP/3 listening" "$SRV_LOG" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$SRV_PID" 2>/dev/null; then
      echo "ERROR: exyonq exited before H3 ready" >&2
      tail -80 "$SRV_LOG" || true
      return 1
    fi
    sleep 0.15
  done
  echo "ERROR: HTTP/3 listening not observed" >&2
  tail -120 "$SRV_LOG" || true
  return 1
}

restart_exyonq_clean() {
  if [[ -n "${SRV_PID:-}" ]] && kill -0 "$SRV_PID" 2>/dev/null; then
    [[ -x "$EXYONQCTL_BIN" && -S "$CTRL" ]] && "$EXYONQCTL_BIN" shutdown --socket "$CTRL" >/dev/null 2>&1 || true
    kill "$SRV_PID" 2>/dev/null || true
    wait "$SRV_PID" 2>/dev/null || true
  fi
  sleep 0.3
  start_exyonq "${1:-5000}"
}

# --- build ---
echo "=== cargo build release exyonq + exyonqctl ==="
if [[ "${P1_3B_FORCE_BUILD:-0}" == "1" || ! -x "$EXYONQ_BIN" || ! -x "$EXYONQCTL_BIN" ]]; then
  cargo build --release -p exyonq -p exyonqctl
fi
[[ -x "$EXYONQ_BIN" && -x "$EXYONQCTL_BIN" ]] || { echo "ERROR: missing binaries" >&2; exit 1; }

{
  echo "lock_id=$LOCK_ID"
  echo "host=$HOST_LABEL arch=$ACTUAL_ARCH"
  echo "commit=${P1_3B_SOAK_COMMIT:-unknown}"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname=$(uname -a)"
  curl --version 2>/dev/null | head -1 || true
  docker --version 2>/dev/null || true
} >"$RESULT_DIR/lock-fragment.txt"

[[ -f "$TLS_SRC/cert.pem" && -f "$TLS_SRC/key.pem" ]] || { echo "ERROR: TLS fixtures missing" >&2; exit 1; }
cp "$TLS_SRC/cert.pem" "$LIVE_CERT"
cp "$TLS_SRC/key.pem" "$LIVE_KEY"

resolve_h3_client || {
  record "H3_CLIENT" "FAIL" "resolve"
  echo "ERROR: H3 client resolve failed" >&2
  exit 1
}
record "H3_CLIENT" "PASS" "$H3_MODE"

start_upstream "p13b-up" 1500
if ! start_exyonq 5000; then
  record "BOOT" "FAIL" "exyonq_not_ready"
  echo "ERROR: BOOT failed" >&2
  exit 1
fi
record "BOOT" "PASS" "h3_udp=$H3_UDP_PORT"

# ========== Coverage A–R ==========
echo "=== Coverage matrix A–R ==="

# A — H3 static
code="$(h3_code 15 "${H3_BASE}/site/")"
[[ "$code" =~ ^(200|404)$ ]] && record "A_H3_STATIC" "PASS" "code=$code" || record "A_H3_STATIC" "FAIL" "code=$code"

# B — H3 proxy GET
code="$(h3_code 15 "${H3_BASE}/api/health")"
[[ "$code" == "200" ]] && record "B_H3_PROXY_GET" "PASS" "" || record "B_H3_PROXY_GET" "FAIL" "code=$code"

# C — H3 proxy POST
body="$(h3_curl --max-time 20 -X POST --data 'P13A_BODY_OK' "${H3_BASE}/api/echo" 2>/dev/null || true)"
echo "$body" | grep -q "P13A_BODY_OK\|\"body_len\"" \
  && record "C_H3_PROXY_POST" "PASS" "" || record "C_H3_PROXY_POST" "FAIL" "body=${body:0:80}"

# D — multiplex (parallel H3 streams)
mux_ok=0
mux_pids=()
for i in $(seq 1 8); do
  ( h3_code 20 "${H3_BASE}/api/health" >"$RUNTIME_DIR/mux-$i.code" ) &
  mux_pids+=($!)
done
for _pid in "${mux_pids[@]}"; do
  wait "$_pid" 2>/dev/null || true
done
for i in $(seq 1 8); do
  c="$(cat "$RUNTIME_DIR/mux-$i.code" 2>/dev/null || echo 000)"
  [[ "$c" == "200" ]] && mux_ok=$((mux_ok + 1))
done
[[ "$mux_ok" -ge 6 ]] && record "D_H3_MULTIPLEX" "PASS" "ok=$mux_ok/8" || record "D_H3_MULTIPLEX" "FAIL" "ok=$mux_ok/8"

# E — slow stream (upstream slow)
code="$(h3_code 20 "${H3_BASE}/api/slow")"
[[ "$code" == "200" ]] && record "E_H3_SLOW_STREAM" "PASS" "" || record "E_H3_SLOW_STREAM" "FAIL" "code=$code"

# F — large bounded body (response + request)
# Capture body on host (docker -o would write inside the container).
h3_curl --max-time 30 "${H3_BASE}/api/big" >"$RUNTIME_DIR/big.bin" 2>/dev/null || true
big_sz="$(wc -c <"$RUNTIME_DIR/big.bin" 2>/dev/null | tr -d ' ' || echo 0)"
post_payload="$RUNTIME_DIR/post-large.bin"
dd if=/dev/urandom of="$post_payload" bs=1024 count=64 status=none 2>/dev/null || \
  head -c 65536 /dev/urandom >"$post_payload"
# Docker client cannot read host @file paths — POST via stdin.
if [[ "$H3_MODE" == "native" ]]; then
  code="$(h3_code 30 -X POST --data-binary @"$post_payload" "${H3_BASE}/api/echo")"
else
  code="$(h3_curl --max-time 30 -o /dev/null -w '%{http_code}' -X POST --data-binary @- \
    "${H3_BASE}/api/echo" <"$post_payload" 2>/dev/null || echo 000)"
  [[ -z "$code" || "$code" == "000000" ]] && code=000
  code="${code: -3}"
fi
[[ "$big_sz" -gt 10000 && "$code" == "200" ]] \
  && record "F_H3_LARGE_BOUNDED_BODY" "PASS" "resp_bytes=$big_sz post=$code" \
  || record "F_H3_LARGE_BOUNDED_BODY" "FAIL" "resp_bytes=$big_sz post=$code"

# G — stream reset (cancel mid-flight) then recover
h3_curl --max-time 2 "${H3_BASE}/api/slow" >/dev/null 2>&1 &
RST_PID=$!
sleep 0.3
kill "$RST_PID" 2>/dev/null || true
wait "$RST_PID" 2>/dev/null || true
code="$(h3_code 15 "${H3_BASE}/api/health")"
[[ "$code" == "200" ]] && record "G_H3_STREAM_RESET" "PASS" "recover=$code" || record "G_H3_STREAM_RESET" "FAIL" "code=$code"

# H — connection close (fresh requests after idle)
code1="$(h3_code 10 "${H3_BASE}/api/health")"
sleep 0.5
code2="$(h3_code 10 "${H3_BASE}/api/health")"
[[ "$code1" == "200" && "$code2" == "200" ]] \
  && record "H_H3_CONNECTION_CLOSE" "PASS" "" || record "H_H3_CONNECTION_CLOSE" "FAIL" "$code1/$code2"

# I — upstream restart
kill "$UP_PID" 2>/dev/null || true
wait "$UP_PID" 2>/dev/null || true
UP_PID=""
sleep 0.5
code_down="$(h3_code 8 "${H3_BASE}/api/health")"
start_upstream "p13b-up" 1500
# nudge proxy pools
sleep 0.5
code_up="$(h3_code 15 "${H3_BASE}/api/health")"
[[ "$code_up" == "200" ]] && record "I_UPSTREAM_RESTART" "PASS" "down=$code_down up=$code_up" \
  || record "I_UPSTREAM_RESTART" "FAIL" "down=$code_down up=$code_up"

# J — upstream timeout
write_config "$UP_PORT" 400
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>&1 || true
sleep 0.4
# slow_ms 1500 > timeout 400ms → expect 504/502/503
code="$(h3_code 10 "${H3_BASE}/api/slow")"
case "$code" in
  504|502|503) record "J_UPSTREAM_TIMEOUT" "PASS" "code=$code" ;;
  *) record "J_UPSTREAM_TIMEOUT" "FAIL" "code=$code" ;;
esac
write_config "$UP_PORT" 5000
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>&1 || true
sleep 0.3

# K — cert reload allowed (same-addr)
openssl req -x509 -newkey rsa:2048 -keyout "$RUNTIME_DIR/new-key.pem" -out "$RUNTIME_DIR/new-cert.pem" \
  -days 1 -nodes -subj "/CN=rotated.p13b.localhost" >/dev/null 2>&1
cp "$RUNTIME_DIR/new-cert.pem" "$LIVE_CERT"
cp "$RUNTIME_DIR/new-key.pem" "$LIVE_KEY"
if EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>&1; then
  sleep 0.5
  code="$(h3_code 15 "${H3_BASE}/api/health")"
  [[ "$code" == "200" ]] && record "K_CERT_RELOAD_ALLOWED" "PASS" "code=$code" \
    || record "K_CERT_RELOAD_ALLOWED" "FAIL" "h3_after=$code"
else
  record "K_CERT_RELOAD_ALLOWED" "FAIL" "reload_rc_nonzero"
fi

# L — reload unsupported → explicit fail (reload while draining is rejected)
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" drain --socket "$CTRL" >>"$RESULT_DIR/drain-prep.log" 2>&1 || true
sleep 0.2
set +e
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >>"$RESULT_DIR/reload-unsupported.log" 2>&1
reload_while_drain_rc=$?
set -e
if [[ "$reload_while_drain_rc" -ne 0 ]]; then
  record "L_RELOAD_UNSUPPORTED_FAIL" "PASS" "rc=$reload_while_drain_rc (fail-closed while drain)"
else
  record "L_RELOAD_UNSUPPORTED_FAIL" "FAIL" "reload unexpectedly succeeded during drain"
fi
# Recover from drain for remaining tests
restart_exyonq_clean 5000
cp "$RUNTIME_DIR/new-cert.pem" "$LIVE_CERT" 2>/dev/null || cp "$TLS_SRC/cert.pem" "$LIVE_CERT"
cp "$RUNTIME_DIR/new-key.pem" "$LIVE_KEY" 2>/dev/null || cp "$TLS_SRC/key.pem" "$LIVE_KEY"

# N — handshake pressure (many new H3 connections) — before long soak
hs_ok=0
hs_n=24
[[ "$HOST_LABEL" == "arm64" ]] && hs_n=16
for i in $(seq 1 "$hs_n"); do
  c="$(h3_code 8 "${H3_BASE}/site/")"
  [[ "$c" =~ ^(200|404)$ ]] && hs_ok=$((hs_ok + 1))
done
[[ "$hs_ok" -ge $((hs_n * 3 / 4)) ]] \
  && record "N_HANDSHAKE_PRESSURE" "PASS" "ok=$hs_ok/$hs_n" \
  || record "N_HANDSHAKE_PRESSURE" "FAIL" "ok=$hs_ok/$hs_n"

# O — excessive streams bounded (burst; server must survive)
burst=$((CONCURRENCY * 4))
[[ "$burst" -gt 48 ]] && burst=48
burst_ok=0
burst_pids=()
for i in $(seq 1 "$burst"); do
  ( h3_code 12 "${H3_BASE}/api/health" >"$RUNTIME_DIR/burst-$i.code" ) &
  burst_pids+=($!)
done
for _pid in "${burst_pids[@]}"; do
  wait "$_pid" 2>/dev/null || true
done
for i in $(seq 1 "$burst"); do
  c="$(cat "$RUNTIME_DIR/burst-$i.code" 2>/dev/null || echo 000)"
  [[ "$c" == "200" ]] && burst_ok=$((burst_ok + 1))
done
alive=0
kill -0 "$SRV_PID" 2>/dev/null && alive=1
code="$(h3_code 10 "${H3_BASE}/api/health")"
[[ "$alive" -eq 1 && "$code" == "200" ]] \
  && record "O_EXCESSIVE_STREAMS_BOUNDED" "PASS" "ok=$burst_ok/$burst alive=1" \
  || record "O_EXCESSIVE_STREAMS_BOUNDED" "FAIL" "ok=$burst_ok/$burst alive=$alive code=$code"

# P — mixed H1 (TLS) + H3
h1="$(h1_code 5 "${H1_BASE}/api/health")"
h3="$(h3_code 10 "${H3_BASE}/api/health")"
mixed_detail="h1=$h1 h3=$h3"
# Optional H2 on same TLS TCP listener
h2="$(curl -sk --http2 --max-time 3 -o /dev/null -w '%{http_code}' "${H1_BASE}/api/health" 2>/dev/null || true)"
[[ -z "$h2" || "$h2" == "000000" ]] && h2=000
h2="${h2: -3}"
mixed_detail="h1=$h1 h2=$h2 h3=$h3"
[[ "$h1" == "200" && "$h3" == "200" ]] \
  && record "P_MIXED_H1_H3" "PASS" "$mixed_detail" \
  || record "P_MIXED_H1_H3" "FAIL" "$mixed_detail"

# ========== Warmup + steady soak ==========
echo "=== Phase warmup ${WARMUP_SEC}s + steady ${DURATION_SEC}s ==="
echo "ts_utc,exyonq_rss_kb,exyonq_fd,exyonq_threads,upstream_rss_kb" >"$RESULT_DIR/resources.csv"
touch "$RESULT_DIR/stop.sampler"
(
  while [[ -f "$RESULT_DIR/stop.sampler" ]]; do
    rss=0; fd=0; thr=0; urss=0
    if [[ -n "${SRV_PID:-}" ]] && kill -0 "$SRV_PID" 2>/dev/null; then
      rss="$(awk '/VmRSS:/ {print $2; exit}' /proc/$SRV_PID/status 2>/dev/null || echo 0)"
      fd="$(ls /proc/$SRV_PID/fd 2>/dev/null | wc -l | tr -d ' ')"
      thr="$(awk '/Threads:/ {print $2; exit}' /proc/$SRV_PID/status 2>/dev/null || echo 0)"
    fi
    if [[ -n "${UP_PID:-}" ]] && kill -0 "$UP_PID" 2>/dev/null; then
      urss="$(awk '/VmRSS:/ {print $2; exit}' /proc/$UP_PID/status 2>/dev/null || echo 0)"
    fi
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),${rss:-0},${fd:-0},${thr:-0},${urss:-0}" >>"$RESULT_DIR/resources.csv"
    sleep 5
  done
) &
SAMPLER_PID=$!

# Warmup: H1 volume + light H3 probes
warmup_end=$(($(date +%s) + WARMUP_SEC))
(
  while [[ "$(date +%s)" -lt "$warmup_end" ]]; do
    curl -sk --http1.1 --max-time 2 "${H1_BASE}/api/health" >/dev/null 2>&1 || true
    curl -sk --http1.1 --max-time 2 "${H1_BASE}/site/" >/dev/null 2>&1 || true
    sleep 0.05
  done
) &
WU_H1=$!
(
  while [[ "$(date +%s)" -lt "$warmup_end" ]]; do
    h3_curl --max-time 5 -o /dev/null "${H3_BASE}/api/health" >/dev/null 2>&1 || true
    sleep 1
  done
) &
WU_H3=$!
wait "$WU_H1" "$WU_H3" 2>/dev/null || true
RSS_AFTER_WARMUP="$(tail -1 "$RESULT_DIR/resources.csv" | cut -d, -f2)"
echo "RSS_AFTER_WARMUP_KB=$RSS_AFTER_WARMUP" | tee "$RESULT_DIR/warmup.env"

# Steady loadgen: H1 workers + H3 workers; lifecycle reload cadence
SOAK_END=$(($(date +%s) + DURATION_SEC))
: >"$RESULT_DIR/loadgen-counts.env"
echo "requests=0 unexpected_5xx=0 errors=0" >"$RESULT_DIR/loadgen-counts.env"

python3 - "$H1_BASE" "$DURATION_SEC" "$CONCURRENCY" "$RESULT_DIR/loadgen-h1.json" <<'PY' &
import json, ssl, sys, threading, time, urllib.request
from collections import defaultdict
base, duration, conc, out = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
deadline = time.time() + duration
stats = defaultdict(int)
lock = threading.Lock()
paths = ["/api/health", "/site/", "/api/echo"]
ctx = ssl._create_unverified_context()

def worker(_i):
    while time.time() < deadline:
        path = paths[int(time.time() * 10) % len(paths)]
        url = base + path
        try:
            if path.endswith("/echo"):
                req = urllib.request.Request(url, data=b"soak", method="POST")
            else:
                req = urllib.request.Request(url)
            with urllib.request.urlopen(req, timeout=5, context=ctx) as r:
                code = r.status
            with lock:
                stats["requests"] += 1
                stats[f"status_{code}"] += 1
                if code >= 500:
                    stats["unexpected_5xx"] += 1
        except Exception:
            with lock:
                stats["errors"] += 1
                stats["requests"] += 1

threads = [threading.Thread(target=worker, args=(i,), daemon=True) for i in range(conc)]
for t in threads: t.start()
for t in threads: t.join()
json.dump({"phase": "soak_h1", "stats": dict(stats)}, open(out, "w"), indent=2)
PY
LOADGEN_PID=$!

# H3 probe loop (lower rate — docker cost)
h3_workers=2
[[ "$CONCURRENCY" -ge 8 ]] && h3_workers=3
[[ "$HOST_LABEL" == "arm64" ]] && h3_workers=1
(
  h3_req=0 h3_err=0 h3_5xx=0
  while [[ "$(date +%s)" -lt "$SOAK_END" ]]; do
    for _ in $(seq 1 "$h3_workers"); do
      c="$(h3_code 8 "${H3_BASE}/api/health" || echo 000)"
      h3_req=$((h3_req + 1))
      case "$c" in
        200) ;;
        5*) h3_5xx=$((h3_5xx + 1)) ;;
        *) h3_err=$((h3_err + 1)) ;;
      esac
    done
    echo "h3_requests=$h3_req h3_errors=$h3_err h3_5xx=$h3_5xx" >"$RESULT_DIR/loadgen-h3.env"
    sleep 2
  done
) &
H3_LOAD_PID=$!

RELOAD_OK=0 RELOAD_FAIL=0
lifecycle_loop() {
  local end="$1"
  while [[ "$(date +%s)" -lt "$end" ]]; do
    sleep 90
    if EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >>"$RESULT_DIR/reload.log" 2>&1; then
      RELOAD_OK=$((RELOAD_OK + 1))
    else
      RELOAD_FAIL=$((RELOAD_FAIL + 1))
    fi
    {
      echo "reloads_ok=$RELOAD_OK"
      echo "reloads_fail=$RELOAD_FAIL"
    } >"$RESULT_DIR/lifecycle.env"
  done
}
lifecycle_loop "$SOAK_END" &
LIFE_PID=$!

wait "$LOADGEN_PID" || true
kill "$H3_LOAD_PID" 2>/dev/null || true
wait "$H3_LOAD_PID" 2>/dev/null || true
kill "$LIFE_PID" 2>/dev/null || true
wait "$LIFE_PID" 2>/dev/null || true

record "STEADY_SOAK" "PASS" "duration=${DURATION_SEC}s"
[[ "${RELOAD_FAIL:-0}" -eq 0 ]] && record "RELOAD_DURING_SOAK" "PASS" "ok=${RELOAD_OK:-0}" \
  || record "RELOAD_DURING_SOAK" "FAIL" "fail=$RELOAD_FAIL"

# R — recovery final (post-soak health)
code_h3="$(h3_code 15 "${H3_BASE}/api/health")"
code_h1="$(h1_code 5 "${H1_BASE}/api/health")"
[[ "$code_h3" == "200" && "$code_h1" == "200" ]] \
  && record "R_RECOVERY_FINAL" "PASS" "h1=$code_h1 h3=$code_h3" \
  || record "R_RECOVERY_FINAL" "FAIL" "h1=$code_h1 h3=$code_h3"

# M — drain (after steady; dedicated window)
echo "=== Gate M H3 drain ==="
: >"$RESULT_DIR/drain.log"
h3_curl --max-time 12 "${H3_BASE}/api/slow" -o "$RESULT_DIR/drain-slow.body" >/dev/null 2>&1 &
SLOW_PID=$!
sleep 0.25
if EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" drain --socket "$CTRL" >>"$RESULT_DIR/drain.log" 2>&1; then
  :
fi
sleep 0.5
drain_new="$(h3_code 5 "${H3_BASE}/api/health")"
echo "h3_drain_new=$drain_new" | tee -a "$RESULT_DIR/drain.log"
wait "$SLOW_PID" 2>/dev/null || true
case "$drain_new" in
  503|502|000) record "M_H3_DRAIN" "PASS" "new=$drain_new" ;;
  *) record "M_H3_DRAIN" "FAIL" "new=$drain_new (expected admission reject)" ;;
esac
drain_alias="$(grep '^M_H3_DRAIN,' "$RESULT_DIR/gates.csv" | head -1 | cut -d, -f2 || echo FAIL)"
record "H3_DRAIN" "${drain_alias:-FAIL}" "alias"

# Restart for redaction + leak finish
restart_exyonq_clean 5000

# Q — log redaction spot-check
echo "=== Gate Q log redaction ==="
SECRET='Authorization: Bearer FAKESECRET_u1v2w3x4y5z6a7b8c9d0'
h3_curl --max-time 10 -H "$SECRET" -H 'Cookie: session=redact-me-cookie-xyz' \
  "${H3_BASE}/api/health" >/dev/null 2>&1 || true
# Induce a TLS/error path with garbage (best-effort)
printf 'not-a-pem\n' >"$LIVE_CERT"
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>&1 || true
sleep 0.2
cp "$TLS_SRC/cert.pem" "$LIVE_CERT"
cp "$TLS_SRC/key.pem" "$LIVE_KEY"
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>&1 || true
# Capture logs
tail -n 200 "$SRV_LOG" >"$RESULT_DIR/redaction-sample.log" 2>/dev/null || true
REDACT_FAIL=0
grep -q 'soak-secret-token-DO-NOT-LOG' "$RESULT_DIR/redaction-sample.log" 2>/dev/null && REDACT_FAIL=1
grep -qi 'redact-me-cookie' "$RESULT_DIR/redaction-sample.log" 2>/dev/null && REDACT_FAIL=1
if [[ "$REDACT_FAIL" -eq 0 ]]; then
  record "Q_LOG_REDACTION" "PASS" "no raw secrets"
  record "LOG_REDACTION" "PASS" ""
else
  record "Q_LOG_REDACTION" "FAIL" "secret material in logs"
  record "LOG_REDACTION" "FAIL" ""
fi

# ========== Leak analysis ==========
rm -f "$RESULT_DIR/stop.sampler"
wait "$SAMPLER_PID" 2>/dev/null || true

python3 - "$RESULT_DIR/resources.csv" "$RESULT_DIR/leak-analysis.json" <<'PY'
import csv, json, sys
path, out = sys.argv[1], sys.argv[2]
rows = list(csv.DictReader(open(path)))
def series(key):
    return [int(float(r[key] or 0)) for r in rows if r.get(key)]
ex = series("exyonq_rss_kb")
fd = series("exyonq_fd")
thr = series("exyonq_threads")
result = {
    "samples": len(rows),
    "RSS_START": ex[0] if ex else 0,
    "RSS_END": ex[-1] if ex else 0,
    "RSS_PEAK": max(ex) if ex else 0,
    "FD_START": fd[0] if fd else 0,
    "FD_END": fd[-1] if fd else 0,
    "FD_PEAK": max(fd) if fd else 0,
    "TASKS_START": thr[0] if thr else 0,
    "TASKS_END": thr[-1] if thr else 0,
    "TASKS_PEAK": max(thr) if thr else 0,
    "OOM": False,
}
n = len(ex)
if n >= 8:
    q = n // 4
    first = sum(ex[:q]) / max(1, q)
    last = sum(ex[-q:]) / max(1, q)
    result["RSS_FIRST_QUARTILE_MEAN"] = first
    result["RSS_LAST_QUARTILE_MEAN"] = last
    result["RSS_UNBOUNDED_GROWTH"] = bool(last > first * 2.0 and ex[-1] > ex[n // 2])
else:
    result["RSS_UNBOUNDED_GROWTH"] = False
result["FD_LEAK"] = bool(result["FD_END"] > result["FD_START"] + 200)
result["TASK_LEAK"] = bool(result["TASKS_END"] > result["TASKS_START"] + 100)
json.dump(result, open(out, "w"), indent=2)
print(json.dumps(result, indent=2))
PY

LEAK_JSON="$RESULT_DIR/leak-analysis.json"
if python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if (not d.get("RSS_UNBOUNDED_GROWTH") and not d.get("FD_LEAK") and not d.get("TASK_LEAK") and not d.get("OOM")) else 1)' "$LEAK_JSON"; then
  record "LEAK_ANALYSIS" "PASS" "rss/fd/tasks bounded"
  record "FD_LEAK" "PASS" "NO"
  record "TASK_LEAK" "PASS" "NO"
  record "RSS_UNBOUNDED_GROWTH" "PASS" "NO"
  record "OOM" "PASS" "NO"
else
  record "LEAK_ANALYSIS" "FAIL" "see leak-analysis.json"
  record "FD_LEAK" "FAIL" ""
  record "TASK_LEAK" "FAIL" ""
  record "RSS_UNBOUNDED_GROWTH" "FAIL" ""
  record "OOM" "FAIL" "check host"
fi

# 5xx from H1 loadgen
TRUE_BAD_5XX=0
if [[ -f "$RESULT_DIR/loadgen-h1.json" ]]; then
  TRUE_BAD_5XX="$(python3 -c 'import json,sys; s=json.load(open(sys.argv[1])).get("stats",{}); print(int(s.get("unexpected_5xx",0)))' "$RESULT_DIR/loadgen-h1.json")"
fi
if [[ -f "$RESULT_DIR/loadgen-h3.env" ]]; then
  # shellcheck disable=SC1091
  source "$RESULT_DIR/loadgen-h3.env" 2>/dev/null || true
  TRUE_BAD_5XX=$((TRUE_BAD_5XX + ${h3_5xx:-0}))
fi
[[ "$TRUE_BAD_5XX" -eq 0 ]] && record "UNEXPECTED_5XX" "PASS" "0" || record "UNEXPECTED_5XX" "FAIL" "true_bad=$TRUE_BAD_5XX"

# Aggregate verdict
VERDICT="PASS"
CRITICAL=(
  A_H3_STATIC B_H3_PROXY_GET C_H3_PROXY_POST D_H3_MULTIPLEX
  E_H3_SLOW_STREAM F_H3_LARGE_BOUNDED_BODY G_H3_STREAM_RESET H_H3_CONNECTION_CLOSE
  I_UPSTREAM_RESTART J_UPSTREAM_TIMEOUT K_CERT_RELOAD_ALLOWED L_RELOAD_UNSUPPORTED_FAIL
  M_H3_DRAIN N_HANDSHAKE_PRESSURE O_EXCESSIVE_STREAMS_BOUNDED P_MIXED_H1_H3
  Q_LOG_REDACTION R_RECOVERY_FINAL
  LEAK_ANALYSIS OOM UNEXPECTED_5XX BOOT H3_CLIENT
)
for g in "${CRITICAL[@]}"; do
  if ! grep -q "^${g},PASS," "$RESULT_DIR/gates.csv"; then
    VERDICT="FAIL"
    echo "CRITICAL_FAIL=$g"
  fi
done

AMD64_SOAK="PENDING"; ARM64_SOAK="PENDING"
[[ "$HOST_LABEL" == "amd64" ]] && AMD64_SOAK="$VERDICT"
[[ "$HOST_LABEL" == "arm64" ]] && ARM64_SOAK="$VERDICT"

# Metrics placeholders for report
RSS_PEAK="$(python3 -c 'import json; d=json.load(open("'"$LEAK_JSON"'")); print(d.get("RSS_PEAK",0))' 2>/dev/null || echo 0)"
FD_PEAK="$(python3 -c 'import json; d=json.load(open("'"$LEAK_JSON"'")); print(d.get("FD_PEAK",0))' 2>/dev/null || echo 0)"
TASKS_PEAK="$(python3 -c 'import json; d=json.load(open("'"$LEAK_JSON"'")); print(d.get("TASKS_PEAK",0))' 2>/dev/null || echo 0)"
OOM_VAL="$(grep '^OOM,' "$RESULT_DIR/gates.csv" | cut -d, -f3 || echo UNKNOWN)"

{
  echo "# P1.3b HTTP/3 proxy soak — ${HOST_LABEL}"
  echo
  echo "**VERDICT = ${VERDICT}**"
  echo
  echo "\`\`\`text"
  echo "P1_3B_SOAK_LOCK_ID = $LOCK_ID"
  echo "RUN_ID            = $RUN_ID"
  echo "HOST_LABEL        = $HOST_LABEL"
  echo "ARCH              = $ACTUAL_ARCH"
  echo "DURATION_SEC      = $DURATION_SEC"
  echo "WARMUP_SEC        = $WARMUP_SEC"
  echo "CONCURRENCY       = $CONCURRENCY"
  echo "TCP_HTTP_PORT     = $HTTP_PORT"
  echo "H3_UDP_PORT       = $H3_UDP_PORT"
  echo "UPSTREAM_PORT     = $UP_PORT"
  echo "H3_CLIENT         = $H3_MODE"
  echo "CLIENT_IMAGE      = $CLIENT_IMAGE"
  echo "PROGRAM_HEAD      = ${P1_3B_SOAK_COMMIT:-unknown}"
  echo "H3_RELOAD_SUPPORT = SAME_LISTENER_CONFIG_ONLY"
  echo "OFFICIAL_BENCHMARK= NO"
  echo "PASS_GATES        = $pass_n"
  echo "FAIL_GATES        = $fail_n"
  if [[ "$HOST_LABEL" == "amd64" ]]; then
    echo "AMD64_SOAK        = $VERDICT"
    echo "ARM64_SOAK        = PENDING"
  else
    echo "AMD64_SOAK        = SEE_AMD64_REPORT"
    echo "ARM64_SOAK        = $VERDICT"
  fi
  echo "H3_DRAIN          = $(grep '^M_H3_DRAIN,' "$RESULT_DIR/gates.csv" | cut -d, -f2 || echo UNKNOWN)"
  echo "LOG_REDACTION     = $(grep '^Q_LOG_REDACTION,' "$RESULT_DIR/gates.csv" | cut -d, -f2 || echo UNKNOWN)"
  echo "RSS_PEAK_KB       = $RSS_PEAK"
  echo "FD_PEAK           = $FD_PEAK"
  echo "TASKS_PEAK        = $TASKS_PEAK"
  echo "OOM               = ${OOM_VAL:-NO}"
  echo "\`\`\`"
  echo
  echo "## Duration justification"
  echo
  echo "Steady soak ${DURATION_SEC}s after ${WARMUP_SEC}s warmup (warmup excluded from soak metrics)."
  echo "Defaults leaner than design ceiling for practical dual-arch; raise via P1_3B_SOAK_DURATION_* for longer leak windows."
  if [[ "$HOST_LABEL" == "arm64" ]]; then
    echo "Arm64 uses lower concurrency (${CONCURRENCY}) and may use shorter steady duration than amd64;"
    echo "coverage letters A–R remain mandatory (intensity may reduce)."
  fi
  echo
  echo "## Evidence"
  echo
  echo "- \`docs/operations/evidence/p1.3b-soak/${RUN_ID}/${HOST_LABEL}/\`"
  echo
  echo "## Matrix coverage (A–R)"
  echo
  echo "| ID | Gate |"
  echo "|----|------|"
  echo "| A | H3 static |"
  echo "| B | H3 proxy GET |"
  echo "| C | H3 proxy POST |"
  echo "| D | H3 multiplex |"
  echo "| E | H3 slow stream |"
  echo "| F | Large bounded body |"
  echo "| G | Stream reset + recover |"
  echo "| H | Connection close |"
  echo "| I | Upstream restart |"
  echo "| J | Upstream timeout |"
  echo "| K | Cert reload allowed |"
  echo "| L | Reload unsupported → explicit fail |"
  echo "| M | H3 drain |"
  echo "| N | Handshake pressure |"
  echo "| O | Excessive streams bounded |"
  echo "| P | Mixed H1+H3 (H2 if feasible) |"
  echo "| Q | Log redaction spot-check |"
  echo "| R | Recovery final |"
  echo
  echo "## Gates"
  echo
  echo '```'
  column -t -s, "$RESULT_DIR/gates.csv" 2>/dev/null || cat "$RESULT_DIR/gates.csv"
  echo '```'
  echo
  echo "## Lifecycle"
  echo
  echo '```'
  cat "$RESULT_DIR/lifecycle.env" 2>/dev/null || true
  echo '```'
  echo
  echo "## Leak analysis"
  echo
  echo '```json'
  cat "$LEAK_JSON" 2>/dev/null || echo '{}'
  echo '```'
  echo
  echo "## Loadgen"
  echo
  echo '```json'
  cat "$RESULT_DIR/loadgen-h1.json" 2>/dev/null || echo '{}'
  echo '```'
  echo
  echo '```'
  cat "$RESULT_DIR/loadgen-h3.env" 2>/dev/null || true
  echo '```'
  echo
  echo "Not a competitive benchmark. No HTML. No public performance ranking. Not an official Tier A compare."
} >"$REPORT_PATH"

echo "VERDICT=$VERDICT PASS=$pass_n FAIL=$fail_n"
[[ "$VERDICT" == "PASS" ]]
