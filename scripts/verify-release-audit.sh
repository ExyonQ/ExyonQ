#!/usr/bin/env bash
# Verify release audit artifact exists and has no open blockers.
# FAIL_CLOSED: missing paths, missing template, or open blockers → non-zero exit.
#
# Usage:
#   verify-release-audit.sh X.Y.Z
#   verify-release-audit.sh --selftest
#   verify-release-audit.sh --dry-run-infra
#
# --dry-run-infra checks that the audit *system* is executable against the
# current tree (template + checklist + verifier present). It does NOT approve
# any version audit and must not be treated as release authorization.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATE="$ROOT/docs/security/audit-template.md"
CHECKLIST="$ROOT/docs/security/release-checklist.md"
REVIEW="$ROOT/docs/security/review-checklist.md"

fail() {
  echo "ERROR: RELEASE_AUDIT: $*" >&2
  exit 1
}

require_file() {
  local label="$1" path="$2"
  [[ -f "$path" ]] || fail "missing required path ($label): $path"
}

verify_version_audit() {
  local VERSION="$1"
  local AUDIT="$ROOT/docs/security/audit-v${VERSION}.md"

  require_file "audit-template" "$TEMPLATE"
  require_file "release-checklist" "$CHECKLIST"
  require_file "audit-v${VERSION}" "$AUDIT"

  if ! grep -Fq "**Version** | ${VERSION}" "$AUDIT"; then
    fail "CONTROL=version_bind audit version mismatch: expected ${VERSION} in $AUDIT"
  fi

  # Optional but recommended: bind commit SHA when present in template shape
  if grep -qE '^\| \*\*Commit\*\* \|' "$AUDIT"; then
    local commit_cell
    commit_cell="$(awk -F'|' '/^\| \*\*Commit\*\* \|/{gsub(/^ +| +$/,"",$3); print $3; exit}' "$AUDIT")"
    if [[ -z "$commit_cell" || "$commit_cell" == *'<'* || "$commit_cell" == *X.Y.Z* ]]; then
      fail "CONTROL=commit_bind audit Commit field is placeholder or empty"
    fi
  fi

  local blockers_section trimmed
  blockers_section="$(awk '/^## Blockers/{flag=1; next} /^## / && flag{exit} flag' "$AUDIT")"
  if echo "$blockers_section" | grep -qE '^### |^- \*\*'; then
    fail "CONTROL=blockers_empty audit has open blockers under ## Blockers"
  fi
  trimmed="$(echo "$blockers_section" | sed '/^[[:space:]]*$/d')"
  if [[ -n "$trimmed" ]] && ! echo "$trimmed" | grep -qiE '^(none|ninguno)\b'; then
    fail "CONTROL=blockers_empty audit ## Blockers must be empty or state 'None'"
  fi

  echo "RELEASE_AUDIT_GATE=PASS"
  echo "Release audit OK: audit-v${VERSION}.md"
}

dry_run_infra() {
  require_file "audit-template" "$TEMPLATE"
  require_file "release-checklist" "$CHECKLIST"
  require_file "review-checklist" "$REVIEW"
  grep -Fq '**Version** | X.Y.Z' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing Version placeholder"
  grep -Fq '## Blockers' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing ## Blockers"
  # Fail closed if a called-out version is missing when EXYONQ_AUDIT_EXPECT_VERSION is set
  if [[ -n "${EXYONQ_AUDIT_EXPECT_VERSION:-}" ]]; then
    verify_version_audit "$EXYONQ_AUDIT_EXPECT_VERSION"
  fi
  echo "RELEASE_AUDIT_INFRA=PASS"
  echo "RELEASE_AUDIT_MISSING_PATHS=0"
  echo "NOTE: dry-run-infra does not approve any release audit document"
}

run_selftest() {
  local tmp
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-release-audit-selftest.XXXXXX")"
  cleanup() { rm -rf "${tmp:?}"; }
  trap cleanup EXIT

  mkdir -p "$tmp/docs/security"
  # Point ROOT temporarily by copying verifier expectations into a fake tree via env
  # We test the verifier logic by invoking functions against a substituted ROOT.
  cp "$TEMPLATE" "$tmp/docs/security/audit-template.md"
  cp "$CHECKLIST" "$tmp/docs/security/release-checklist.md"
  cp "$REVIEW" "$tmp/docs/security/review-checklist.md"

  # MUST_FAIL: missing audit file
  if (
    ROOT="$tmp"
    verify_version_audit "9.9.9"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on missing audit-v9.9.9.md"
  fi
  echo "PASS_EXPECT_FAIL missing_audit"

  # MUST_FAIL: open blockers
  cat >"$tmp/docs/security/audit-v9.9.8.md" <<'EOF'
| Field | Value |
|-------|-------|
| **Version** | 9.9.8 |
| **Commit** | abcdef0123456789abcdef0123456789abcdef01 |
| **Date** | 2099-01-01 |

## Blockers

### Critical hole

- **Severidad:** crítica

## Findings (open)

None.
EOF
  if (
    ROOT="$tmp"
    verify_version_audit "9.9.8"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on open blockers"
  fi
  echo "PASS_EXPECT_FAIL open_blockers"

  # MUST_FAIL: version mismatch
  cat >"$tmp/docs/security/audit-v9.9.7.md" <<'EOF'
| Field | Value |
|-------|-------|
| **Version** | 0.0.0 |
| **Commit** | abcdef0123456789abcdef0123456789abcdef01 |
| **Date** | 2099-01-01 |

## Blockers

None

## Findings (open)

None.
EOF
  if (
    ROOT="$tmp"
    verify_version_audit "9.9.7"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on version mismatch"
  fi
  echo "PASS_EXPECT_FAIL version_mismatch"

  # MUST_PASS: clean audit
  cat >"$tmp/docs/security/audit-v9.9.6.md" <<'EOF'
| Field | Value |
|-------|-------|
| **Version** | 9.9.6 |
| **Commit** | abcdef0123456789abcdef0123456789abcdef01 |
| **Date** | 2099-01-01 |

## Blockers

None

## Findings (open)

None.
EOF
  (
    ROOT="$tmp"
    verify_version_audit "9.9.6"
  ) >/dev/null
  echo "PASS_EXPECT_PASS clean_audit"

  # Infra dry-run on real tree
  dry_run_infra >/dev/null

  trap - EXIT
  cleanup
  echo "RELEASE_AUDIT_SELFTEST=PASS"
  echo "RELEASE_AUDIT_GATE_STATUS=PASS"
  echo "RELEASE_AUDIT_MISSING_PATHS=0"
}

case "${1:-}" in
  --selftest)
    run_selftest
    ;;
  --dry-run-infra)
    dry_run_infra
    ;;
  "")
    echo "usage: verify-release-audit.sh X.Y.Z | --selftest | --dry-run-infra" >&2
    exit 2
    ;;
  *)
    verify_version_audit "$1"
    ;;
esac
