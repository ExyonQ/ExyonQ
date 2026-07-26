#!/usr/bin/env bash
# P1.3a — ALPN negotiation (openssl s_client -alpn h2,http/1.1).
set -euo pipefail
# shellcheck source=lib-p13a-tls.sh
source "$(cd "$(dirname "$0")" && pwd)/lib-p13a-tls.sh"

ensure_bins
TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"
write_tls_config "$CFG" "$PORT"
SRV_PID="$(start_exyonq_tls "$CFG" "$CTRL" "$LOG")"
wait_listen "$PORT" || { echo "FAIL listen"; tail -50 "$LOG" || true; exit 1; }

OUT="$TMP/alpn.txt"
set +o pipefail
echo | openssl s_client -connect "127.0.0.1:${PORT}" -servername localhost \
  -alpn h2,http/1.1 2>&1 | tee "$OUT" | grep -qiE 'ALPN protocol:[[:space:]]*h2'
ALPN_H2=$?
echo | openssl s_client -connect "127.0.0.1:${PORT}" -servername localhost \
  -alpn http/1.1 2>&1 | tee "$TMP/alpn11.txt" | grep -qiE 'ALPN protocol:[[:space:]]*http/1\.1'
ALPN_11=$?
set -o pipefail
[[ "$ALPN_H2" -eq 0 ]] || { echo "FAIL alpn_h2"; cat "$OUT"; exit 1; }
echo "PASS alpn_h2"
[[ "$ALPN_11" -eq 0 ]] || { echo "FAIL alpn_http11"; cat "$TMP/alpn11.txt"; exit 1; }
echo "PASS alpn_http11"
echo "P13A_TLS_ALPN=PASS"
exit 0
