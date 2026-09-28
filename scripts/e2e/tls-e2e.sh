#!/usr/bin/env bash
# AUTHORITATIVE TLS E2E: ALPN + real HTTPS GET body + cert reload retain.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

# shellcheck source=lib/tls.sh
source "$ROOT/scripts/e2e/lib/tls.sh"
ensure_bins
ensure_ephemeral_tls

PORT="$(e2e_pick_port)"
TMP="$(mktemp -d)"
WWW="$TMP/www"
mkdir -p "$WWW"
printf 'tls-e2e-body' >"$WWW/index.html"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"

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

EXYONQ_CONFIG="$CFG" EXYONQ_CONTROL_SOCKET="$CTRL" RUST_LOG=error \
  "$EXYONQ_BIN" serve --config "$CFG" >"$TMP/ex.log" 2>&1 &
PID=$!
wait_listen "$PORT" || { cat "$TMP/ex.log" >&2; exit 1; }

# ALPN h2 preferred
ALPN_H2="$(echo | openssl s_client -connect "127.0.0.1:${PORT}" -alpn h2,http/1.1 -servername localhost 2>/dev/null | rg -o 'ALPN protocol: .*' || true)"
echo "$ALPN_H2" | rg -q 'h2' && e2e_record_pass "ALPN negotiates h2" || e2e_record_fail "ALPN h2 ($ALPN_H2)"

# Real HTTPS GET (http/1.1)
BODY="$(curl -skf --http1.1 "https://127.0.0.1:${PORT}/site/" )"
[[ "$BODY" == "tls-e2e-body" ]] && e2e_record_pass "HTTPS GET body" || e2e_record_fail "HTTPS GET got='$BODY'"

# Reload path: reuse p13a-tls-reload when ctl present
if [[ -x "$EXYONQCTL_BIN" ]]; then
  set +e
  bash "$ROOT/scripts/e2e/suites/tls-reload-suite.sh"
  REL_EC=$?
  set -e
  [[ "$REL_EC" -eq 0 ]] && e2e_record_pass "tls-reload suite" || e2e_record_fail "tls-reload exit=$REL_EC"
else
  e2e_record_fail "exyonqctl missing for reload suite"
fi

e2e_finish "tls-e2e"
