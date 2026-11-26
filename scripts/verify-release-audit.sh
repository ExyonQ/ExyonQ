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

# The seal binds the audited bytes with two components: the tracked `git diff`
# and a sorted manifest of every untracked file (path + SHA-256 content), plus
# a combined fingerprint. Counts are never the only proof.
sha256_stream() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum
  else
    fail "CONTROL=worktree_fingerprint_tool no shasum or sha256sum available"
  fi
}

SEAL_NORMALIZE_FP="${SEAL_NORMALIZE_FP:-}"
SEAL_NORMALIZE_DIFF="${SEAL_NORMALIZE_DIFF:-}"
SEAL_NORMALIZE_UNTRACKED="${SEAL_NORMALIZE_UNTRACKED:-}"

# A seal cannot hash the bytes that contain its own hash. Normalize only the
# seal-managed values (and their 8-hex citations) to one token, both while
# generating (placeholders) and while verifying (final values), so every other
# byte changes the fingerprint and the cited values remain stable.
normalize_seal_stream() {
  awk -v fp="$SEAL_NORMALIZE_FP" -v df="$SEAL_NORMALIZE_DIFF" -v us="$SEAL_NORMALIZE_UNTRACKED" '
    function scrub(v, full_token, prefix_token) {
      if (v == "") return
      gsub(v, full_token)
      if (length(v) >= 8) gsub(substr(v, 1, 8), prefix_token)
    }
    {
      gsub("__EXYONQ_SEAL_FP__", "<SEAL_FP_FULL>")
      gsub("__EXYONQ_SEAL_FP8__", "<SEAL_FP8>")
      gsub("__EXYONQ_SEAL_DIFF__", "<SEAL_DIFF_FULL>")
      gsub("__EXYONQ_SEAL_UNTRACKED__", "<SEAL_UNTRACKED_FULL>")
      scrub(fp, "<SEAL_FP_FULL>", "<SEAL_FP8>")
      scrub(df, "<SEAL_DIFF_FULL>", "<SEAL_DIFF8>")
      scrub(us, "<SEAL_UNTRACKED_FULL>", "<SEAL_UNTRACKED8>")
      print
    }'
}

sha256_file() {
  local path="$1"
  sha256_stream <"$path" | awk '{print $1}'
}

# The seal document contains its own fingerprint block, so it cannot hash the
# block that contains the hash. The block is skipped for the seal's manifest
# entry; every other byte (and every other untracked file) is hashed normally.
hash_untracked_file() {
  local path="$1" audit_rel="$2"
  if [[ -n "$audit_rel" && "$path" == "$audit_rel" ]]; then
    awk 'BEGIN { skip = 0 }
         /^WORKTREE_INTEGRITY_BEGIN$/ { skip = 1; next }
         /^WORKTREE_INTEGRITY_END$/ { skip = 0; next }
         !skip' "$path" | normalize_seal_stream | sha256_stream | awk '{print $1}'
  else
    normalize_seal_stream <"$path" | sha256_stream | awk '{print $1}'
  fi
}

current_untracked_manifest() {
  local root="${1:-$ROOT}" audit_rel="${2:-}" path hash
  (
    cd "$root" || return 1
    git ls-files --others --exclude-standard -z |
      while IFS= read -r -d '' path; do
        hash="$(hash_untracked_file "$path" "$audit_rel")" || exit 1
        printf '%s\t%s\n' "$path" "$hash"
      done |
      LC_ALL=C sort -t $'\t' -k1,1
  )
}

current_untracked_sha() {
  current_untracked_manifest "${1:-$ROOT}" "${2:-}" | sha256_stream | awk '{print $1}'
}

current_untracked_files() {
  current_untracked_manifest "${1:-$ROOT}" "${2:-}" | wc -l | tr -d ' '
}

audit_worktree_sha() {
  local audit="$1"
  sed -nE 's/^WORKTREE_DIFF_SHA256[[:space:]]*=[[:space:]]*([0-9a-fA-F]{64}).*/\1/p' "$audit" | head -n 1 | tr '[:upper:]' '[:lower:]'
}

audit_worktree_files() {
  local audit="$1"
  sed -nE 's/^WORKTREE_DIFF_FILES[[:space:]]*=[[:space:]]*([0-9]+).*/\1/p' "$audit" | head -n 1
}

audit_worktree_untracked_sha() {
  local audit="$1"
  sed -nE 's/^WORKTREE_UNTRACKED_SHA256[[:space:]]*=[[:space:]]*([0-9a-fA-F]{64}).*/\1/p' "$audit" | head -n 1 | tr '[:upper:]' '[:lower:]'
}

audit_worktree_untracked_files() {
  local audit="$1"
  sed -nE 's/^WORKTREE_UNTRACKED_FILES[[:space:]]*=[[:space:]]*([0-9]+).*/\1/p' "$audit" | head -n 1
}

audit_worktree_fingerprint_sha() {
  local audit="$1"
  sed -nE 's/^WORKTREE_FINGERPRINT_SHA256[[:space:]]*=[[:space:]]*([0-9a-fA-F]{64}).*/\1/p' "$audit" | head -n 1 | tr '[:upper:]' '[:lower:]'
}

audit_worktree_fingerprint_files() {
  local audit="$1"
  sed -nE 's/^WORKTREE_FINGERPRINT_FILES[[:space:]]*=[[:space:]]*([0-9]+).*/\1/p' "$audit" | head -n 1
}

audit_untracked_manifest() {
  local audit="$1"
  sed -n '/^WORKTREE_UNTRACKED_MANIFEST_BEGIN$/,/^WORKTREE_UNTRACKED_MANIFEST_END$/p' "$audit" | sed '1d;$d'
}

verify_sealed_manifest() {
  local audit="$1" expected_sha="$2" expected_count="$3"
  local actual_sha actual_count sealed_fp current_fp
  actual_sha="$(audit_untracked_manifest "$audit" | sha256_stream | awk '{print $1}')"
  actual_count="$(audit_untracked_manifest "$audit" | wc -l | tr -d ' ')"
  if [[ "$actual_sha" != "$expected_sha" || "$actual_count" != "$expected_count" ]]; then
    sealed_fp="$(audit_worktree_fingerprint_sha "$audit")"
    current_fp="$(current_fingerprint_sha "$ROOT" "$audit")"
    fail "CONTROL=worktree_manifest_mismatch file=${audit#"$ROOT"/} sealed_manifest=${actual_sha}/${actual_count} fields=${expected_sha}/${expected_count} sealed_fingerprint=${sealed_fp} current_fingerprint=${current_fp}; embedded manifest does not hash to WORKTREE_UNTRACKED_SHA256/FILES"
  fi
  [[ "$actual_count" == "$expected_count" ]] \
    || fail "CONTROL=worktree_manifest_count sealed_manifest=${actual_count} fields=${expected_count}"
  if ! audit_untracked_manifest "$audit" | awk -F'\t' '
      NF != 2 || $1 == "" || $2 !~ /^[0-9a-f]{64}$/ { bad = 1 }
      END { exit bad ? 1 : 0 }'; then
    fail "CONTROL=worktree_manifest_format embedded manifest must be path<TAB>64-hex, one line per untracked file"
  fi
}

verify_seal_citations() {
  local VERSION="$1" AUDIT="$2" sealed_fp sealed_fp8 file line lineno token rest
  sealed_fp="$(audit_worktree_fingerprint_sha "$AUDIT")"
  [[ -n "$sealed_fp" ]] || return 0
  sealed_fp8="${sealed_fp:0:8}"
  local files=(
    "$AUDIT"
    "$ROOT/docs/release/v${VERSION}.md"
    "$ROOT/ROADMAP EXYONQ/ROADMAP/Roadmap 0.9.2.md"
    "$ROOT/ROADMAP EXYONQ/ROADMAP/Roadmap 0.9.4.md"
    "$ROOT/ROADMAP EXYONQ/ROADMAP/0.9.2-BLOCKERS/README.md"
    "$ROOT/ROADMAP EXYONQ/ROADMAP/0.9.2-BLOCKERS/36-dod-release.md"
  )
  for file in "${files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS=: read -r lineno line; do
      [[ "$line" == *WORKTREE_FINGERPRINT_SHA256* ]] || continue
      [[ "$line" == *=* ]] || continue
      rest="${line#*WORKTREE_FINGERPRINT_SHA256}"
      token="$(printf '%s\n' "$rest" | sed -n 's/^[[:space:]]*=[[:space:]]*\([0-9a-f]\{8,64\}\).*/\1/p' | head -n 1)"
      [[ -n "$token" ]] \
        || fail "CONTROL=seal_citation_mismatch file=${file#"$ROOT"/} line=${lineno} expected=${sealed_fp} got=missing"
      case "${#token}" in
        64) [[ "$token" == "$sealed_fp" ]] \
              || fail "CONTROL=seal_citation_mismatch file=${file#"$ROOT"/} line=${lineno} expected=${sealed_fp} got=${token}" ;;
        8)  [[ "$token" == "$sealed_fp8" ]] \
              || fail "CONTROL=seal_citation_mismatch file=${file#"$ROOT"/} line=${lineno} expected=${sealed_fp8} got=${token}" ;;
        *)  fail "CONTROL=seal_citation_mismatch file=${file#"$ROOT"/} line=${lineno} expected=${sealed_fp} got=${token}" ;;
      esac
    done < <(grep -n -F 'WORKTREE_FINGERPRINT_SHA256' "$file" || true)
  done
}

current_worktree_sha() {
  (cd "${1:-$ROOT}" && git diff -- . | normalize_seal_stream | sha256_stream | awk '{print $1}')
}

current_worktree_files() {
  (cd "${1:-$ROOT}" && git diff --name-only -- . | wc -l | tr -d ' ')
}

current_fingerprint_sha() {
  local root="${1:-$ROOT}" audit_rel="${2:-}"
  printf '%s\n%s\n%s\n%s\n' \
    "$(current_worktree_sha "$root")" \
    "$(current_worktree_files "$root")" \
    "$(current_untracked_sha "$root" "$audit_rel")" \
    "$(current_untracked_files "$root" "$audit_rel")" |
    sha256_stream | awk '{print $1}'
}

current_fingerprint_files() {
  local root="${1:-$ROOT}" audit_rel="${2:-}"
  echo $(( $(current_worktree_files "$root") + $(current_untracked_files "$root" "$audit_rel") ))
}

changed_untracked_paths() {
  local audit="$1" root="$2" audit_rel="$3" sealed current out
  sealed="$(mktemp "${TMPDIR:-/tmp}/exyonq-sealed.XXXXXX")"
  current="$(mktemp "${TMPDIR:-/tmp}/exyonq-current.XXXXXX")"
  audit_untracked_manifest "$audit" >"$sealed"
  current_untracked_manifest "$root" "$audit_rel" >"$current"
  out="$(awk -F'\t' 'NR==FNR { s[$1] = $2; next } { c[$1] = $2 }
    END { for (p in s) if (!(p in c) || s[p] != c[p]) print p
          for (p in c) if (!(p in s)) print p }' "$sealed" "$current" | LC_ALL=C sort | paste -sd ',' -)"
  rm -f "$sealed" "$current"
  printf '%s' "$out"
}

# Fail closed when a dirty tree does not match the bytes the seal audited.
# A clean tree is rejected because there is no diff to check; a legacy seal
# without the manifest fields is rejected too. There is no PASS-by-omission.
verify_worktree_binding() {
  local AUDIT="$1" audit_rel="${2:-${1#"$ROOT"/}}"
  local sealed_sha sealed_files sealed_us sealed_uf sealed_fp sealed_total
  local current_sha current_files current_us current_uf current_fp current_total changed
  sealed_sha="$(audit_worktree_sha "$AUDIT")"
  sealed_files="$(audit_worktree_files "$AUDIT")"
  sealed_us="$(audit_worktree_untracked_sha "$AUDIT")"
  sealed_uf="$(audit_worktree_untracked_files "$AUDIT")"
  sealed_fp="$(audit_worktree_fingerprint_sha "$AUDIT")"
  sealed_total="$(audit_worktree_fingerprint_files "$AUDIT")"

  if [[ -z "$sealed_sha" || -z "$sealed_files" || -z "$sealed_us" || -z "$sealed_uf" || -z "$sealed_fp" || -z "$sealed_total" ]]; then
    fail "CONTROL=worktree_fingerprint_missing seal must name WORKTREE_DIFF_SHA256/FILES, WORKTREE_UNTRACKED_SHA256/FILES and WORKTREE_FINGERPRINT_SHA256/FILES; a legacy/unbound seal is not verifiable"
  fi

  verify_sealed_manifest "$AUDIT" "$sealed_us" "$sealed_uf"
  if (( sealed_files + sealed_uf != sealed_total )); then
    fail "CONTROL=worktree_count_mismatch sealed counts disagree: tracked=${sealed_files} + untracked=${sealed_uf} != total=${sealed_total}"
  fi

  if (cd "$ROOT" && git diff --quiet -- .); then
    fail "CONTROL=worktree_fingerprint_unverifiable clean tree: git diff is empty, so the sealed WORKTREE_DIFF_SHA256 cannot be checked; run against the audited dirty tree"
  fi

  SEAL_NORMALIZE_FP="$sealed_fp"
  SEAL_NORMALIZE_DIFF="$sealed_sha"
  SEAL_NORMALIZE_UNTRACKED="$sealed_us"
  current_sha="$(current_worktree_sha "$ROOT")"
  current_files="$(current_worktree_files "$ROOT")"
  current_us="$(current_untracked_sha "$ROOT" "$audit_rel")"
  current_uf="$(current_untracked_files "$ROOT" "$audit_rel")"
  current_fp="$(current_fingerprint_sha "$ROOT" "$audit_rel")"
  current_total="$(current_fingerprint_files "$ROOT" "$audit_rel")"
  if [[ "$sealed_sha" != "$current_sha" || "$sealed_files" != "$current_files" ||
        "$sealed_us" != "$current_us" || "$sealed_uf" != "$current_uf" ||
        "$sealed_fp" != "$current_fp" || "$sealed_total" != "$current_total" ]]; then
    changed="$(changed_untracked_paths "$AUDIT" "$ROOT" "$audit_rel")"
    fail "CONTROL=worktree_fingerprint_mismatch sealed_fingerprint=${sealed_fp}/${sealed_total} current_fingerprint=${current_fp}/${current_total}; tracked sealed=${sealed_sha}/${sealed_files} current=${current_sha}/${current_files}; untracked sealed=${sealed_us}/${sealed_uf} current=${current_us}/${current_uf}; changed_untracked=${changed:-none}; re-run the audit stack and regenerate the seal"
  fi
  echo "WORKTREE_FINGERPRINT=MATCH fingerprint=${current_fp} files=${current_total} tracked=${current_sha}/${current_files} untracked=${current_us}/${current_uf}"
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

  verify_worktree_binding "$AUDIT" "docs/security/audit-v${VERSION}.md"
  verify_seal_citations "$VERSION" "$AUDIT"

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
  # Fail closed if a called-out version is missing when
  # EXYONQ_AUDIT_EXPECT_VERSION is set. This is a presence/version check only:
  # a dry run must never execute the release verdict or print its PASS line.
  if [[ -n "${EXYONQ_AUDIT_EXPECT_VERSION:-}" ]]; then
    local expected_audit="$ROOT/docs/security/audit-v${EXYONQ_AUDIT_EXPECT_VERSION}.md"
    require_file "audit-v${EXYONQ_AUDIT_EXPECT_VERSION}" "$expected_audit"
    grep -Fq "**Version** | ${EXYONQ_AUDIT_EXPECT_VERSION}" "$expected_audit" \
      || fail "CONTROL=version_bind audit version mismatch: expected ${EXYONQ_AUDIT_EXPECT_VERSION} in $expected_audit"
  fi
  echo "RELEASE_AUDIT_INFRA=READY"
  echo "RELEASE_AUDIT_MISSING_PATHS=0"
  echo "NOTE: dry-run-infra does not approve any release audit document"
}

write_audit() {
  local path="$1" version="$2" commit="$3" blockers="${4:-None}"
  local worktree_sha="${5:-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855}"
  local worktree_files="${6:-0}"
  local root="${path%/docs/security/*}"
  local audit_rel="${path#"$root"/}"
  local manifest untracked_sha untracked_files fingerprint fingerprint_files

  # Provisional block; the self-hash skips this block, so rewriting the real
  # values below does not change the audit file's manifest entry.
  cat >"$path" <<EOF
| Field | Value |
|-------|-------|
| **Version** | ${version} |
| **Commit** | \`${commit}\` |
| **Date** | 2099-01-01 |

\`\`\`text
WORKTREE_INTEGRITY_BEGIN
WORKTREE_DIFF_SHA256 = ${worktree_sha}
WORKTREE_DIFF_FILES = ${worktree_files}
WORKTREE_UNTRACKED_SHA256 = 0000000000000000000000000000000000000000000000000000000000000000
WORKTREE_UNTRACKED_FILES = 0
WORKTREE_FINGERPRINT_SHA256 = 0000000000000000000000000000000000000000000000000000000000000000
WORKTREE_FINGERPRINT_FILES = 0
WORKTREE_UNTRACKED_MANIFEST_BEGIN
WORKTREE_UNTRACKED_MANIFEST_END
WORKTREE_INTEGRITY_END
\`\`\`

## Blockers

${blockers}

## Findings (open)

None.
EOF

  manifest="$(current_untracked_manifest "$root" "$audit_rel")"
  untracked_sha="$(current_untracked_sha "$root" "$audit_rel")"
  untracked_files="$(current_untracked_files "$root" "$audit_rel")"
  fingerprint="$(printf '%s\n%s\n%s\n%s\n' "$worktree_sha" "$worktree_files" "$untracked_sha" "$untracked_files" | sha256_stream | awk '{print $1}')"
  fingerprint_files=$(( worktree_files + untracked_files ))

  cat >"$path" <<EOF
| Field | Value |
|-------|-------|
| **Version** | ${version} |
| **Commit** | \`${commit}\` |
| **Date** | 2099-01-01 |

\`\`\`text
WORKTREE_INTEGRITY_BEGIN
WORKTREE_DIFF_SHA256 = ${worktree_sha}
WORKTREE_DIFF_FILES = ${worktree_files}
WORKTREE_UNTRACKED_SHA256 = ${untracked_sha}
WORKTREE_UNTRACKED_FILES = ${untracked_files}
WORKTREE_FINGERPRINT_SHA256 = ${fingerprint}
WORKTREE_FINGERPRINT_FILES = ${fingerprint_files}
WORKTREE_UNTRACKED_MANIFEST_BEGIN
${manifest}
WORKTREE_UNTRACKED_MANIFEST_END
WORKTREE_INTEGRITY_END
\`\`\`

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

  # MUST_FAIL: a clean tree has no git diff, so the sealed worktree fingerprint
  # cannot be checked. This branch must never silently pass a stale seal.
  write_audit "$tmp/docs/security/audit-v9.9.6.md" "9.9.6" "$base"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.6"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on unverifiable clean tree"
  fi
  echo "PASS_EXPECT_FAIL clean_tree_unverifiable"

  # MUST_FAIL: a dirty tree whose bytes do not match the sealed fingerprint.
  printf '%s\n' 'selftest dirty byte' >>"$tmp/docs/security/release-checklist.md"
  write_audit "$tmp/docs/security/audit-v9.9.12.md" "9.9.12" "$base" "None" \
    "0000000000000000000000000000000000000000000000000000000000000000" "1"
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.12"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on worktree fingerprint mismatch"
  fi
  echo "PASS_EXPECT_FAIL worktree_diff_mismatch"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_PASS: a dirty tree whose tracked bytes exactly match the seal.
  printf '%s\n' 'selftest matching byte' >>"$tmp/docs/security/release-checklist.md"
  local dirty_sha dirty_files
  dirty_sha="$(cd "$tmp" && git diff -- . | sha256_stream | awk '{print $1}')"
  dirty_files="$(cd "$tmp" && git diff --name-only -- . | wc -l | tr -d ' ')"
  write_audit "$tmp/docs/security/audit-v9.9.13.md" "9.9.13" "$base" "None" \
    "$dirty_sha" "$dirty_files"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.13"
  ) >/dev/null
  echo "PASS_EXPECT_PASS worktree_diff_match"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_FAIL/MUST_PASS: an untracked file's content is part of the seal, not
  # just its count. Modify it after sealing, prove both fingerprints and the
  # changed path are reported, then restore the exact bytes.
  local untracked_file untracked_before untracked_after mismatch_out
  printf '%s\n' 'selftest dirty byte for untracked case' >>"$tmp/docs/security/release-checklist.md"
  untracked_file="$tmp/docs/security/selftest-untracked.txt"
  untracked_before="$'untracked payload before seal\n'"
  printf '%s' "$untracked_before" >"$untracked_file"
  dirty_sha="$(cd "$tmp" && git diff -- . | sha256_stream | awk '{print $1}')"
  dirty_files="$(cd "$tmp" && git diff --name-only -- . | wc -l | tr -d ' ')"
  write_audit "$tmp/docs/security/audit-v9.9.15.md" "9.9.15" "$base" "None" \
    "$dirty_sha" "$dirty_files"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.15"
  ) >/dev/null
  echo "PASS_EXPECT_PASS untracked_content_match"

  untracked_after="$'untracked payload AFTER seal\n'"
  printf '%s' "$untracked_after" >"$untracked_file"
  if mismatch_out="$(
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.15" 2>&1
  )"; then
    fail "selftest expected FAIL on untracked content mismatch"
  fi
  grep -Fq 'CONTROL=worktree_fingerprint_mismatch' <<<"$mismatch_out" \
    || fail "selftest untracked mismatch did not identify the fingerprint mismatch: $mismatch_out"
  grep -Fq 'sealed_fingerprint=' <<<"$mismatch_out" \
    || fail "selftest untracked mismatch did not report the sealed fingerprint"
  grep -Fq 'current_fingerprint=' <<<"$mismatch_out" \
    || fail "selftest untracked mismatch did not report the current fingerprint"
  grep -Fq 'changed_untracked=docs/security/selftest-untracked.txt' <<<"$mismatch_out" \
    || fail "selftest untracked mismatch did not identify the changed path: $mismatch_out"
  echo "PASS_EXPECT_FAIL untracked_content_mismatch"

  printf '%s' "$untracked_before" >"$untracked_file"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.15"
  ) >/dev/null
  echo "PASS_EXPECT_PASS untracked_content_restored"
  rm -f "$untracked_file"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_FAIL/MUST_PASS: adding or deleting an untracked file changes the
  # manifest and the count; restore the exact set to return to PASS.
  local add_delete_file added_file
  printf '%s\n' 'selftest dirty byte for add/delete' >>"$tmp/docs/security/release-checklist.md"
  add_delete_file="$tmp/docs/security/selftest-add-delete.txt"
  printf '%s\n' 'base untracked' >"$add_delete_file"
  dirty_sha="$(cd "$tmp" && git diff -- . | sha256_stream | awk '{print $1}')"
  dirty_files="$(cd "$tmp" && git diff --name-only -- . | wc -l | tr -d ' ')"
  write_audit "$tmp/docs/security/audit-v9.9.16.md" "9.9.16" "$base" "None" \
    "$dirty_sha" "$dirty_files"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.16"
  ) >/dev/null
  echo "PASS_EXPECT_PASS untracked_add_delete_base"

  added_file="$tmp/docs/security/selftest-added.txt"
  printf '%s\n' 'added after seal' >"$added_file"
  if mismatch_out="$(
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.16" 2>&1
  )"; then
    fail "selftest expected FAIL after adding an untracked file"
  fi
  grep -Fq 'changed_untracked=docs/security/selftest-added.txt' <<<"$mismatch_out" \
    || fail "selftest added-untracked failure did not name the new file: $mismatch_out"
  echo "PASS_EXPECT_FAIL untracked_added"
  rm -f "$added_file"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.16"
  ) >/dev/null
  echo "PASS_EXPECT_PASS untracked_add_removed"

  rm -f "$add_delete_file"
  if mismatch_out="$(
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.16" 2>&1
  )"; then
    fail "selftest expected FAIL after deleting an untracked file"
  fi
  grep -Fq 'changed_untracked=docs/security/selftest-add-delete.txt' <<<"$mismatch_out" \
    || fail "selftest deleted-untracked failure did not name the removed file: $mismatch_out"
  echo "PASS_EXPECT_FAIL untracked_deleted"
  printf '%s\n' 'base untracked' >"$add_delete_file"
  (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.16"
  ) >/dev/null
  echo "PASS_EXPECT_PASS untracked_delete_restored"
  rm -f "$add_delete_file"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_FAIL: a citation that names a different fingerprint, even though the
  # citation itself is not part of the audit integrity block.
  printf '%s\n' 'selftest dirty byte for citation' >>"$tmp/docs/security/release-checklist.md"
  mkdir -p "$tmp/docs/release"
  printf 'WORKTREE_FINGERPRINT_SHA256 = 0000000000000000000000000000000000000000000000000000000000000000\n' \
    >"$tmp/docs/release/v9.9.17.md"
  dirty_sha="$(cd "$tmp" && git diff -- . | sha256_stream | awk '{print $1}')"
  dirty_files="$(cd "$tmp" && git diff --name-only -- . | wc -l | tr -d ' ')"
  write_audit "$tmp/docs/security/audit-v9.9.17.md" "9.9.17" "$base" "None" \
    "$dirty_sha" "$dirty_files"
  if mismatch_out="$(
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.17" 2>&1
  )"; then
    fail "selftest expected FAIL on an incoherent seal citation"
  fi
  grep -Fq 'CONTROL=seal_citation_mismatch' <<<"$mismatch_out" \
    || fail "selftest citation case did not report seal_citation_mismatch: $mismatch_out"
  echo "PASS_EXPECT_FAIL citation_mismatch"
  rm -f "$tmp/docs/release/v9.9.17.md"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_FAIL: a legacy seal with no fingerprint is unverifiable even when the
  # tree is dirty.
  printf '%s\n' 'selftest legacy byte' >>"$tmp/docs/security/release-checklist.md"
  cat >"$tmp/docs/security/audit-v9.9.14.md" <<EOF
| Field | Value |
|-------|-------|
| **Version** | 9.9.14 |
| **Commit** | \`$base\` |
| **Date** | 2099-01-01 |

## Blockers

None
EOF
  if (
    ROOT="$tmp" GITHUB_SHA="$base"
    verify_version_audit "9.9.14"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on legacy seal without worktree fingerprint"
  fi
  echo "PASS_EXPECT_FAIL legacy_no_fingerprint"
  git -C "$tmp" checkout -- docs/security/release-checklist.md

  # MUST_FAIL: audited code commit plus an audit-only seal commit; a clean tree
  # cannot prove the worktree fingerprint, so the verifier fails explicitly.
  # Branch from the audited commit so the seal's first parent is exactly $base.
  git -C "$tmp" checkout -q "$base"
  write_audit "$tmp/docs/security/audit-v9.9.10.md" "9.9.10" "$base"
  git -C "$tmp" add docs/security/audit-v9.9.10.md
  git -C "$tmp" commit -qm "selftest audit seal"
  local seal
  seal="$(git -C "$tmp" rev-parse HEAD)"
  git -C "$tmp" tag v9.9.10 "$seal"
  if (
    ROOT="$tmp" GITHUB_SHA="$seal"
    verify_version_audit "9.9.10"
  ) >/dev/null 2>&1; then
    fail "selftest expected FAIL on clean seal commit (unverifiable fingerprint)"
  fi
  echo "PASS_EXPECT_FAIL audit_seal_clean_unverifiable"

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

  # A dry run asked to bind a version must still never print the release PASS
  # line; it only checks presence and version shape.
  local dry_out
  dry_out="$(
    ROOT="$tmp" EXYONQ_AUDIT_EXPECT_VERSION=9.9.14 dry_run_infra 2>&1
  )" || fail "selftest dry-run-infra with expected version failed unexpectedly: $dry_out"
  if grep -Fq 'RELEASE_AUDIT_GATE=PASS' <<<"$dry_out"; then
    fail "selftest dry-run-infra printed the release PASS line"
  fi
  grep -Fq 'RELEASE_AUDIT_INFRA=READY' <<<"$dry_out" \
    || fail "selftest dry-run-infra did not report READY: $dry_out"
  echo "PASS_EXPECT_NO_RELEASE_PASS dry_run_expect_version"

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
