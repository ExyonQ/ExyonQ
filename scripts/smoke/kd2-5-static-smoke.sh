#!/usr/bin/env bash
# KD2.5 — Linux static functional smoke (wire path via Hyper dispatch, no competitive bench).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: Linux only (got $(uname -s))"
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$EXYONQ_BIN" ]]; then
  echo "Building exyonq (debug)..."
  cargo build -p exyonq --bin exyonq
fi

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

PORT="${KD25_SMOKE_PORT:-$(pick_port)}"
TMP="$(mktemp -d)"
DOCROOT="$TMP/public"
mkdir -p "$DOCROOT/sub"
printf 'small-body' >"$DOCROOT/small.txt"
printf 'index-body' >"$DOCROOT/sub/index.html"
# 64 KiB sendfile-sized asset
python3 -c "open('$DOCROOT/send64.bin','wb').write(bytes([0x41])*65536)"

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}
SMALL_HASH="$(sha256_file "$DOCROOT/small.txt")"
SEND64_HASH="$(sha256_file "$DOCROOT/send64.bin")"

CFG="$TMP/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["assets"]

[[route]]
name = "assets"
match = { path = "/assets" }
root = "${DOCROOT}"
index = "index.html"
cache = "default"

[[cache_policy]]
name = "default"
ttl_seconds = 30
max_object_bytes = 1048576
EOF

EXYONQ_PID=""
PASS=0
FAIL=0

record_pass() { echo "PASS  $1"; PASS=$((PASS + 1)); }
record_fail() { echo "FAIL  $1"; FAIL=$((FAIL + 1)); }

stop_exyonq() {
  if [[ -n "${EXYONQ_PID:-}" ]] && kill -0 "$EXYONQ_PID" 2>/dev/null; then
    kill "$EXYONQ_PID" 2>/dev/null || true
    wait "$EXYONQ_PID" 2>/dev/null || true
  fi
  EXYONQ_PID=""
}

cleanup() {
  stop_exyonq
  rm -rf "$TMP"
}
trap cleanup EXIT

start_exyonq() {
  stop_exyonq
  RUST_LOG=error "$EXYONQ_BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
  EXYONQ_PID=$!
  for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:${PORT}/assets/send64.bin" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      echo "exyonq exited early:" >&2
      cat "$TMP/exyonq.log" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for exyonq" >&2
  cat "$TMP/exyonq.log" >&2 || true
  return 1
}

curl_body() {
  curl -sf "$1"
}
curl_head_len() {
  curl -sfI "$1" | awk 'BEGIN{cl=0}tolower($1)=="content-length:"{cl=$2} END{print cl+0}'
}
curl_status() {
  curl -s -o /dev/null -w '%{http_code}' "$1"
}

start_exyonq

# GET small inline
BODY="$(curl_body "http://127.0.0.1:${PORT}/assets/small.txt")"
if [[ "$BODY" == "small-body" ]] && [[ "$(echo -n "$BODY" | sha256sum | awk '{print $1}')" == "$SMALL_HASH" ]]; then
  record_pass "GET small.txt 200 body+hash"
else
  record_fail "GET small.txt body mismatch"
fi

# HEAD small (-I avoids hanging when the server keeps the connection open after HEAD)
HEAD_CL="$(curl_head_len "http://127.0.0.1:${PORT}/assets/small.txt")"
HEAD_BODY="$(curl -sfI --max-time 10 "http://127.0.0.1:${PORT}/assets/small.txt" -o /dev/null -w '%{size_download}')"
if [[ "$HEAD_CL" == "10" ]] && [[ "$HEAD_BODY" == "0" ]]; then
  record_pass "HEAD small.txt content-length + empty body"
else
  record_fail "HEAD small.txt cl=$HEAD_CL body_bytes=$HEAD_BODY"
fi

# GET sendfile-sized (inline or sendfile — body must match)
SEND_BODY_HASH="$(curl_body "http://127.0.0.1:${PORT}/assets/send64.bin" | sha256sum | awk '{print $1}')"
if [[ "$SEND_BODY_HASH" == "$SEND64_HASH" ]]; then
  record_pass "GET send64.bin 200 hash"
else
  record_fail "GET send64.bin hash mismatch got=$SEND_BODY_HASH want=$SEND64_HASH"
fi

# 404
STATUS="$(curl_status "http://127.0.0.1:${PORT}/assets/missing.bin")"
if [[ "$STATUS" == "404" ]]; then
  record_pass "GET missing 404"
else
  record_fail "GET missing status=$STATUS"
fi

# directory index
IDX_BODY="$(curl_body "http://127.0.0.1:${PORT}/assets/sub/")"
if [[ "$IDX_BODY" == "index-body" ]]; then
  record_pass "directory index /assets/sub/"
else
  record_fail "directory index body=$IDX_BODY"
fi

# cache hit (second GET same body)
BODY2="$(curl_body "http://127.0.0.1:${PORT}/assets/small.txt")"
if [[ "$BODY2" == "small-body" ]]; then
  record_pass "cache/repeat GET small.txt"
else
  record_fail "repeat GET small.txt"
fi

# modify → new content visible (cache may bypass or revalidate)
printf 'modified-body' >"$DOCROOT/small.txt"
sleep 0.2
MOD_BODY="$(curl_body "http://127.0.0.1:${PORT}/assets/small.txt")"
if [[ "$MOD_BODY" == "modified-body" ]]; then
  record_pass "modify file → fresh content"
else
  record_fail "modify file still got=$MOD_BODY"
fi

# rename
mv "$DOCROOT/small.txt" "$DOCROOT/renamed.txt"
STATUS_REN="$(curl_status "http://127.0.0.1:${PORT}/assets/small.txt")"
if [[ "$STATUS_REN" == "404" ]]; then
  record_pass "rename → old path 404"
else
  record_fail "rename old path status=$STATUS_REN"
fi

# delete
rm -f "$DOCROOT/renamed.txt"
STATUS_DEL="$(curl_status "http://127.0.0.1:${PORT}/assets/renamed.txt")"
if [[ "$STATUS_DEL" == "404" ]]; then
  record_pass "delete → 404"
else
  record_fail "delete status=$STATUS_DEL"
fi

# reload/generation: restart server after config bump (same docroot, new file)
printf 'after-reload' >"$DOCROOT/reload.txt"
# append route via new config generation is heavy; verify new file served after restart
stop_exyonq
start_exyonq
RELOAD_BODY="$(curl_body "http://127.0.0.1:${PORT}/assets/reload.txt")"
if [[ "$RELOAD_BODY" == "after-reload" ]]; then
  record_pass "reload/restart serves new file"
else
  record_fail "reload body=$RELOAD_BODY"
fi

echo "KD2.5 static smoke: PASS=$PASS FAIL=$FAIL"
if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
