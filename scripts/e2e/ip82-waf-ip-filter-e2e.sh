#!/usr/bin/env bash
# GitHub #82 real E2E: the `[waf.ip]` client-IP allow/deny list on the product
# binary. Covers TTL expiry, the operator `reason`, the matched entry in the
# violation log and the per-entry metrics. Linux required (canonical evidence).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=lib.sh
source "$ROOT/scripts/e2e/lib.sh"
e2e_require_linux
e2e_ensure_bins

EXY="$EXYONQ_BIN"
if command -v sha256sum >/dev/null 2>&1; then
  BIN_SHA256="$(sha256sum "$EXY" | awk '{print $1}')"
else
  BIN_SHA256="$(shasum -a 256 "$EXY" | awk '{print $1}')"
fi
echo "binary=$EXY binary_sha256=$BIN_SHA256 platform=$(uname -srm)"

PORT="$(e2e_pick_port)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-ip82.XXXXXX")"
DOCROOT="$WORK/public"
mkdir -p "$DOCROOT"
printf 'ip82-body' >"$DOCROOT/ok.txt"

SRV_PID=""
LOG=""
cleanup() {
  if [[ -n "$SRV_PID" ]]; then
    kill "$SRV_PID" 2>/dev/null || true
    wait "$SRV_PID" 2>/dev/null || true
  fi
  if [[ "${KEEP_WORK:-0}" == "1" ]]; then
    echo "WORK=$WORK" >&2
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

# One config per scenario. The metrics endpoint is excluded from the IP rules
# so the scenario can scrape its own counters (the engine still logs every
# blocked request); this is the same exclusion an operator uses for probes.
write_config() {
  local cfg="$1" ip_block="$2"
  cat >"$cfg" <<EOF
config_version = 1

[waf]
enabled = true
mode = "block"

$ip_block

[[waf.exclusion]]
path_prefix = "/metrics"
rule_ids = ["EXY-IP-1001", "EXY-IP-1002"]

[[waf.ruleset]]
id = "builtins"

[waf.ruleset.builtins]
sql_injection = "off"
xss = "off"
path_traversal = "off"
command_injection = "off"
header_anomaly = "off"
bot_ua = "off"

[modules.metrics]
enabled = true
path = "/metrics"
health_path = "/exyonq-metrics-health"

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["assets"]

[[route]]
name = "assets"
match = { path = "/assets" }
root = "$DOCROOT"
index = "index.html"
EOF
}

start_server() {
  local cfg="$1" label="$2"
  LOG="$WORK/$label.log"
  RUST_LOG="${RUST_LOG:-exyonq_waf=warn,exyonq=warn}" \
    EXYONQ_CONTROL_SOCKET="$WORK/$label.sock" \
    "$EXY" serve -c "$cfg" >"$LOG" 2>&1 &
  SRV_PID=$!
  local ready=0 code
  for _ in $(seq 1 300); do
    code="$(curl -s -o /dev/null --max-time 1 -w '%{http_code}' \
      "http://127.0.0.1:$PORT/assets/ok.txt" 2>/dev/null || true)"
    if [[ -n "$code" && "$code" != "000" ]]; then
      ready=1
      break
    fi
    kill -0 "$SRV_PID" 2>/dev/null || break
    sleep 0.05
  done
  if [[ "$ready" -ne 1 ]]; then
    echo "FAIL: server did not become ready ($label)" >&2
    cat "$LOG" >&2
    exit 1
  fi
}

stop_server() {
  if [[ -n "$SRV_PID" ]]; then
    kill "$SRV_PID" 2>/dev/null || true
    wait "$SRV_PID" 2>/dev/null || true
  fi
  SRV_PID=""
}

status_of() { curl -s -o /dev/null --max-time 5 -w '%{http_code}' "$1"; }

# ---------------------------------------------------------------------------
# Scenario 1: active deny entry with reason -> 403, entry+reason in the log,
# per-entry counter and active/expired gauges in /metrics.
# ---------------------------------------------------------------------------
CFG="$WORK/deny-active.toml"
write_config "$CFG" '[waf.ip]
blacklist = [
  { ip = "127.0.0.1", reason = "e2e deny entry" },
]'
"$EXY" validate -c "$CFG" >"$WORK/deny-active.validate" 2>&1 \
  || { echo "FAIL: config validation failed" >&2; cat "$WORK/deny-active.validate" >&2; exit 1; }
start_server "$CFG" deny-active

ST1="$(status_of "http://127.0.0.1:$PORT/assets/ok.txt")"
ST2="$(status_of "http://127.0.0.1:$PORT/assets/ok.txt")"
curl -s --max-time 5 "http://127.0.0.1:$PORT/metrics" >"$WORK/deny-active.metrics" || true
stop_server

echo "deny_active_status=$ST1/$ST2"
[[ "$ST1" == "403" && "$ST2" == "403" ]] \
  && e2e_record_pass "[waf.ip] blacklist entry blocks (403)" \
  || e2e_record_fail "[waf.ip] blacklist entry blocks (got $ST1/$ST2)"

if grep -q 'rule_id=EXY-IP-1001' "$LOG"; then
  e2e_record_pass "violation log carries EXY-IP-1001"
else
  e2e_record_fail "violation log missing EXY-IP-1001"
fi
if grep -q 'ip_entry=Some(WafIpEntryEvidence' "$LOG" \
  && grep -q 'entry: "127.0.0.1"' "$LOG" \
  && grep -q 'reason: Some("e2e deny entry")' "$LOG"; then
  e2e_record_pass "violation log carries the matched entry and its reason"
else
  e2e_record_fail "violation log missing entry/reason evidence"
  grep -E 'EXY-IP-1001' "$LOG" | tail -3 >&2 || true
fi
if grep -Eq '^exyonq_waf_ip_entry_hits_total\{list="blacklist",entry="127\.0\.0\.1"\} [2-9][0-9]*$' \
  "$WORK/deny-active.metrics"; then
  e2e_record_pass "exyonq_waf_ip_entry_hits_total counts both blocked requests"
else
  e2e_record_fail "exyonq_waf_ip_entry_hits_total missing or not incremented"
  grep '^exyonq_waf_ip_' "$WORK/deny-active.metrics" >&2 || true
fi
grep -qx 'exyonq_waf_ip_filter_entries{list="blacklist",state="active"} 1' \
  "$WORK/deny-active.metrics" \
  && e2e_record_pass "active entry gauge = 1" \
  || e2e_record_fail "active entry gauge wrong"
grep -qx 'exyonq_waf_ip_filter_entries{list="blacklist",state="expired"} 0' \
  "$WORK/deny-active.metrics" \
  && e2e_record_pass "expired entry gauge = 0" \
  || e2e_record_fail "expired entry gauge wrong"

# ---------------------------------------------------------------------------
# Scenario 2: bare CIDR entry (no table) -> 403 and the CIDR is the label.
# ---------------------------------------------------------------------------
CFG="$WORK/deny-cidr.toml"
write_config "$CFG" '[waf.ip]
blacklist = ["127.0.0.0/8"]'
"$EXY" validate -c "$CFG" >"$WORK/deny-cidr.validate" 2>&1 \
  || { echo "FAIL: CIDR config validation failed" >&2; cat "$WORK/deny-cidr.validate" >&2; exit 1; }
start_server "$CFG" deny-cidr
ST_CIDR="$(status_of "http://127.0.0.1:$PORT/assets/ok.txt")"
curl -s --max-time 5 "http://127.0.0.1:$PORT/metrics" >"$WORK/deny-cidr.metrics" || true
stop_server

echo "deny_cidr_status=$ST_CIDR"
[[ "$ST_CIDR" == "403" ]] \
  && e2e_record_pass "bare CIDR blacklist entry blocks loopback (403)" \
  || e2e_record_fail "bare CIDR blacklist entry did not block (got $ST_CIDR)"
grep -Eq '^exyonq_waf_ip_entry_hits_total\{list="blacklist",entry="127\.0\.0\.0/8"\} [1-9][0-9]*$' \
  "$WORK/deny-cidr.metrics" \
  && e2e_record_pass "per-entry counter labels the CIDR entry" \
  || e2e_record_fail "per-entry counter does not label the CIDR entry"

# ---------------------------------------------------------------------------
# Scenario 3: expired deny entry -> request allowed, gauge reports expiry.
# ---------------------------------------------------------------------------
CFG="$WORK/deny-expired.toml"
write_config "$CFG" '[waf.ip]
blacklist = [
  { ip = "127.0.0.1", until = "2020-01-01T00:00:00Z", reason = "expired ban" },
]'
"$EXY" validate -c "$CFG" >"$WORK/deny-expired.validate" 2>&1 \
  || { echo "FAIL: expired config validation failed" >&2; cat "$WORK/deny-expired.validate" >&2; exit 1; }
start_server "$CFG" deny-expired
ST_EXP="$(status_of "http://127.0.0.1:$PORT/assets/ok.txt")"
BODY_EXP="$(curl -s --max-time 5 "http://127.0.0.1:$PORT/assets/ok.txt" || true)"
curl -s --max-time 5 "http://127.0.0.1:$PORT/metrics" >"$WORK/deny-expired.metrics" || true
stop_server

echo "expired_status=$ST_EXP"
[[ "$ST_EXP" == "200" && "$BODY_EXP" == "ip82-body" ]] \
  && e2e_record_pass "expired deny entry no longer blocks (200 + body)" \
  || e2e_record_fail "expired deny entry still blocking (got $ST_EXP)"
grep -q 'rule_id=EXY-IP-1001' "$LOG" \
  && e2e_record_fail "expired entry still logged as a violation" \
  || e2e_record_pass "expired entry produced no EXY-IP-1001 violation"
grep -qx 'exyonq_waf_ip_filter_entries{list="blacklist",state="active"} 0' \
  "$WORK/deny-expired.metrics" \
  && e2e_record_pass "expired entry leaves 0 active" \
  || e2e_record_fail "expired entry active gauge wrong"
grep -qx 'exyonq_waf_ip_filter_entries{list="blacklist",state="expired"} 1' \
  "$WORK/deny-expired.metrics" \
  && e2e_record_pass "expired entry counted as expired" \
  || e2e_record_fail "expired entry expired gauge wrong"

# ---------------------------------------------------------------------------
# Scenario 4: whitelist (fail-closed) -> 403 EXY-IP-1002 for a local client.
# ---------------------------------------------------------------------------
CFG="$WORK/allow-whitelist.toml"
write_config "$CFG" '[waf.ip]
whitelist = ["10.0.0.0/8"]'
"$EXY" validate -c "$CFG" >"$WORK/allow-whitelist.validate" 2>&1 \
  || { echo "FAIL: whitelist config validation failed" >&2; cat "$WORK/allow-whitelist.validate" >&2; exit 1; }
start_server "$CFG" allow-whitelist
ST_WL="$(status_of "http://127.0.0.1:$PORT/assets/ok.txt")"
curl -s --max-time 5 "http://127.0.0.1:$PORT/metrics" >"$WORK/allow-whitelist.metrics" || true
stop_server

echo "whitelist_status=$ST_WL"
[[ "$ST_WL" == "403" ]] \
  && e2e_record_pass "whitelist fails closed for a client outside it (403)" \
  || e2e_record_fail "whitelist did not fail closed (got $ST_WL)"
grep -q 'rule_id=EXY-IP-1002' "$LOG" \
  && e2e_record_pass "whitelist denial logged as EXY-IP-1002" \
  || e2e_record_fail "whitelist denial not logged as EXY-IP-1002"

# ---------------------------------------------------------------------------
# Scenario 5: config validation is fail-closed (no server started).
# ---------------------------------------------------------------------------
BAD_IP="$WORK/bad-ip.toml"
write_config "$BAD_IP" '[waf.ip]
blacklist = ["not-an-address"]'
if "$EXY" validate -c "$BAD_IP" >"$WORK/bad-ip.out" 2>&1; then
  e2e_record_fail "invalid [waf.ip] address passed validate"
else
  grep -q 'waf.ip.blacklist' "$WORK/bad-ip.out" \
    && e2e_record_pass "invalid address rejected with waf.ip.blacklist context" \
    || e2e_record_fail "invalid address rejected without waf.ip context"
fi

BAD_UNTIL="$WORK/bad-until.toml"
write_config "$BAD_UNTIL" '[waf.ip]
blacklist = [
  { ip = "127.0.0.1", until = "next tuesday" },
]'
if "$EXY" validate -c "$BAD_UNTIL" >"$WORK/bad-until.out" 2>&1; then
  e2e_record_fail "non-RFC3339 until passed validate"
else
  grep -q 'RFC 3339' "$WORK/bad-until.out" \
    && e2e_record_pass "non-RFC3339 until rejected with RFC 3339 context" \
    || e2e_record_fail "non-RFC3339 until rejected without RFC 3339 context"
fi

if e2e_finish "IP-82 [waf.ip]"; then
  echo "RESULT=PASS IP-82 [waf.ip] TTL + reason + per-entry metrics"
else
  exit 1
fi
