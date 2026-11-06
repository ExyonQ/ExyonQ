#!/usr/bin/env bash
# REAL_EVIDENCE_GATE — fail closed when a report/evidence file presents
# smoke, mock, fake, stub, simulated, synthetic or demo work as PASS.
#
# The gate is line-oriented: a line fails when it contains a standalone PASS
# token and a non-real token, unless the same line carries an explicit
# non-authoritative/historical/policy marker such as NON_AUTHORITATIVE,
# DEVELOPMENT_AID, NOT EXECUTED, HISTORICAL, DIAGNOSTIC_ONLY, NO_SMOKE,
# ZERO_FAKE or FORBIDDEN. This is a guardrail, not a semantic proof: a claim
# split across several lines is outside its scope.
#
# Usage:
#   scripts/gates/real-evidence-gate.sh --selftest
#   scripts/gates/real-evidence-gate.sh --tree .
#   scripts/gates/real-evidence-gate.sh --diff <revspec>
#   scripts/gates/real-evidence-gate.sh --files <file>...
#
# --tree is a full inventory (it may expose historical debt).
# --diff scans only added lines in the given revision range; CI uses this mode.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PASS_RE='(^|[^[:alnum:]_])(pass_real_e2e|pass_real|passed|pass)([^[:alnum:]_]|$)'
NONREAL_RE='(^|[^[:alnum:]])(simulated|simulation|synthetic|smoke[sd]?|mock(s|ed)?|fake[sd]?|stub(s|bed)?|demo(s|ed)?)([^[:alnum:]]|$)'
ALLOW_RE='NON_AUTHORITATIVE|DEVELOPMENT_AID|NOT[_ -]EXECUTED|NOT_GATE_CLOSING|HISTORICAL|DIAGNOSTIC[_ -]ONLY|FORBIDDEN|NO[_ -]SMOKE|NO[_ -]MOCK|NO[_ -]FAKE|ZERO[_ -]FAKE|SMOKE_TEST_POLICY|PASS_REQUIRES|PRE[_ -]DEMO|FAKE_RESULT_GENERATION|NOT A SMOKE|NOT SMOKE|WITHOUT MOCK|NO MOCKS|NO FAKES|NO STUBS'

violations=0
SELFTEST_TMP=""

cleanup_selftest() {
  if [[ -n "${SELFTEST_TMP:-}" ]]; then
    rm -rf "$SELFTEST_TMP"
  fi
}
trap cleanup_selftest EXIT

usage() {
  cat <<'EOF'
Usage:
  real-evidence-gate.sh --selftest
  real-evidence-gate.sh --tree [DIR]
  real-evidence-gate.sh --diff <revspec>
  real-evidence-gate.sh --files <file>...

Fails when a report/evidence line presents smoke, mock, fake, stub,
simulated, synthetic or demo work as PASS without an explicit
NON_AUTHORITATIVE / DEVELOPMENT_AID / HISTORICAL / policy marker.
EOF
}

is_evidence_file() {
  local path="$1"
  case "$path" in
    *.md|*.markdown|*.json|*.jsonl|*.html|*.htm|*.csv|*.tsv|*.txt|*.log|*.yaml|*.yml) ;;
    *) return 1 ;;
  esac
  case "$path" in
    .git/*|*/'.git/'*|target/*|*/target/*|target-linux-x86/*|*/target-linux-x86/*|node_modules/*|*/node_modules/*|scripts/gates/fixtures/*|*/scripts/gates/fixtures/*|scripts/gates/tests/*|*/scripts/gates/tests/*)
      return 1
      ;;
  esac
  return 0
}

scan_line() {
  local file="$1"
  local line_no="$2"
  local line="$3"

  if grep -Eiq "$PASS_RE" <<<"$line" \
    && grep -Eiq "$NONREAL_RE" <<<"$line" \
    && ! grep -Eiq "$ALLOW_RE" <<<"$line"; then
    # Report only the location: report/evidence trees can contain private
    # material, and the gate must not copy it into CI logs.
    printf 'VIOLATION %s:%s\n' "$file" "$line_no"
    violations=$((violations + 1))
  fi
}

scan_file() {
  local file="$1"
  local out count
  out="$(awk -v pass_re="$PASS_RE" -v nonreal_re="$NONREAL_RE" -v allow_re="$ALLOW_RE" '
    BEGIN {
      p = tolower(pass_re)
      n = tolower(nonreal_re)
      a = tolower(allow_re)
    }
    {
      lower = tolower($0)
      if (lower ~ p && lower ~ n && lower !~ a) {
        printf "VIOLATION %s:%d\n", FILENAME, FNR
      }
    }
  ' "$file")"
  if [[ -n "$out" ]]; then
    printf '%s\n' "$out"
    count="$(printf '%s\n' "$out" | grep -c '^VIOLATION ' || true)"
    violations=$((violations + ${count:-0}))
  fi
}

scan_files() {
  local file
  for file in "$@"; do
    [[ -f "$file" ]] || continue
    if is_evidence_file "$file"; then
      scan_file "$file"
    fi
  done
}

scan_tree() {
  local tree="$1"
  if [[ ! -d "$tree" ]]; then
    echo "ERROR: not a directory: $tree" >&2
    exit 2
  fi

  local file
  while IFS= read -r -d '' file; do
    if is_evidence_file "$file"; then
      scan_file "$file"
    fi
  done < <(
    find "$tree" \
      \( -type d \( -name .git -o -name target -o -name target-linux-x86 -o -name node_modules \
        -o -path '*/scripts/gates/fixtures' -o -path '*/scripts/gates/tests' \) -prune \) -o \
      \( -type f \( -name '*.md' -o -name '*.markdown' -o -name '*.json' -o -name '*.jsonl' \
        -o -name '*.html' -o -name '*.htm' -o -name '*.csv' -o -name '*.tsv' -o -name '*.txt' \
        -o -name '*.log' -o -name '*.yaml' -o -name '*.yml' \) -print0 \)
  )
}

scan_diff() {
  local rev="$1"
  local diff_output

  if ! diff_output="$(git diff --unified=0 --no-color --diff-filter=ACMRT "$rev" -- . 2>/dev/null)"; then
    echo "ERROR: git diff failed for revision range: $rev" >&2
    exit 2
  fi

  local current_file=""
  local new_line=0
  local line
  while IFS= read -r line; do
    case "$line" in
      '+++ b/'*)
        current_file="${line#+++ b/}"
        continue
        ;;
      '+++ /dev/null'*)
        current_file=""
        continue
        ;;
      '@@ '*)
        if [[ "$line" =~ \+([0-9]+) ]]; then
          new_line="${BASH_REMATCH[1]}"
        fi
        continue
        ;;
      '+'*)
        if [[ -n "$current_file" ]]; then
          if is_evidence_file "$current_file"; then
            scan_line "$current_file" "$new_line" "${line:1}"
          fi
          new_line=$((new_line + 1))
        fi
        continue
        ;;
      ' '*)
        new_line=$((new_line + 1))
        continue
        ;;
      *)
        continue
        ;;
    esac
  done <<<"$diff_output"
}

run_selftest() {
  SELFTEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/real-evidence-gate.XXXXXX")"
  local tmp="$SELFTEST_TMP"

  mkdir -p "$tmp/fail" "$tmp/pass"

  cat >"$tmp/fail/smoke-pass.md" <<'EOF'
RESULT = PASS (smoke test)
EOF
  cat >"$tmp/fail/mock-pass.md" <<'EOF'
E2E | PASS | mock upstream
EOF
  cat >"$tmp/fail/fake-pass.md" <<'EOF'
fake data => PASS
EOF
  cat >"$tmp/fail/stub-pass.md" <<'EOF'
stub PASS
EOF
  cat >"$tmp/fail/simulated-pass.md" <<'EOF'
simulated endpoint PASS
EOF
  cat >"$tmp/fail/demo-pass.md" <<'EOF'
demo PASS
EOF
  cat >"$tmp/fail/release-smoke.md" <<'EOF'
P14V042_OCI_AMD64_SMOKE = PASS
EOF

  cat >"$tmp/pass/real-pass.md" <<'EOF'
RESULT = PASS (real binary, real upstream, real client)
EOF
  cat >"$tmp/pass/non-authoritative.md" <<'EOF'
smoke PASS - NON_AUTHORITATIVE
EOF
  cat >"$tmp/pass/historical.md" <<'EOF'
HISTORICAL: smoke PASS, not gate-closing
EOF
  cat >"$tmp/pass/no-smoke.md" <<'EOF'
NO_SMOKE = PASS
EOF
  cat >"$tmp/pass/zero-fake.md" <<'EOF'
ZERO_FAKE = PASS
EOF
  cat >"$tmp/pass/not-executed.md" <<'EOF'
NOT EXECUTED - smoke unavailable
EOF
  cat >"$tmp/pass/pre-demo.md" <<'EOF'
Pre-demo planning: integration gate PASS
EOF
  cat >"$tmp/pass/passthrough.md" <<'EOF'
passthrough PASS
EOF
  cat >"$tmp/pass/mock-without-pass.md" <<'EOF'
mock upstream used for debugging only
EOF
  cat >"$tmp/pass/pass-requires.md" <<'EOF'
PASS_REQUIRES additional real E2E
EOF

  local failures=0
  local before after file

  for file in "$tmp"/fail/*; do
    before=$violations
    scan_file "$file" >/dev/null
    after=$violations
    if [[ "$after" -gt "$before" ]]; then
      echo "SELFTEST_OK_FAILS: $(basename "$file")"
    else
      echo "SELFTEST_UNEXPECTED_PASS: $(basename "$file")"
      failures=$((failures + 1))
    fi
    violations=$before
  done

  for file in "$tmp"/pass/*; do
    before=$violations
    scan_file "$file" >/dev/null
    after=$violations
    if [[ "$after" -eq "$before" ]]; then
      echo "SELFTEST_OK_PASSES: $(basename "$file")"
    else
      echo "SELFTEST_UNEXPECTED_FAIL: $(basename "$file")"
      failures=$((failures + 1))
    fi
    violations=$before
  done

  if [[ "$failures" -ne 0 ]]; then
    echo "REAL_EVIDENCE_GATE_SELFTEST=FAIL count=$failures"
    return 1
  fi
  echo "REAL_EVIDENCE_GATE_SELFTEST=PASS"
  return 0
}

case "${1:-}" in
  --selftest)
    run_selftest
    exit $?
    ;;
  --tree)
    scan_tree "${2:-.}"
    ;;
  --diff)
    if [[ -z "${2:-}" ]]; then
      usage >&2
      exit 2
    fi
    scan_diff "$2"
    ;;
  --files)
    shift
    if [[ "$#" -eq 0 ]]; then
      usage >&2
      exit 2
    fi
    scan_files "$@"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

if [[ "$violations" -gt 0 ]]; then
  echo "REAL_EVIDENCE_GATE=FAIL count=$violations"
  exit 1
fi
echo "REAL_EVIDENCE_GATE=PASS"
