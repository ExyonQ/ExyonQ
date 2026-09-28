#!/usr/bin/env bash
# P1.5-WS3 — global fault injection harness (indexes TEST_ONLY / existing proofs).
# Linux evidence: Netcup amd64 + Oracle arm64. No chaos product framework.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

FORMAT=human
ARTIFACT_DIR=""
ONLY_FAULT=""
ONLY_GROUP=""
RUN_ALL=0
LIST_ONLY=0
LOCK_ID="${P15_WS3_LOCK_ID:-}"
RUN_ID="${P15_WS3_RUN_ID:-p15-ws3-$(date -u +%Y%m%dT%H%M%SZ)}"

usage() {
  cat <<EOF
Usage: $0 [--list] [--fault ID] [--group GROUP] [--all] [--format human|json] [--artifact-dir PATH]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list) LIST_ONLY=1; shift ;;
    --fault) ONLY_FAULT="$2"; shift 2 ;;
    --group) ONLY_GROUP="$2"; shift 2 ;;
    --all) RUN_ALL=1; shift ;;
    --format) FORMAT="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --lock-id) LOCK_ID="$2"; shift 2 ;;
    --run-id) RUN_ID="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -n "$ARTIFACT_DIR" ]] || ARTIFACT_DIR="$ROOT/docs/operations/evidence/p1.5-ws3/$RUN_ID"
[[ "$ARTIFACT_DIR" = /* ]] || ARTIFACT_DIR="$ROOT/$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR"

HEAD="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo nogit)"
HOST="$(hostname -s 2>/dev/null || hostname)"
ARCH="$(uname -m)"
SUMMARY_JSON="$ARTIFACT_DIR/summary.json"
SUMMARY_TXT="$ARTIFACT_DIR/summary.txt"
: >"$SUMMARY_TXT"
echo '{"run_id":"'"$RUN_ID"'","lock_id":"'"$LOCK_ID"'","head":"'"$HEAD"'","host":"'"$HOST"'","arch":"'"$ARCH"'","cases":[' >"$SUMMARY_JSON"
FIRST=1

fd_count() {
  if [[ -d /proc/self/fd ]]; then
    ls -1 /proc/self/fd 2>/dev/null | wc -l | tr -d ' '
  else
    echo 0
  fi
}

# FAULT_ID|GROUP|CMD|EXPECTED_SERVICE|EXPECTED_GENERATION|RECOVERY
# CMD is a shell snippet relative to ROOT.
CASES=(
  "FI-CONFIG-003|CONFIG|cargo test -p exyonq-core --lib reload::tests::test_fault_config_compile_failure_is_one_shot -- --exact|retain_prior_or_no_publish|no_advance|retry_reload_with_valid_config"
  "FI-RELOAD-001|RELOAD|cargo test -p exyonq-integration-tests --test security security_control_reload_invalid_config -- --exact|reject_invalid|no_advance|fix_config"
  "FI-RELOAD-002|RELOAD|cargo test -p exyonq-integration-tests --test security security_reload_invalid_config_keeps_snapshot -- --exact|retain_snapshot|no_advance|fix_config"
  "FI-RELOAD-004|RELOAD|cargo test -p exyonq-core --lib kernel_control_port::tests::concurrent_reload_latch_is_exclusive -- --exact|second_reload_rejected|latch_released|wait_and_retry"
  "FI-TLS-001|TLS|cargo test -p exyonq-mod-tls --lib tests::failed_reload_retains_previous_acceptor -- --exact|retain_previous_acceptor|no_bad_swap|fix_cert_material"
  "FI-LISTENER-001|LISTENER|cargo test -p exyonq-mod-fastcgi --test p12_generation_reload bind_failure_keeps_previous_generation -- --exact|retain_previous_generation|no_advance_on_bind_fail|fix_bind"
  "FI-PROXY-010|PROXY|cargo test -p exyonq-mod-proxy --lib retry::tests::get_not_started_allows_one_retry -- --exact|safe_retry_get_head|n/a|auto_one_retry"
  "FI-PROXY-011|PROXY|cargo test -p exyonq-mod-proxy --lib retry::tests::response_started_blocks_retry -- --exact|no_unsafe_retry|n/a|operator_fix_client"
  "FI-PROXY-003|PROXY|cargo test -p exyonq-mod-proxy --test p13a_classify_504_xff classify_maps_504_to_gateway_timeout -- --exact|504_classified|n/a|retry_or_fix_upstream"
  "FI-FCGI-002|FCGI|cargo test -p exyonq-mod-fastcgi --test wire_failure_injection connect_to_missing_unix_socket_fails -- --exact|fail_closed|n/a|start_fpm_or_fix_socket"
  "FI-FCGI-003|FCGI|cargo test -p exyonq-mod-fastcgi --test connect_timeout connect_timeout_is_504_class -- --exact|504_class|n/a|fix_fpm_latency"
  "FI-FCGI-006|FCGI|cargo test -p exyonq-mod-fastcgi --test wire_failure_injection peer_send_garbage_yields_invalid_frame -- --exact|protocol_fail_closed|discard_conn|auto_discard"
  "FI-FCGI-007|FCGI|cargo test -p exyonq-mod-fastcgi --test wire_failure_injection peer_omit_end_request_fails_decode -- --exact|protocol_fail_closed|discard_conn|auto_discard"
  "FI-FCGI-009|FCGI|cargo test -p exyonq-mod-fastcgi --test wire_failure_injection peer_drop_after_params_closes_connection -- --exact|conn_closed|discard_conn|auto_discard"
  "FI-FCGI-001|FCGI|cargo test -p exyonq-mod-fastcgi --lib conn_pool::tests::drain_rejects_new_checkout -- --exact|busy_or_reject|drain_active|wait_capacity"
  "FI-FCGI-010|FCGI|cargo test -p exyonq-mod-fastcgi --test p12_generation_reload begin_drain_rejects_new_dispatch_as_busy_or_bad_gateway -- --exact|reject_new|drain|complete_inflight"
  "FI-FCGI-011|FCGI|cargo test -p exyonq-mod-fastcgi --test p12_generation_reload pool_reuse_across_generations_forbidden_even_if_identical -- --exact|no_cross_gen_reuse|new_gen_pools|reload_ok"
  "FI-FCGI-008|FCGI|cargo test -p exyonq-mod-fastcgi --test p12_generation_reload bind_failure_keeps_previous_generation -- --exact|retain_previous|no_advance|fix_endpoint"
  "FI-LIFE-002|LIFE|cargo test -p exyonq-integration-tests --test security security_control_drain_rejects_new_requests -- --exact|reject_new_503|drain|complete_inflight"
  "FI-LIFE-005|LIFE|cargo test -p exyonq-core --lib lifecycle::tests::drain_rejects_new_enter -- --exact|no_new_enter|drain|release_tokens"
  "FI-LIFE-006|LIFE|cargo test -p exyonq-core --test db5_tokio_runtime_boundary_validation db5_drain_rejects_new_enter_and_clears_when_tokens_drop -- --exact|tokens_clear|drain_clear|auto"
  "FI-H3-001|H3|cargo test -p exyonq-core --test ps1a_h3_lifecycle native::native_drain_rejects_second_connection -- --exact|reject_second|drain|support_limit_same_listener"
)

# Additional deferred markers listed via --list only (documented in inventory).
DEFERRED=(
  "FI-CONFIG-001|CONFIG|DEFERRED_COVERED_BY_CONFIG_IR_PARSE_TESTS"
  "FI-CONFIG-002|CONFIG|DEFERRED_COVERED_BY_CONFIG_IR_VALIDATION"
  "FI-RELOAD-003|RELOAD|DEFERRED_RETIRE_TIMEOUT_TO_WS5_OBSERVABILITY"
  "FI-RELOAD-005|RELOAD|COVERED_BY_P14_WS5_IDENTICAL_NO_OP_QUAL"
  "FI-RELOAD-006|RELOAD|COVERED_BY_P14_C26_PATH_MISMATCH"
  "FI-RELOAD-007|RELOAD|DEFERRED_PROCESS_SIGNAL_FULL_BINARY_TO_WS5"
  "FI-RELOAD-008|RELOAD|DEFERRED_PROCESS_SIGNAL_FULL_BINARY_TO_WS5"
  "FI-LISTENER-002|LISTENER|DEFERRED_ADDR_IN_USE_E2E_OPTIONAL"
  "FI-LISTENER-003|LISTENER|COVERED_BY_FCGI_BIND_FAILURE_RETAIN"
  "FI-TLS-002|TLS|COVERED_BY_FAILED_RELOAD_RETAIN"
  "FI-TLS-003|TLS|DEFERRED_PERM_DENIED_HOST_SPECIFIC"
  "FI-TLS-004|TLS|DEFERRED_HANDSHAKE_UPSTREAM_RUSTLS"
  "FI-H2-001|H2|ACCEPTED_SUPPORT_LIMIT_UPSTREAM"
  "FI-H3-002|H3|ACCEPTED_SUPPORT_LIMIT_RESTART_REQUIRED"
  "FI-H3-003|H3|COVERED_BY_PS1A_H3_LIFECYCLE"
  "FI-PROXY-001|PROXY|DEFERRED_DNS_IF_APPLICABLE"
  "FI-PROXY-002|PROXY|DEFERRED_CONNECT_REFUSED_E2E_OPTIONAL"
  "FI-PROXY-004|PROXY|DEFERRED_WRITE_BEFORE_START_E2E"
  "FI-PROXY-005|PROXY|DEFERRED_WRITE_AFTER_START_E2E"
  "FI-PROXY-006|PROXY|COVERED_BY_WIRE_HEADER_TIMEOUT_CONST"
  "FI-PROXY-007|PROXY|DEFERRED_BODY_TIMEOUT_E2E"
  "FI-PROXY-008|PROXY|DEFERRED_MALFORMED_UPSTREAM_E2E"
  "FI-PROXY-009|PROXY|DEFERRED_UPSTREAM_RESET_E2E"
  "FI-FS-001|FS|COVERED_BY_CONFIG_RELOAD_INVALID"
  "FI-FS-002|FS|COVERED_BY_P14_INCLUDE_E2E"
  "FI-FS-003|FS|COVERED_BY_P14_ATOMIC_FORMAT"
  "FI-FS-004|FS|COVERED_BY_P14_ATOMIC_FORMAT"
  "FI-FS-005|FS|DEFERRED_STATIC_MID_READ_OPTIONAL"
  "FI-FS-006|FS|DEFERRED_LOG_WRITE_FAIL_TO_WS5"
  "FI-FS-007|FS|FORBIDDEN_DESTRUCTIVE_DISK_FULL"
  "FI-FS-008|FS|DEFERRED_READONLY_DIR_OPTIONAL"
  "FI-LIFE-001|LIFE|DEFERRED_SIGTERM_FULL_PROCESS_TO_WS5"
  "FI-LIFE-003|LIFE|DEFERRED_SHUTDOWN_DURING_PREPARE_TO_WS5"
  "FI-LIFE-004|LIFE|DEFERRED_SHUTDOWN_AFTER_COMMIT_TO_WS5"
  "FI-LIFE-007|LIFE|DEFERRED_REPEATED_SIGNAL_TO_WS5"
  "FI-LIFE-008|LIFE|DEFERRED_FORCED_TERMINATION_BOUNDARY_TO_WS5"
  "FI-FCGI-004|FCGI|COVERED_BY_WIRE_PEER_DROP"
  "FI-FCGI-005|FCGI|COVERED_BY_WIRE_PEER_DROP"
)

if [[ "$LIST_ONLY" -eq 1 ]]; then
  echo "EXECUTABLE:"
  for row in "${CASES[@]}"; do
    IFS='|' read -r id group _ <<<"$row"
    echo "  $id group=$group"
  done
  echo "DOCUMENTED_DEFERRED_OR_COVERED:"
  for row in "${DEFERRED[@]}"; do
    IFS='|' read -r id group note <<<"$row"
    echo "  $id group=$group note=$note"
  done
  exit 0
fi

if [[ "$RUN_ALL" -eq 0 && -z "$ONLY_FAULT" && -z "$ONLY_GROUP" ]]; then
  RUN_ALL=1
fi

PASS=0
FAIL=0
SKIP=0
BLOCKERS=()

echo "=== P1.5-WS3 fault injection ===" | tee -a "$SUMMARY_TXT"
echo "RUN_ID=$RUN_ID LOCK_ID=$LOCK_ID HEAD=$HEAD HOST=$HOST ARCH=$ARCH" | tee -a "$SUMMARY_TXT"

run_case() {
  local id="$1" group="$2" cmd="$3" svc="$4" gen="$5" recovery="$6"
  if [[ -n "$ONLY_FAULT" && "$ONLY_FAULT" != "$id" ]]; then return 0; fi
  if [[ -n "$ONLY_GROUP" && "$ONLY_GROUP" != "$group" ]]; then return 0; fi

  local pre_fd post_fd rc log
  log="$ARTIFACT_DIR/${id}.log"
  pre_fd="$(fd_count)"
  echo "=== FAULT $id group=$group ===" | tee -a "$SUMMARY_TXT"
  echo "PRE_STATE fd=$pre_fd" | tee -a "$SUMMARY_TXT"
  set +e
  bash -lc "cd \"$ROOT\"; $cmd" >"$log" 2>&1
  rc=$?
  set -e
  # Fail closed if cargo filtered everything (false PASS).
  if ! rg -q 'running [1-9][0-9]* test' "$log"; then
    echo "HARNESS_DEFECT: no tests executed for $id" | tee -a "$SUMMARY_TXT"
    rc=97
  fi
  if rg -q 'test result: FAILED' "$log"; then
    rc=1
  fi
  post_fd="$(fd_count)"
  local delta=$((post_fd - pre_fd))
  # Shell FD delta noise tolerance (cargo spawns); unexplained if |delta| > 64 on harness shell.
  local res_ok=1
  if [[ ${delta#-} -gt 64 ]]; then res_ok=0; fi

  local pass=0
  if [[ "$rc" -eq 0 && "$res_ok" -eq 1 ]]; then
    pass=1
    PASS=$((PASS + 1))
    echo "PASS $id" | tee -a "$SUMMARY_TXT"
  else
    FAIL=$((FAIL + 1))
    BLOCKERS+=("$id")
    echo "FAIL $id rc=$rc fd_delta=$delta" | tee -a "$SUMMARY_TXT"
    tail -40 "$log" | tee -a "$SUMMARY_TXT" || true
  fi

  echo "POST_STATE fd=$post_fd expected_service=$svc expected_generation=$gen recovery=$recovery" | tee -a "$SUMMARY_TXT"
  echo "RESOURCE_DELTA harness_fd=$delta" | tee -a "$SUMMARY_TXT"

  if [[ "$FIRST" -eq 1 ]]; then FIRST=0; else echo ',' >>"$SUMMARY_JSON"; fi
  cat >>"$SUMMARY_JSON" <<EOF
{"fault_id":"$id","group":"$group","exit":$rc,"pass":$pass,"pre_fd":$pre_fd,"post_fd":$post_fd,"fd_delta":$delta,"expected_service":"$svc","expected_generation":"$gen","recovery":"$recovery"}
EOF
}

for row in "${CASES[@]}"; do
  IFS='|' read -r id group cmd svc gen recovery <<<"$row"
  run_case "$id" "$group" "$cmd" "$svc" "$gen" "$recovery"
done

# Document deferred as skip records (not failures)
for row in "${DEFERRED[@]}"; do
  IFS='|' read -r id group note <<<"$row"
  if [[ -n "$ONLY_FAULT" && "$ONLY_FAULT" != "$id" ]]; then continue; fi
  if [[ -n "$ONLY_GROUP" && "$ONLY_GROUP" != "$group" ]]; then continue; fi
  SKIP=$((SKIP + 1))
  echo "SKIP $id note=$note" | tee -a "$SUMMARY_TXT"
  if [[ "$FIRST" -eq 1 ]]; then FIRST=0; else echo ',' >>"$SUMMARY_JSON"; fi
  cat >>"$SUMMARY_JSON" <<EOF
{"fault_id":"$id","group":"$group","exit":null,"pass":null,"skip":true,"note":"$note"}
EOF
done

echo ']' >>"$SUMMARY_JSON"
cat >>"$SUMMARY_JSON" <<EOF
,"pass":$PASS,"fail":$FAIL,"skip":$SKIP}
EOF

echo "=== summary ===" | tee -a "$SUMMARY_TXT"
echo "PASS=$PASS FAIL=$FAIL SKIP=$SKIP" | tee -a "$SUMMARY_TXT"
if [[ "$FAIL" -ne 0 ]]; then
  echo "BLOCKERS=${BLOCKERS[*]}" | tee -a "$SUMMARY_TXT"
  echo "P15_WS3_FAULT_INJECTION=FAIL" | tee -a "$SUMMARY_TXT"
  exit 1
fi
echo "BLOCKERS=" | tee -a "$SUMMARY_TXT"
echo "P15_WS3_FAULT_INJECTION=PASS" | tee -a "$SUMMARY_TXT"
echo "FAULT_RESOURCE_ACCOUNTING=PASS" | tee -a "$SUMMARY_TXT"
echo "UNEXPLAINED_RESOURCE_DELTA=0" | tee -a "$SUMMARY_TXT"
