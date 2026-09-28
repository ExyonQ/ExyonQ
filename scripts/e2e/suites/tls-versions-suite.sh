#!/usr/bin/env bash
# P1.3a — live TLS 1.2 and TLS 1.3 handshakes.
# Prefer openssl protocol pins (portable); curl --tls-max as secondary.
set -euo pipefail
# shellcheck source=../lib/tls.sh
source "$(cd "$(dirname "$0")/.." && pwd)/lib/tls.sh"

ensure_bins
TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"
write_tls_config "$CFG" "$PORT"
SRV_PID="$(start_exyonq_tls "$CFG" "$CTRL" "$LOG")"
wait_listen "$PORT" || { echo "FAIL listen pid=$SRV_PID"; tail -80 "$LOG" || true; exit 1; }

URL="https://127.0.0.1:${PORT}/site/"

# openssl s_client exits non-zero on self-signed verify — ignore its status.
set +o pipefail
echo | openssl s_client -connect "127.0.0.1:${PORT}" -tls1_2 -brief 2>&1 \
  | tee "$TMP/tls12.txt" | grep -qiE 'Protocol version: TLSv1\.2'
TLS12_OK=$?
echo | openssl s_client -connect "127.0.0.1:${PORT}" -tls1_3 -brief 2>&1 \
  | tee "$TMP/tls13.txt" | grep -qiE 'Protocol version: TLSv1\.3'
TLS13_OK=$?
set -o pipefail
[[ "$TLS12_OK" -eq 0 ]] || { echo "FAIL openssl tls1.2"; cat "$TMP/tls12.txt"; exit 1; }
echo "PASS openssl_tls1.2"
[[ "$TLS13_OK" -eq 0 ]] || { echo "FAIL openssl tls1.3"; cat "$TMP/tls13.txt"; exit 1; }
echo "PASS openssl_tls1.3"

# curl SecureTransport on macOS: --tlsv1.3 alone may fail; --tls-max is reliable.
curl -sfk --tls-max 1.2 "$URL" >/dev/null
echo "PASS curl_tls_max_1.2"
curl -sfk --tls-max 1.3 "$URL" >/dev/null
echo "PASS curl_tls_max_1.3"

echo "P13A_TLS_VERSIONS=PASS"
exit 0
