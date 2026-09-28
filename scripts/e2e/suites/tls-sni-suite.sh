#!/usr/bin/env bash
# P1.3a — SNI presents configured cert (SINGLE_CERT_SNI SUPPORT_LIMIT).
# IR has one cert/key; ClientHello -servername still receives that leaf.
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
wait_listen "$PORT" || { echo "FAIL listen"; tail -50 "$LOG" || true; exit 1; }

EXPECTED_FP="$(openssl x509 -in "$CERT_PEM" -noout -fingerprint -sha256 | sed 's/.*=//')"
got_fp() {
  local name="$1"
  set +o pipefail
  echo | openssl s_client -connect "127.0.0.1:${PORT}" -servername "$name" 2>/dev/null \
    | openssl x509 -noout -fingerprint -sha256 | sed 's/.*=//'
  set -o pipefail
}

FP_LOCAL="$(got_fp localhost)"
FP_OTHER="$(got_fp other.example.test)"
[[ -n "$FP_LOCAL" && "$FP_LOCAL" == "$EXPECTED_FP" ]] || {
  echo "FAIL sni_localhost expected=$EXPECTED_FP got=$FP_LOCAL"
  exit 1
}
echo "PASS sni_localhost"
[[ -n "$FP_OTHER" && "$FP_OTHER" == "$EXPECTED_FP" ]] || {
  echo "FAIL sni_other expected=$EXPECTED_FP got=$FP_OTHER"
  exit 1
}
echo "PASS sni_other_same_cert (SINGLE_CERT_SNI)"
echo "SUPPORT_LIMIT=SINGLE_CERT_SNI"
echo "P13A_TLS_SNI=PASS"
exit 0
