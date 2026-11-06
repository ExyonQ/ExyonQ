#!/usr/bin/env bash
# AUTHORITATIVE EXY-17/18 E2E: cache/security headers on HTML plus host rewrite
# to the apex, including path and query preservation. Real exyonq binary + files.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d)"
SITE="$TMP/site"
mkdir -p "$SITE/assets"
printf '<!doctype html><title>apex</title>\n' >"$SITE/index.html"
printf 'body{}\n' >"$SITE/assets/x.css"

CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site", "www"]

[[route]]
name = "site"
match = { path = "/", host = "exyonq.org" }
root = "${SITE}"
index = "index.html"

[route.cache_headers.by_mime."text/html"]
max_age = 0

[[route.response_headers]]
name = "x-content-type-options"
value = "nosniff"
always = true

[[route.response_headers]]
name = "referrer-policy"
value = "strict-origin-when-cross-origin"
always = true

[[route.response_headers]]
name = "permissions-policy"
value = "geolocation=(), camera=(), microphone=()"
always = true

[[route.response_headers]]
name = "x-frame-options"
value = "DENY"
always = true

[[route]]
name = "www"
match = { path = "/", host = "www.exyonq.org" }
rewrite = { pattern = "^/(.*)$", replacement = "https://exyonq.org/\$1", flag = "permanent" }
EOF

PID=""
cleanup() { [[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=error \
  "$EXYONQ_BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
PID=$!
for _ in $(seq 1 50); do
  curl -sf -H 'Host: exyonq.org' "http://127.0.0.1:${PORT}/" >/dev/null 2>&1 && break
  kill -0 "$PID" 2>/dev/null || { cat "$TMP/exyonq.log" >&2; exit 1; }
  sleep 0.1
done

APEX_HDR="$TMP/apex.hdr"
APEX_CODE="$(curl -sS -D "$APEX_HDR" -o "$TMP/apex.body" -w '%{http_code}' \
  -H 'Host: exyonq.org' "http://127.0.0.1:${PORT}/")"
[[ "$APEX_CODE" == "200" ]] && e2e_record_pass "apex HTML status 200" \
  || e2e_record_fail "apex status=$APEX_CODE"

grep -iq '^cache-control: no-cache' "$APEX_HDR" \
  && e2e_record_pass "HTML Cache-Control: no-cache" \
  || e2e_record_fail "missing Cache-Control: no-cache"
for pair in \
  'x-content-type-options: nosniff' \
  'referrer-policy: strict-origin-when-cross-origin' \
  'permissions-policy: geolocation=(), camera=(), microphone=()' \
  'x-frame-options: DENY'; do
  grep -iq "^${pair%%:*}: ${pair#*: }" "$APEX_HDR" \
    && e2e_record_pass "header ${pair%%:*}" \
    || e2e_record_fail "missing header ${pair%%:*}"
done

WWW_HDR="$TMP/www-root.hdr"
WWW_CODE="$(curl -sS -D "$WWW_HDR" -o /dev/null -w '%{http_code}' \
  -H 'Host: www.exyonq.org' "http://127.0.0.1:${PORT}/")"
[[ "$WWW_CODE" == "301" ]] && e2e_record_pass "www root 301" \
  || e2e_record_fail "www root status=$WWW_CODE"
if tr -d '\r' <"$WWW_HDR" | grep -iqx 'location: https://exyonq\.org/'; then
  e2e_record_pass "www root Location apex"
else
  e2e_record_fail "www root Location wrong"
fi

QUERY='utm_source=e2e&a=1'
PATH_HDR="$TMP/www-path.hdr"
PATH_CODE="$(curl -sS -D "$PATH_HDR" -o /dev/null -w '%{http_code}' \
  -H 'Host: www.exyonq.org' "http://127.0.0.1:${PORT}/assets/x.css?${QUERY}")"
[[ "$PATH_CODE" == "301" ]] && e2e_record_pass "www path 301" \
  || e2e_record_fail "www path status=$PATH_CODE"
if tr -d '\r' <"$PATH_HDR" | grep -iqx "location: https://exyonq\.org/assets/x\.css?${QUERY}"; then
  e2e_record_pass "www path and query preserved"
else
  e2e_record_fail "www path Location wrong"
fi

e2e_finish "security-headers-redirect-e2e"