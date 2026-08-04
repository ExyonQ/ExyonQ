#!/usr/bin/env bash
# AUTHORITATIVE static functional E2E — real exyonq binary + real filesystem.
# USES_REAL_DATA=YES | NOT a smoke gate.
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d)"
DOCROOT="$TMP/public"
mkdir -p "$DOCROOT/sub" "$DOCROOT/safe"
printf 'small-body' >"$DOCROOT/small.txt"
printf 'index-body' >"$DOCROOT/sub/index.html"
printf 'secret' >"$DOCROOT/../outside.txt" 2>/dev/null || printf 'secret' >"$TMP/outside.txt"
python3 -c "open('$DOCROOT/send64.bin','wb').write(bytes([0x41])*65536)"
python3 -c "open('$DOCROOT/bin1k.bin','wb').write(bytes([0x42])*1024)"

SMALL_HASH="$(sha256sum "$DOCROOT/small.txt" | awk '{print $1}')"
SEND64_HASH="$(sha256sum "$DOCROOT/send64.bin" | awk '{print $1}')"
BIN1K_HASH="$(sha256sum "$DOCROOT/bin1k.bin" | awk '{print $1}')"

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
EOF

PID=""
cleanup() { [[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

RUST_LOG=error "$EXYONQ_BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
PID=$!
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:${PORT}/assets/small.txt" >/dev/null 2>&1 && break
  kill -0 "$PID" 2>/dev/null || { cat "$TMP/exyonq.log" >&2; exit 1; }
  sleep 0.1
done

BODY="$(curl -sf "http://127.0.0.1:${PORT}/assets/small.txt")"
[[ "$BODY" == "small-body" && "$(printf '%s' "$BODY" | sha256sum | awk '{print $1}')" == "$SMALL_HASH" ]] \
  && e2e_record_pass "GET small body+hash" || e2e_record_fail "GET small"

HEAD_CL="$(curl -sfI "http://127.0.0.1:${PORT}/assets/small.txt" | awk 'tolower($1)=="content-length:"{print $2+0}')"
HEAD_DL="$(curl -sfI --max-time 10 "http://127.0.0.1:${PORT}/assets/small.txt" -o /dev/null -w '%{size_download}')"
[[ "$HEAD_CL" == "10" && "$HEAD_DL" == "0" ]] && e2e_record_pass "HEAD empty body" || e2e_record_fail "HEAD"

H64="$(curl -sf "http://127.0.0.1:${PORT}/assets/send64.bin" | sha256sum | awk '{print $1}')"
[[ "$H64" == "$SEND64_HASH" ]] && e2e_record_pass "GET 64KiB hash" || e2e_record_fail "64KiB"

H1K="$(curl -sf "http://127.0.0.1:${PORT}/assets/bin1k.bin" | sha256sum | awk '{print $1}')"
[[ "$H1K" == "$BIN1K_HASH" ]] && e2e_record_pass "GET 1KiB binary hash" || e2e_record_fail "1KiB"

ST="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/assets/missing.bin")"
[[ "$ST" == "404" ]] && e2e_record_pass "missing 404" || e2e_record_fail "missing"

IDX="$(curl -sf "http://127.0.0.1:${PORT}/assets/sub/")"
[[ "$IDX" == "index-body" ]] && e2e_record_pass "directory index" || e2e_record_fail "index"

# Traversal rejection (must not leak outside root)
TRAV_ST="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/assets/../outside.txt")"
TRAV_ST2="$(curl -s -o /dev/null -w '%{http_code}' --path-as-is "http://127.0.0.1:${PORT}/assets/../../outside.txt")"
if [[ "$TRAV_ST" != "200" && "$TRAV_ST2" != "200" ]]; then
  e2e_record_pass "traversal rejected (status ${TRAV_ST}/${TRAV_ST2})"
else
  e2e_record_fail "traversal leaked status=${TRAV_ST}/${TRAV_ST2}"
fi

# Repeated GETs
ok=1
for _ in 1 2 3 4 5; do
  B="$(curl -sf "http://127.0.0.1:${PORT}/assets/bin1k.bin" | sha256sum | awk '{print $1}')"
  [[ "$B" == "$BIN1K_HASH" ]] || ok=0
done
[[ "$ok" == "1" ]] && e2e_record_pass "repeated GET integrity" || e2e_record_fail "repeat"

e2e_finish "static-e2e"
