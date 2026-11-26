#!/usr/bin/env bash
# Real EXQ-033 E2E: size rotation with keep+N startup pruning and native
# journald delivery on a live server.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s)"
  exit 2
fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
[[ -x "$EXYONQ_BIN" ]] || { echo "RESULT=BLOCKED missing=$EXYONQ_BIN"; exit 3; }
command -v journalctl >/dev/null || { echo "RESULT=BLOCKED missing=journalctl"; exit 3; }

TMP="$(mktemp -d)"
PID=""
cleanup() {
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/www" "$TMP/log"
printf 'ok\n' >"$TMP/www/index.html"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
LOG="$TMP/log/exyonq.log"
# Stale generations above keep=2 must be pruned at startup.
for i in 3 4 5; do printf 'stale-%s\n' "$i" >"$LOG.$i"; done

write_cfg() { # journald false|true
  cat >"$TMP/exyonq.toml" <<EOF
config_version = 2
[logging]
level = "info"
format = "text"
[logging.console]
enabled = false
[logging.file]
enabled = true
path = "$LOG"
[logging.file.rotation]
enabled = true
max_bytes = 2048
keep = 2
[logging.journald]
enabled = $1
[logging.access]
enabled = true
[logging.audit]
enabled = false
[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "$TMP/www"
EOF
}

start() {
  EXYONQ_CONTROL_SOCKET="$TMP/control.sock" RUST_LOG=info \
    "$EXYONQ_BIN" serve -c "$TMP/exyonq.toml" >"$TMP/console.log" 2>&1 &
  PID=$!
  for _ in $(seq 1 100); do
    if curl -fsS -o /dev/null "http://127.0.0.1:${PORT}/" 2>/dev/null; then return 0; fi
    kill -0 "$PID" 2>/dev/null || { echo "FAIL server exited"; cat "$TMP/console.log"; return 1; }
    sleep 0.1
  done
  echo "FAIL server not ready"
  return 1
}
stop() {
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
  PID=""
}

write_cfg false
start || exit 1
STALE_AT_START="$(find "$TMP/log" -maxdepth 1 -name 'exyonq.log.[345]' | wc -l)"
for i in $(seq 1 250); do curl -sS -o /dev/null "http://127.0.0.1:${PORT}/x${i}" || break; done
stop

GENS="$(find "$TMP/log" -maxdepth 1 -name 'exyonq.log.*' -printf '%f\n' | sort | tr '\n' ' ')"
GEN_COUNT="$(find "$TMP/log" -maxdepth 1 -name 'exyonq.log.*' | wc -l)"
STALE="$(find "$TMP/log" -maxdepth 1 -name 'exyonq.log.[345]' | wc -l)"
LATEST_BYTES="$(stat -c '%s' "$LOG" 2>/dev/null || echo 0)"
KEEP_OK=1
[[ "$GEN_COUNT" == "2" ]] || KEEP_OK=0

write_cfg true
MARK="exq033-$(date +%s)-$$"
start || exit 1
curl -fsS -o /dev/null "http://127.0.0.1:${PORT}/${MARK}" || true
sleep 0.3
stop

JOURNAL=""
if journalctl -n 1 >/dev/null 2>&1; then
  JOURNAL="$(journalctl --since '-5 min' SYSLOG_IDENTIFIER=exyonq -o cat 2>/dev/null | grep -F "$MARK" | head -1 || true)"
elif sudo -n journalctl -n 1 >/dev/null 2>&1; then
  JOURNAL="$(sudo -n journalctl --since '-5 min' SYSLOG_IDENTIFIER=exyonq -o cat 2>/dev/null | grep -F "$MARK" | head -1 || true)"
fi

OK=1
[[ "$STALE" == "0" ]] || { echo "FAIL startup_prune_stale=$STALE"; OK=0; }
[[ "$STALE_AT_START" == "2" ]] || { echo "FAIL startup_prune_before_traffic=$STALE_AT_START"; OK=0; }
[[ "$KEEP_OK" == "1" ]] || { echo "FAIL rotation_generations=$GENS"; OK=0; }
[[ "$LATEST_BYTES" -gt 0 ]] || { echo "FAIL latest_bytes=$LATEST_BYTES"; OK=0; }
[[ -n "$JOURNAL" ]] || { echo "FAIL journald_marker_missing"; OK=0; }
echo "EXQ033_STARTUP_PRUNED_STALE=$((5 - STALE))"
echo "EXQ033_STALE_AT_START=$STALE_AT_START"
echo "EXQ033_ROTATED_GENERATIONS=$GENS"
echo "EXQ033_ROTATED_GENERATION_COUNT=$GEN_COUNT"
echo "EXQ033_KEEP_OK=$KEEP_OK"
echo "EXQ033_LATEST_BYTES=$LATEST_BYTES"
if [[ -n "$JOURNAL" ]]; then echo "EXQ033_JOURNALD=present"; else echo "EXQ033_JOURNALD=missing"; fi
if [[ "$OK" == "1" ]]; then echo "EXQ033=PASS"; exit 0; fi
echo "EXQ033=FAIL"; exit 1
