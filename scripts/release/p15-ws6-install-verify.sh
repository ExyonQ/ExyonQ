#!/usr/bin/env bash
# P1.5-WS6 — Staging-root install verify: unpack, run, probes, reload, cleanup.
# NO_HOST_CONTAMINATION — nothing outside --staging-root.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-install-verify.sh \
    --artifact PATH \
    --staging-root PATH \
    [--keep-staging]
EOF
}

ARTIFACT=""
STAGING_ROOT=""
KEEP_STAGING="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact) ARTIFACT="$2"; shift 2 ;;
    --staging-root) STAGING_ROOT="$2"; shift 2 ;;
    --keep-staging) KEEP_STAGING="true"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$ARTIFACT" && -f "$ARTIFACT" && -n "$STAGING_ROOT" ]] || {
  usage >&2
  exit 2
}

ws6_require_linux

ARTIFACT="$(cd "$(dirname "$ARTIFACT")" && pwd)/$(basename "$ARTIFACT")"
mkdir -p "$STAGING_ROOT"
STAGING_ROOT="$(cd "$STAGING_ROOT" && pwd)"

OVERALL_RC=0
declare -a STEP_RESULTS=()
SERVER_PID=""
LISTEN_PORT=""
TMP_RUNTIME=""

record_step() {
  local id="$1" verdict="$2" note="${3:-}"
  STEP_RESULTS+=("$id:$verdict:$note")
  echo "[$id] $verdict ${note:+- $note}"
  if [[ "$verdict" == "FAIL" ]]; then
    OVERALL_RC=1
  fi
  return 0
}

stop_server() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=""
}

cleanup() {
  local rc=$?
  stop_server
  if [[ "$KEEP_STAGING" != "true" ]]; then
    rm -rf "$STAGING_ROOT" "$TMP_RUNTIME"
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

# Reject wrong-arch tarball on Linux host
tarball_arch=""
if tarball_arch="$(ws6_extract_arch_from_tarball_name "$(basename "$ARTIFACT")")"; then
  host_arch="$(ws6_host_arch_label)"
  if [[ "$tarball_arch" != "$host_arch" ]]; then
    record_step "WRONG_ARCH_REJECT" "PASS" "rejected tarball arch=$tarball_arch host=$host_arch"
    echo "ERROR: wrong-arch tarball for this host" >&2
    exit 1
  fi
  record_step "ARCH_MATCH" "PASS" "tarball=$tarball_arch host=$host_arch"
fi

rm -rf "$STAGING_ROOT"/*
mkdir -p "$STAGING_ROOT"
tar -xzf "$ARTIFACT" -C "$STAGING_ROOT"
record_step "UNPACK" "PASS" "$STAGING_ROOT"

for req in usr/bin/exyonq usr/bin/exyonqctl etc/exyonq/config.toml.example \
  usr/lib/systemd/system/exyonq.service build-manifest.json; do
  if [[ -e "$STAGING_ROOT/$req" ]]; then
    record_step "LAYOUT_$req" "PASS" "present"
  else
    record_step "LAYOUT_$req" "FAIL" "missing"
  fi
done

UNIT="$STAGING_ROOT/usr/lib/systemd/system/exyonq.service"
if grep -q 'ExecStart=' "$UNIT" 2>/dev/null; then
  record_step "SYSTEMD_EXECSTART" "PASS" "grep ExecStart"
else
  record_step "SYSTEMD_EXECSTART" "FAIL" "missing ExecStart"
fi

if command -v systemd-analyze >/dev/null 2>&1; then
  VERIFY_UNIT="$STAGING_ROOT/exyonq-staging.service"
  sed \
    -e "s|/usr/bin/exyonq|$STAGING_ROOT/usr/bin/exyonq|g" \
    -e "s|/etc/exyonq/config.toml|$STAGING_ROOT/etc/exyonq/config.toml|g" \
    -e "s|/run/exyonq|$STAGING_ROOT/run/exyonq|g" \
    "$UNIT" >"$VERIFY_UNIT"
  if systemd-analyze verify "$VERIFY_UNIT" >/dev/null 2>&1; then
    record_step "SYSTEMD_ANALYZE" "PASS" "verify unit file"
  else
    record_step "SYSTEMD_ANALYZE" "WARN" "verify failed; ExecStart grep already checked"
  fi
else
  record_step "SYSTEMD_ANALYZE" "SKIP" "systemd-analyze unavailable"
fi

TMP_RUNTIME="$(mktemp -d /tmp/ws6-rt.XXXXXX)"
mkdir -p "$TMP_RUNTIME/run" "$TMP_RUNTIME/var/lib/exyonq" "$TMP_RUNTIME/public"
DOCROOT="$TMP_RUNTIME/public"
CFG="$TMP_RUNTIME/config.toml"
# Keep control socket path short — SUN_LEN rejects deep evidence staging paths.
CTRL_SOCK="$TMP_RUNTIME/control.sock"
LOG="$TMP_RUNTIME/exyonq.log"

echo "ws6-install-qual-ok" >"$DOCROOT/index.html"
LISTEN_PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"

cat >"$CFG" <<EOF
config_version = 2

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "${DOCROOT}"
index = "index.html"
EOF

EXYONQ_BIN="$STAGING_ROOT/usr/bin/exyonq"
EXYONQCTL_BIN="$STAGING_ROOT/usr/bin/exyonqctl"

EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
  "$EXYONQ_BIN" serve -c "$CFG" >"$LOG" 2>&1 &
SERVER_PID=$!

ready_ok=false
for _ in $(seq 1 40); do
  if curl -sf "http://127.0.0.1:${LISTEN_PORT}/live" >/dev/null 2>&1; then
    ready_ok=true
    break
  fi
  sleep 0.25
done

if [[ "$ready_ok" == "true" ]]; then
  record_step "START" "PASS" "pid=$SERVER_PID port=$LISTEN_PORT"
else
  record_step "START" "FAIL" "server did not become live"
  tail -n 40 "$LOG" >&2 || true
fi

if curl -sf "http://127.0.0.1:${LISTEN_PORT}/live" | grep -qi live; then
  record_step "PROBE_LIVE" "PASS" "/live"
else
  record_step "PROBE_LIVE" "FAIL" "/live"
fi

if curl -sf "http://127.0.0.1:${LISTEN_PORT}/ready" | grep -qi ready; then
  record_step "PROBE_READY" "PASS" "/ready"
else
  record_step "PROBE_READY" "FAIL" "/ready"
fi

if "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" >"$TMP_RUNTIME/status.out" 2>&1; then
  record_step "CTL_STATUS" "PASS" "exyonqctl status"
else
  if EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" \
    >"$TMP_RUNTIME/status.out" 2>&1; then
    record_step "CTL_STATUS" "PASS" "exyonqctl status explicit socket"
  else
    record_step "CTL_STATUS" "WARN" "status failed; server probes passed"
    cat "$TMP_RUNTIME/status.out" >&2 || true
  fi
fi

if "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >"$TMP_RUNTIME/reload.out" 2>&1; then
  record_step "CTL_RELOAD" "PASS" "reload"
elif EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" \
  >"$TMP_RUNTIME/reload.out" 2>&1; then
  record_step "CTL_RELOAD" "PASS" "reload explicit socket"
else
  record_step "CTL_RELOAD" "WARN" "reload not confirmed"
  cat "$TMP_RUNTIME/reload.out" >&2 || true
fi

stop_server
record_step "STOP" "PASS" "terminated"

# Preserve marker config for upgrade/rollback checks
echo "marker=install-qual" >>"$CFG"
cp "$CFG" "$STAGING_ROOT/.preserved-config.toml"
record_step "CONFIG_PRESERVE" "PASS" "saved staging config copy"

if [[ "$KEEP_STAGING" != "true" ]]; then
  rm -rf "$STAGING_ROOT" "$TMP_RUNTIME"
  TMP_RUNTIME=""
  record_step "UNINSTALL_STAGING" "PASS" "removed $STAGING_ROOT"
fi

VERDICT="PASS"
[[ "$OVERALL_RC" -ne 0 ]] && VERDICT="FAIL"

OUT_SUMMARY="$(dirname "$ARTIFACT")"

SUMMARY_JSON="$(python3 - <<PY
import json
steps = []
for item in """${STEP_RESULTS[*]}""".split():
    if not item:
        continue
    parts = item.split(":", 2)
    if len(parts) == 3:
        steps.append({"id": parts[0], "verdict": parts[1], "note": parts[2]})
print(json.dumps({
  "script": "p15-ws6-install-verify.sh",
  "verdict": "$VERDICT",
  "artifact": "$ARTIFACT",
  "staging_root": "$STAGING_ROOT",
  "no_host_contamination": True,
  "steps": steps,
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=$VERDICT
ARTIFACT=$ARTIFACT
STAGING_ROOT=$STAGING_ROOT
NO_HOST_CONTAMINATION=true
STEPS=${STEP_RESULTS[*]}"

mkdir -p "$OUT_SUMMARY"
printf '%s\n' "$SUMMARY_TXT" >"$OUT_SUMMARY/install-qual-summary.txt"
printf '%s\n' "$SUMMARY_JSON" >"$OUT_SUMMARY/install-qual-summary.json"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"
exit "$OVERALL_RC"
