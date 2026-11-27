#!/usr/bin/env bash
# RELEASE-STATUS-SINGLE-SOURCE
# Fail closed when the canonical release identity (RELEASE_STATUS) and the
# generated blocks in README.md, SECURITY.md and PROJECT_STATUS.md disagree, or
# when the workspace version, release note or audit note do not match it.
#
# Usage:
#   readme-release-version-gate.sh --selftest
#   readme-release-version-gate.sh --tree DIR
#   readme-release-version-gate.sh --write DIR
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SELFTEST_TMP=""
BLOCK_BEGIN='<!-- BEGIN RELEASE_STATUS (generated from RELEASE_STATUS; do not edit) -->'
BLOCK_END='<!-- END RELEASE_STATUS -->'

usage() {
  cat <<'EOF'
Usage:
  readme-release-version-gate.sh --selftest
  readme-release-version-gate.sh --tree DIR
  readme-release-version-gate.sh --write DIR
EOF
}

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

version_lt() {
  local left="$1"
  local right="$2"
  if [[ "$left" == "$right" ]]; then
    return 1
  fi
  [[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | head -n 1)" == "$left" ]]
}

is_release_line() {
  local content="$1"
  [[ "$content" =~ (^|[^[:alnum:]_])[vV][0-9]+\.[0-9]+ ]] && return 0
  [[ "$content" =~ [Ee]xyon[Qq] ]] && return 0
  [[ "$content" == *"releases/"* ]] && return 0
  [[ "$content" == *"What's in"* ]] && return 0
  [[ "$content" == *"VERSION="* ]] && return 0
  [[ "$content" =~ [Rr][Ee][Ll][Ee][Aa][Ss][Ee] ]] && return 0
  return 1
}

# Existing guard: README must not pin a release older than the workspace.
run_readme_check() {
  local tree="$1"
  local readme="$tree/README.md"
  local cargo="$tree/Cargo.toml"
  local current current_mm line lineno content version target
  local failures=0

  [[ -f "$readme" ]] || {
    echo "FAIL: missing $readme" >&2
    return 1
  }
  [[ -f "$cargo" ]] || {
    echo "FAIL: missing $cargo" >&2
    return 1
  }

  current="$(awk -F'"' '
    /^\[workspace\.package\][[:space:]]*$/ { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }
  ' "$cargo")"
  if [[ -z "$current" ]]; then
    echo "FAIL: no [workspace.package] version in $cargo" >&2
    return 1
  fi
  current_mm="${current%.*}"

  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    lineno="${line%%:*}"
    content="${line#*:}"
    is_release_line "$content" || continue
    while IFS= read -r version; do
      [[ -z "$version" ]] && continue
      if [[ "$version" =~ ^[0-9]+\.[0-9]+$ ]]; then
        target="$current_mm"
      else
        target="$current"
      fi
      if version_lt "$version" "$target"; then
        echo "FAIL: $readme:$lineno pins release version $version older than $current" >&2
        printf '  %s\n' "$content" >&2
        failures=$((failures + 1))
      fi
    done < <(grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' <<<"$content" || true)
  done < <(awk -v begin="$BLOCK_BEGIN" -v end="$BLOCK_END" '
    $0 == begin { skip = 1 }
    !skip { print NR ":" $0 }
    $0 == end { skip = 0 }
  ' "$readme" | grep -E '[0-9]+\.[0-9]+(\.[0-9]+)?' || true)

  if (( failures > 0 )); then
    echo "README_RELEASE_VERSION_GATE=FAIL current=$current stale_matches=$failures"
    return 1
  fi
  echo "README_RELEASE_VERSION_GATE=PASS current=$current"
  return 0
}

extract_block() {
  local file="$1"
  awk -v begin="$BLOCK_BEGIN" -v end="$BLOCK_END" '
    $0 == begin { inside = 1 }
    inside { print }
    $0 == end { exit }
  ' "$file"
}

render_block() {
  printf '%s\n' "$BLOCK_BEGIN"
  printf 'VERSION = %s\n' "$RS_VERSION"
  printf 'PUBLICATION_STATUS = %s\n' "$RS_PUBLICATION_STATUS"
  printf 'PREVIOUS_PUBLIC_VERSION = %s\n' "$RS_PREVIOUS_PUBLIC_VERSION"
  printf 'PUBLIC_RELEASE = v%s\n' "$RS_PREVIOUS_PUBLIC_VERSION"
  printf 'RELEASE_NOTE = %s\n' "$RS_RELEASE_NOTE"
  printf 'AUDIT_NOTE = %s\n' "$RS_AUDIT_NOTE"
  printf '%s\n' "$BLOCK_END"
}

read_release_status() {
  local tree="$1"
  local source="$tree/RELEASE_STATUS"
  local line key value
  local seen_version=0 seen_status=0 seen_previous=0 seen_note=0 seen_audit=0

  RS_VERSION=""
  RS_PUBLICATION_STATUS=""
  RS_PREVIOUS_PUBLIC_VERSION=""
  RS_RELEASE_NOTE=""
  RS_AUDIT_NOTE=""

  [[ -f "$source" ]] || {
    echo "FAIL: missing $source" >&2
    return 1
  }

  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    [[ -z "$(trim "$line")" ]] && continue
    [[ "$line" == *=* ]] || {
      echo "FAIL: malformed RELEASE_STATUS line: $line" >&2
      return 1
    }
    key="$(trim "${line%%=*}")"
    value="$(trim "${line#*=}")"
    case "$key" in
      VERSION) RS_VERSION="$value"; seen_version=1 ;;
      PUBLICATION_STATUS) RS_PUBLICATION_STATUS="$value"; seen_status=1 ;;
      PREVIOUS_PUBLIC_VERSION) RS_PREVIOUS_PUBLIC_VERSION="$value"; seen_previous=1 ;;
      RELEASE_NOTE) RS_RELEASE_NOTE="$value"; seen_note=1 ;;
      AUDIT_NOTE) RS_AUDIT_NOTE="$value"; seen_audit=1 ;;
      *)
        echo "FAIL: unknown RELEASE_STATUS key: $key" >&2
        return 1
        ;;
    esac
  done <"$source"

  (( seen_version && seen_status && seen_previous && seen_note && seen_audit )) || {
    echo "FAIL: RELEASE_STATUS is missing one of VERSION, PUBLICATION_STATUS, PREVIOUS_PUBLIC_VERSION, RELEASE_NOTE, AUDIT_NOTE" >&2
    return 1
  }
  [[ "$RS_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "FAIL: invalid VERSION in RELEASE_STATUS: $RS_VERSION" >&2
    return 1
  }
  [[ "$RS_PREVIOUS_PUBLIC_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "FAIL: invalid PREVIOUS_PUBLIC_VERSION in RELEASE_STATUS: $RS_PREVIOUS_PUBLIC_VERSION" >&2
    return 1
  }
  [[ "$RS_PUBLICATION_STATUS" == "NOT_PUBLISHED" || "$RS_PUBLICATION_STATUS" == "PUBLISHED" ]] || {
    echo "FAIL: PUBLICATION_STATUS must be PUBLISHED or NOT_PUBLISHED, got: $RS_PUBLICATION_STATUS" >&2
    return 1
  }
  [[ "$RS_RELEASE_NOTE" != /* && "$RS_RELEASE_NOTE" != *".."* ]] || {
    echo "FAIL: RELEASE_NOTE must be a relative path without ..: $RS_RELEASE_NOTE" >&2
    return 1
  }
  [[ "$RS_AUDIT_NOTE" != /* && "$RS_AUDIT_NOTE" != *".."* ]] || {
    echo "FAIL: AUDIT_NOTE must be a relative path without ..: $RS_AUDIT_NOTE" >&2
    return 1
  }
  [[ -f "$tree/$RS_RELEASE_NOTE" ]] || {
    echo "FAIL: missing release note: $RS_RELEASE_NOTE" >&2
    return 1
  }
  [[ -f "$tree/$RS_AUDIT_NOTE" ]] || {
    echo "FAIL: missing audit note: $RS_AUDIT_NOTE" >&2
    return 1
  }
  grep -qE "^VERSION[[:space:]]*=[[:space:]]*${RS_VERSION}([[:space:]]|$)" "$tree/$RS_RELEASE_NOTE" || {
    echo "FAIL: $RS_RELEASE_NOTE does not declare VERSION = $RS_VERSION" >&2
    return 1
  }
  version_lt "$RS_PREVIOUS_PUBLIC_VERSION" "$RS_VERSION" || {
    echo "FAIL: PREVIOUS_PUBLIC_VERSION $RS_PREVIOUS_PUBLIC_VERSION is not older than VERSION $RS_VERSION" >&2
    return 1
  }
  return 0
}

run_release_status() {
  local tree="$1"
  local cargo="$tree/Cargo.toml"
  local current block file extracted line lineno value failures=0
  local docs=("$tree/README.md" "$tree/SECURITY.md" "$tree/PROJECT_STATUS.md")

  read_release_status "$tree" || return 1
  block="$(render_block)"

  [[ -f "$cargo" ]] || {
    echo "FAIL: missing $cargo" >&2
    return 1
  }
  current="$(awk -F'"' '
    /^\[workspace\.package\][[:space:]]*$/ { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }
  ' "$cargo")"
  [[ "$current" == "$RS_VERSION" ]] || {
    echo "FAIL: RELEASE_STATUS VERSION=$RS_VERSION does not match Cargo.toml version=$current" >&2
    return 1
  }

  for file in "${docs[@]}"; do
    [[ -f "$file" ]] || {
      echo "FAIL: missing $file" >&2
      return 1
    }
    grep -Fq "$BLOCK_BEGIN" "$file" || {
      echo "FAIL: missing RELEASE_STATUS begin marker in $file" >&2
      return 1
    }
    grep -Fq "$BLOCK_END" "$file" || {
      echo "FAIL: missing RELEASE_STATUS end marker in $file" >&2
      return 1
    }
    extracted="$(extract_block "$file")"
    if [[ "$extracted" != "$block" ]]; then
      echo "FAIL: generated RELEASE_STATUS block is stale in $file" >&2
      diff -u <(printf '%s\n' "$block") <(printf '%s\n' "$extracted") >&2 || true
      failures=$((failures + 1))
    fi
  done

  # A canonical doc must not claim a different public release in an ad-hoc line.
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    if [[ "$line" =~ ^([^:]+):([0-9]+):[[:space:]]*(PUBLIC_RELEASE|CURRENT_RELEASE)[[:space:]]*=[[:space:]]*v?([0-9]+\.[0-9]+\.[0-9]+) ]]; then
      file="${BASH_REMATCH[1]}"
      lineno="${BASH_REMATCH[2]}"
      value="${BASH_REMATCH[4]}"
      if [[ "$value" != "$RS_PREVIOUS_PUBLIC_VERSION" ]]; then
        echo "FAIL: $file:$lineno claims public release $value, expected $RS_PREVIOUS_PUBLIC_VERSION" >&2
        failures=$((failures + 1))
      fi
    fi
  done < <(grep -HnE '^[[:space:]]*(PUBLIC_RELEASE|CURRENT_RELEASE)[[:space:]]*=' "${docs[@]}" 2>/dev/null || true)

  if (( failures > 0 )); then
    echo "RELEASE_STATUS_GATE=FAIL version=$RS_VERSION public=$RS_PREVIOUS_PUBLIC_VERSION failures=$failures"
    return 1
  fi
  echo "RELEASE_STATUS_GATE=PASS version=$RS_VERSION public=$RS_PREVIOUS_PUBLIC_VERSION status=$RS_PUBLICATION_STATUS"
  return 0
}

write_blocks() {
  local tree="$1"
  local block file tmp
  read_release_status "$tree" || return 1
  block="$(render_block)"

  for file in "$tree/README.md" "$tree/SECURITY.md" "$tree/PROJECT_STATUS.md"; do
    [[ -f "$file" ]] || {
      echo "FAIL: missing $file" >&2
      return 1
    }
    grep -Fq "$BLOCK_BEGIN" "$file" || {
      echo "FAIL: missing RELEASE_STATUS begin marker in $file" >&2
      return 1
    }
    tmp="$(mktemp "${TMPDIR:-/tmp}/exyonq-release-status.XXXXXX")"
    EXYONQ_RELEASE_BLOCK="$block" awk '
      BEGIN { block = ENVIRON["EXYONQ_RELEASE_BLOCK"] }
      /^<!-- BEGIN RELEASE_STATUS/ { print block; inside = 1; next }
      /^<!-- END RELEASE_STATUS/ { inside = 0; next }
      !inside { print }
    ' "$file" >"$tmp"
    mv "$tmp" "$file"
  done
  run_release_status "$tree"
}

selftest() {
  local tmp failures=0 dir
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-readme-release-gate.XXXXXX")"
  SELFTEST_TMP="$tmp"
  trap 'rm -rf "$SELFTEST_TMP"' EXIT

  make_case() {
    local name="$1"
    local body="$2"
    mkdir -p "$tmp/$name"
    printf '[workspace.package]\nversion = "0.9.1"\n' >"$tmp/$name/Cargo.toml"
    printf '%b\n' "$body" >"$tmp/$name/README.md"
  }

  make_case pass-current "## What's in the 0.9.x line\nreleases/latest\nPHP 8.5.12"
  make_case pass-php "Requires PHP 8.5.12"
  make_case pass-current-patch "Current release: 0.9.1"
  make_case fail-old-patch "Public product release: v0.8.0"
  make_case fail-image "docker pull ghcr.io/exyonq/exyonq:0.8.0"
  make_case fail-line "## What's in the 0.8.x line"
  make_case fail-previous-patch "Current release: 0.9.0"

  for dir in pass-current pass-php pass-current-patch; do
    if ! run_readme_check "$tmp/$dir" >"$tmp/$dir.out" 2>&1; then
      echo "SELFTEST_UNEXPECTED_FAIL: $dir"
      cat "$tmp/$dir.out" >&2
      failures=$((failures + 1))
    else
      echo "SELFTEST_OK_PASS: $dir"
    fi
  done

  for dir in fail-old-patch fail-image fail-line fail-previous-patch; do
    if run_readme_check "$tmp/$dir" >"$tmp/$dir.out" 2>&1; then
      echo "SELFTEST_UNEXPECTED_PASS: $dir"
      failures=$((failures + 1))
    else
      echo "SELFTEST_OK_FAILS: $dir"
    fi
  done

  # Release-status fixture: a real tree can be generated with --write and then
  # fails closed when one canonical block or the public release claim drifts.
  local rel="$tmp/release-status"
  mkdir -p "$rel/docs/release" "$rel/docs/security"
  printf '[workspace.package]\nversion = "0.9.2"\n' >"$rel/Cargo.toml"
  printf 'VERSION = 0.9.2\nPUBLICATION_STATUS = NOT_PUBLISHED\nPREVIOUS_PUBLIC_VERSION = 0.9.1\nRELEASE_NOTE = docs/release/v0.9.2.md\nAUDIT_NOTE = docs/security/audit-v0.9.2.md\n' >"$rel/RELEASE_STATUS"
  printf 'VERSION = 0.9.2\nPUBLICATION_STATUS = NOT_PUBLISHED\n' >"$rel/docs/release/v0.9.2.md"
  printf 'AUDIT\n' >"$rel/docs/security/audit-v0.9.2.md"
  for f in README.md SECURITY.md PROJECT_STATUS.md; do
    printf '# %s\n\n<!-- BEGIN RELEASE_STATUS (generated from RELEASE_STATUS; do not edit) -->\n<!-- END RELEASE_STATUS -->\n' "$f" >"$rel/$f"
  done
  if ! "$0" --write "$rel" >"$tmp/release-write.out" 2>&1; then
    echo "SELFTEST_UNEXPECTED_FAIL: release-status-write"
    cat "$tmp/release-write.out" >&2
    failures=$((failures + 1))
  elif ! run_release_status "$rel" >"$tmp/release-pass.out" 2>&1; then
    echo "SELFTEST_UNEXPECTED_FAIL: release-status-pass"
    cat "$tmp/release-pass.out" >&2
    failures=$((failures + 1))
  else
    echo "SELFTEST_OK_PASS: release-status-pass"
  fi
  printf 'PUBLIC_RELEASE = v0.9.0\n' >>"$rel/PROJECT_STATUS.md"
  if run_release_status "$rel" >"$tmp/release-fail.out" 2>&1; then
    echo "SELFTEST_UNEXPECTED_PASS: release-status-stale-public"
    failures=$((failures + 1))
  else
    echo "SELFTEST_OK_FAILS: release-status-stale-public"
  fi
  sed -i.bak 's/VERSION = 0.9.2/VERSION = 0.9.3/' "$rel/RELEASE_STATUS"
  rm -f "$rel/RELEASE_STATUS.bak"
  if run_release_status "$rel" >"$tmp/release-mismatch.out" 2>&1; then
    echo "SELFTEST_UNEXPECTED_PASS: release-status-cargo-mismatch"
    failures=$((failures + 1))
  else
    echo "SELFTEST_OK_FAILS: release-status-cargo-mismatch"
  fi

  if (( failures > 0 )); then
    echo "README_RELEASE_VERSION_GATE_SELFTEST=FAIL failures=$failures"
    return 1
  fi
  echo "README_RELEASE_VERSION_GATE_SELFTEST=PASS"
  return 0
}

MODE=""
TREE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --selftest)
      MODE=selftest
      shift
      ;;
    --tree)
      [[ $# -ge 2 ]] || { echo "ERROR: --tree requires a directory" >&2; exit 2; }
      MODE=tree
      TREE="$2"
      shift 2
      ;;
    --write)
      [[ $# -ge 2 ]] || { echo "ERROR: --write requires a directory" >&2; exit 2; }
      MODE=write
      TREE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$MODE" ]] || {
  echo "ERROR: mode required (--selftest|--tree DIR|--write DIR)" >&2
  exit 2
}

case "$MODE" in
  selftest)
    selftest
    ;;
  tree)
    [[ -d "$TREE" ]] || {
      echo "ERROR: not a directory: $TREE" >&2
      exit 2
    }
    run_readme_check "$TREE"
    run_release_status "$TREE"
    ;;
  write)
    [[ -d "$TREE" ]] || {
      echo "ERROR: not a directory: $TREE" >&2
      exit 2
    }
    write_blocks "$TREE"
    ;;
esac
