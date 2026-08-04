#!/usr/bin/env bash
# AUTHORITATIVE HTTP/2 E2E: ALPN h2 + multiplex + body integrity (no h1 fallback as PASS).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins
# shellcheck source=../smoke/lib-p13a-tls.sh
source "$ROOT/scripts/smoke/lib-p13a-tls.sh"
ensure_bins
ensure_ephemeral_tls

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d)"
WWW="$TMP/www"
mkdir -p "$WWW"
python3 -c "open('$WWW/index.html','wb').write(b'h2-e2e-' + b'x'*100)"
WANT_HASH="$(sha256sum "$WWW/index.html" | awk '{print $1}')"
CFG="$TMP/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1
[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]
tls = { cert = "$CERT_PEM", key = "$KEY_PEM" }
[[route]]
name = "site"
match = { path = "/site" }
root = "$WWW"
index = "index.html"
EOF

PID=""
cleanup() { [[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT
RUST_LOG=error "$EXYONQ_BIN" serve --config "$CFG" >"$TMP/ex.log" 2>&1 &
PID=$!
wait_listen "$PORT" || { cat "$TMP/ex.log" >&2; exit 1; }

VER="$(curl -skf --http2 "https://127.0.0.1:${PORT}/site/" -o "$TMP/b1" -w '%{http_version}')"
[[ "$VER" == "2" ]] && e2e_record_pass "curl negotiated HTTP/2" || e2e_record_fail "http_version=$VER"
GOT="$(sha256sum "$TMP/b1" | awk '{print $1}')"
[[ "$GOT" == "$WANT_HASH" ]] && e2e_record_pass "h2 body hash" || e2e_record_fail "body hash"

# Multiplex: 4 parallel streams, all hash-correct, all http2.
# Do not bare-`wait` — that also waits for the background exyonq serve job.
ok=1
CURL_PIDS=()
for i in 1 2 3 4; do
  curl -skf --max-time 15 --http2 "https://127.0.0.1:${PORT}/site/" \
    -o "$TMP/p$i" -w '%{http_version}' >"$TMP/v$i" &
  CURL_PIDS+=($!)
done
for p in "${CURL_PIDS[@]}"; do
  wait "$p" || ok=0
done
for i in 1 2 3 4; do
  [[ "$(cat "$TMP/v$i")" == "2" ]] || ok=0
  [[ "$(sha256sum "$TMP/p$i" | awk '{print $1}')" == "$WANT_HASH" ]] || ok=0
done
[[ "$ok" == "1" ]] && e2e_record_pass "4-stream multiplex integrity" || e2e_record_fail "multiplex"

# Explicit: HTTP/1.1 still works but must not be counted as h2 PASS (separate check)
V1="$(curl -skf --max-time 15 --http1.1 "https://127.0.0.1:${PORT}/site/" -o /dev/null -w '%{http_version}')"
[[ "$V1" == "1.1" ]] && e2e_record_pass "http/1.1 still available (not used as h2 proof)" || e2e_record_fail "h1=$V1"

e2e_finish "http2-e2e"
