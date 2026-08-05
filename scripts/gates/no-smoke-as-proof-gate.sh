#!/usr/bin/env bash
# NO_SMOKE_AS_PROOF_GATE — fail closed if canonical qualification docs treat smoke as PASS proof.
# Historical study/reports and explicit NON_AUTHORITATIVE diagnostics are allowed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:-}"
SELFTEST_DIR=""

usage() {
  cat <<'EOF'
Usage:
  scripts/gates/no-smoke-as-proof-gate.sh --selftest
  scripts/gates/no-smoke-as-proof-gate.sh --tree <dir>
EOF
}

scan_tree() {
  local tree="$1"
  local status_f="$tree/PROJECT_STATUS.md"
  local roadmap_f="$tree/ROADMAP.md"
  local fail=0

  if [[ ! -f "$status_f" ]]; then
    echo "FAIL: missing PROJECT_STATUS.md under $tree"
    return 1
  fi

  # Forbidden: capability/release PASS materially attributed to smoke without
  # an explicit NON_AUTHORITATIVE / historical / diagnostic qualifier on the same line.
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    # Policy / meta / negation lines are never authoritative smoke-PASS claims.
    if echo "$line" | rg -qi \
      'NON_AUTHORITATIVE|NOT_GATE_CLOSING|HISTORICAL|diagnostic.only|SMOKE_TEST_POLICY|misnamed|FUNCTIONAL_E2E|NO_SMOKE|PASS_REQUIRES|No claim that smoke|SMOKE_TEST_MAY_|smoke is |paused|NOT_YET|REQUIRED_NOT'; then
      continue
    fi
    # Table rows or assignments claiming PASS via smoke
    if echo "$line" | rg -qi 'PASS' && echo "$line" | rg -qi 'smoke'; then
      echo "FAIL_AUTHORITATIVE_SMOKE_PASS: $line"
      fail=1
    fi
  done < <(rg -n -i 'smoke' "$status_f" "$roadmap_f" 2>/dev/null || true)

  # Forbidden release/benchmark admission phrasing
  if rg -n -i 'RELEASE.*(PASS|READY).*smoke|smoke.*(RELEASE_READY|PRODUCTION_READY|BENCHMARK_ADMISSION)' \
      "$status_f" "$roadmap_f" 2>/dev/null | rg -vi 'NON_AUTHORITATIVE|NOT_GATE_CLOSING|FORBIDDEN|NO '; then
    echo "FAIL_RELEASE_OR_BENCH_SMOKE"
    fail=1
  fi

  if [[ "$fail" -ne 0 ]]; then
    echo "NO_SMOKE_AS_PROOF_GATE=FAIL"
    return 1
  fi
  echo "NO_SMOKE_AS_PROOF_GATE=PASS"
  return 0
}

run_selftest() {
  SELFTEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/no-smoke-gate.XXXXXX")"
  cleanup() { rm -rf "$SELFTEST_DIR"; }
  trap cleanup EXIT

  mkdir -p "$SELFTEST_DIR/must_fail_a" "$SELFTEST_DIR/must_fail_b" "$SELFTEST_DIR/must_fail_c" \
           "$SELFTEST_DIR/must_pass_a" "$SELFTEST_DIR/must_pass_b" "$SELFTEST_DIR/must_pass_c" "$SELFTEST_DIR/must_pass_d"

  # MUST_FAIL: capability PASS because smoke passed
  cat >"$SELFTEST_DIR/must_fail_a/PROJECT_STATUS.md" <<'EOF'
| static serving | PASS | Linux static smoke dual-arch | — | — |
EOF
  : >"$SELFTEST_DIR/must_fail_a/ROADMAP.md"

  # MUST_FAIL: release PASS based on container smoke
  cat >"$SELFTEST_DIR/must_fail_b/PROJECT_STATUS.md" <<'EOF'
FULL_RELEASE_QUALIFICATION = PASS because OCI container smoke PASS
EOF
  : >"$SELFTEST_DIR/must_fail_b/ROADMAP.md"

  # MUST_FAIL: benchmark admitted because runtime smoke passed
  cat >"$SELFTEST_DIR/must_fail_c/PROJECT_STATUS.md" <<'EOF'
BENCHMARK_ADMISSION = PASS after runtime smoke PASS
EOF
  : >"$SELFTEST_DIR/must_fail_c/ROADMAP.md"

  # MUST_PASS: diagnostic smoke clearly NON_AUTHORITATIVE
  cat >"$SELFTEST_DIR/must_pass_a/PROJECT_STATUS.md" <<'EOF'
| kd2-5-static-smoke.sh | static | NON_AUTHORITATIVE_DIAGNOSTIC_ONLY | — | rename to e2e |
SMOKE_TEST_POLICY = DIAGNOSTIC_ONLY_NON_AUTHORITATIVE
EOF
  : >"$SELFTEST_DIR/must_pass_a/ROADMAP.md"

  # MUST_PASS: historical record of old smoke
  cat >"$SELFTEST_DIR/must_pass_b/PROJECT_STATUS.md" <<'EOF'
HISTORICAL: R2D smoke_static cell ran kd2-5-static-smoke.sh (NOT_GATE_CLOSING under NO-SMOKE policy).
EOF
  : >"$SELFTEST_DIR/must_pass_b/ROADMAP.md"

  # MUST_PASS: real E2E qualification
  cat >"$SELFTEST_DIR/must_pass_c/PROJECT_STATUS.md" <<'EOF'
| static serving | PASS | Linux static-e2e dual-arch (class C) | — | — |
EOF
  : >"$SELFTEST_DIR/must_pass_c/ROADMAP.md"

  # MUST_PASS: production E2E qualification
  cat >"$SELFTEST_DIR/must_pass_d/PROJECT_STATUS.md" <<'EOF'
| reverse proxy | PASS | proxy-production-e2e dual-arch (class D) | — | — |
EOF
  : >"$SELFTEST_DIR/must_pass_d/ROADMAP.md"

  local failures=0
  for d in must_fail_a must_fail_b must_fail_c; do
    if scan_tree "$SELFTEST_DIR/$d" >/tmp/no-smoke-selftest.out 2>&1; then
      echo "SELFTEST_UNEXPECTED_PASS: $d"
      failures=$((failures + 1))
    else
      echo "SELFTEST_OK_FAILS: $d"
    fi
  done
  for d in must_pass_a must_pass_b must_pass_c must_pass_d; do
    if scan_tree "$SELFTEST_DIR/$d" >/tmp/no-smoke-selftest.out 2>&1; then
      echo "SELFTEST_OK_PASSES: $d"
    else
      echo "SELFTEST_UNEXPECTED_FAIL: $d"
      cat /tmp/no-smoke-selftest.out || true
      failures=$((failures + 1))
    fi
  done

  if [[ "$failures" -ne 0 ]]; then
    echo "NO_SMOKE_AS_PROOF_GATE_SELFTEST=FAIL count=$failures"
    return 1
  fi
  echo "NO_SMOKE_AS_PROOF_GATE_SELFTEST=PASS"
  return 0
}

case "$MODE" in
  --selftest) run_selftest ;;
  --tree)
    TREE="${2:-}"
    [[ -n "$TREE" ]] || { usage; exit 2; }
    scan_tree "$TREE"
    ;;
  *) usage; exit 2 ;;
esac
