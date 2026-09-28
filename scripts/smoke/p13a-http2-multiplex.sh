#!/usr/bin/env bash
# NON_AUTHORITATIVE_DIAGNOSTIC_ONLY / NOT_GATE_CLOSING under EXYONQ_NO_SMOKE_POLICY.
# Depth class (audit 2026-08-02) recorded in .exyonq-local/tmp/no-smoke-audit-20260802/REPORT.md.
# Do not use this script alone to close capability / release / benchmark admission.
# P1.3a — client-facing HTTP/2 multiplex (curl --http2; nghttp if present).
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

URL="https://127.0.0.1:${PORT}/site/"
# Parallel requests over HTTP/2 (curl may reuse connection when --http2).
for i in 1 2 3 4; do
  curl -sfk --http2 "$URL" -o "$TMP/body-$i" &
done
wait
for i in 1 2 3 4; do
  [[ -s "$TMP/body-$i" ]] || { echo "FAIL empty body $i"; exit 1; }
done
echo "PASS curl_http2_parallel"

# Verify negotiated HTTP/2 on a single request
PROTO="$(curl -sk --http2 -o /dev/null -w '%{http_version}' "$URL")"
[[ "$PROTO" == "2" ]] || { echo "FAIL http_version=$PROTO (want 2)"; exit 1; }
echo "PASS curl_http_version_2"

if command -v nghttp >/dev/null 2>&1; then
  nghttp -v -n 4 "https://127.0.0.1:${PORT}/site/" 2>&1 | tee "$TMP/nghttp.txt" \
    | grep -qi 'HTTP/2' || { echo "FAIL nghttp"; exit 1; }
  echo "PASS nghttp_multiplex"
else
  echo "SKIP nghttp (not installed)"
fi

echo "P13A_HTTP2_MULTIPLEX=PASS"
exit 0
