#!/usr/bin/env bash
# P1.3a — TLS cert reload: valid rotate + invalid retain previous.
set -euo pipefail
# shellcheck source=lib-p13a-tls.sh
source "$(cd "$(dirname "$0")" && pwd)/lib-p13a-tls.sh"

ensure_bins
[[ -x "$EXYONQCTL_BIN" ]] || { echo "FAIL missing exyonqctl"; exit 1; }

TMP="$(mktemp -d)"
trap '[[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
CTRL="$TMP/control.sock"
LOG="$TMP/exyonq.log"
LIVE_CERT="$TMP/live-cert.pem"
LIVE_KEY="$TMP/live-key.pem"
cp "$CERT_PEM" "$LIVE_CERT"
cp "$KEY_PEM" "$LIVE_KEY"
write_tls_config "$CFG" "$PORT" "$LIVE_CERT" "$LIVE_KEY"
SRV_PID="$(start_exyonq_tls "$CFG" "$CTRL" "$LOG")"
wait_listen "$PORT" || { echo "FAIL listen"; tail -50 "$LOG" || true; exit 1; }

fp_of() { openssl x509 -in "$1" -noout -fingerprint -sha256 | sed 's/.*=//'; }
served_fp() {
  set +o pipefail
  echo | openssl s_client -connect "127.0.0.1:${PORT}" -servername localhost 2>/dev/null \
    | openssl x509 -noout -fingerprint -sha256 | sed 's/.*=//'
  set -o pipefail
}

FP0="$(fp_of "$LIVE_CERT")"
[[ "$(served_fp)" == "$FP0" ]] || { echo "FAIL initial fp"; exit 1; }
echo "PASS initial_cert"

# Valid rotation: generate new self-signed and reload via config path swap + ctl reload
NEW_CERT="$TMP/new-cert.pem"
NEW_KEY="$TMP/new-key.pem"
openssl req -x509 -newkey rsa:2048 -keyout "$NEW_KEY" -out "$NEW_CERT" \
  -days 1 -nodes -subj "/CN=rotated.localhost" >/dev/null 2>&1
cp "$NEW_CERT" "$LIVE_CERT"
cp "$NEW_KEY" "$LIVE_KEY"
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null
sleep 0.3
FP1="$(fp_of "$LIVE_CERT")"
SERVED1="$(served_fp)"
[[ "$SERVED1" == "$FP1" && "$SERVED1" != "$FP0" ]] || {
  echo "FAIL rotate expected=$FP1 got=$SERVED1 prev=$FP0"
  tail -40 "$LOG" || true
  exit 1
}
echo "PASS valid_rotate"

# Invalid reload: corrupt cert PEM; previous must remain
printf 'not-a-pem\n' >"$LIVE_CERT"
set +e
EXYONQ_CONFIG="$CFG" "$EXYONQCTL_BIN" reload --socket "$CTRL" >/dev/null 2>"$TMP/reload.err"
RELOAD_RC=$?
set -e
# Reload should fail (or report ok=false); either way served cert must stay FP1
SERVED2="$(served_fp)"
[[ "$SERVED2" == "$FP1" ]] || {
  echo "FAIL invalid_retain expected=$FP1 got=$SERVED2 rc=$RELOAD_RC"
  cat "$TMP/reload.err" || true
  tail -40 "$LOG" || true
  exit 1
}
echo "PASS invalid_retain (rc=$RELOAD_RC)"
echo "P13A_TLS_RELOAD=PASS"
exit 0
