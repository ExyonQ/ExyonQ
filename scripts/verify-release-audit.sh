#!/usr/bin/env bash
# Verify the release audit artifact is bound to the exact audited code commit.
# FAIL_CLOSED: missing paths, missing/invalid commit, tag/GITHUB_SHA mismatch,
# a seal that changes anything beyond the audit document, or open blockers →
# non-zero exit.
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

lower_hex() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

audit_commit_cell() {
  local audit="$1"
  awk -F'|' 'index($0, "| **Commit** |") == 1 {gsub(/^ +| +$/, "", $3); gsub(/`/, "", $3); print $3; exit}' "$audit"
}

# L-03: an audit is evidence only when it names the exact code commit it
# audited. The tagged commit may be that commit, or an audit-only seal commit
# whose first parent is the audited commit and whose only change is the audit
# document. The full 40-hex object must exist here; when CI provides
# GITHUB_SHA, it must equal the tagged commit (or the audit commit if no tag
# exists yet).
verify_commit_binding() {
  local VERSION="$1" AUDIT="$2"
  local commit_cell commit_sha tag_commit tag_parent release_commit github_sha
  local changed audit_rel

  commit_cell="$(audit_commit_cell "$AUDIT")"
  if [[ -z "$commit_cell" || "$commit_cell" == *'<'* || "$commit_cell" == *X.Y.Z* ]]; then
    fail "CONTROL=commit_bind audit Commit field is missing or a placeholder"
  fi
  commit_sha="$(lower_hex "$commit_cell")"
  if [[ ! "$commit_sha" =~ ^[0-9a-f]{40}$ ]]; then
    fail "CONTROL=commit_full_sha audit Commit must be a full 40-hex SHA (got: $commit_cell)"
  fi

  git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1 \
    || fail "CONTROL=commit_exists audit verification requires a git checkout: $ROOT"
  git -C "$ROOT" cat-file -e "${commit_sha}^{commit}" 2>/dev/null \
    || fail "CONTROL=commit_exists audited commit does not exist: $commit_sha"

  release_commit="$commit_sha"
  if tag_commit="$(git -C "$ROOT" rev-parse -q --verify "refs/tags/v${VERSION}^{commit}" 2>/dev/null)"; then
    tag_commit="$(lower_hex "$tag_commit")"
    release_commit="$tag_commit"
    if [[ "$commit_sha" != "$tag_commit" ]]; then
      tag_parent="$(git -C "$ROOT" rev-parse -q --verify "${tag_commit}^" 2>/dev/null || true)"
      tag_parent="$(lower_hex "$tag_parent")"
      if [[ "$commit_sha" != "$tag_parent" ]]; then
        fail "CONTROL=tag_mismatch audit commit $commit_sha != refs/tags/v${VERSION} ($tag_commit)"
      fi
      audit_rel="docs/security/audit-v${VERSION}.md"
      changed="$(git -C "$ROOT" diff --name-only "$commit_sha" "$tag_commit" --)"
      [[ "$changed" == "$audit_rel" ]] \
        || fail "CONTROL=seal_diff refs/tags/v${VERSION} must change only ${audit_rel} after the audited commit (got: ${changed:-no changes})"
    fi
  fi

  if [[ -n "${GITHUB_SHA:-}" ]]; then
    github_sha="$(lower_hex "$GITHUB_SHA")"
    [[ "$github_sha" =~ ^[0-9a-f]{40}$ ]] \
      || fail "CONTROL=github_sha_format GITHUB_SHA is not a full 40-hex SHA: $GITHUB_SHA"
    [[ "$github_sha" == "$release_commit" ]] \
      || fail "CONTROL=github_sha_mismatch audit release commit $release_commit != GITHUB_SHA $github_sha"
  fi
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

  verify_commit_binding "$VERSION" "$AUDIT"

  local blockers_section trimmed
  blockers_section="$(awk '/^## Blockers/{flag=1; next} /^## / && flag{exit} flag' "$AUDIT")"
  if echo "$blockers_section" | grep -qE '^### |^- \\*\\*'; then
    fail "CONTROL=blockers_empty audit has open blockers under ## Blockers"
  fi
  trimmed="$(echo "$blockers_section" | sed '/^[[:space:]]*$/d')"
  trimmed_lc="$(printf '%s' "$trimmed" | tr '[:upper:]' '[:lower:]')"
  if [[ -n "$trimmed_lc" && "$trimmed_lc" != "none" && "$trimmed_lc" != "ninguno" ]]; then
    fail "CONTROL=blockers_empty audit ## Blockers must be empty or state 'None'"
  fi

  echo "RELEASE_AUDIT_GATE=PASS"
  echo "Release audit OK: audit-v${VERSION}.md @ $(audit_commit_cell "$AUDIT")"
}

dry_run_infra() {
  require_file "audit-template" "$TEMPLATE"
  require_file "release-checklist" "$CHECKLIST"
  require_file "review-checklist" "$REVIEW"
  grep -Fq '**Version** | X.Y.Z' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing Version placeholder"
  grep -Fq '**Commit** | `<full sha>`' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing full Commit placeholder"
  grep -Fq '## Blockers' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing ## Blockers"
  grep -Fq '## Wire and route parity' "$TEMPLATE" \
    || fail "CONTROL=template_shape audit-template missing wire/route parity evidence"
  # Fail closed if a called-out version is missing when EXYONQ_AUDIT_EXPECT_VERSION is set
  if [[ -n "${EXYONQ_AUDIT_EXPECT_VERSION:-}" ]]; then
    verify_version_audit "$EXYONQ_AUDIT_EXPECT_VERSION"
  fi
  echo "RELEASE_AUDIT_INFRA=PASS"
  echo "RELEASE_AUDIT_MISSING_PATHS=0"
  echo "NOTE: dry-run-infra does not approve any release audit document"
}

write_audit() {
  local path="$1" version="$2" commit="$3" blockers="${4:-None}"
  cat >"$path" <<EOF
| Field | Value |
|-------|-------|
| **Version** | ${version} |
| **Commit** | \`${commit}\` |
| **Date** | 2099-01-01 |

## Blockers

${blockers}

## Findings (open)

None.
EOF
}

run_selftest() {
  local tmp
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-release-audit-selftest.XXXXXX")"
  cleanup() { rm -rf "${tmp:?}"; }
  trap cleanup EXIT

  command -v git >/dev/null 2>&1 || fail "selftest requires git"
  mkdir -p "$tmp/docs/security"
  cp "$TEMPLATE" "$tmp/docs/security/audit-template.md"
  cp "$CHECKLIST" "$tmp/docs/security/release-checklist.md"
  cp "$REVIEW" "$tmp/docs/security/review-checklist.md"

  # A real temporary git object store is required: the gate must prove the
  # audited SHA exists and that annotated/lightweight tag peeling matches.
  git -C "$tmp" init -q
  git -C "$tmp" config user.email "selftest@exyonq.invalid"
  git -C "$tmp" config user.name "ExyonQ selftest"
  git -C "$tmp" add docs
  git -C "$tmp" commit -qm "selftest base"
  local base other
  base="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" tag v9.9.6 "$base"

  # Another real commit; used to prove a tag that points elsewhere fails.
  git -C "$tmp" commit -q --allow-empty -m "selftest other"
  other="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" tag v9.9.2 "$other"

  # MUST_FAIL: missing audit file
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.9"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on missing audit-v9.9.9.md"
  fi
  echo "PASS_EXPECT_FAIL missing_audit"

  # MUST_FAIL: open blockers
  write_audit "$tmp/docs/security/audit-v9.9.8.md" "9.9.8" "$base" "### Critical hole

- **Severidad:** crítica"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.8"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on open blockers"
  fi
  echo "PASS_EXPECT_FAIL open_blockers"

  # MUST_FAIL: version mismatch
  write_audit "$tmp/docs/security/audit-v9.9.7.md" "0.0.0" "$base"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.7"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on version mismatch"
  fi
  echo "PASS_EXPECT_FAIL version_mismatch"

  # MUST_FAIL: missing Commit row (a version-only document is not evidence)
  cat >"$tmp/docs/security/audit-v9.9.5.md" <<'EOF'
| Field | Value |
|-------|-------|
| **Version** | 9.9.5 |
| **Date** | 2099-01-01 |

## Blockers

None
EOF
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.5"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on missing Commit field"
  fi
  echo "PASS_EXPECT_FAIL missing_commit"

  # MUST_FAIL: abbreviated SHA is not a full commit binding
  write_audit "$tmp/docs/security/audit-v9.9.4.md" "9.9.4" "abc123"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.4"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on abbreviated Commit"
  fi
  echo "PASS_EXPECT_FAIL abbreviated_commit"

  # MUST_FAIL: syntactically full SHA that does not exist
  write_audit "$tmp/docs/security/audit-v9.9.3.md" "9.9.3" "1111111111111111111111111111111111111111"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.3"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on nonexistent Commit"
  fi
  echo "PASS_EXPECT_FAIL nonexistent_commit"

  # MUST_FAIL: audit commit is not the commit the tag points at
  write_audit "$tmp/docs/security/audit-v9.9.2.md" "9.9.2" "$base"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.2"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on tag/commit mismatch"
  fi
  echo "PASS_EXPECT_FAIL tag_mismatch"

  # MUST_FAIL: audit commit differs from the CI checkout SHA
  write_audit "$tmp/docs/security/audit-v9.9.1.md" "9.9.1" "$base"
  if (
    ROOT="$tmp" GITHUB_SHA="$other"
    verify_version_audit "9.9.1"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on GITHUB_SHA mismatch"
  fi
  echo "PASS_EXPECT_FAIL github_sha_mismatch"

  # MUST_PASS: exact commit, matching tag and matching CI SHA
  write_audit "$tmp/docs/security/audit-v9.9.6.md" "9.9.6" "$base"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.6"
  ) >/dev/null
  echo "PASS_EXPECT_PASS clean_audit"

  # MUST_PASS: audited code commit plus an audit-only seal commit; tag the seal.
  # Branch from the audited commit so the seal's first parent is exactly $base.
  git -C "$tmp" checkout -q "$base"
  write_audit "$tmp/docs/security/audit-v9.9.10.md" "9.9.10" "$base"
  git -C "$tmp" add docs/security/audit-v9.9.10.md
  git -C "$tmp" commit -qm "selftest audit seal"
  local seal
  seal="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" tag v9.9.10 "$seal"
  (
    ROOT="$tmp" GITHUB_SHA="$seal"
    verify_version_audit "9.9.10"
  ) >/dev/null
  echo "PASS_EXPECT_PASS audit_seal"

  # MUST_FAIL: the seal commit also changes code.
  write_audit "$tmp/docs/security/audit-v9.9.11.md" "9.9.11" "$seal"
  mkdir -p "$tmp/src"
  printf '%s\n' 'fn main() {}' >"$tmp/src/main.rs"
  git -C "$tmp" add docs/security/audit-v9.9.11.md src/main.rs
  git -C "$tmp" commit -qm "selftest bad seal"
  local bad_seal
  bad_seal="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" tag v9.9.11 "$bad_seal"
  if (
    ROOT="$tmp" GITHUB_SHA="$bad_seal"
    verify_version_audit "9.9.11"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on audit seal with code changes"
  fi
  echo "PASS_EXPECT_FAIL seal_code_change"

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
